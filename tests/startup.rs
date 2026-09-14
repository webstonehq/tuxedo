use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "tuxedo-startup-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create test directory");
        Self(path)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tuxedo"))
            .args(args)
            .current_dir(&self.0)
            .env("TODO_FILE", self.0.join("todo.txt"))
            .env("DONE_FILE", self.0.join("done.txt"))
            .env("TUXEDO_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::null())
            .output()
            .expect("run tuxedo")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn help_alias_works_without_a_terminal_or_creating_a_file() {
    let sandbox = Sandbox::new();
    let help = sandbox.run(&["help"]);
    let flag = sandbox.run(&["--help"]);
    assert!(help.status.success());
    assert!(flag.status.success());
    assert_eq!(help.stdout, flag.stdout);
    assert!(String::from_utf8_lossy(&help.stdout).contains("usage: tuxedo"));
    assert_eq!(std::fs::read_dir(&sandbox.0).unwrap().count(), 0);
}

#[test]
fn interactive_startup_without_terminal_exits_cleanly_before_creating_files() {
    let sandbox = Sandbox::new();
    for args in [&[][..], &["new-tasks.txt"][..], &["--sample"][..]] {
        let output = sandbox.run(args);
        assert_eq!(output.status.code(), Some(1), "args: {args:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("requires a terminal"), "{error}");
        assert!(!error.contains("panicked"), "{error}");
        assert_eq!(std::fs::read_dir(&sandbox.0).unwrap().count(), 0);
    }
}

#[test]
fn headless_list_keeps_working_and_preserves_tasks() {
    let sandbox = Sandbox::new();
    let tasks = "Check startup regression +tuxedo\n";
    std::fs::write(sandbox.0.join("todo.txt"), tasks).unwrap();
    let output = sandbox.run(&["ls"]);
    assert!(output.status.success(), "{:?}", output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Check startup regression"));
    assert_eq!(
        std::fs::read_to_string(sandbox.0.join("todo.txt")).unwrap(),
        tasks
    );
}
