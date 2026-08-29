use super::Store;
use super::outcome::{DrainReport, Reconcile};
use crate::hooks::HookEvent;
use crate::{inbox, todo};

impl Store {
    /// Reconcile in-memory tasks against the todo file on disk. Mutators call
    /// this first; `Reloaded`/`ReadError` mean they must abort.
    ///
    /// Unlike the old `App::check_external_changes`, this does **not** drain
    /// the inbox — draining is a separate, explicitly-invoked step (the TUI run
    /// loop and the CLI's mutating commands call [`Store::drain_inbox`]).
    pub fn reconcile(&mut self) -> Reconcile {
        let read = std::fs::read_to_string(&self.file_path);
        self.apply_external_state(read)
    }

    /// Decide what to do with a read result for the todo file. `NotFound`
    /// reloads as empty (the user may have deleted the file out from under us);
    /// any other I/O error preserves in-memory tasks and signals the caller to
    /// abort, since persisting on top of an unverified file could overwrite
    /// content we couldn't read.
    pub(crate) fn apply_external_state(&mut self, read: std::io::Result<String>) -> Reconcile {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return Reconcile::ReadError,
        };
        if on_disk == self.last_disk {
            return Reconcile::Unchanged;
        }
        self.tasks = todo::parse_file(&on_disk);
        self.last_disk = on_disk;
        self.history.clear();
        Reconcile::Reloaded
    }

    /// Merge any sibling `inbox.txt` (or recovered staging file from a previous
    /// interrupted drain) into `todo.txt`. Each line is run through the
    /// natural-language pipeline; invalid lines are skipped and counted. The
    /// returned [`DrainReport`] tells the caller what happened so it can render
    /// a message — the core never flashes.
    pub fn drain_inbox(&mut self) -> DrainReport {
        let staging = inbox::staging_path_for(&self.file_path);
        let inbox_path = inbox::path_for(&self.file_path);
        let err = |msg: String| DrainReport {
            error: Some(msg),
            ..Default::default()
        };

        // Fast path: if neither the inbox nor a recovered staging file
        // exists, there's nothing to drain — skip the lock entirely so the
        // common case (no capture activity) doesn't litter a lock file next
        // to todo.txt. A POST that creates inbox.txt between this check and
        // the next tick is benign: it takes the lock itself when appending,
        // and the next drain picks the line up.
        if !inbox_path.exists() && !staging.exists() {
            return DrainReport::default();
        }

        // Coordinate with `tuxedo serve`'s POST handler (and other tuxedo
        // instances). The lock spans the rename + read + cleanup.
        let _lock = match inbox::acquire_lock(&self.file_path) {
            Ok(l) => l,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DrainReport::default(),
            Err(e) => return err(format!("inbox lock failed: {e}")),
        };

        // Step 1: stage. Reuse an existing staging file (crash recovery);
        // otherwise atomically rename inbox.txt → staging so concurrent
        // appends go to a fresh file. If neither exists, nothing to do.
        let staging_body = match std::fs::read_to_string(&staging) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match std::fs::rename(&inbox_path, &staging) {
                    Ok(()) => match std::fs::read_to_string(&staging) {
                        Ok(body) => body,
                        Err(e) => return err(format!("inbox read failed: {e}")),
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return DrainReport::default();
                    }
                    Err(e) => return err(format!("inbox stage failed: {e}")),
                }
            }
            Err(e) => return err(format!("inbox read failed: {e}")),
        };

        // Step 2: parse each non-empty, non-comment line. Today is re-parsed
        // from the snapshot so relative dates resolve at merge time.
        let today = match chrono::NaiveDate::parse_from_str(&self.today, "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => return err("inbox: invalid today date".to_string()),
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

        // Nothing parseable — drop staging without recording undo.
        if new_tasks.is_empty() {
            if let Err(e) = std::fs::remove_file(&staging) {
                return err(format!("inbox cleanup failed: {e}"));
            }
            return DrainReport {
                merged: 0,
                skipped,
                error: None,
            };
        }

        // Step 3: snapshot for undo, append, persist atomically.
        self.push_history();
        let merged = new_tasks.len();
        self.tasks.extend(new_tasks);
        let body = todo::serialize(&self.tasks);
        match todo::write_atomic(&self.file_path, &body) {
            Ok(()) => {
                self.last_disk = body;
                self.post_commit(HookEvent::Create);
            }
            Err(e) => {
                // Roll back the in-memory append; leave staging for retry.
                self.tasks.truncate(self.tasks.len() - merged);
                self.history.pop();
                return err(format!("inbox write failed: {e}"));
            }
        }

        // Step 4: only after the write succeeds, delete staging.
        if let Err(e) = std::fs::remove_file(&staging) {
            return DrainReport {
                merged,
                skipped,
                error: Some(format!("inbox cleanup failed: {e}")),
            };
        }
        DrainReport {
            merged,
            skipped,
            error: None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::test_support::{build_store, test_path};
    use crate::hooks::{HookConfig, HookEvent};

    #[test]
    fn external_edit_reloads_and_aborts_mutation() {
        let path = test_path();
        std::fs::write(&path, "(A) 2026-05-01 a\n").unwrap();
        let mut store = Store::open_sync(
            path.clone(),
            "(A) 2026-05-01 a\n".to_string(),
            "2026-05-06".into(),
        );
        std::fs::write(&path, "(B) 2026-05-02 b\n(B) 2026-05-02 c\n").unwrap();
        // A delete that would otherwise remove the only task is aborted.
        assert!(matches!(
            store.delete(0),
            crate::core::DeleteOutcome::Aborted(Reconcile::Reloaded)
        ));
        assert_eq!(store.tasks().len(), 2);
        assert_eq!(store.tasks()[0].priority, Some('B'));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("(B) 2026-05-02 b"));
        assert!(on_disk.contains("(B) 2026-05-02 c"));
    }

    #[test]
    fn reconcile_reports_reloaded_then_unchanged() {
        let path = test_path();
        std::fs::write(&path, "a\nb\n").unwrap();
        let mut store = Store::open_sync(path.clone(), "a\nb\n".to_string(), "2026-05-06".into());
        std::fs::write(&path, "x\ny\nz\n").unwrap();
        assert_eq!(store.reconcile(), Reconcile::Reloaded);
        assert_eq!(store.tasks().len(), 3);
        assert_eq!(store.tasks()[0].raw, "x");
        assert_eq!(store.reconcile(), Reconcile::Unchanged);
    }

    #[test]
    fn external_edit_clears_undo_history() {
        let path = test_path();
        std::fs::write(&path, "a\nb\n").unwrap();
        let mut store = Store::open_sync(path.clone(), "a\nb\n".to_string(), "2026-05-06".into());
        store.delete(0);
        assert!(!store.history.is_empty());
        std::fs::write(&path, "x\ny\nz\n").unwrap();
        store.delete(0);
        assert!(store.history.is_empty());
    }

    #[test]
    fn apply_external_state_preserves_tasks_on_io_error() {
        let mut store = build_store("(A) 2026-05-01 keep me\n");
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert_eq!(store.apply_external_state(Err(err)), Reconcile::ReadError);
        assert_eq!(store.tasks().len(), 1);
        assert_eq!(store.tasks()[0].priority, Some('A'));
    }

    #[test]
    fn apply_external_state_treats_not_found_as_empty() {
        let mut store = build_store("(A) 2026-05-01 a\n");
        let err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        assert_eq!(store.apply_external_state(Err(err)), Reconcile::Reloaded);
        assert!(store.tasks().is_empty());
    }

    // ----- inbox drain tests ---------------------------------------------

    fn build_store_with_dir(todo_raw: &str) -> (Store, std::path::PathBuf, std::path::PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tuxedo-inbox-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, todo_raw).unwrap();
        let store = Store::open_sync(todo_path.clone(), todo_raw.to_string(), "2026-05-13".into());
        (store, dir, todo_path)
    }

    #[test]
    fn drain_merges_natural_language_lines() {
        let (mut store, dir, todo_path) = build_store_with_dir("(A) 2026-05-01 existing\n");
        std::fs::write(
            dir.join("inbox.txt"),
            "Buy milk tomorrow\nCall mom every friday\n",
        )
        .unwrap();
        let report = store.drain_inbox();
        assert_eq!(report.merged, 2);
        assert_eq!(report.skipped, 0);
        assert!(report.error.is_none());
        assert_eq!(store.tasks().len(), 3);
        assert!(store.tasks()[1].raw.contains("Buy milk"));
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-14"));
        assert!(store.tasks()[2].rec.is_some());
        let on_disk = std::fs::read_to_string(&todo_path).unwrap();
        assert!(on_disk.contains("Buy milk"));
        assert!(on_disk.contains("Call mom"));
        assert!(!dir.join("inbox.txt").exists());
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
    }

    #[test]
    fn drain_emits_one_create_hook_for_a_batch() {
        let (mut store, dir, _) = build_store_with_dir("existing\n");
        store.set_hooks(HookConfig {
            after_create: Some("/definitely/not/a/tuxedo-hook".into()),
            ..HookConfig::default()
        });
        std::fs::write(dir.join("inbox.txt"), "one\ntwo\n").unwrap();

        assert_eq!(store.drain_inbox().merged, 2);
        let reports = store.take_hook_reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].event, HookEvent::Create);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drain_with_no_inbox_is_noop() {
        let (mut store, _dir, _) = build_store_with_dir("a\n");
        let report = store.drain_inbox();
        assert!(report.is_noop());
        assert_eq!(store.tasks().len(), 1);
    }

    #[test]
    fn drain_with_no_inbox_does_not_create_lock_file() {
        let (mut store, dir, _) = build_store_with_dir("a\n");
        let report = store.drain_inbox();
        assert!(report.is_noop());
        assert!(
            !dir.join(inbox::LOCK_FILENAME).exists(),
            "drain with no inbox must not leave a lock file behind",
        );
    }

    #[test]
    fn drain_skips_invalid_and_reports_count() {
        let (mut store, dir, _) = build_store_with_dir("a\n");
        std::fs::write(dir.join("inbox.txt"), "good line\n\n# this is a comment\n").unwrap();
        let report = store.drain_inbox();
        assert_eq!(report.merged, 1);
        assert_eq!(report.skipped, 0);
        assert_eq!(store.tasks().len(), 2);
        assert!(store.tasks()[1].raw.contains("good line"));
    }

    #[test]
    fn drain_recovers_existing_staging_file() {
        let (mut store, dir, _) = build_store_with_dir("a\n");
        std::fs::write(dir.join("inbox.txt.tuxedo-staging"), "recovered task\n").unwrap();
        assert_eq!(store.drain_inbox().merged, 1);
        assert_eq!(store.tasks().len(), 2);
        assert!(store.tasks()[1].raw.contains("recovered task"));
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
    }

    #[test]
    fn drain_is_undoable_as_single_batch() {
        let (mut store, dir, _) = build_store_with_dir("a\n");
        std::fs::write(dir.join("inbox.txt"), "one\ntwo\nthree\n").unwrap();
        assert_eq!(store.drain_inbox().merged, 3);
        assert_eq!(store.tasks().len(), 4);
        store.undo();
        assert_eq!(store.tasks().len(), 1);
        assert!(store.tasks()[0].raw.contains('a'));
    }

    #[test]
    fn drain_with_only_blank_and_comment_lines_does_not_record_undo() {
        let (mut store, dir, todo_path) = build_store_with_dir("(A) 2026-05-01 a\n");
        store.toggle_complete(0);
        let toggled = store.tasks()[0].done;
        let after_toggle_disk = std::fs::read_to_string(&todo_path).unwrap();
        std::fs::write(dir.join("inbox.txt"), "\n  \n# just a comment\n\n").unwrap();
        let report = store.drain_inbox();
        assert_eq!(report.merged, 0);
        assert_eq!(
            std::fs::read_to_string(&todo_path).unwrap(),
            after_toggle_disk,
        );
        assert!(!dir.join("inbox.txt.tuxedo-staging").exists());
        store.undo();
        assert_ne!(store.tasks()[0].done, toggled);
    }
}
