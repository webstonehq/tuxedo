//! The `trash.txt` bin: deleted tasks, held for a second delete.
//!
//! `dd` in the list moves a task here rather than dropping it. Removing it for
//! good takes a second delete, from inside the Trash view. Lines are stored
//! byte-identical to how they appeared in `todo.txt` — no deletion-date tag is
//! stamped on them — so a restore is a perfect round trip and the file stays
//! hand-editable like every other file tuxedo writes.

use super::Store;
use super::outcome::{
    Reconcile, StoreError, TrashDeleteOutcome, TrashEmptyOutcome, TrashRestoreOutcome,
};
use super::sidefile::{Refresh, SideFile};
use crate::todo::Task;

pub type Trash = SideFile;

/// Default filename for the trash, used when no `TRASH_FILE` overrides it.
pub(crate) const TRASH_NAME: &str = "trash.txt";

impl Store {
    /// Pump trash state (external `trash.txt` edits). The trash has no startup
    /// loader — it's read synchronously — so this only picks up outside edits.
    pub fn poll_trash(&mut self) -> bool {
        let changed = self.trash.poll();
        if changed {
            self.note_side_changed();
        }
        changed
    }

    pub fn trash(&self) -> &SideFile {
        &self.trash
    }

    /// Append `tasks` to `trash.txt`, returning the body it had beforehand so
    /// the caller can roll back if its own write then fails.
    pub(crate) fn push_to_trash(&mut self, tasks: &[Task]) -> Result<String, StoreError> {
        let previous = self.trash.last_disk.clone();
        let mut combined = self.trash.tasks.clone();
        combined.extend(tasks.iter().cloned());
        self.trash
            .write(combined)
            .map_err(StoreError::TrashIo)
            .map(|()| previous)
    }

