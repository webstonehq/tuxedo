use super::App;
use crate::{inbox, todo};

impl App {
    /// Reconcile the in-memory state against the file on disk. Returns true
    /// when it's safe to proceed with a mutation, false when the caller
    /// should abort. False indicates either an I/O error or that the file
    /// changed on disk — in which case the in-memory tasks have been
    /// replaced with the on-disk version, undo history is dropped (indices
    /// no longer line up), and a flash explains the reload. Mutators must
    /// call this before touching `tasks`.
    ///
    /// Also drains any sibling `inbox.txt` into `todo.txt`. The drain runs
    /// after the external-state reconciliation so a freshly-reloaded
    /// `last_disk` snapshot is in place before the merge writes.
    pub fn check_external_changes(&mut self) -> bool {
        let read = std::fs::read_to_string(&self.file_path);
        let safe = self.apply_external_state(read);
        // Drain regardless of `safe`: even when an external edit reloaded
        // the file, the inbox is still a valid source of new tasks and
        // its merge is independent of the in-memory mutation that the
        // caller was about to make.
        self.drain_inbox();
        safe
    }

    /// Merge any sibling `inbox.txt` (or recovered staging file from a
    /// previous interrupted drain) into `todo.txt`. Each line is run
    /// through the natural-language pipeline; invalid lines are skipped
    /// and counted in the flash message. Returns `true` when at least
    /// one task was merged (so the caller can mark the frame dirty).
    pub fn drain_inbox(&mut self) -> bool {
        let staging = inbox::staging_path_for(&self.file_path);
        let inbox_path = inbox::path_for(&self.file_path);

        // Coordinate with `tuxedo serve`'s POST handler (and other
        // tuxedo instances). The lock spans the rename + read + cleanup
        // so a producer can't strand its `O_APPEND` write on the
        // staging inode after we've consumed-and-deleted it.
        let _lock = match inbox::acquire_lock(&self.file_path) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
            Err(e) => {
                self.flash(format!("inbox lock failed: {e}"));
                return false;
            }
        };

