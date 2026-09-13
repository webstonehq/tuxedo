#![allow(clippy::unwrap_used)]

use super::App;
use crate::config::Config;

/// Each test gets a unique *directory* so parallel runs don't race — not just
/// on the todo file, but on the `done.txt` and `trash.txt` siblings the store
/// derives from it. We seed the file with `raw` so `check_external_changes`
/// sees a consistent disk-vs-memory state going in.
pub(crate) fn test_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tuxedo-test-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("todo.txt")
}

pub(crate) fn build_app(raw: &str) -> App {
    build_app_with_config(raw, Config::default())
}

pub(crate) fn build_app_with_config(raw: &str, cfg: Config) -> App {
    let path = test_path();
    std::fs::write(&path, raw).unwrap();
    App::new(path, raw.to_string(), "2026-05-06".into(), cfg)
}
