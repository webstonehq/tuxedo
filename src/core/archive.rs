use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use super::Store;
use super::outcome::{
    ArchiveDeleteOutcome, ArchiveOutcome, Reconcile, StoreError, UnarchiveOutcome,
};
use crate::todo::{self, Task};

/// Owns the archived (`done.txt`) tasks and the lifecycle around loading them
/// off-thread at startup. Fields are `pub(crate)` so the `Store` methods in this
/// file can mutate the archive directly; external callers go through the read
/// methods.
pub struct Archive {
    pub(crate) tasks: Vec<Task>,
    pub(crate) path: PathBuf,
    pub(crate) last_disk: String,
    pub(crate) loader: Option<Receiver<(String, Vec<Task>)>>,
}

fn done_path(todo_path: &Path) -> PathBuf {
    todo_path
        .parent()
        .map(|p| p.join("done.txt"))
        .unwrap_or_else(|| PathBuf::from("done.txt"))
}

impl Archive {
    /// Construct an `Archive` for the sibling `done.txt` of `todo_path` and
    /// spawn a worker thread to read+parse it. The first frame can render
    /// `todo.txt` immediately while the loader runs in the background.
    pub fn spawn(todo_path: &Path) -> Self {
        Self::spawn_at(done_path(todo_path))
    }

    /// Like [`Archive::spawn`] but for an explicit `done.txt` path (e.g. a
    /// `DONE_FILE` that isn't a sibling of the todo file).
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

    /// Read and parse the sibling `done.txt` inline (no background thread).
    /// Used by the one-shot CLI, where spawning a loader would be wasteful.
    pub fn load_sync(todo_path: &Path) -> Self {
        Self::load_sync_at(done_path(todo_path))
    }

    /// Like [`Archive::load_sync`] but for an explicit `done.txt` path.
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
}

/// Internal result of refreshing `done.txt` before a mutation that writes it.
enum ArchiveRefresh {
    Ready,
    Reloaded,
    Error(std::io::Error),
}

