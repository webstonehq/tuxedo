use super::App;
use crate::todo;

impl App {
    /// Reconcile the in-memory state against the file on disk. Returns true
    /// when it's safe to proceed with a mutation, false when the caller
    /// should abort. False indicates either an I/O error or that the file
    /// changed on disk — in which case the in-memory tasks have been
    /// replaced with the on-disk version, undo history is dropped (indices
    /// no longer line up), and a flash explains the reload. Mutators must
    /// call this before touching `tasks`.
    pub fn check_external_changes(&mut self) -> bool {
        let read = std::fs::read_to_string(&self.file_path);
        self.apply_external_state(read)
    }

    /// Decide what to do with a read result for the todo file. NotFound
    /// reloads as empty (the user may have deleted the file out from under
    /// us); any other I/O error preserves in-memory tasks and aborts the
    /// caller's mutation, since persisting on top of an unverified file
    /// could overwrite content we couldn't read.
    pub(super) fn apply_external_state(&mut self, read: std::io::Result<String>) -> bool {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                self.flash(format!("read failed: {}", e));
                return false;
            }
        };
        if on_disk == self.last_disk {
            return true;
        }
        self.tasks = todo::parse_file(&on_disk);
        self.last_disk = on_disk;
        self.history.clear();
        self.selection.clear();
        self.selection.exit_edit();
        self.recompute_visible();
        self.clamp_cursor();
        self.flash("file changed on disk — reloaded");
        false
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::app::test_support::{build_app, test_path};
    use crate::config::Config;

    #[test]
    fn external_edit_reloads_and_aborts_mutation() {
        let path = test_path();
        std::fs::write(&path, "(A) 2026-05-01 a\n").unwrap();
        let mut app = App::new(
            path.clone(),
            "(A) 2026-05-01 a\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        // Simulate an external editor rewriting the file.
        std::fs::write(&path, "(B) 2026-05-02 b\n(B) 2026-05-02 c\n").unwrap();
        // A mutation that would otherwise delete the only task...
        app.delete(0);
        // ...is aborted; instead the in-memory state mirrors the new disk.
        assert_eq!(app.tasks.len(), 2);
        assert_eq!(app.tasks[0].priority, Some('B'));
        assert_eq!(app.tasks[1].priority, Some('B'));
        assert!(app.flash_active().is_some());
        // Disk content was NOT overwritten with the stale tasks.
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("(B) 2026-05-02 b"));
        assert!(on_disk.contains("(B) 2026-05-02 c"));
    }

    #[test]
    fn check_external_changes_reloads_without_mutation() {
        // The TUI's run loop calls check_external_changes on idle ticks
        // and at the top of handle_key, so external edits are picked up
        // even when the user only navigates / sits idle.
        let path = test_path();
        std::fs::write(&path, "a\nb\n").unwrap();
        let mut app = App::new(
            path.clone(),
            "a\nb\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        std::fs::write(&path, "x\ny\nz\n").unwrap();
        // First call returns false (external change detected, state reloaded).
        assert!(!app.check_external_changes());
        assert_eq!(app.tasks.len(), 3);
        assert_eq!(app.tasks[0].raw, "x");
        assert!(app.flash_active().is_some());
        // Second call returns true: nothing changed since the reload.
        assert!(app.check_external_changes());
    }

    #[test]
    fn external_edit_clears_undo_history() {
        let path = test_path();
        std::fs::write(&path, "a\nb\n").unwrap();
        let mut app = App::new(
            path.clone(),
            "a\nb\n".to_string(),
            "2026-05-06".into(),
            Config::default(),
        );
        // Build up some history through a normal mutation.
        app.delete(0);
        assert!(!app.history.is_empty());
        // External edit invalidates the indices captured in history.
        std::fs::write(&path, "x\ny\nz\n").unwrap();
        app.delete(0);
        assert!(app.history.is_empty());
    }

    #[test]
    fn apply_external_state_preserves_tasks_on_io_error() {
        // A non-NotFound read error must not be silently treated as "file
        // is empty" — that path would replace the in-memory tasks and the
        // next persist() would overwrite the on-disk file with the empty
        // state. Instead: keep tasks, surface an error, abort the mutation.
        let mut app = build_app("(A) 2026-05-01 keep me\n");
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let ok = app.apply_external_state(Err(err));
        assert!(!ok, "I/O error must abort the in-progress mutation");
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].priority, Some('A'));
        assert!(app.flash_active().is_some(), "user must see the I/O error");
    }

    #[test]
    fn apply_external_state_treats_not_found_as_empty() {
        // If the file genuinely disappears, treat it the same as an empty
        // file — the user may have deleted it externally, and the previous
        // implementation already had that behavior.
        let mut app = build_app("(A) 2026-05-01 a\n");
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let ok = app.apply_external_state(Err(err));
        assert!(!ok);
        assert!(app.tasks.is_empty());
    }
}
