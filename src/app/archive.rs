use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use super::App;
use crate::todo::{self, Task};

/// Owns the archived (`done.txt`) tasks and the lifecycle around loading them
/// off-thread at startup. Fields are `pub(crate)` so the methods on `App` in
/// this file can mutate the archive directly; external callers go through the
/// read methods.
pub struct Archive {
    pub(crate) tasks: Vec<Task>,
    pub(crate) path: PathBuf,
    pub(crate) last_disk: String,
    pub(crate) loader: Option<Receiver<(String, Vec<Task>)>>,
}

impl Archive {
    /// Construct an `Archive` for a sibling `done.txt` next to the given todo
    /// file, and spawn a worker thread to read+parse it. The first frame can
    /// render `todo.txt` immediately while the loader runs in the background.
    pub fn spawn(todo_path: &Path) -> Self {
        let path = todo_path
            .parent()
            .map(|p| p.join("done.txt"))
            .unwrap_or_else(|| PathBuf::from("done.txt"));
        let loader_path = path.clone();
        let (tx, rx) = mpsc::sync_channel::<(String, Vec<Task>)>(1);
        thread::spawn(move || {
            let body = std::fs::read_to_string(&loader_path).unwrap_or_default();
            let parsed = todo::parse_file(&body);
            let _ = tx.send((body, parsed));
        });
        Self {
            tasks: Vec::new(),
            path,
            last_disk: String::new(),
            loader: Some(rx),
        }
    }

