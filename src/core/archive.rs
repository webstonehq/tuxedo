//! The `done.txt` archive: completed tasks moved out of the live list.
//!
//! The generic sibling-file machinery (loading, refresh, atomic write) lives in
//! [`super::sidefile`]; this module holds only what is archive-specific.

use super::Store;
use super::outcome::{
    ArchiveDeleteOutcome, ArchiveOutcome, Reconcile, StoreError, UnarchiveOutcome,
};
use super::sidefile::{Refresh, SideFile};
use crate::todo::{self, Task};

/// The archive is a plain sibling file; the alias keeps callers and tests
/// reading as `Archive` rather than as the generic type.
pub type Archive = SideFile;

/// Default filename for the archive, used when no `DONE_FILE` overrides it.
pub(crate) const DONE_NAME: &str = "done.txt";

impl Store {
    /// Pump archive state (startup loader + external `done.txt` edits).
    pub fn poll_archive(&mut self) -> bool {
        let had_pending = self.history_pending.is_some();
        let changed = self.archive.poll();
        if had_pending {
            // The first read is not an external edit; it's what the deferred
            // history load has been waiting for.
            self.resolve_pending_history();
        } else if changed {
            self.note_archive_changed();
        }
        changed
    }

    pub fn archive_completed(&mut self) -> ArchiveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return ArchiveOutcome::Aborted(other),
        }
        let to_move: Vec<Task> = self.tasks.iter().filter(|t| t.done).cloned().collect();
        if to_move.is_empty() {
            return ArchiveOutcome::Nothing;
        }
        // Read fresh so an external edit to done.txt since startup isn't lost.
        let previous_archive_body = match self.archive.read_body() {
            Ok(b) => b,
            Err(e) => return ArchiveOutcome::Error(StoreError::ArchiveIo(e)),
        };
        let mut combined = previous_archive_body.clone();
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&todo::serialize(&to_move));
        // Snapshot before touching either file so one undo puts both back.
        self.push_history();
        // Write done.txt before truncating todo.txt so a failed archive can't
        // lose data; if the todo write fails, roll done.txt back.
        if let Err(e) = todo::write_atomic(&self.archive.path, &combined) {
            return ArchiveOutcome::Error(StoreError::ArchiveIo(e));
        }
        let remaining: Vec<Task> = self.tasks.iter().filter(|t| !t.done).cloned().collect();
        let remaining_body = todo::serialize(&remaining);
        if let Err(e) = todo::write_atomic(&self.file_path, &remaining_body) {
            let _ = todo::write_atomic(&self.archive.path, &previous_archive_body);
            return ArchiveOutcome::Error(StoreError::Write(e));
        }
        let count = to_move.len();
        self.tasks = remaining;
        self.last_disk = remaining_body;
        self.archive.tasks = todo::parse_file(&combined);
        self.archive.last_disk = combined;
        self.archive.loader = None;
        ArchiveOutcome::Archived { count }
    }

    /// Move an archived task back into the live list. `archive_idx` indexes
    /// `self.archive.tasks()`.
    pub fn unarchive(&mut self, archive_idx: usize) -> UnarchiveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return UnarchiveOutcome::Aborted(other),
        }
        match self.archive.refresh_for_mutation() {
            Refresh::Ready => {}
            Refresh::Reloaded => {
                self.note_archive_changed();
                return UnarchiveOutcome::DoneReloaded;
            }
            Refresh::Error(e) => return UnarchiveOutcome::Error(StoreError::ArchiveIo(e)),
        }
        if archive_idx >= self.archive.tasks.len() {
            return UnarchiveOutcome::OutOfRange;
        }
        let mut task = self.archive.tasks[archive_idx].clone();
        if let Err(e) = task.unmark_done() {
            return UnarchiveOutcome::Error(StoreError::Parse(e));
        }
        let previous_archive_body = self.archive.last_disk.clone();
        let new_archive: Vec<Task> = self
            .archive
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != archive_idx)
            .map(|(_, t)| t.clone())
            .collect();
        self.push_history();
        if let Err(e) = self.archive.write(new_archive) {
            return UnarchiveOutcome::Error(StoreError::ArchiveIo(e));
        }
        self.tasks.push(task);
        if let Err(e) = self.persist() {
            self.archive.rollback(&previous_archive_body);
            return UnarchiveOutcome::Error(e);
        }
        UnarchiveOutcome::Unarchived
    }

    /// Permanently remove an archived task from `done.txt`.
    pub fn archive_delete(&mut self, archive_idx: usize) -> ArchiveDeleteOutcome {
        match self.archive.refresh_for_mutation() {
            Refresh::Ready => {}
            Refresh::Reloaded => {
                self.note_archive_changed();
                return ArchiveDeleteOutcome::DoneReloaded;
            }
            Refresh::Error(e) => {
                return ArchiveDeleteOutcome::Error(StoreError::ArchiveIo(e));
            }
        }
        if archive_idx >= self.archive.tasks.len() {
            return ArchiveDeleteOutcome::OutOfRange;
        }
        let new_archive: Vec<Task> = self
            .archive
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != archive_idx)
            .map(|(_, t)| t.clone())
            .collect();
        self.push_history();
        if let Err(e) = self.archive.write(new_archive) {
            return ArchiveDeleteOutcome::Error(StoreError::ArchiveIo(e));
        }
        ArchiveDeleteOutcome::Deleted
    }

    pub(crate) fn persist(&mut self) -> Result<(), StoreError> {
        let body = todo::serialize(&self.tasks);
        match todo::write_atomic(&self.file_path, &body) {
            Ok(()) => {
                self.last_disk = body;
                Ok(())
            }
            Err(e) => Err(StoreError::Write(e)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::test_support::build_store;
    use std::time::{Duration, Instant};

    fn dir_for(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-archive-test-{}-{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn archive_writes_done_file_then_truncates_todo() {
        let dir = dir_for("ok");
        let todo_path = dir.join("todo.txt");
        let raw = "(A) 2026-05-01 keep this +work\n\
                   x 2026-05-05 2026-05-01 archive this +work\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut store = Store::open_sync(todo_path.clone(), raw.to_string(), "2026-05-06".into());
        assert!(matches!(
            store.archive_completed(),
            ArchiveOutcome::Archived { count: 1 }
        ));
        let done = std::fs::read_to_string(dir.join("done.txt")).unwrap();
        assert!(done.contains("archive this"));
        let todo = std::fs::read_to_string(&todo_path).unwrap();
        assert!(todo.contains("keep this"));
        assert!(!todo.contains("archive this"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_appends_to_existing_done_file() {
        let dir = dir_for("append");
        let todo_path = dir.join("todo.txt");
        std::fs::write(dir.join("done.txt"), "x 2026-04-01 2026-03-01 prior\n").unwrap();
        let raw = "x 2026-05-05 2026-05-01 fresh +work\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut store = Store::open_sync(todo_path, raw.to_string(), "2026-05-06".into());
        store.archive_completed();
        let done = std::fs::read_to_string(dir.join("done.txt")).unwrap();
        assert!(done.contains("prior"));
        assert!(done.contains("fresh"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_nothing_when_no_completed() {
        let mut store = build_store("a\nb\n");
        assert!(matches!(store.archive_completed(), ArchiveOutcome::Nothing));
    }

    fn wait_archive_loaded(store: &mut Store) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while store.archive.loader.is_some() && Instant::now() < deadline {
            let _ = store.poll_archive();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(store.archive.loader.is_none());
    }

    #[test]
    fn archive_loader_populates_archived_from_done_file() {
        let dir = dir_for("loader");
        let todo_path = dir.join("todo.txt");
        std::fs::write(
            dir.join("done.txt"),
            "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-15 second\n",
        )
        .unwrap();
        std::fs::write(&todo_path, "(A) 2026-05-06 still open\n").unwrap();
        let mut store = Store::new(
            todo_path,
            "(A) 2026-05-06 still open\n".to_string(),
            "2026-05-06".into(),
        );
        wait_archive_loaded(&mut store);
        assert_eq!(store.archive.len(), 2);
        assert!(
            store
                .archive
                .tasks()
                .iter()
                .any(|t| t.raw.contains("first"))
        );
        assert_eq!(store.tasks().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_completed_populates_in_memory_archived() {
        let dir = dir_for("memsync");
        let todo_path = dir.join("todo.txt");
        let raw = "x 2026-05-05 2026-05-01 done one\nx 2026-05-06 2026-05-01 done two\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut store = Store::new(todo_path, raw.to_string(), "2026-05-06".into());
        store.archive_completed();
        assert_eq!(store.archive.len(), 2);
        let _ = store.poll_archive();
        std::thread::sleep(Duration::from_millis(20));
        let _ = store.poll_archive();
        assert_eq!(store.archive.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_detects_external_done_edit() {
        let dir = dir_for("external");
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "(A) 2026-05-06 a\n").unwrap();
        std::fs::write(dir.join("done.txt"), "").unwrap();
        let mut store = Store::new(
            todo_path,
            "(A) 2026-05-06 a\n".to_string(),
            "2026-05-06".into(),
        );
        wait_archive_loaded(&mut store);
        assert!(store.archive.is_empty());
        std::fs::write(
            dir.join("done.txt"),
            "x 2026-05-05 2026-05-01 added externally\n",
        )
        .unwrap();
        assert!(store.poll_archive());
        assert_eq!(store.archive.len(), 1);
        assert!(store.archive.tasks()[0].raw.contains("added externally"));
        assert!(!store.poll_archive());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn poll_archive_preserves_archived_on_io_error() {
        let mut store = build_store("a\n");
        let path = store.archive.path().to_path_buf();
        store.archive = Archive::for_test(
            todo::parse_file("x 2026-05-01 2026-04-01 prior\n"),
            "x 2026-05-01 2026-04-01 prior\n".to_string(),
            path,
        );
        let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(!store.archive.apply_read(Err(err)));
        assert_eq!(store.archive.len(), 1);
    }

    #[test]
    fn archive_delete_refreshes_done_txt_before_writing() {
        let dir = dir_for("delete-refresh");
        let todo_path = dir.join("todo.txt");
        let done_path = dir.join("done.txt");
        std::fs::write(&todo_path, "open\n").unwrap();
        std::fs::write(&done_path, "x 2026-05-01 2026-04-01 stale\n").unwrap();
        let mut store = Store::new(todo_path, "open\n".to_string(), "2026-05-06".into());
        wait_archive_loaded(&mut store);
        std::fs::write(
            &done_path,
            "x 2026-05-01 2026-04-01 stale\nx 2026-05-02 2026-04-02 external\n",
        )
        .unwrap();
        assert!(matches!(
            store.archive_delete(0),
            ArchiveDeleteOutcome::DoneReloaded
        ));
        let done = std::fs::read_to_string(&done_path).unwrap();
        assert!(done.contains("stale"));
        assert!(done.contains("external"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unarchive_recomplete_does_not_duplicate_recurring_successor() {
        let dir = dir_for("rec-roundtrip");
        let todo_path = dir.join("todo.txt");
        let raw = "Water plants due:2026-05-06 rec:1d\n";
        std::fs::write(&todo_path, raw).unwrap();
        let mut store = Store::new(todo_path, raw.to_string(), "2026-05-06".into());
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        store.archive_completed();
        assert_eq!(store.tasks().len(), 1);
        assert_eq!(store.archive.len(), 1);
        store.unarchive(0);
        assert_eq!(store.tasks().len(), 2);
        let idx = store
            .tasks()
            .iter()
            .position(|t| !t.done && t.due.as_deref() == Some("2026-05-06"))
            .unwrap();
        store.toggle_complete(idx);
        assert_eq!(store.tasks().len(), 2);
        let next_count = store
            .tasks()
            .iter()
            .filter(|t| !t.done && t.due.as_deref() == Some("2026-05-07"))
            .count();
        assert_eq!(next_count, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_reports_write_failure() {
        let mut store = build_store("a\n");
        let missing_parent = std::env::temp_dir()
            .join(format!("tuxedo-missing-parent-{}", std::process::id()))
            .join("todo.txt");
        let _ = std::fs::remove_dir_all(missing_parent.parent().unwrap());
        store.file_path = missing_parent;
        assert!(store.persist().is_err());
    }
}
