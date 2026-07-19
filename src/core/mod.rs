//! Headless core: the durable task store, its persistence/I-O, and all task
//! mutations. Carries no view, input, or presentation state — operations return
//! structured [`outcome`] values rather than user-facing strings. Both the TUI
//! (`App` wraps a `Store`) and the CLI (`cmd`) drive this type.

use std::path::{Path, PathBuf};

use crate::todo::{self, Task};

mod archive;
mod external;
mod history;
mod mutations;

pub mod filter;
pub mod outcome;

#[cfg(test)]
pub(crate) mod test_support;

pub use archive::Archive;
pub use history::History;
pub use outcome::{
    AddOutcome, ArchiveDeleteOutcome, ArchiveOutcome, BulkCompleteOutcome, BulkDeleteOutcome,
    CompleteOutcome, DeleteOutcome, DrainReport, EditOutcome, MoveOutcome, PriorityOutcome,
    Reconcile, StoreError, TagOutcome, UnarchiveOutcome, UndoOutcome,
};

/// The durable task store. Owns the live task list, the sibling `done.txt`
/// archive, undo history, and the on-disk reconciliation snapshot.
pub struct Store {
    pub(crate) tasks: Vec<Task>,
    pub(crate) history: History,
    pub(crate) archive: Archive,
    pub(crate) file_path: PathBuf,
    /// Snapshot of the file body the last time we read or wrote it; used by
    /// `reconcile` to detect external edits.
    pub(crate) last_disk: String,
    pub(crate) today: String,
}

impl Store {
    /// Construct a store, loading the archive (`done.txt`) off-thread from the
    /// sibling of `file_path`. Used by the TUI so the first frame doesn't wait
    /// on the archive read.
    pub fn new(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::spawn(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::new`] but with an explicit `done.txt` path (e.g. from a
    /// `DONE_FILE` env var that isn't a sibling of the todo file).
    pub fn new_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::spawn_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Construct a store, loading the sibling archive synchronously (no
    /// background thread). Used by the one-shot CLI.
    pub fn open_sync(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::load_sync(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::open_sync`] but with an explicit `done.txt` path.
    pub fn open_sync_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::load_sync_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    fn assemble(file_path: PathBuf, archive: Archive, body: String, today: String) -> Self {
        let tasks = todo::parse_file(&body);
        Self {
            tasks,
            history: History::default(),
            archive,
            file_path,
            last_disk: body,
            today,
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