    /// Test-only constructor that skips the worker thread and seeds in-memory
    /// state directly.
    #[cfg(test)]
    pub(crate) fn for_test(tasks: Vec<Task>, last_disk: String, path: PathBuf) -> Self {
        Self {
            tasks,
            path,
            last_disk,
            loader: None,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

impl App {
    fn read_archive_body(&mut self, action: &str) -> Option<String> {
        match std::fs::read_to_string(&self.archive.path) {
            Ok(body) => Some(body),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Some(String::new()),
            Err(e) => {
                self.flash(format!("{action} failed: done.txt read failed: {e}"));
                None
            }
        }
    }

    fn refresh_archive_for_mutation(&mut self, action: &str) -> bool {
        let Some(body) = self.read_archive_body(action) else {
            return false;
        };
        if body != self.archive.last_disk {
            self.archive.tasks = todo::parse_file(&body);
            self.archive.last_disk = body;
            if matches!(self.view, super::View::Archive) {
                self.recompute_visible();
                self.clamp_cursor();
            }
            self.flash("done.txt changed on disk — reloaded");
            self.archive.loader = None;
            return false;
        }
        self.archive.loader = None;
        true
    }

    /// Pump archive state. Returns true when the visible archive changed:
    /// the startup loader landed, or an external edit to `done.txt` was
    /// picked up. Non-blocking; the run loop calls this each iteration so
    /// idle ticks pick up edits the same way `check_external_changes` does
    /// for `todo.txt`.
    pub fn poll_archive(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.archive.loader {
            match rx.try_recv() {
                Ok((body, tasks)) => {
                    self.archive.last_disk = body;
                    self.archive.tasks = tasks;
                    self.archive.loader = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => return false,
                // Defensive: loader thread dropped its sender without
                // sending. Fall through and treat `done.txt` as if it
                // hadn't been loaded yet so the disk check still runs.
                Err(TryRecvError::Disconnected) => {
                    self.archive.loader = None;
                }
            }
        }
        if !changed {
            let read = std::fs::read_to_string(&self.archive.path);
            changed = self.apply_archive_read(read);
        }
        if changed && matches!(self.view, super::View::Archive) {
            // The archive feeds visible_cache in Archive view; refresh so
            // the next frame mirrors the new archive contents.
            self.recompute_visible();
            self.clamp_cursor();
        }
        changed
    }

    /// Apply a read result for `done.txt`. NotFound is treated as an empty
    /// archive (so users can delete done.txt to start fresh); any other I/O
    /// error preserves in-memory state and surfaces a flash, rather than
    /// silently wiping the archive view.
    pub(super) fn apply_archive_read(&mut self, read: std::io::Result<String>) -> bool {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.flash(format!("done.txt read failed: {}", e));
                return false;
            }
        };
        if on_disk == self.archive.last_disk {
            return false;
        }
        self.archive.tasks = todo::parse_file(&on_disk);
        self.archive.last_disk = on_disk;
        true
    }

    pub fn archive_completed(&mut self) {
        if !self.check_external_changes() {
            return;
        }
        let to_move: Vec<Task> = self.tasks.iter().filter(|t| t.done).cloned().collect();
        if to_move.is_empty() {
            self.flash("nothing to archive");
            return;
        }
        // Build new done.txt content (existing + appended). Read fresh
        // from disk so an external edit since startup isn't clobbered.
        let Some(previous_archive_body) = self.read_archive_body("archive") else {
            return;
        };
        let mut combined = previous_archive_body.clone();
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&todo::serialize(&to_move));
        // Write done.txt *before* removing tasks from todo.txt so a failed
        // archive can't lose data. If the todo write then fails, best-effort
        // roll done.txt back so the two files do not keep duplicate tasks.
        if let Err(e) = todo::write_atomic(&self.archive.path, &combined) {
            self.flash(format!("archive failed: {}", e));
            return;
        }
        let remaining: Vec<Task> = self.tasks.iter().filter(|t| !t.done).cloned().collect();
        let remaining_body = todo::serialize(&remaining);
        if let Err(e) = todo::write_atomic(&self.file_path, &remaining_body) {
            let rollback = todo::write_atomic(&self.archive.path, &previous_archive_body);
            if let Err(rollback_err) = rollback {
                self.flash(format!(
                    "archive failed: todo write failed: {e}; rollback failed: {rollback_err}"
                ));
            } else {
                self.flash(format!("archive failed: todo write failed: {e}"));
            }
            return;
        }
        self.push_history();
        self.tasks = remaining;
        self.last_disk = remaining_body;
        self.archive.tasks = todo::parse_file(&combined);
        self.archive.last_disk = combined;
        self.archive.loader = None;
        self.flash(format!("archived {}", to_move.len()));
        self.recompute_visible();
        self.clamp_cursor();
    }

    /// Move an archived task back into the live list. `archive_idx` is an
    /// index into `self.archive.tasks()` (the cursor source in Archive view).
    pub fn unarchive(&mut self, archive_idx: usize) {
        if !self.check_external_changes() {
            return;
        }
        if !self.refresh_archive_for_mutation("unarchive") {
            return;
        }
        if archive_idx >= self.archive.tasks.len() {
            return;
        }
        let mut task = self.archive.tasks[archive_idx].clone();
        if let Err(e) = task.unmark_done() {
            self.flash(format!("unarchive failed: {e}"));
            return;
        }
        // Persist done.txt without the moved row first; if that fails, abort
        // before touching todo.txt so we never duplicate the row.
        let new_archive: Vec<Task> = self
            .archive
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if i != archive_idx {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        let archive_body = todo::serialize(&new_archive);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            self.flash(format!("unarchive failed: {e}"));
            return;
        }
        self.archive.tasks = new_archive;
        self.archive.last_disk = archive_body;
        self.push_history();
        self.tasks.push(task);
        if self.persist() {
            self.flash("unarchived");
        }
        self.recompute_visible();
        self.clamp_cursor();
    }

    /// Permanently remove an archived task from `done.txt`.
    pub fn archive_delete(&mut self, archive_idx: usize) {
        if !self.refresh_archive_for_mutation("delete") {
            return;
        }
        if archive_idx >= self.archive.tasks.len() {
            return;
        }
        let new_archive: Vec<Task> = self
            .archive
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                if i != archive_idx {
                    Some(t.clone())
                } else {
                    None
                }
            })
            .collect();
        let archive_body = todo::serialize(&new_archive);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            self.flash(format!("delete failed: {e}"));
            return;
        }
        self.archive.tasks = new_archive;
        self.archive.last_disk = archive_body;
        self.flash("deleted from archive");
        self.recompute_visible();
        self.clamp_cursor();
    }

    pub fn persist(&mut self) -> bool {
        let body = todo::serialize(&self.tasks);
        match todo::write_atomic(&self.file_path, &body) {
            Ok(()) => {
                self.last_disk = body;
                true
            }
            Err(e) => {
                self.flash(format!("write failed: {e}"));
                false
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;
    use crate::config::Config;
    use std::time::{Duration, Instant};

    #[test]
    fn archive_writes_done_file_then_truncates_todo() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "ok"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        let raw = "(A) 2026-05-01 keep this +work\n\
                   x 2026-05-05 2026-05-01 archive this +work\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut app = App::new(
            todo_path.clone(),
            raw.to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        app.archive_completed();
        let done = std::fs::read_to_string(dir.join("done.txt"))
            .expect("done.txt should be written before todo.txt is truncated");
        assert!(done.contains("archive this"));
        let todo = std::fs::read_to_string(&todo_path).expect("todo.txt should still exist");
        assert!(todo.contains("keep this"));
        assert!(!todo.contains("archive this"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_appends_to_existing_done_file() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "append"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(dir.join("done.txt"), "x 2026-04-01 2026-03-01 prior\n").unwrap();
        let raw = "x 2026-05-05 2026-05-01 fresh +work\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut app = App::new(
            todo_path,
            raw.to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        app.archive_completed();
        let done = std::fs::read_to_string(dir.join("done.txt")).unwrap();
        assert!(done.contains("prior"));
        assert!(done.contains("fresh"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Spin until the startup `done.txt` loader thread sends its result.
    /// The Archive UI reads `app.archived` directly, so a test that wants
    /// to assert post-load state needs to wait the loader out.
    fn wait_archive_loaded(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.archive.loader.is_some() && Instant::now() < deadline {
            let _ = app.poll_archive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            app.archive.loader.is_none(),
            "archive loader did not complete in time"
        );
    }

    #[test]
    fn archive_loader_populates_archived_from_done_file() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "loader"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(
            dir.join("done.txt"),
            "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-15 second\n",
        )
        .unwrap();
        std::fs::write(&todo_path, "(A) 2026-05-06 still open\n").unwrap();
        let mut app = App::new(
            todo_path,
            "(A) 2026-05-06 still open\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        wait_archive_loaded(&mut app);
        assert_eq!(app.archive.len(), 2);
        assert!(app.archive.tasks().iter().any(|t| t.raw.contains("first")));
        assert!(app.archive.tasks().iter().any(|t| t.raw.contains("second")));
        // todo.txt remains untouched and visible.
        assert_eq!(app.tasks.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_completed_populates_in_memory_archived() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "memsync"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        let raw = "x 2026-05-05 2026-05-01 done one\nx 2026-05-06 2026-05-01 done two\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut app = App::new(
            todo_path,
            raw.to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        // Don't wait for the loader: archive_completed must work whether or
        // not the startup loader has landed yet.
        app.archive_completed();
        assert_eq!(app.archive.len(), 2);
        // A subsequent poll_archive must not undo the in-memory state when
        // the loader's stale result eventually arrives.
        let _ = app.poll_archive();
        std::thread::sleep(Duration::from_millis(20));
        let _ = app.poll_archive();
        assert_eq!(app.archive.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_detects_external_done_edit() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "external"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "(A) 2026-05-06 a\n").unwrap();
        // Empty done.txt at startup so the loader returns an empty archive.
        std::fs::write(dir.join("done.txt"), "").unwrap();
        let mut app = App::new(
            todo_path,
            "(A) 2026-05-06 a\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        wait_archive_loaded(&mut app);
        assert!(app.archive.is_empty());
        // Simulate an external edit to done.txt.
        std::fs::write(
            dir.join("done.txt"),
            "x 2026-05-05 2026-05-01 added externally\n",
        )
        .unwrap();
        assert!(app.poll_archive());
        assert_eq!(app.archive.len(), 1);
        assert!(app.archive.tasks()[0].raw.contains("added externally"));
        // No further changes → poll returns false.
        assert!(!app.poll_archive());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_preserves_archived_on_io_error() {
        let mut app = build_app("a\n");
        // Seed a known archive state and then verify a read error doesn't
        // wipe it out.
        let path = app.archive.path().to_path_buf();
        app.archive = Archive::for_test(
            todo::parse_file("x 2026-05-01 2026-04-01 prior\n"),
            "x 2026-05-01 2026-04-01 prior\n".to_string(),
            path,
        );
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let changed = app.apply_archive_read(Err(err));
        assert!(!changed);
        assert_eq!(app.archive.len(), 1);
        assert!(app.flash_active().is_some());
    }

    #[test]
    fn archive_delete_refreshes_done_txt_before_writing() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            "delete-refresh"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        let done_path = dir.join("done.txt");
        std::fs::write(&todo_path, "open\n").unwrap();
        std::fs::write(&done_path, "x 2026-05-01 2026-04-01 stale\n").unwrap();
        let mut app = App::new(
            todo_path,
            "open\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        wait_archive_loaded(&mut app);

        std::fs::write(
            &done_path,
            "x 2026-05-01 2026-04-01 stale\nx 2026-05-02 2026-04-02 external\n",
        )
        .unwrap();
        app.archive_delete(0);

        let done = std::fs::read_to_string(&done_path).unwrap();
        assert!(done.contains("stale"));
        assert!(done.contains("external"));
        assert!(app.flash_active().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_reports_write_failure() {
        let mut app = build_app("a\n");
        let missing_parent = std::env::temp_dir()
            .join(format!("tuxedo-missing-parent-{}", std::process::id()))
            .join("todo.txt");
        let _ = std::fs::remove_dir_all(missing_parent.parent().unwrap());
        app.file_path = missing_parent;

        assert!(!app.persist());
        assert!(app.flash_active().is_some());
    }
}
