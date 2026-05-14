//! Sibling `inbox.txt` capture flow.
//!
//! External producers (shell appends, iOS Shortcuts writing to a sync
//! folder, `tuxedo serve`'s POST handler) drop one task per line into a
//! sibling `inbox.txt`. The running TUI drains it on each external-change
//! poll (~250 ms): each line is run through the natural-language
//! pipeline, given a creation date if missing, validated, and merged
//! into `todo.txt`. See [`crate::app::App::drain_inbox`] for the merge
//! wiring; this module owns the pure per-line transformation.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;

use crate::{nl, todo};

pub const FILENAME: &str = "inbox.txt";
pub const STAGING_FILENAME: &str = "inbox.txt.tuxedo-staging";
pub const LOCK_FILENAME: &str = "inbox.txt.tuxedo-lock";

/// Sibling `inbox.txt` next to the given todo.txt path. Falls back to
/// the current directory if `todo_path` has no parent.
pub fn path_for(todo_path: &Path) -> PathBuf {
    sibling(todo_path, FILENAME)
}

/// Staging file used during drain. The merge step renames
/// `inbox.txt` → `inbox.txt.tuxedo-staging` *before* reading, so any
/// concurrent external append after the rename lands in a fresh
/// `inbox.txt` rather than being lost. The staging file is deleted only
/// after the merged `todo.txt` has been written atomically; if tuxedo
/// crashes between, the next drain picks the staging file up and merges
/// it as if it were a regular inbox.
pub fn staging_path_for(todo_path: &Path) -> PathBuf {
    sibling(todo_path, STAGING_FILENAME)
}

/// Advisory-lock file guarding `inbox.txt`. Held briefly by both the
/// `tuxedo serve` POST handler (around its append) and the TUI drain
/// (around its rename-and-merge). Without it the writer's `open` could
/// pin the inode after the drain has renamed it to `staging`, the
/// drain reads the still-empty staging, deletes it, and the writer's
/// subsequent `write` is silently lost when the unlinked inode is
/// reclaimed.
pub fn lock_path_for(todo_path: &Path) -> PathBuf {
    sibling(todo_path, LOCK_FILENAME)
}

/// Acquire the inbox lock. The returned handle holds an exclusive
/// `flock`-style lock for its lifetime — drop it to release. Both
/// producers and the drain take this around any operation touching
/// `inbox.txt` or `staging`. Cross-platform via `std::fs::File::lock`
/// (`flock` on Unix, `LockFileEx` on Windows); released automatically
/// on process exit if the holder crashes.
pub fn acquire_lock(todo_path: &Path) -> std::io::Result<std::fs::File> {
    let path = lock_path_for(todo_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)?;
    file.lock()?;
    Ok(file)
}

