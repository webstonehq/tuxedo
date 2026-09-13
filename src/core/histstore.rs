//! Persisting the undo/redo history across runs.
//!
//! The file is **content-addressed**: each distinct task-list body is written
//! once as a numbered blob, and the stacks are lines of three blob ids. Since
//! almost every mutation touches only one of the three lists, a hundred
//! snapshots of a file whose archive never changed store that archive exactly
//! once — which is what keeps a 50-deep, three-list history small on disk.
//!
//! ```text
//! tuxedo-history 1
//! path /Users/you/todo/todo.txt
//! base 0 1 2
//! blob 0 2
//! (A) buy milk
//! (B) call dentist
//! blob 1 0
//! blob 2 0
//! undo 3 1 2
//! redo 0 1 2
//! ```
//!
//! `base` is the state the history expects to find on disk *now*. On load the
//! three ids are recomputed from what was actually read; any mismatch discards
//! the whole history, the same rule `apply_external_state` applies in-session.
//!
//! Every I/O error here is non-fatal. History is a convenience; a failure to
//! read or write it must never block a task edit.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::history::Snapshot;
use crate::todo::{self, Task};

const MAGIC: &str = "tuxedo-history 1";

/// FNV-1a, 64-bit. Inlined rather than pulled in as a dependency — the project
/// hand-rolls its parsers and carries no serialization crates.
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where the history for `todo_path` lives: `$XDG_STATE_HOME/tuxedo/history/`
/// keyed by a hash of the file's path, so two todo files never collide and no
/// path separator ever ends up in a filename.
pub(crate) fn path_for(todo_path: &Path) -> Option<PathBuf> {
    let canonical = todo_path
        .canonicalize()
        .unwrap_or_else(|_| todo_path.to_path_buf());
    let key = fnv1a(canonical.to_string_lossy().as_bytes());
    Some(
        crate::xdg::state_home()?
            .join("tuxedo")
            .join("history")
            .join(format!("{key:016x}")),
    )
}

/// Handle to the on-disk history file. Present only in the TUI — the one-shot
/// CLI has no session worth persisting.
#[derive(Debug)]
pub struct HistStore {
    path: PathBuf,
    /// Set after a write fails, so a broken state dir costs one attempt per
    /// session rather than one per keystroke.
    disabled: bool,
}

