#![allow(clippy::unwrap_used)]

use super::Store;

/// Each test gets a unique *directory* so parallel runs don't race — not just
/// on the todo file, but on the `done.txt` and `trash.txt` siblings the store
/// derives from it. The file is seeded with `raw` so `reconcile` sees a
/// consistent disk-vs-memory state.
pub(crate) fn test_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tuxedo-core-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("todo.txt")
}

/// Build a `Store` rooted at a fresh temp file seeded with `raw`. Archive loads
/// synchronously (`open_sync`), and today is fixed at 2026-05-06.
pub(crate) fn build_store(raw: &str) -> Store {
    let path = test_path();
    std::fs::write(&path, raw).unwrap();
    Store::open_sync(path, raw.to_string(), "2026-05-06".into())
}
