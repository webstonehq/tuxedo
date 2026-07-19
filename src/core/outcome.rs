//! Structured results returned by [`Store`](super::Store) operations.
//!
//! The core layer never produces user-facing strings (no flash, no stdout).
//! Each mutation returns an outcome enum carrying the data a caller needs to
//! render a message and, for the TUI, re-derive the cursor. The TUI maps these
//! to flash strings; the CLI maps them to stdout/exit codes.

use crate::todo::{ParseError, TagError};

/// An I/O or parse failure from a [`Store`](super::Store) operation.
#[derive(Debug)]
pub enum StoreError {
    /// Writing the todo file (`write_atomic`) failed.
    Write(std::io::Error),
    /// Reading or writing the sibling `done.txt` failed.
    ArchiveIo(std::io::Error),
    /// A constructed line failed to parse.
    Parse(ParseError),
    /// A `+project` / `@context` mutation was rejected.
    Tag(TagError),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Write(e) => write!(f, "write failed: {e}"),
            StoreError::ArchiveIo(e) => write!(f, "done.txt: {e}"),
            StoreError::Parse(e) => write!(f, "{e}"),
            StoreError::Tag(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Result of reconciling in-memory state against the file on disk before a
/// mutation. `Reloaded`/`ReadError` mean the caller's mutation was aborted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconcile {
    /// Disk matches memory; safe to proceed.
    Unchanged,
    /// Disk differed; in-memory tasks were replaced and history cleared.
    Reloaded,
    /// A non-`NotFound` I/O error; tasks were preserved, mutation must abort.
    ReadError,
}

#[derive(Debug)]
pub enum CompleteOutcome {
    Completed {
        abs: usize,
    },
    /// Completed a recurring task; the successor was inserted at `next`.
    CompletedSpawned {
        abs: usize,
        next: usize,
    },
    Uncompleted {
        abs: usize,
    },
    Aborted(Reconcile),
    OutOfRange,
    Error(StoreError),
}

#[derive(Debug)]
pub enum PriorityOutcome {
    Changed { abs: usize, priority: Option<char> },
    Aborted(Reconcile),
    OutOfRange,
    Error(StoreError),
}

#[derive(Debug)]
pub enum MoveOutcome {
    Moved,
    Unchanged,
    Aborted(Reconcile),
    OutOfRange,
    Error(StoreError),
}

#[derive(Debug)]
pub enum DeleteOutcome {
    Deleted { abs: usize },
    Aborted(Reconcile),
    OutOfRange,
    Error(StoreError),
}

/// Core-level add result. Distinct from the App's `AddOutcome`
/// (`app::types::AddOutcome`), which additionally models the interactive
/// natural-language preview step — that lives only in the TUI.
#[derive(Debug)]
pub enum AddOutcome {
    Added { abs: usize },
    Empty,
    Aborted(Reconcile),
    Error(StoreError),
}

/// Shared by `edit_line` (replace), `append_at`, `prepend_at`, and
/// `remove_term_at` — all "rewrite raw, re-parse, persist" operations.
#[derive(Debug)]
pub enum EditOutcome {
    Saved {
        abs: usize,
    },
    Empty,
    /// `remove_term_at`: the requested term wasn't present on the line.
    TermNotFound,
    OutOfRange,
    Aborted(Reconcile),
    Error(StoreError),
}

#[derive(Debug)]
pub enum TagOutcome {
    Added {
        abs: usize,
        name: String,
    },
    Removed {
        abs: usize,
        name: String,
    },
    /// Project already present (no-op).
    Unchanged,
    InvalidName,
    OutOfRange,
    Aborted(Reconcile),
    Error(StoreError),
}

#[derive(Debug)]
pub enum BulkCompleteOutcome {
    Done { completed: usize, spawned: usize },
    NothingToComplete,
    Aborted(Reconcile),
    Error(StoreError),
}

#[derive(Debug)]
pub enum BulkDeleteOutcome {
    Done { deleted: usize },
    Nothing,
    Aborted(Reconcile),
    Error(StoreError),
}

#[derive(Debug)]
pub enum ArchiveOutcome {
    Archived { count: usize },
    Nothing,
    Aborted(Reconcile),
    Error(StoreError),
}

#[derive(Debug)]
pub enum UnarchiveOutcome {
    Unarchived,
    OutOfRange,
    Aborted(Reconcile),
    /// `done.txt` changed under us; the mutation was refused and the archive
    /// reloaded from disk.
    DoneReloaded,
    Error(StoreError),
}

#[derive(Debug)]
pub enum ArchiveDeleteOutcome {
    Deleted,
    OutOfRange,
    DoneReloaded,
    Error(StoreError),
}

#[derive(Debug)]
pub enum UndoOutcome {
    Undone,
    Nothing,
    Aborted(Reconcile),
    Error(StoreError),
}

/// Result of draining a sibling `inbox.txt`. Replaces the drain flash strings
/// the TUI used to emit inline; the caller renders this however it likes.
#[derive(Debug, Default)]
pub struct DrainReport {
    pub merged: usize,
    pub skipped: usize,
    /// A lock/read/write/cleanup failure message, if any.
    pub error: Option<String>,
}

impl DrainReport {
    /// True when nothing happened and there was no error — the common case.
    pub fn is_noop(&self) -> bool {
        self.merged == 0 && self.skipped == 0 && self.error.is_none()
    }
}