fn sibling(todo_path: &Path, name: &str) -> PathBuf {
    todo_path
        .parent()
        .map(|p| p.join(name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Full save pipeline for one free-text line: natural-language rewrite,
/// creation-date prepend, validation. Returns the parsed [`todo::Task`]
/// ready to push onto `App::tasks`. Used by the inbox drain and the
/// `tuxedo serve` POST handler.
pub fn canonicalize_line(text: &str, today: NaiveDate) -> Result<todo::Task, todo::ParseError> {
    let mut text = text.trim().to_string();
    if text.is_empty() {
        return Err(todo::ParseError::Empty);
    }
    if nl::looks_like_natural_language(&text)
        && let Some(parsed) = nl::try_parse(&text, today)
    {
        text = nl::format_as_todo_txt(&parsed);
    }
    let today_str = today.format("%Y-%m-%d").to_string();
    finalize_line(&text, &today_str)
}

/// The post-NL half of [`canonicalize_line`]: skip the natural-language
/// rewrite (the caller has already produced canonical form) and just
/// prepend a creation date if missing, then validate. The add-prompt's
/// second-Enter save path uses this directly — the draft buffer is
/// already canonical after the first-Enter preview.
pub fn finalize_line(text: &str, today_str: &str) -> Result<todo::Task, todo::ParseError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(todo::ParseError::Empty);
    }
    let final_text = if !todo::starts_with_priority(text)
        && !todo::starts_with_iso_date(text)
        && !text.starts_with("x ")
    {
        format!("{today_str} {text}")
    } else {
        text.to_string()
    };
    todo::parse_line(&final_text)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 13).unwrap()
    }

    #[test]
    fn path_for_uses_sibling_directory() {
        let p = PathBuf::from("/tmp/work/todo.txt");
        assert_eq!(path_for(&p), PathBuf::from("/tmp/work/inbox.txt"));
    }

    #[test]
    fn path_for_falls_back_to_relative_when_no_parent() {
        // A bare filename like "todo.txt" still has parent = Some("")
        // on Unix, which joins to "inbox.txt" — same result either way.
        let p = PathBuf::from("todo.txt");
        let got = path_for(&p);
        assert_eq!(got.file_name().unwrap(), "inbox.txt");
    }

    #[test]
    fn staging_path_for_uses_distinct_name() {
        let p = PathBuf::from("/tmp/work/todo.txt");
        assert_eq!(
            staging_path_for(&p),
            PathBuf::from("/tmp/work/inbox.txt.tuxedo-staging"),
        );
    }

    #[test]
    fn acquire_lock_blocks_a_concurrent_holder() {
        use std::sync::mpsc;
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("tuxedo-inbox-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "").unwrap();
        let held = acquire_lock(&todo_path).unwrap();

        // A second acquisition from another thread must block until we
        // drop `held`. We assert that by checking the channel hasn't
        // received yet after a short wait, then dropping the lock and
        // verifying it does arrive.
        let (tx, rx) = mpsc::channel();
        let todo_path_clone = todo_path.clone();
        let t = std::thread::spawn(move || {
            let second = acquire_lock(&todo_path_clone).unwrap();
            tx.send(()).unwrap();
            drop(second);
        });

        // The second `acquire_lock` should still be blocked on flock.
        assert!(rx.recv_timeout(Duration::from_millis(150)).is_err());
        drop(held);
        // Now it should make progress.
        rx.recv_timeout(Duration::from_secs(2))
            .expect("second acquire_lock should unblock once we release");
        t.join().unwrap();
    }

    #[test]
    fn canonicalize_rewrites_natural_language() {
        let task = canonicalize_line("Buy milk tomorrow", today()).unwrap();
        assert!(task.raw.contains("Buy milk"));
        assert_eq!(task.due.as_deref(), Some("2026-05-14"));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
    }

    #[test]
    fn canonicalize_preserves_canonical_input() {
        // Already contains `due:` so NL detection skips.
        let task = canonicalize_line("Call dentist due:2026-06-01", today()).unwrap();
        assert!(task.raw.contains("Call dentist"));
        assert_eq!(task.due.as_deref(), Some("2026-06-01"));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
    }

    #[test]
    fn canonicalize_does_not_prepend_date_when_already_present() {
        let task = canonicalize_line("2026-04-01 already dated", today()).unwrap();
        assert_eq!(task.created_date.as_deref(), Some("2026-04-01"));
        assert_eq!(task.raw, "2026-04-01 already dated");
    }

    #[test]
    fn canonicalize_does_not_prepend_date_when_priority_leads() {
        let task = canonicalize_line("(A) urgent thing", today()).unwrap();
        assert_eq!(task.priority, Some('A'));
        assert!(task.created_date.is_none());
    }

    #[test]
    fn canonicalize_preserves_done_lines() {
        let task = canonicalize_line("x 2026-05-10 2026-05-01 wrap-up", today()).unwrap();
        assert!(task.done);
        assert_eq!(task.done_date.as_deref(), Some("2026-05-10"));
    }

    #[test]
    fn canonicalize_rejects_empty() {
        assert_eq!(
            canonicalize_line("", today()).unwrap_err(),
            todo::ParseError::Empty,
        );
        assert_eq!(
            canonicalize_line("   \t  ", today()).unwrap_err(),
            todo::ParseError::Empty,
        );
    }

    #[test]
    fn canonicalize_natural_language_with_project_and_priority() {
        // Prose with priority, project, recurrence, threshold — should
        // produce a fully canonical line with creation date.
        let task = canonicalize_line(
            "Pay rent monthly on the first show 3 days before project home",
            today(),
        )
        .unwrap();
        assert_eq!(task.priority, None);
        assert!(task.projects.contains(&"home".to_string()));
        assert_eq!(task.due.as_deref(), Some("2026-06-01"));
        assert_eq!(task.rec.as_deref(), Some("+1m"));
        assert_eq!(task.threshold.as_deref(), Some("-3d"));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
    }

    #[test]
    fn finalize_prepends_date_to_bare_body() {
        let task = finalize_line("buy bread", "2026-05-13").unwrap();
        assert_eq!(task.raw, "2026-05-13 buy bread");
    }

    #[test]
    fn finalize_skips_date_on_priority() {
        let task = finalize_line("(B) cleanup", "2026-05-13").unwrap();
        assert_eq!(task.raw, "(B) cleanup");
    }

    #[test]
    fn finalize_rejects_empty() {
        assert_eq!(
            finalize_line("", "2026-05-13").unwrap_err(),
            todo::ParseError::Empty,
        );
    }
}
