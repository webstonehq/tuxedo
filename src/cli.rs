//! Shared CLI helpers used by the TUI entry point and the `tuxedo serve`
//! subcommand. Kept in the library crate so both binaries (the main TUI
//! and any future headless commands) resolve target paths the same way.

use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use crate::sample;

/// Resolve the todo.txt path from an optional positional CLI argument.
/// Mirrors the TUI's default behavior:
///
/// * `Some(path)` — use that path verbatim, creating an empty file if it
///   doesn't exist. Uses `create_new` to avoid the TOCTOU window where a
///   concurrently-created file would otherwise be truncated.
/// * `None` and `./todo.txt` exists — use that.
/// * Otherwise — fall back to the bundled sample in the temp dir.
pub fn resolve_path(arg: Option<String>) -> io::Result<PathBuf> {
    if let Some(p) = arg {
        let pb = PathBuf::from(p);
        match OpenOptions::new().write(true).create_new(true).open(&pb) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        return Ok(pb);
    }
    let cwd_todo = PathBuf::from("todo.txt");
    if cwd_todo.is_file() {
        return Ok(cwd_todo);
    }
    sample_path()
}

/// Write the bundled sample todo.txt to the system temp dir and return
/// its path. Also resets the sibling `done.txt` so a previous session's
/// archived rows don't leak back as duplicates.
pub fn sample_path() -> io::Result<PathBuf> {
    let dir = std::env::temp_dir();
    let pb = dir.join("tuxedo-sample.txt");
    std::fs::write(&pb, sample::TODO_RAW)?;
    match std::fs::remove_file(dir.join("done.txt")) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(pb)
}