impl HistStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            disabled: false,
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Assigns stable ids to distinct list bodies as they're first seen.
    fn encode(
        base: &Snapshot,
        undo: &VecDeque<Snapshot>,
        redo: &VecDeque<Snapshot>,
        todo_path: &Path,
    ) -> String {
        let mut ids: HashMap<String, usize> = HashMap::new();
        let mut blobs: Vec<String> = Vec::new();
        let mut intern = |tasks: &[Task]| -> usize {
            let body = todo::serialize(tasks);
            if let Some(&id) = ids.get(&body) {
                return id;
            }
            let id = blobs.len();
            ids.insert(body.clone(), id);
            blobs.push(body);
            id
        };
        let triple = |s: &Snapshot, intern: &mut dyn FnMut(&[Task]) -> usize| {
            (intern(s.tasks()), intern(s.trash()), intern(s.archive()))
        };

        let base_ids = triple(base, &mut intern);
        let undo_ids: Vec<_> = undo.iter().map(|s| triple(s, &mut intern)).collect();
        let redo_ids: Vec<_> = redo.iter().map(|s| triple(s, &mut intern)).collect();

        let mut out = String::new();
        out.push_str(MAGIC);
        out.push('\n');
        out.push_str(&format!("path {}\n", todo_path.display()));
        out.push_str(&format!(
            "base {} {} {}\n",
            base_ids.0, base_ids.1, base_ids.2
        ));
        for (id, body) in blobs.iter().enumerate() {
            let lines = if body.is_empty() {
                0
            } else {
                body.lines().count()
            };
            out.push_str(&format!("blob {id} {lines}\n"));
            out.push_str(body);
        }
        for (kind, list) in [("undo", &undo_ids), ("redo", &redo_ids)] {
            for (t, r, a) in list.iter() {
                out.push_str(&format!("{kind} {t} {r} {a}\n"));
            }
        }
        out
    }

    /// Write the history, mirroring `Config::save_to`'s write-tmp-then-rename.
    pub(crate) fn save(
        &mut self,
        base: &Snapshot,
        undo: &VecDeque<Snapshot>,
        redo: &VecDeque<Snapshot>,
        todo_path: &Path,
    ) -> std::io::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let body = Self::encode(base, undo, redo, todo_path);
        let result = (|| {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = self.path.with_extension("tmp");
            std::fs::write(&tmp, &body)?;
            std::fs::rename(&tmp, &self.path)
        })();
        if result.is_err() {
            self.disabled = true;
        }
        result
    }

    /// Drop the persisted history — used when it has been invalidated.
    pub(crate) fn discard(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A parsed history file, before validation against what's on disk now.
pub(crate) struct Decoded {
    pub(crate) base: (usize, usize, usize),
    pub(crate) blobs: Vec<Arc<Vec<Task>>>,
    /// Kept as raw bodies so validation can compare without re-serializing.
    pub(crate) bodies: Vec<String>,
    pub(crate) undo: Vec<(usize, usize, usize)>,
    pub(crate) redo: Vec<(usize, usize, usize)>,
}

fn triple(rest: &str) -> Option<(usize, usize, usize)> {
    let mut it = rest.split_whitespace();
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    Some((a, b, c))
}

/// Parse a history file. Returns `None` on anything malformed — a corrupt
/// history is discarded, never repaired.
pub(crate) fn decode(text: &str) -> Option<Decoded> {
    let mut lines = text.lines();
    if lines.next()? != MAGIC {
        return None;
    }
    let mut base = None;
    let mut bodies: Vec<String> = Vec::new();
    let mut undo = Vec::new();
    let mut redo = Vec::new();

    while let Some(line) = lines.next() {
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        match kind {
            "path" => {}
            "base" => base = Some(triple(rest)?),
            "blob" => {
                let mut it = rest.split_whitespace();
                let id: usize = it.next()?.parse().ok()?;
                let count: usize = it.next()?.parse().ok()?;
                if id != bodies.len() {
                    return None;
                }
                let mut body = String::new();
                for _ in 0..count {
                    body.push_str(lines.next()?);
                    body.push('\n');
                }
                bodies.push(body);
            }
            "undo" => undo.push(triple(rest)?),
            "redo" => redo.push(triple(rest)?),
            "" => {}
            _ => return None,
        }
    }

    let base = base?;
    let max_id = bodies.len();
    let in_range = |&(a, b, c): &(usize, usize, usize)| a < max_id && b < max_id && c < max_id;
    if !in_range(&base) || !undo.iter().all(in_range) || !redo.iter().all(in_range) {
        return None;
    }
    // Parse each blob once and share the Arc across every snapshot that
    // references it, so the structural sharing survives the round trip.
    let blobs: Vec<Arc<Vec<Task>>> = bodies
        .iter()
        .map(|b| Arc::new(todo::parse_file(b)))
        .collect();
    Some(Decoded {
        base,
        blobs,
        bodies,
        undo,
        redo,
    })
}

impl Decoded {
    pub(crate) fn snapshot(&self, ids: (usize, usize, usize)) -> Snapshot {
        Snapshot {
            tasks: Arc::clone(&self.blobs[ids.0]),
            trash: Arc::clone(&self.blobs[ids.1]),
            archive: Arc::clone(&self.blobs[ids.2]),
        }
    }

    /// True when `base` still describes what is actually on disk.
    pub(crate) fn matches(&self, tasks: &str, trash: &str, archive: &str) -> bool {
        self.bodies[self.base.0] == tasks
            && self.bodies[self.base.1] == trash
            && self.bodies[self.base.2] == archive
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::Store;
    use crate::core::outcome::UndoOutcome;
    use crate::core::test_support::build_store;

    /// Rebuild a store over the same files, as a fresh session would, with
    /// history persistence pointed at `hist_path`.
    fn reopen(path: &Path, hist_path: PathBuf) -> Store {
        let body = std::fs::read_to_string(path).unwrap_or_default();
        let mut store = Store::open_sync(path.to_path_buf(), body, "2026-05-06".into());
        store.hist_store = Some(HistStore::new(hist_path.clone()));
        store.history_pending = Some(hist_path);
        // `open_sync` has no async loader, so everything is already known.
        store.resolve_pending_history();
        store
    }

    fn hist_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tuxedo-hist-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("history")
    }

    fn armed(raw: &str, hp: &Path) -> Store {
        let mut store = build_store(raw);
        store.hist_store = Some(HistStore::new(hp.to_path_buf()));
        store
    }

    #[test]
    fn history_round_trips_through_disk() {
        let hp = hist_path("roundtrip");
        let mut store = armed("(A) keep\n(B) toss\n", &hp);
        store.delete(1);
        assert!(store.flush_history());
        let todo_path = store.file_path.clone();

        // A new session over the same files picks the history back up.
        let mut next = reopen(&todo_path, hp);
        assert_eq!(next.tasks().len(), 1);
        assert!(matches!(next.undo(), UndoOutcome::Undone));
        assert_eq!(next.tasks().len(), 2);
        assert_eq!(
            std::fs::read_to_string(next.trash.path()).unwrap_or_default(),
            "",
            "the restored undo must empty trash.txt too"
        );
    }

    #[test]
    fn history_discarded_when_todo_changed_underneath() {
        let hp = hist_path("stale-todo");
        let mut store = armed("a\nb\n", &hp);
        store.delete(1);
        store.flush_history();
        let todo_path = store.file_path.clone();

        // Something edited todo.txt while tuxedo wasn't running.
        std::fs::write(&todo_path, "totally different\n").unwrap();
        let mut next = reopen(&todo_path, hp);
        assert!(next.history.is_empty());
        assert!(matches!(next.undo(), UndoOutcome::Nothing));
    }

    #[test]
    fn history_discarded_when_done_changed_underneath() {
        let hp = hist_path("stale-done");
        let mut store = armed("a\n", &hp);
        store.delete(0);
        store.flush_history();
        let todo_path = store.file_path.clone();
        let done_path = store.archive.path().to_path_buf();

        std::fs::write(&done_path, "x 2026-05-01 2026-04-01 snuck in\n").unwrap();
        let mut next = reopen(&todo_path, hp);
        assert!(next.history.is_empty());
        assert!(matches!(next.undo(), UndoOutcome::Nothing));
    }

    #[test]
    fn blob_table_dedupes_unchanged_components() {
        // The size guarantee: many snapshots, but the untouched archive and
        // trash bodies are each stored exactly once.
        let hp = hist_path("dedupe");
        let mut store = armed("seed\n", &hp);
        for i in 0..20 {
            store.add_finalized(&format!("task {i}"));
        }
        store.flush_history();

        let text = std::fs::read_to_string(&hp).unwrap();
        let blobs = text.lines().filter(|l| l.starts_with("blob ")).count();
        let undos = text.lines().filter(|l| l.starts_with("undo ")).count();
        assert_eq!(undos, 20, "one undo entry per add");
        // 21 distinct task-list bodies + one shared empty body for the
        // untouched trash and archive.
        assert_eq!(blobs, 22, "unchanged lists must not be re-stored");
    }

    #[test]
    fn cli_store_never_writes_history() {
        let store = build_store("a\n");
        assert!(
            store.history_path().is_none(),
            "open_sync must leave persistence off"
        );
    }

    #[test]
    fn corrupt_history_is_discarded_not_repaired() {
        let hp = hist_path("corrupt");
        let mut store = armed("a\n", &hp);
        store.delete(0);
        store.flush_history();
        let todo_path = store.file_path.clone();

        std::fs::write(&hp, "not a tuxedo history file\n").unwrap();
        let mut next = reopen(&todo_path, hp);
        assert!(next.history.is_empty());
        assert!(matches!(next.undo(), UndoOutcome::Nothing));
    }

    #[test]
    fn decode_rejects_out_of_range_blob_ids() {
        let text = "tuxedo-history 1\nbase 0 0 0\nblob 0 0\nundo 9 0 0\n";
        assert!(decode(text).is_none());
    }

    #[test]
    fn fnv_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_ne!(fnv1a(b"todo.txt"), fnv1a(b"other.txt"));
    }
}
