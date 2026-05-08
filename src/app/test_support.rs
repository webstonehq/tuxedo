#![allow(clippy::unwrap_used)]

use super::App;
use crate::config::Config;

/// Each test gets a unique path so parallel runs don't race on /tmp/x.
/// We seed the file with `raw` so `check_external_changes` sees a
/// consistent disk-vs-memory state going in.
pub(crate) fn test_path() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("tuxedo-test-{}-{}.txt", std::process::id(), n))
}

pub(crate) fn build_app(raw: &str) -> App {
    let path = test_path();
    std::fs::write(&path, raw).unwrap();
    App::new(
        path,
        raw.to_string(),
        "2026-05-06".into(),
        Config::default(),
    )
}
