//! The sibling-file machinery shared by the `done.txt` archive and the
//! `trash.txt` bin: a task list mirrored to a file next to `todo.txt`, loaded
//! either off-thread at startup or inline, refreshed when something else edits
//! it, and written atomically.
//!
//! Both sides are the same shape and differ only in semantics, so the loading
//! and refresh logic lives here once. Archive-specific behaviour (completion
//! dates, `unmark_done`) stays in [`super::archive`]; trash-specific behaviour
//! stays in [`super::trash`].

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::todo::{self, Task};

/// A task list backed by a sibling file. Fields are `pub(crate)` so the
/// `Store` methods that own the mutations can reach in directly; external
/// callers go through the read methods.
pub struct SideFile {
    pub(crate) tasks: Vec<Task>,
    pub(crate) path: PathBuf,
    pub(crate) last_disk: String,
    pub(crate) loader: Option<Receiver<(String, Vec<Task>)>>,
}

/// The sibling of `todo_path` named `default_name`, falling back to a bare
/// relative path when `todo_path` has no parent.
pub(crate) fn sibling_path(todo_path: &Path, default_name: &str) -> PathBuf {
    todo_path
        .parent()
        .map(|p| p.join(default_name))
        .unwrap_or_else(|| PathBuf::from(default_name))
}

impl SideFile {
    /// Construct for the sibling `default_name` of `todo_path` and spawn a
    /// worker thread to read+parse it, so the first frame can render
    /// `todo.txt` while the read is still in flight.
    pub fn spawn(todo_path: &Path, default_name: &str) -> Self {
        Self::spawn_at(sibling_path(todo_path, default_name))
    }

    /// Like [`SideFile::spawn`] but for an explicit path (e.g. a `DONE_FILE`
    /// that isn't a sibling of the todo file).
    pub fn spawn_at(path: PathBuf) -> Self {
        let loader_path = path.clone();
        let (tx, rx) = mpsc::sync_channel::<(String, Vec<Task>)>(1);
        thread::spawn(move || {
            let body = std::fs::read_to_string(&loader_path).unwrap_or_default();
            let parsed = todo::parse_file(&body);
            let _ = tx.send((body, parsed));
        });
        Self {
            tasks: Vec::new(),
            path,
            last_disk: String::new(),
            loader: Some(rx),
        }
    }

    /// Read and parse inline (no background thread). Used by the one-shot CLI,
    /// and by the trash in the TUI — it's small, and it has to be loaded before
    /// the first delete anyway.
    pub fn load_sync(todo_path: &Path, default_name: &str) -> Self {
        Self::load_sync_at(sibling_path(todo_path, default_name))
    }

    /// Like [`SideFile::load_sync`] but for an explicit path.
    pub fn load_sync_at(path: PathBuf) -> Self {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let tasks = todo::parse_file(&body);
        Self {
            tasks,
            path,
            last_disk: body,
            loader: None,
        }
    }

    /// Test-only constructor that skips the worker thread and seeds in-memory
    /// state directly.
    #[cfg(test)]
    pub(crate) fn for_test(tasks: Vec<Task>, last_disk: String, path: PathBuf) -> Self {
        Self {
            tasks,
            path,
            last_disk,
            loader: None,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Read the backing file. A missing file is an empty list, not an error.
    pub(crate) fn read_body(&self) -> std::io::Result<String> {
        match std::fs::read_to_string(&self.path) {
            Ok(body) => Ok(body),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    /// Re-read before a mutation that will rewrite the file, so an external
    /// edit since startup isn't silently clobbered.
    pub(crate) fn refresh_for_mutation(&mut self) -> Refresh {
        let body = match self.read_body() {
            Ok(b) => b,
            Err(e) => return Refresh::Error(e),
        };
        if body != self.last_disk {
            self.tasks = todo::parse_file(&body);
            self.last_disk = body;
            self.loader = None;
            return Refresh::Reloaded;
        }
        self.loader = None;
        Refresh::Ready
    }

    /// Pump state. Returns true when the visible list changed: the startup
    /// loader landed, or an external edit was picked up. Non-blocking.
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.loader {
            match rx.try_recv() {
                Ok((body, tasks)) => {
                    self.last_disk = body;
                    self.tasks = tasks;
                    self.loader = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    self.loader = None;
                }
            }
        }
        if !changed {
            let read = std::fs::read_to_string(&self.path);
            changed = self.apply_read(read);
        }
        changed
    }

    /// Apply a read result. `NotFound` is an empty list; any other I/O error
    /// preserves in-memory state and returns `false` rather than wiping it.
    pub(crate) fn apply_read(&mut self, read: std::io::Result<String>) -> bool {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return false,
        };
        if on_disk == self.last_disk {
            return false;
        }
        self.tasks = todo::parse_file(&on_disk);
        self.last_disk = on_disk;
        true
    }

    /// Replace the list and write it atomically. On success the in-memory
    /// state and `last_disk` baseline move together; on failure neither moves.
    pub(crate) fn write(&mut self, tasks: Vec<Task>) -> std::io::Result<()> {
        let body = todo::serialize(&tasks);
        todo::write_atomic(&self.path, &body)?;
        self.tasks = tasks;
        self.last_disk = body;
        self.loader = None;
        Ok(())
    }

    /// Restore a previously-captured body after a later write in the same
    /// operation failed. Best-effort: the caller is already returning an error.
    pub(crate) fn rollback(&mut self, body: &str) {
        if todo::write_atomic(&self.path, body).is_ok() {
            self.tasks = todo::parse_file(body);
            self.last_disk = body.to_string();
        }
    }
}

/// Result of refreshing a sibling file before a mutation that writes it.
pub(crate) enum Refresh {
    Ready,
    Reloaded,
    Error(std::io::Error),
}