impl Store {
    fn read_archive_body(&self) -> std::io::Result<String> {
        match std::fs::read_to_string(&self.archive.path) {
            Ok(body) => Ok(body),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    fn refresh_archive_for_mutation(&mut self) -> ArchiveRefresh {
        let body = match self.read_archive_body() {
            Ok(b) => b,
            Err(e) => return ArchiveRefresh::Error(e),
        };
        if body != self.archive.last_disk {
            self.archive.tasks = todo::parse_file(&body);
            self.archive.last_disk = body;
            self.archive.loader = None;
            return ArchiveRefresh::Reloaded;
        }
        self.archive.loader = None;
        ArchiveRefresh::Ready
    }

    /// Pump archive state. Returns true when the visible archive changed: the
    /// startup loader landed, or an external edit to `done.txt` was picked up.
    /// Non-blocking. The caller (TUI) is responsible for any view recompute.
    pub fn poll_archive(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.archive.loader {
            match rx.try_recv() {
                Ok((body, tasks)) => {
                    self.archive.last_disk = body;
                    self.archive.tasks = tasks;
                    self.archive.loader = None;
                    changed = true;
                }
                Err(TryRecvError::Empty) => return false,
                Err(TryRecvError::Disconnected) => {
                    self.archive.loader = None;
                }
            }
        }
        if !changed {
            let read = std::fs::read_to_string(&self.archive.path);
            changed = self.apply_archive_read(read);
        }
        changed
    }

    /// Apply a read result for `done.txt`. `NotFound` is treated as an empty
    /// archive; any other I/O error preserves in-memory state and returns
    /// `false` rather than wiping the archive.
    pub(crate) fn apply_archive_read(&mut self, read: std::io::Result<String>) -> bool {
        let on_disk = match read {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(_) => return false,
        };
        if on_disk == self.archive.last_disk {
            return false;
        }
        self.archive.tasks = todo::parse_file(&on_disk);
        self.archive.last_disk = on_disk;
        true
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
        let previous_archive_body = match self.read_archive_body() {
            Ok(b) => b,
            Err(e) => return ArchiveOutcome::Error(StoreError::ArchiveIo(e)),
        };
        let mut combined = previous_archive_body.clone();
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&todo::serialize(&to_move));
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
        self.push_history();
        let count = to_move.len();
        self.tasks = remaining;
        self.last_disk = remaining_body;
        self.archive.tasks = todo::parse_file(&combined);
        self.archive.last_disk = combined;
        self.archive.loader = None;
        self.post_commit(crate::hooks::HookEvent::Archive);
        ArchiveOutcome::Archived { count }
    }

    /// Move an archived task back into the live list. `archive_idx` indexes
    /// `self.archive.tasks()`.
    pub fn unarchive(&mut self, archive_idx: usize) -> UnarchiveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return UnarchiveOutcome::Aborted(other),
        }
        match self.refresh_archive_for_mutation() {
            ArchiveRefresh::Ready => {}
            ArchiveRefresh::Reloaded => return UnarchiveOutcome::DoneReloaded,
            ArchiveRefresh::Error(e) => return UnarchiveOutcome::Error(StoreError::ArchiveIo(e)),
        }
        if archive_idx >= self.archive.tasks.len() {
            return UnarchiveOutcome::OutOfRange;
        }
        let mut task = self.archive.tasks[archive_idx].clone();
        if let Err(e) = task.unmark_done() {
            return UnarchiveOutcome::Error(StoreError::Parse(e));
        }
        let new_archive: Vec<Task> = self
            .archive
            .tasks
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != archive_idx)
            .map(|(_, t)| t.clone())
            .collect();
        let archive_body = todo::serialize(&new_archive);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            return UnarchiveOutcome::Error(StoreError::ArchiveIo(e));
        }
        self.archive.tasks = new_archive;
        self.archive.last_disk = archive_body;
        self.push_history();
        self.tasks.push(task);
        if let Err(e) = self.persist_with_hook(crate::hooks::HookEvent::Unarchive) {
            return UnarchiveOutcome::Error(e);
        }
        UnarchiveOutcome::Unarchived
    }

    /// Permanently remove an archived task from `done.txt`.
    pub fn archive_delete(&mut self, archive_idx: usize) -> ArchiveDeleteOutcome {
        match self.refresh_archive_for_mutation() {
            ArchiveRefresh::Ready => {}
            ArchiveRefresh::Reloaded => return ArchiveDeleteOutcome::DoneReloaded,
            ArchiveRefresh::Error(e) => {
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
        let archive_body = todo::serialize(&new_archive);
        if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
            return ArchiveDeleteOutcome::Error(StoreError::ArchiveIo(e));
        }
        self.archive.tasks = new_archive;
        self.archive.last_disk = archive_body;
        self.post_commit(crate::hooks::HookEvent::Delete);
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

    pub(crate) fn persist_with_hook(
        &mut self,
        event: crate::hooks::HookEvent,
    ) -> Result<(), StoreError> {
        self.persist()?;
        self.post_commit(event);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::test_support::build_store;
    use crate::hooks::{HookConfig, HookEvent};
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

    #[test]
    fn archive_emits_one_hook_after_both_files_commit() {
        let mut store = build_store("x 2026-05-05 2026-05-01 done\n");
        store.set_hooks(HookConfig {
            after_mutation: Some("/definitely/not/a/tuxedo-hook".into()),
        });

        assert!(matches!(
            store.archive_completed(),
            ArchiveOutcome::Archived { count: 1 }
        ));
        let reports = store.take_hook_reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].event, HookEvent::Archive);
    }

    #[test]
    fn generic_hook_covers_unarchive_and_archive_delete() {
        let dir = dir_for("generic-events");
        let todo_path = dir.join("todo.txt");
        let done_path = dir.join("done.txt");
        let generic = std::path::PathBuf::from("/definitely/not/a/generic-hook");

        std::fs::write(&todo_path, "").unwrap();
        std::fs::write(&done_path, "x 2026-05-05 2026-05-01 archived\n").unwrap();
        let mut store = Store::open_sync(todo_path.clone(), "".into(), "2026-05-06".into());
        store.set_hooks(HookConfig {
            after_mutation: Some(generic.clone()),
        });
        assert!(matches!(store.unarchive(0), UnarchiveOutcome::Unarchived));
        let report = store.take_hook_reports().pop().expect("unarchive report");
        assert_eq!(report.event, HookEvent::Unarchive);
        assert_eq!(report.script, generic);

        std::fs::write(&todo_path, "").unwrap();
        std::fs::write(&done_path, "x 2026-05-05 2026-05-01 archived\n").unwrap();
        let mut store = Store::open_sync(todo_path, "".into(), "2026-05-06".into());
        store.set_hooks(HookConfig {
            after_mutation: Some("/definitely/not/a/generic-hook".into()),
        });
        assert!(matches!(
            store.archive_delete(0),
            ArchiveDeleteOutcome::Deleted
        ));
        let reports = store.take_hook_reports();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].event, HookEvent::Delete);
        let _ = std::fs::remove_dir_all(&dir);
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
        assert!(!store.apply_archive_read(Err(err)));
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
