use std::collections::VecDeque;

use super::Store;
use super::outcome::{Reconcile, RedoOutcome, UndoOutcome};
use crate::app::UNDO_LIMIT;
use crate::todo::Task;

/// Two snapshot stacks. `undo` holds pre-mutation states, newest at the back;
/// `redo` holds states that were undone away, newest at the back. Both are
/// capped at [`UNDO_LIMIT`] and evict oldest-first.
#[derive(Debug, Default, Clone)]
pub struct History {
    undo: VecDeque<Vec<Task>>,
    redo: VecDeque<Vec<Task>>,
}

impl History {
    /// Record a pre-mutation snapshot. A fresh edit invalidates the redo
    /// branch — you can't redo past a divergence.
    pub fn push(&mut self, snapshot: Vec<Task>) {
        self.push_undo_raw(snapshot);
        self.redo.clear();
    }

    /// Push onto the undo stack *without* discarding the redo branch. Used by
    /// rollback paths and by `redo` itself, where the redo stack is being
    /// walked rather than invalidated.
    fn push_undo_raw(&mut self, snapshot: Vec<Task>) {
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(snapshot);
    }

    fn push_redo(&mut self, snapshot: Vec<Task>) {
        if self.redo.len() >= UNDO_LIMIT {
            self.redo.pop_front();
        }
        self.redo.push_back(snapshot);
    }

    pub fn pop(&mut self) -> Option<Vec<Task>> {
        self.undo.pop_back()
    }

    fn pop_redo(&mut self) -> Option<Vec<Task>> {
        self.redo.pop_back()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    pub fn len(&self) -> usize {
        self.undo.len()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

impl Store {
    pub(crate) fn push_history(&mut self) {
        self.history.push(self.tasks.clone());
    }

    pub fn undo(&mut self) -> UndoOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return UndoOutcome::Aborted(other),
        }
        match self.history.pop() {
            Some(prev) => {
                let current = std::mem::replace(&mut self.tasks, prev);
                match self.persist() {
                    Ok(()) => {
                        self.history.push_redo(current);
                        UndoOutcome::Undone
                    }
                    Err(e) => {
                        let prev = std::mem::replace(&mut self.tasks, current);
                        // Raw push: a failed write must leave the redo branch
                        // exactly as it was, not discard it.
                        self.history.push_undo_raw(prev);
                        UndoOutcome::Error(e)
                    }
                }
            }
            None => UndoOutcome::Nothing,
        }
    }

    pub fn redo(&mut self) -> RedoOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return RedoOutcome::Aborted(other),
        }
        match self.history.pop_redo() {
            Some(next) => {
                let current = std::mem::replace(&mut self.tasks, next);
                match self.persist() {
                    Ok(()) => {
                        self.history.push_undo_raw(current);
                        RedoOutcome::Redone
                    }
                    Err(e) => {
                        let next = std::mem::replace(&mut self.tasks, current);
                        self.history.push_redo(next);
                        RedoOutcome::Error(e)
                    }
                }
            }
            None => RedoOutcome::Nothing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AddOutcome;
    use crate::core::test_support::build_store;

    #[test]
    fn history_evicts_fifo_at_undo_limit() {
        let mut store = build_store("a\n");
        for _ in 0..(UNDO_LIMIT + 5) {
            store.push_history();
        }
        assert_eq!(store.history.len(), UNDO_LIMIT);
    }

    /// Snapshot of the on-disk body, so tests assert persistence and not just
    /// in-memory state.
    fn on_disk(store: &Store) -> String {
        std::fs::read_to_string(&store.file_path).expect("read todo.txt")
    }

    fn raws(store: &Store) -> Vec<String> {
        store.tasks().iter().map(|t| t.raw.clone()).collect()
    }

    #[test]
    fn add_undo_redo_round_trips() {
        let mut store = build_store("a\n");
        assert!(matches!(store.add_finalized("b"), AddOutcome::Added { .. }));
        assert_eq!(raws(&store), ["a", "2026-05-06 b"]);
        assert_eq!(on_disk(&store), "a\n2026-05-06 b\n");

        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(raws(&store), ["a"]);
        assert_eq!(on_disk(&store), "a\n");

        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(&store), ["a", "2026-05-06 b"]);
        assert_eq!(on_disk(&store), "a\n2026-05-06 b\n");
    }