    /// Move a trashed task back into the live list, unchanged.
    pub fn trash_restore(&mut self, trash_idx: usize) -> TrashRestoreOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return TrashRestoreOutcome::Aborted(other),
        }
        match self.trash.refresh_for_mutation() {
            Refresh::Ready => {}
            Refresh::Reloaded => {
                self.note_side_changed();
                return TrashRestoreOutcome::TrashReloaded;
            }
            Refresh::Error(e) => return TrashRestoreOutcome::Error(StoreError::TrashIo(e)),
        }
        if trash_idx >= self.trash.tasks.len() {
            return TrashRestoreOutcome::OutOfRange;
        }
        let task = self.trash.tasks[trash_idx].clone();
        let previous_body = self.trash.last_disk.clone();
        let remaining: Vec<Task> = self
            .trash
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != trash_idx)
            .map(|(_, t)| t.clone())
            .collect();
        // Snapshot before either write so one undo puts both files back.
        self.push_history();
        if let Err(e) = self.trash.write(remaining) {
            return TrashRestoreOutcome::Error(StoreError::TrashIo(e));
        }
        self.tasks.push(task);
        if let Err(e) = self.persist() {
            self.trash.rollback(&previous_body);
            return TrashRestoreOutcome::Error(e);
        }
        TrashRestoreOutcome::Restored
    }

    /// The second delete: remove a trashed task for good.
    pub fn trash_delete(&mut self, trash_idx: usize) -> TrashDeleteOutcome {
        match self.trash.refresh_for_mutation() {
            Refresh::Ready => {}
            Refresh::Reloaded => {
                self.note_side_changed();
                return TrashDeleteOutcome::TrashReloaded;
            }
            Refresh::Error(e) => return TrashDeleteOutcome::Error(StoreError::TrashIo(e)),
        }
        if trash_idx >= self.trash.tasks.len() {
            return TrashDeleteOutcome::OutOfRange;
        }
        let remaining: Vec<Task> = self
            .trash
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != trash_idx)
            .map(|(_, t)| t.clone())
            .collect();
        self.push_history();
        if let Err(e) = self.trash.write(remaining) {
            return TrashDeleteOutcome::Error(StoreError::TrashIo(e));
        }
        TrashDeleteOutcome::Deleted
    }

    /// Empty the trash in one go.
    pub fn trash_empty(&mut self) -> TrashEmptyOutcome {
        match self.trash.refresh_for_mutation() {
            Refresh::Ready => {}
            Refresh::Reloaded => {
                self.note_side_changed();
                return TrashEmptyOutcome::TrashReloaded;
            }
            Refresh::Error(e) => return TrashEmptyOutcome::Error(StoreError::TrashIo(e)),
        }
        if self.trash.tasks.is_empty() {
            return TrashEmptyOutcome::Nothing;
        }
        let emptied = self.trash.tasks.len();
        self.push_history();
        if let Err(e) = self.trash.write(Vec::new()) {
            return TrashEmptyOutcome::Error(StoreError::TrashIo(e));
        }
        TrashEmptyOutcome::Emptied { emptied }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::outcome::{DeleteOutcome, UndoOutcome};
    use crate::core::test_support::build_store;
    use crate::core::{BulkDeleteOutcome, RedoOutcome};

    fn raws(tasks: &[Task]) -> Vec<String> {
        tasks.iter().map(|t| t.raw.clone()).collect()
    }

    fn trash_body(store: &Store) -> String {
        std::fs::read_to_string(store.trash.path()).unwrap_or_default()
    }

    fn todo_body(store: &Store) -> String {
        std::fs::read_to_string(&store.file_path).unwrap()
    }

    #[test]
    fn delete_moves_task_to_trash_file() {
        let mut store = build_store("(A) keep\n(B) toss\n");
        assert!(matches!(store.delete(1), DeleteOutcome::Deleted { .. }));
        assert_eq!(raws(store.tasks()), ["(A) keep"]);
        assert_eq!(todo_body(&store), "(A) keep\n");
        // Stored byte-identical, so a restore is a perfect round trip.
        assert_eq!(trash_body(&store), "(B) toss\n");
        assert_eq!(raws(store.trash().tasks()), ["(B) toss"]);
    }

    #[test]
    fn delete_then_undo_removes_from_trash() {
        // The whole reason snapshots cover all three lists: without it, undo
        // would restore the task to todo.txt and leave a copy in trash.txt.
        let mut store = build_store("(A) keep\n(B) toss\n");
        store.delete(1);
        assert_eq!(trash_body(&store), "(B) toss\n");

        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(raws(store.tasks()), ["(A) keep", "(B) toss"]);
        assert_eq!(todo_body(&store), "(A) keep\n(B) toss\n");
        assert_eq!(trash_body(&store), "", "undo must empty the trash again");
        assert!(store.trash().is_empty());

        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(store.tasks()), ["(A) keep"]);
        assert_eq!(trash_body(&store), "(B) toss\n");
    }

    #[test]
    fn trash_delete_is_permanent() {
        let mut store = build_store("(A) keep\n(B) toss\n");
        store.delete(1);
        assert!(matches!(store.trash_delete(0), TrashDeleteOutcome::Deleted));
        assert_eq!(trash_body(&store), "");
        // Gone from both files — the second delete is the real one.
        assert_eq!(todo_body(&store), "(A) keep\n");
    }

    #[test]
    fn trash_restore_round_trips() {
        let mut store = build_store("(A) keep\n(B) toss\n");
        store.delete(1);
        assert!(matches!(
            store.trash_restore(0),
            TrashRestoreOutcome::Restored
        ));
        assert_eq!(raws(store.tasks()), ["(A) keep", "(B) toss"]);
        assert_eq!(trash_body(&store), "");
    }

    #[test]
    fn trash_empty_clears_file_and_is_undoable() {
        let mut store = build_store("a\nb\nc\n");
        store.delete(2);
        store.delete(1);
        assert_eq!(store.trash().len(), 2);

        assert!(matches!(
            store.trash_empty(),
            TrashEmptyOutcome::Emptied { emptied: 2 }
        ));
        assert_eq!(trash_body(&store), "");
        assert!(matches!(store.trash_empty(), TrashEmptyOutcome::Nothing));

        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(store.trash().len(), 2);
    }

    #[test]
    fn delete_rolls_back_trash_when_todo_write_fails() {
        let mut store = build_store("(A) keep\n(B) toss\n");
        // Block the todo write by occupying its temp path with a directory.
        let tmp = store.file_path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        std::fs::create_dir(&tmp).unwrap();

        assert!(matches!(store.delete(1), DeleteOutcome::Error(_)));

        std::fs::remove_dir(&tmp).unwrap();
        // Neither file moved: the task is still live and not in the trash.
        assert_eq!(raws(store.tasks()), ["(A) keep", "(B) toss"]);
        assert_eq!(todo_body(&store), "(A) keep\n(B) toss\n");
        assert_eq!(trash_body(&store), "", "trash.txt must be rolled back");
        assert!(store.trash().is_empty());
    }

    #[test]
    fn delete_many_moves_all_to_trash_in_list_order() {
        let mut store = build_store("a\nb\nc\nd\n");
        assert!(matches!(
            store.delete_many(&[2, 0]),
            BulkDeleteOutcome::Done { deleted: 2 }
        ));
        assert_eq!(raws(store.tasks()), ["b", "d"]);
        assert_eq!(trash_body(&store), "a\nc\n");

        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(raws(store.tasks()), ["a", "b", "c", "d"]);
        assert_eq!(trash_body(&store), "");
    }

    #[test]
    fn snapshot_before_archive_loader_lands_does_not_wipe_done_txt() {
        // The archive loads off-thread. A mutation in that window would
        // otherwise snapshot an *empty* archive, and undoing it would write
        // that empty list over a real done.txt.
        let dir = std::env::temp_dir().join(format!("tuxedo-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "a\nb\n").unwrap();
        std::fs::write(dir.join("done.txt"), "x 2026-05-01 2026-04-01 precious\n").unwrap();

        // `Store::new` spawns the loader; mutate immediately, without polling.
        let mut store = Store::new(todo_path, "a\nb\n".to_string(), "2026-05-06".into());
        store.delete(1);
        assert!(matches!(store.undo(), UndoOutcome::Undone));

        assert_eq!(
            std::fs::read_to_string(dir.join("done.txt")).unwrap(),
            "x 2026-05-01 2026-04-01 precious\n",
            "undo must not clobber an archive it never read"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loader_landing_between_mutation_and_undo_does_not_wipe_done_txt() {
        // The dangerous interleaving: snapshot taken while the archive was
        // still unread, loader lands, then undo. Without forcing the read at
        // snapshot time, undo would write the empty list over done.txt.
        let dir = std::env::temp_dir().join(format!("tuxedo-race2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "a\nb\n").unwrap();
        std::fs::write(dir.join("done.txt"), "x 2026-05-01 2026-04-01 precious\n").unwrap();

        let mut store = Store::new(todo_path, "a\nb\n".to_string(), "2026-05-06".into());
        store.delete(1);
        // Now let the archive land, as the event loop's next tick would.
        for _ in 0..500 {
            let _ = store.poll_archive();
            if !store.archive.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!store.archive.is_empty(), "archive should have loaded");

        store.undo();
        assert_eq!(
            std::fs::read_to_string(dir.join("done.txt")).unwrap(),
            "x 2026-05-01 2026-04-01 precious\n",
            "undo must not clobber an archive the snapshot never saw"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn early_mutation_snapshots_the_real_archive() {
        // With history persistence on, an early mutation's snapshot is not
        // discarded by the loader landing, so it must carry the true archive.
        let dir = std::env::temp_dir().join(format!("tuxedo-race3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "a\nb\n").unwrap();
        std::fs::write(dir.join("done.txt"), "x 2026-05-01 2026-04-01 precious\n").unwrap();

        let mut store = Store::new(todo_path, "a\nb\n".to_string(), "2026-05-06".into());
        // Mutate before any poll: push_history must force the archive read.
        store.delete(1);
        assert_eq!(
            store.archive.len(),
            1,
            "push_history must have forced the archive load"
        );
        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(
            std::fs::read_to_string(dir.join("done.txt")).unwrap(),
            "x 2026-05-01 2026-04-01 precious\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_then_undo_removes_from_done() {
        // Pre-existing bug, fixed by snapshotting the archive alongside the
        // other two lists: undo used to leave the moved lines in done.txt.
        let mut store = build_store("x 2026-05-05 2026-05-01 finished\n(A) open\n");
        assert!(matches!(
            store.archive_completed(),
            crate::core::ArchiveOutcome::Archived { count: 1 }
        ));
        let done_path = store.archive.path().to_path_buf();
        assert!(
            std::fs::read_to_string(&done_path)
                .unwrap()
                .contains("finished")
        );

        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(raws(store.tasks()).len(), 2);
        assert_eq!(
            std::fs::read_to_string(&done_path).unwrap(),
            "",
            "undo must take the lines back out of done.txt"
        );
    }
}
