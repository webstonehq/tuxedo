use std::collections::VecDeque;
use std::sync::Arc;

use super::Store;
use super::histstore;
use super::outcome::{Reconcile, RedoOutcome, StoreError, UndoOutcome};
use crate::app::UNDO_LIMIT;
use crate::todo::{self, Task};

/// One point in time across all three task lists.
///
/// Each component is an `Arc` because almost every mutation touches exactly
/// one of them: editing a task clones two refcounts and one `Vec`, not three
/// `Vec`s. That is what makes a 50-deep stack over three lists affordable in
/// memory, and `Arc::ptr_eq` is what lets undo skip rewriting the files whose
/// component didn't actually move.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub(crate) tasks: Arc<Vec<Task>>,
    pub(crate) trash: Arc<Vec<Task>>,
    pub(crate) archive: Arc<Vec<Task>>,
}

impl Snapshot {
    pub(crate) fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub(crate) fn trash(&self) -> &[Task] {
        &self.trash
    }

    pub(crate) fn archive(&self) -> &[Task] {
        &self.archive
    }
}

/// Two snapshot stacks. `undo` holds pre-mutation states, newest at the back;
/// `redo` holds states that were undone away, newest at the back. Both are
/// capped at [`UNDO_LIMIT`] and evict oldest-first.
#[derive(Debug, Default, Clone)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
}

impl History {
    /// Record a pre-mutation snapshot. A fresh edit invalidates the redo
    /// branch — you can't redo past a divergence.
    pub fn push(&mut self, snapshot: Snapshot) {
        self.push_undo_raw(snapshot);
        self.redo.clear();
    }

    /// Push onto the undo stack *without* discarding the redo branch. Used by
    /// rollback paths and by `redo` itself, where the redo stack is being
    /// walked rather than invalidated.
    fn push_undo_raw(&mut self, snapshot: Snapshot) {
        if self.undo.len() >= UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(snapshot);
    }

    fn push_redo(&mut self, snapshot: Snapshot) {
        if self.redo.len() >= UNDO_LIMIT {
            self.redo.pop_front();
        }
        self.redo.push_back(snapshot);
    }

    pub fn pop(&mut self) -> Option<Snapshot> {
        self.undo.pop_back()
    }

    fn pop_redo(&mut self) -> Option<Snapshot> {
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

    pub(crate) fn undo_stack(&self) -> &VecDeque<Snapshot> {
        &self.undo
    }

    pub(crate) fn redo_stack(&self) -> &VecDeque<Snapshot> {
        &self.redo
    }

    pub(crate) fn restore(&mut self, undo: VecDeque<Snapshot>, redo: VecDeque<Snapshot>) {
        self.undo = undo;
        self.redo = redo;
    }
}

/// One list's `Arc` alongside the serialized body it was built from.
///
/// Without this, every call to [`Store::snapshot`] would allocate three fresh
/// `Arc`s and nothing would ever be shared — the stacks would deep-copy all
/// three lists per entry, and `Arc::ptr_eq` would be uselessly always-false.
/// Keyed on the body string, it is self-correcting: if the list changed, the
/// bodies differ and a new `Arc` is minted.
type Slot = Option<(String, Arc<Vec<Task>>)>;

fn cached(slot: &mut Slot, list: &[Task]) -> Arc<Vec<Task>> {
    let body = todo::serialize(list);
    if let Some((cached_body, arc)) = slot
        && *cached_body == body
    {
        return Arc::clone(arc);
    }
    let arc = Arc::new(list.to_vec());
    *slot = Some((body, Arc::clone(&arc)));
    arc
}

/// Per-list `Arc` cache; see [`Slot`].
#[derive(Debug, Default)]
pub(crate) struct SnapCache {
    tasks: Slot,
    trash: Slot,
    archive: Slot,
}

impl Store {
    /// The current state of all three lists. Components that haven't changed
    /// since the last call come back as the *same* `Arc`, so consecutive
    /// snapshots share them.
    pub(crate) fn snapshot(&mut self) -> Snapshot {
        // Disjoint field borrows: the cache slot and the list it mirrors.
        let tasks = cached(&mut self.snap_cache.tasks, &self.tasks);
        let trash = cached(&mut self.snap_cache.trash, &self.trash.tasks);
        let archive = cached(&mut self.snap_cache.archive, &self.archive.tasks);
        Snapshot {
            tasks,
            trash,
            archive,
        }
    }

    pub(crate) fn push_history(&mut self) {
        // The archive loads off-thread. Snapshotting before it lands would
        // record an *empty* archive, and undoing that snapshot would write the
        // empty list over a real done.txt. Force the read first — it costs one
        // small synchronous read, on the first mutation of a session only.
        if self.archive.loader.is_some() {
            let _ = self.archive.refresh_for_mutation();
        }
        let snap = self.snapshot();
        self.history.push(snap);
        self.mark_history_dirty();
    }