    #[test]
    fn redo_walks_back_up_a_multi_step_chain() {
        let mut store = build_store("a\n");
        store.add_finalized("b");
        store.add_finalized("c");
        store.undo();
        store.undo();
        assert_eq!(raws(&store), ["a"]);
        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(&store), ["a", "2026-05-06 b"]);
        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(&store), ["a", "2026-05-06 b", "2026-05-06 c"]);
        assert!(matches!(store.redo(), RedoOutcome::Nothing));
    }

    #[test]
    fn new_mutation_clears_redo_branch() {
        let mut store = build_store("a\n");
        store.add_finalized("b");
        store.undo();
        assert!(store.history.can_redo());

        store.add_finalized("c");
        assert!(!store.history.can_redo());
        assert!(matches!(store.redo(), RedoOutcome::Nothing));
        assert_eq!(raws(&store), ["a", "2026-05-06 c"]);
        assert_eq!(on_disk(&store), "a\n2026-05-06 c\n");
    }

    #[test]
    fn redo_with_empty_stack_is_nothing() {
        let mut store = build_store("a\n");
        assert!(matches!(store.redo(), RedoOutcome::Nothing));
        assert_eq!(raws(&store), ["a"]);
    }

    #[test]
    fn redo_evicts_fifo_at_undo_limit() {
        let mut store = build_store("a\n");
        for i in 0..(UNDO_LIMIT + 5) {
            store.add_finalized(&format!("task {i}"));
        }
        for _ in 0..(UNDO_LIMIT + 5) {
            store.undo();
        }
        assert_eq!(store.history.redo_len(), UNDO_LIMIT);
    }

    #[test]
    fn failed_undo_preserves_redo_branch() {
        let mut store = build_store("a\n");
        store.add_finalized("b");
        store.add_finalized("c");
        assert!(matches!(store.undo(), UndoOutcome::Undone));
        assert_eq!(store.history.redo_len(), 1);

        // Block the atomic write by occupying the temp path with a directory.
        let tmp_path = store.file_path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp_path);
        let _ = std::fs::remove_dir_all(&tmp_path);
        std::fs::create_dir(&tmp_path).expect("create blocking temp directory");
        assert!(matches!(store.undo(), UndoOutcome::Error(_)));
        std::fs::remove_dir(&tmp_path).expect("remove blocking temp directory");

        // The failed undo must not have discarded what was already redoable.
        assert_eq!(store.history.redo_len(), 1);
        assert_eq!(raws(&store), ["a", "2026-05-06 b"]);
        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(&store), ["a", "2026-05-06 b", "2026-05-06 c"]);
        assert_eq!(on_disk(&store), "a\n2026-05-06 b\n2026-05-06 c\n");
    }

    #[test]
    fn failed_redo_restores_tasks_and_history() {
        let mut store = build_store("a\n");
        store.add_finalized("b");
        assert!(matches!(store.undo(), UndoOutcome::Undone));

        let tmp_path = store.file_path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp_path);
        let _ = std::fs::remove_dir_all(&tmp_path);
        std::fs::create_dir(&tmp_path).expect("create blocking temp directory");

        assert!(matches!(store.redo(), RedoOutcome::Error(_)));
        assert_eq!(raws(&store), ["a"]);
        assert_eq!(store.history.redo_len(), 1);
        assert_eq!(on_disk(&store), "a\n");

        std::fs::remove_dir(&tmp_path).expect("remove blocking temp directory");

        // Once the write path is usable again the redo entry is still there.
        assert!(matches!(store.redo(), RedoOutcome::Redone));
        assert_eq!(raws(&store), ["a", "2026-05-06 b"]);
    }

    #[test]
    fn failed_undo_restores_tasks_and_history() {
        let mut store = build_store("first\nsecond\n");
        assert!(matches!(
            store.move_tasks(&[(0, 1)]),
            crate::core::MoveOutcome::Moved
        ));
        let tmp_path = store.file_path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp_path);
        let _ = std::fs::remove_dir_all(&tmp_path);
        std::fs::create_dir(&tmp_path).expect("create blocking temp directory");

        assert!(matches!(store.undo(), UndoOutcome::Error(_)));
        assert_eq!(
            store
                .tasks()
                .iter()
                .map(|task| task.raw.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        assert_eq!(store.history.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&store.file_path).expect("read todo.txt"),
            "second\nfirst\n"
        );

        std::fs::remove_dir(&tmp_path).expect("remove blocking temp directory");
    }
}
