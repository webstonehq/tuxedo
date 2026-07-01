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
/// creation-date insertion, validation. Returns the parsed [`todo::Task`]
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
/// rewrite (the caller has already produced canonical form), then give the
/// line a creation date if it lacks one. A done line, or one that already
/// carries a creation date, is returned unchanged; otherwise today's date is
/// inserted after any leading priority token and the result validated. The
/// add-prompt's second-Enter save path uses this directly, since the draft
/// buffer is already canonical after the first-Enter preview.
pub fn finalize_line(text: &str, today_str: &str) -> Result<todo::Task, todo::ParseError> {
    let text = text.trim();
    let mut task = todo::parse_line(text)?;

    if task.done || task.created_date.is_some() {
        return Ok(task);
    }

    // A creation date is only worth inserting when `today_str` is a canonical
    // `YYYY-MM-DD`. Callers may hand us a malformed `today` (see the defensive
    // fallback in `Store::add_with`); splicing a non-date in would push a bogus
    // token into the body. The length check pins the zero-padded form: chrono
    // also accepts `2026-5-13`, which a later re-parse would not recognize as a
    // creation date, so without it the reused task below would drift from a
    // fresh parse.
    if today_str.len() != 10 || NaiveDate::parse_from_str(today_str, "%Y-%m-%d").is_err() {
        return Ok(task);
    }

    let stripped = todo::strip_priority(text);
    let body = stripped.trim_start();
    if todo::starts_with_iso_date(body) {
        return Ok(task);
    }

    // Reuse the task we already parsed rather than re-parsing the rebuilt line.
    // Inserting the date between the priority and the body leaves every
    // body-derived field (projects, contexts, due, rec, threshold, notes)
    // untouched, so only `raw`, `clean_raw`, and `created_date` differ.
    let prefix = &text[..text.len() - stripped.len()];
    let raw = format!("{prefix}{today_str} {body}");
    task.clean_raw = todo::body_after_quoted_kv(&raw);
    task.created_date = Some(today_str.to_string());
    task.raw = raw;
    Ok(task)
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
    fn canonicalize_prepends_date_after_priority() {
        let task = canonicalize_line("(A) urgent thing", today()).unwrap();
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
        assert_eq!(task.raw, "(A) 2026-05-13 urgent thing");
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
    fn finalize_inserts_date_after_priority() {
        let task = finalize_line("(B) cleanup", "2026-05-13").unwrap();
        assert_eq!(task.raw, "(B) 2026-05-13 cleanup");
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
    }

    #[test]
    fn finalize_normalizes_extra_space_after_priority() {
        let task = finalize_line("(A)  urgent", "2026-05-13").unwrap();
        assert_eq!(task.raw, "(A) 2026-05-13 urgent");
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-13"));
    }

    #[test]
    fn finalize_keeps_existing_date_after_priority() {
        let task = finalize_line("(C) 2026-04-01 already dated", "2026-05-13").unwrap();
        assert_eq!(task.raw, "(C) 2026-04-01 already dated");
        assert_eq!(task.created_date.as_deref(), Some("2026-04-01"));
    }

    #[test]
    fn finalize_leaves_date_shaped_invalid_prefix_untouched() {
        let task = finalize_line("9999-99-99 bogus", "2026-05-13").unwrap();
        assert_eq!(task.raw, "9999-99-99 bogus");
        assert!(task.created_date.is_none());
    }

    #[test]
    fn finalize_leaves_invalid_date_after_priority_untouched() {
        let task = finalize_line("(A) 1234-56-78 order parts", "2026-05-13").unwrap();
        assert_eq!(task.raw, "(A) 1234-56-78 order parts");
        assert_eq!(task.priority, Some('A'));
        assert!(task.created_date.is_none());
    }

    #[test]
    fn finalize_reuse_matches_fresh_parse() {
        // The date-insertion path reuses the first parse instead of parsing
        // the rebuilt line again. Guard that shortcut: it must produce exactly
        // what a fresh parse of the canonical line would, every field included.
        let line = r#"(A) ship +rel @work due:2026-06-01 rec:+1w t:2026-05-20 note:"call ops""#;
        let got = finalize_line(line, "2026-05-13").unwrap();
        let want = todo::parse_line(
            r#"(A) 2026-05-13 ship +rel @work due:2026-06-01 rec:+1w t:2026-05-20 note:"call ops""#,
        )
        .unwrap();
        assert_eq!(got.raw, want.raw);
        assert_eq!(got.clean_raw, want.clean_raw);
        assert_eq!(got.created_date, want.created_date);
        assert_eq!(got.priority, want.priority);
        assert_eq!(got.projects, want.projects);
        assert_eq!(got.contexts, want.contexts);
        assert_eq!(got.due, want.due);
        assert_eq!(got.rec, want.rec);
        assert_eq!(got.threshold, want.threshold);
        assert_eq!(got.notes, want.notes);
        assert_eq!(got.done, want.done);
        assert_eq!(got.done_date, want.done_date);
    }

    #[test]
    fn finalize_skips_injection_when_today_noncanonical() {
        // chrono parses `2026-5-13`, but a re-parse would not treat it as a
        // creation date; the length guard keeps us from inserting it.
        let task = finalize_line("(A) task", "2026-5-13").unwrap();
        assert_eq!(task.raw, "(A) task");
        assert!(task.created_date.is_none());
    }

    #[test]
    fn finalize_skips_injection_when_today_invalid() {
        // A malformed `today` must not be spliced into the body. Priority-led
        // lines (the path this change touches) and bare bodies alike are left
        // verbatim rather than gaining a bogus token.
        let task = finalize_line("(A) task", "not-a-date").unwrap();
        assert_eq!(task.raw, "(A) task");
        assert_eq!(task.priority, Some('A'));
        assert!(task.created_date.is_none());

        let bare = finalize_line("task", "not-a-date").unwrap();
        assert_eq!(bare.raw, "task");
        assert!(bare.created_date.is_none());
    }

    #[test]
    fn finalize_rejects_empty() {
        assert_eq!(
            finalize_line("", "2026-05-13").unwrap_err(),
            todo::ParseError::Empty,
        );
    }
}