    /// Swap all three lists to `next`, writing only the files whose component
    /// actually changed. On failure every file already written is rolled back,
    /// preserving the all-or-nothing behaviour callers expect.
    fn apply_snapshot(&mut self, next: &Snapshot) -> Result<Snapshot, StoreError> {
        let current = self.snapshot();

        // Compare against the on-disk baseline rather than by `Arc` identity:
        // the question is "does the file already hold what we want", and a
        // rewrite of an unchanged file is wasted I/O.
        let trash_changed = todo::serialize(next.trash()) != self.trash.last_disk;
        let archive_changed = todo::serialize(next.archive()) != self.archive.last_disk;
        let tasks_changed = todo::serialize(next.tasks()) != self.last_disk;

        let prev_trash_body = self.trash.last_disk.clone();
        let prev_archive_body = self.archive.last_disk.clone();

        if trash_changed && let Err(e) = self.trash.write(next.trash.as_ref().clone()) {
            return Err(StoreError::TrashIo(e));
        }
        if archive_changed && let Err(e) = self.archive.write(next.archive.as_ref().clone()) {
            if trash_changed {
                self.trash.rollback(&prev_trash_body);
            }
            return Err(StoreError::ArchiveIo(e));
        }
        if tasks_changed {
            let restore = std::mem::replace(&mut self.tasks, next.tasks.as_ref().clone());
            if let Err(e) = self.persist() {
                self.tasks = restore;
                if archive_changed {
                    self.archive.rollback(&prev_archive_body);
                }
                if trash_changed {
                    self.trash.rollback(&prev_trash_body);
                }
                return Err(e);
            }
        }
        Ok(current)
    }

    pub fn undo(&mut self) -> UndoOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return UndoOutcome::Aborted(other),
        }
        match self.history.pop() {
            Some(prev) => match self.apply_snapshot(&prev) {
                Ok(current) => {
                    self.history.push_redo(current);
                    self.mark_history_dirty();
                    UndoOutcome::Undone
                }
                Err(e) => {
                    // Raw push: a failed write must leave the redo branch
                    // exactly as it was, not discard it.
                    self.history.push_undo_raw(prev);
                    UndoOutcome::Error(e)
                }
            },
            None => UndoOutcome::Nothing,
        }
    }

    pub fn redo(&mut self) -> RedoOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return RedoOutcome::Aborted(other),
        }
        match self.history.pop_redo() {
            Some(next) => match self.apply_snapshot(&next) {
                Ok(current) => {
                    self.history.push_undo_raw(current);
                    self.mark_history_dirty();
                    RedoOutcome::Redone
                }
                Err(e) => {
                    self.history.push_redo(next);
                    RedoOutcome::Error(e)
                }
            },
            None => RedoOutcome::Nothing,
        }
    }
}

/// History persistence: loading at startup, and the debounced write.
impl Store {
    /// Turn on cross-session history for this store. Called by the TUI only;
    /// the one-shot CLI leaves it off. The load itself is deferred — see
    /// [`Store::resolve_pending_history`].
    pub fn enable_history_persistence(&mut self) {
        let Some(path) = histstore::path_for(&self.file_path) else {
            return;
        };
        self.history_pending = Some(path.clone());
        self.hist_store = Some(histstore::HistStore::new(path));
    }

    pub(crate) fn mark_history_dirty(&mut self) {
        if self.hist_store.is_some() {
            self.history_dirty = true;
            self.history_touched = Some(std::time::Instant::now());
        }
    }

    /// Called when a sibling file changed underneath us. The saved snapshots
    /// describe a world that no longer exists, so both stacks go.
    pub(crate) fn note_side_changed(&mut self) {
        // During startup the archive loader landing is not an external edit —
        // it's the first read. Let the deferred load handle that tick.
        if self.history_pending.is_some() {
            return;
        }
        if !self.history.is_empty() || self.history.can_redo() {
            self.history.clear();
            self.mark_history_dirty();
        }
    }

    pub(crate) fn note_archive_changed(&mut self) {
        self.note_side_changed();
    }

    /// Load the saved history, if any, once all three lists are known. The
    /// archive arrives asynchronously, so this runs on the tick its loader
    /// lands rather than at construction.
    pub(crate) fn resolve_pending_history(&mut self) {
        let Some(path) = self.history_pending.take() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Some(decoded) = histstore::decode(&text) else {
            if let Some(hs) = &mut self.hist_store {
                hs.discard();
            }
            return;
        };
        let tasks_body = todo::serialize(&self.tasks);
        let trash_body = todo::serialize(&self.trash.tasks);
        let archive_body = todo::serialize(&self.archive.tasks);
        if !decoded.matches(&tasks_body, &trash_body, &archive_body) {
            // Something edited a file while tuxedo wasn't running.
            if let Some(hs) = &mut self.hist_store {
                hs.discard();
            }
            return;
        }
        let undo: VecDeque<Snapshot> = decoded.undo.iter().map(|&t| decoded.snapshot(t)).collect();
        let redo: VecDeque<Snapshot> = decoded.redo.iter().map(|&t| decoded.snapshot(t)).collect();
        self.history.restore(undo, redo);
    }

    /// Write the history if it changed and has been quiet for `debounce`.
    /// Returns true when a write actually happened.
    pub fn flush_history_if_due(&mut self, debounce: std::time::Duration) -> bool {
        if !self.history_dirty {
            return false;
        }
        match self.history_touched {
            Some(t) if t.elapsed() < debounce => return false,
            _ => {}
        }
        self.flush_history()
    }

    /// Write the history now, regardless of the debounce. Used on clean exit.
    pub fn flush_history(&mut self) -> bool {
        if !self.history_dirty || self.hist_store.is_none() {
            return false;
        }
        let base = self.snapshot();
        let undo = self.history.undo_stack().clone();
        let redo = self.history.redo_stack().clone();
        let file_path = self.file_path.clone();
        let Some(hs) = &mut self.hist_store else {
            return false;
        };
        let ok = hs.save(&base, &undo, &redo, &file_path).is_ok();
        self.history_dirty = false;
        self.history_touched = None;
        ok
    }

    /// Depth of the undo stack.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Path of the history file, for tests and diagnostics.
    pub fn history_path(&self) -> Option<&std::path::Path> {
        self.hist_store.as_ref().map(|h| h.path())
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
