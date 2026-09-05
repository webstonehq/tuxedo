//! Headless core: the durable task store, its persistence/I-O, and all task
//! mutations. Carries no view, input, or presentation state — operations return
//! structured [`outcome`] values rather than user-facing strings. Both the TUI
//! (`App` wraps a `Store`) and the CLI (`cmd`) drive this type.

use std::path::{Path, PathBuf};

use crate::todo::{self, Task};

mod archive;
mod external;
mod history;
mod histstore;
mod mutations;
mod sidefile;
mod trash;

pub mod filter;
pub mod outcome;

#[cfg(test)]
pub(crate) mod test_support;

pub use archive::Archive;
pub use history::{History, Snapshot};
pub use outcome::{
    AddOutcome, ArchiveDeleteOutcome, ArchiveOutcome, BulkCompleteOutcome, BulkDeleteOutcome,
    CompleteOutcome, DeleteOutcome, DrainReport, EditOutcome, MoveOutcome, PriorityOutcome,
    Reconcile, RedoOutcome, RenameOutcome, StoreError, TagOutcome, TrashDeleteOutcome,
    TrashEmptyOutcome, TrashRestoreOutcome, UnarchiveOutcome, UndoOutcome,
};
pub use sidefile::SideFile;
pub use trash::Trash;

/// The durable task store. Owns the live task list, the sibling `done.txt`
/// archive, undo history, and the on-disk reconciliation snapshot.
pub struct Store {
    pub(crate) tasks: Vec<Task>,
    pub(crate) history: History,
    pub(crate) archive: Archive,
    pub(crate) trash: Trash,
    pub(crate) file_path: PathBuf,
    /// Snapshot of the file body the last time we read or wrote it; used by
    /// `reconcile` to detect external edits.
    pub(crate) last_disk: String,
    pub(crate) today: String,
    /// On-disk undo/redo history. `None` for the one-shot CLI, which has no
    /// session to persist. Enabled by [`Store::enable_history_persistence`].
    pub(crate) hist_store: Option<histstore::HistStore>,
    /// Set when the history changed and hasn't been flushed yet; drives the
    /// debounced write in the TUI event loop.
    pub(crate) history_dirty: bool,
    /// Instant of the most recent history change, for the debounce.
    pub(crate) history_touched: Option<std::time::Instant>,
    /// Path of a history file that hasn't been loaded yet. The load is
    /// deferred until the async archive loader lands, because validating the
    /// saved history requires knowing all three lists.
    pub(crate) history_pending: Option<PathBuf>,
    /// Per-list `Arc` cache backing [`Store::snapshot`]'s structural sharing.
    pub(crate) snap_cache: history::SnapCache,
}

impl Store {
    /// Construct a store, loading the archive (`done.txt`) off-thread from the
    /// sibling of `file_path`. Used by the TUI so the first frame doesn't wait
    /// on the archive read. The trash is read synchronously — it's small, and
    /// it has to be known before the first delete anyway.
    pub fn new(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::spawn(&file_path, archive::DONE_NAME);
        let trash = Trash::load_sync(&file_path, trash::TRASH_NAME);
        Self::assemble(file_path, archive, trash, body, today)
    }

    /// Like [`Store::new`] but with explicit sibling paths (e.g. from
    /// `DONE_FILE` / `TRASH_FILE` env vars that aren't siblings of the todo
    /// file).
    pub fn new_with_sides(
        file_path: PathBuf,
        done_path: PathBuf,
        trash_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::spawn_at(done_path);
        let trash = Trash::load_sync_at(trash_path);
        Self::assemble(file_path, archive, trash, body, today)
    }

    /// Construct a store, loading both sibling files synchronously (no
    /// background thread). Used by the one-shot CLI.
    pub fn open_sync(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::load_sync(&file_path, archive::DONE_NAME);
        let trash = Trash::load_sync(&file_path, trash::TRASH_NAME);
        Self::assemble(file_path, archive, trash, body, today)
    }

    /// Like [`Store::open_sync`] but with explicit sibling paths.
    pub fn open_sync_with_sides(
        file_path: PathBuf,
        done_path: PathBuf,
        trash_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::load_sync_at(done_path);
        let trash = Trash::load_sync_at(trash_path);
        Self::assemble(file_path, archive, trash, body, today)
    }

    fn assemble(
        file_path: PathBuf,
        archive: Archive,
        trash: Trash,
        body: String,
        today: String,
    ) -> Self {
        let tasks = todo::parse_file(&body);
        Self {
            tasks,
            history: History::default(),
            archive,
            trash,
            file_path,
            last_disk: body,
            today,
            hist_store: None,
            history_dirty: false,
            history_touched: None,
            history_pending: None,
            snap_cache: history::SnapCache::default(),
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn archive(&self) -> &Archive {
        &self.archive
    }

    pub fn today(&self) -> &str {
        &self.today
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.tasks.get(abs).map(|t| t.raw.clone())
    }

    /// True when at least one live task is marked done.
    pub fn has_completed(&self) -> bool {
        self.tasks.iter().any(|t| t.done)
    }

    /// Update the cached "today". Returns `true` iff the value changed, so the
    /// caller knows to recompute any date-dependent view state.
    pub fn set_today(&mut self, today: String) -> bool {
        if self.today == today {
            return false;
        }
        self.today = today;
        true
    }
}
