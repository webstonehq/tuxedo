//! Post-commit hook configuration and shell-free process execution.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A task mutation that can trigger a configured post-commit hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Create,
    Update,
    Complete,
    Archive,
    Delete,
    Uncomplete,
    Undo,
    Unarchive,
}

impl HookEvent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Complete => "complete",
            Self::Archive => "archive",
            Self::Delete => "delete",
            Self::Uncomplete => "uncomplete",
            Self::Undo => "undo",
            Self::Unarchive => "unarchive",
        }
    }
}

/// Optional executable paths for each supported post-commit event.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HookConfig {
    pub after_mutation: Option<PathBuf>,
    pub after_create: Option<PathBuf>,
    pub after_update: Option<PathBuf>,
    pub after_complete: Option<PathBuf>,
    pub after_archive: Option<PathBuf>,
}

impl HookConfig {
    pub fn script_for(&self, event: HookEvent) -> Option<&Path> {
        self.after_mutation.as_deref().or(match event {
            HookEvent::Create => self.after_create.as_deref(),
            HookEvent::Update => self.after_update.as_deref(),
            HookEvent::Complete => self.after_complete.as_deref(),
            HookEvent::Archive => self.after_archive.as_deref(),
            HookEvent::Delete | HookEvent::Uncomplete | HookEvent::Undo | HookEvent::Unarchive => {
                None
            }
        })
    }
}

/// Paths exported to the hook process. All values must be absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookContext {
    pub root: PathBuf,
    pub todo_file: PathBuf,
    pub done_file: PathBuf,
}

/// Outcome of executing a configured hook. Child output is deliberately not
/// retained or rendered: scripts cannot contaminate the TUI or CLI JSON stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookExecution {
    Succeeded,
    InvalidPath,
    SpawnFailed,
    Exited(i32),
    Signaled,
}

/// Follow-up disk synchronization result after running a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookRefresh {
    Succeeded,
    Failed,
}

/// A presentation-free result queued by [`crate::core::Store`] after a hook
/// attempt and its required disk refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookReport {
    pub event: HookEvent,
    pub script: PathBuf,
    pub execution: HookExecution,
    pub refresh: HookRefresh,
}

impl HookReport {
    pub fn failed(&self) -> bool {
        self.execution != HookExecution::Succeeded || self.refresh != HookRefresh::Succeeded
    }

    /// Stable, child-output-free diagnostic for TUI flashes and CLI stderr.
    pub fn diagnostic(&self) -> String {
        let event = self.event.as_str();
        let script = self.script.display();
        let mut message = match self.execution {
            HookExecution::Succeeded => String::new(),
            HookExecution::InvalidPath => {
                format!("post-commit hook {event} failed: {script} is not an absolute path")
            }
            HookExecution::SpawnFailed => {
                format!("post-commit hook {event} failed: could not start {script}")
            }
            HookExecution::Exited(code) => {
                format!("post-commit hook {event} failed: {script} exited with status {code}")
            }
            HookExecution::Signaled => {
                format!("post-commit hook {event} failed: {script} terminated by signal")
            }
        };
        if self.refresh == HookRefresh::Failed {
            if !message.is_empty() {
                message.push_str("; ");
            }
            message.push_str(&format!(
                "post-commit hook {event} post-hook synchronization failed for {script}"
            ));
        }
        message
    }
}

/// Execute one configured hook without invoking a shell or passing task data.
pub fn run(event: HookEvent, script: &Path, context: &HookContext) -> HookReport {
    let execution = if !script.is_absolute() {
        HookExecution::InvalidPath
    } else {
        match Command::new(script)
            .current_dir(&context.root)
            .env("TUXEDO_HOOK_EVENT", event.as_str())
            .env("TUXEDO_ROOT", &context.root)
            .env("TUXEDO_TODO_FILE", &context.todo_file)
            .env("TUXEDO_DONE_FILE", &context.done_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) if output.status.success() => HookExecution::Succeeded,
            Ok(output) => match output.status.code() {
                Some(code) => HookExecution::Exited(code),
                None => HookExecution::Signaled,
            },
            Err(_) => HookExecution::SpawnFailed,
        }
    };
    HookReport {
        event,
        script: script.to_path_buf(),
        execution,
        // Store fills this after reloading both task files.
        refresh: HookRefresh::Succeeded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_scripts_are_reported_without_execution() {
        let context = HookContext {
            root: PathBuf::from("/tmp"),
            todo_file: PathBuf::from("/tmp/todo.txt"),
            done_file: PathBuf::from("/tmp/done.txt"),
        };
        let report = run(HookEvent::Create, Path::new("hooks/lint"), &context);
        assert_eq!(report.execution, HookExecution::InvalidPath);
        assert!(report.failed());
        assert!(report.diagnostic().contains("not an absolute path"));
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_exit_is_reported_without_child_output() {
        let context = HookContext {
            root: PathBuf::from("/tmp"),
            todo_file: PathBuf::from("/tmp/todo.txt"),
            done_file: PathBuf::from("/tmp/done.txt"),
        };
        let report = run(HookEvent::Update, Path::new("/bin/false"), &context);
        assert_eq!(report.execution, HookExecution::Exited(1));
        assert!(report.diagnostic().contains("exited with status 1"));
    }

    #[cfg(unix)]
    #[test]
    fn runner_sets_only_the_documented_context() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "tuxedo-hook-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test directory");
        let script = dir.join("hook");
        let output = dir.join("context");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s\\n%s\\n%s\\n%s\\n%s\\n' \"$PWD\" \"$TUXEDO_HOOK_EVENT\" \"$TUXEDO_ROOT\" \"$TUXEDO_TODO_FILE\" \"$TUXEDO_DONE_FILE\" > context\n",
        )
        .expect("write script");
        let mut permissions = std::fs::metadata(&script)
            .expect("script metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).expect("make script executable");
        let context = HookContext {
            root: dir.clone(),
            todo_file: dir.join("todo.txt"),
            done_file: dir.join("done.txt"),
        };

        let report = run(HookEvent::Archive, &script, &context);
        assert_eq!(report.execution, HookExecution::Succeeded);
        assert_eq!(
            std::fs::read_to_string(output).expect("read hook context"),
            format!(
                "{}\narchive\n{}\n{}\n{}\n",
                dir.display(),
                dir.display(),
                context.todo_file.display(),
                context.done_file.display(),
            )
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