        // Step 1: stage. If a staging file already exists (crash recovery
        // from a previous interrupted drain), use it as-is; otherwise
        // atomically rename inbox.txt → staging so concurrent appends go
        // to a fresh file. If neither exists, nothing to do.
        let staging_body = match std::fs::read_to_string(&staging) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::rename(&inbox_path, &staging) {
                    Ok(()) => match std::fs::read_to_string(&staging) {
                        Ok(body) => body,
                        Err(e) => {
                            self.flash(format!("inbox read failed: {e}"));
                            return false;
                        }
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
                    Err(e) => {
                        self.flash(format!("inbox stage failed: {e}"));
                        return false;
                    }
                }
            }
            Err(e) => {
                self.flash(format!("inbox read failed: {e}"));
                return false;
            }
        };

        // Step 2: parse each non-empty, non-comment line. Today is
        // re-parsed from the App's string snapshot so relative dates
        // ("tomorrow") resolve correctly at merge time.
        let today = match chrono::NaiveDate::parse_from_str(&self.today, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => {
                // App.today is set from chrono::Local at startup; a parse
                // failure means a test built the App with a bad value.
                // Bail without clobbering the inbox.
                self.flash("inbox: invalid today date");
                return false;
            }
        };
        let mut new_tasks: Vec<todo::Task> = Vec::new();
        let mut skipped = 0usize;
        for line in staging_body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            match inbox::canonicalize_line(trimmed, today) {
                Ok(task) => new_tasks.push(task),
                Err(_) => skipped += 1,
            }
        }
        // Nothing parseable in staging — drop it without recording an
        // undo entry or rewriting `todo.txt`. A skipped-only inbox
        // would otherwise leave a phantom undo step that visibly does
        // nothing when the user presses `u`.
        if new_tasks.is_empty() {
            if let Err(e) = std::fs::remove_file(&staging) {
                self.flash(format!("inbox cleanup failed: {e}"));
            } else if skipped > 0 {
                self.flash(format!("inbox: {skipped} unparseable, nothing merged"));
            }
            return false;
        }

        // Step 3: snapshot for undo, append, persist atomically.
        self.push_history();
        let merged = new_tasks.len();
        self.tasks.extend(new_tasks);
        let body = todo::serialize(&self.tasks);
        match todo::write_atomic(&self.file_path, &body) {
            Ok(()) => {
                self.last_disk = body;
            }
            Err(e) => {
                // Roll back the in-memory append since the write failed.
                // The staging file is left intact so the next drain can
                // retry — this is why we don't delete it on the error
                // path.
                self.tasks.truncate(self.tasks.len() - merged);
                self.history.pop();
                self.flash(format!("inbox write failed: {e}"));
                return false;
            }
        }

        // Step 4: only after the write succeeds, delete the staging
        // file. A crash between steps 3 and 4 means a subsequent drain
        // would re-merge the same lines (dup risk). We accept that
        // narrow window in exchange for never losing inbox content.
        if let Err(e) = std::fs::remove_file(&staging) {
            self.flash(format!("inbox cleanup failed: {e}"));
        } else if skipped > 0 {
            self.flash(format!("merged {merged} from inbox ({skipped} skipped)"));
        } else {
            self.flash(format!("merged {merged} from inbox"));
        }
        self.recompute_visible();
        self.clamp_cursor();
        true
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

    // ----- inbox drain tests ---------------------------------------------

    /// Build an App rooted in a fresh per-test directory so inbox.txt can
    /// be a real sibling file. Returns (app, dir, todo_path).
    fn build_app_with_dir(todo_raw: &str) -> (App, std::path::PathBuf, std::path::PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tuxedo-inbox-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, todo_raw).unwrap();
        let app = App::new(
            todo_path.clone(),
            todo_raw.to_string(),
            "2026-05-13".into(),
            Config::default(),
        );
        (app, dir, todo_path)
    }

    #[test]
    fn drain_merges_natural_language_lines() {
        let (mut app, dir, todo_path) = build_app_with_dir("(A) 2026-05-01 existing\n");
        std::fs::write(
            dir.join("inbox.txt"),
            "Buy milk tomorrow\nCall mom every friday\n",
        )
        .unwrap();
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 3, "two tasks merged onto existing");
        assert!(app.tasks[1].raw.contains("Buy milk"));
        assert_eq!(app.tasks[1].due.as_deref(), Some("2026-05-14"));
        assert!(app.tasks[2].rec.is_some(), "weekly recurrence parsed");
        // todo.txt was atomically rewritten and matches in-memory state.
        let on_disk = std::fs::read_to_string(&todo_path).unwrap();
        assert!(on_disk.contains("Buy milk"));
        assert!(on_disk.contains("Call mom"));
        // inbox.txt and the staging file are both gone.
        assert!(!dir.join("inbox.txt").exists());
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
        assert_eq!(app.flash_active(), Some("merged 2 from inbox"));
    }

    #[test]
    fn drain_with_no_inbox_is_noop() {
        let (mut app, _dir, _) = build_app_with_dir("a\n");
        assert!(!app.drain_inbox());
        assert_eq!(app.tasks.len(), 1);
        assert!(app.flash_active().is_none());
    }

    #[test]
    fn drain_skips_invalid_and_reports_count() {
        let (mut app, dir, _) = build_app_with_dir("a\n");
        // The second line is empty after trimming; the third is a
        // comment. Only "good line" should merge.
        std::fs::write(dir.join("inbox.txt"), "good line\n\n# this is a comment\n").unwrap();
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 2);
        assert!(app.tasks[1].raw.contains("good line"));
        // Empty/comment lines aren't counted as skipped — they're filtered
        // before canonicalize_line, so the flash reads cleanly.
        assert_eq!(app.flash_active(), Some("merged 1 from inbox"));
    }

    #[test]
    fn drain_recovers_existing_staging_file() {
        let (mut app, dir, _) = build_app_with_dir("a\n");
        // Simulate a crash mid-drain: staging file exists, no inbox.txt.
        std::fs::write(dir.join("inbox.txt.tuxedo-staging"), "recovered task\n").unwrap();
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 2);
        assert!(app.tasks[1].raw.contains("recovered task"));
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
    }

    #[test]
    fn drain_preserves_post_rename_appends() {
        // After drain renames inbox.txt → staging, any external append
        // is supposed to land in a fresh inbox.txt. We simulate that
        // by recreating inbox.txt between the rename and the merge.
        // The drain consumes the staged content; the freshly-created
        // inbox.txt survives for the next drain.
        let (mut app, dir, _) = build_app_with_dir("a\n");
        std::fs::write(dir.join("inbox.txt"), "first\n").unwrap();
        // Manually do step 1 of drain by renaming.
        std::fs::rename(dir.join("inbox.txt"), dir.join("inbox.txt.tuxedo-staging")).unwrap();
        // Now an external producer writes to a new inbox.txt.
        std::fs::write(dir.join("inbox.txt"), "second\n").unwrap();
        // Drain picks up only the staged file.
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 2);
        assert!(app.tasks[1].raw.contains("first"));
        // The fresh inbox.txt is still pending.
        assert_eq!(
            std::fs::read_to_string(dir.join("inbox.txt")).unwrap(),
            "second\n",
        );
        // A second drain merges it.
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 3);
        assert!(app.tasks[2].raw.contains("second"));
    }

    #[test]
    fn drain_is_undoable_as_single_batch() {
        let (mut app, dir, _) = build_app_with_dir("a\n");
        std::fs::write(dir.join("inbox.txt"), "one\ntwo\nthree\n").unwrap();
        assert!(app.drain_inbox());
        assert_eq!(app.tasks.len(), 4);
        app.undo();
        // The whole batch unwinds in a single undo step.
        assert_eq!(app.tasks.len(), 1);
        assert!(app.tasks[0].raw.contains('a'));
    }

    #[test]
    fn drain_with_only_blank_and_comment_lines_does_not_record_undo() {
        // An inbox containing only whitespace and `#` comments has no
        // parseable content. The drain should clean up staging without
        // calling `push_history` — otherwise pressing `u` after the
        // drain would unwind a phantom snapshot that visibly did
        // nothing.
        let (mut app, dir, todo_path) = build_app_with_dir("(A) 2026-05-01 a\n");
        // Mutate state first so undo has something real to revert to.
        app.toggle_complete(0);
        let toggled = app.tasks[0].done;
        let after_toggle_disk = std::fs::read_to_string(&todo_path).unwrap();
        std::fs::write(dir.join("inbox.txt"), "\n  \n# just a comment\n\n").unwrap();
        assert!(!app.drain_inbox(), "no-merge drain returns false");
        // Disk wasn't rewritten by the drain.
        assert_eq!(
            std::fs::read_to_string(&todo_path).unwrap(),
            after_toggle_disk,
        );
        // The inbox staging file is cleaned up either way.
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
        // Single `undo` unwinds the toggle directly — no phantom step
        // from the drain sitting on top of the history stack.
        app.undo();
        assert_ne!(app.tasks[0].done, toggled, "undo reverted the toggle");
    }
}
