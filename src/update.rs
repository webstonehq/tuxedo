//! Update-availability check and `tuxedo update` subcommand.
//!
//! Two pieces:
//!
//! 1. [`run`] — handler for `tuxedo update`. Detects how tuxedo was installed
//!    (Homebrew, Cargo, plain binary) and prints the exact command the user
//!    should run. Does not execute it: we don't want to surprise users with a
//!    `brew upgrade` or a binary self-replace.
//!
//! 2. [`spawn_check`] — background thread invoked at TUI startup that consults
//!    a cache under `$XDG_CACHE_HOME/tuxedo/latest_version.json`. If the cache
//!    is missing or older than 24h, it shells out to `curl` to read the
//!    `tag_name` of the latest GitHub release, rewrites the cache, and returns
//!    the tag through an mpsc channel. The TUI's status bar reads it and
//!    appends an `↑ <version>` hint when newer than the running build. All
//!    failures are silent — a stale or missing cache simply means no hint.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How tuxedo appears to have been installed, judged by the path of the
/// currently-running executable. Used to recommend the right upgrade command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Homebrew,
    Cargo,
    Binary,
    Unknown,
}

/// Run the `tuxedo update` subcommand: detect install method and print
/// instructions. Exits with code 0 on success; the caller (`main`) should
/// `return Ok(())` after invoking this.
pub fn run() -> io::Result<()> {
    let exe = std::env::current_exe().ok();
    let kind = exe
        .as_deref()
        .map(detect_kind)
        .unwrap_or(InstallKind::Unknown);
    let current = env!("CARGO_PKG_VERSION");
    println!("tuxedo {current}");
    if let Some(p) = &exe {
        println!("installed at: {}", p.display());
    }
    println!();
    match kind {
        InstallKind::Homebrew => {
            println!("Looks like a Homebrew install. Update with:");
            println!();
            println!("    brew update && brew upgrade webstonehq/tap/tuxedo");
        }
        InstallKind::Cargo => {
            println!("Looks like a `cargo install` build. Update with:");
            println!();
            println!("    cargo install --git https://github.com/webstonehq/tuxedo --force");
        }
        InstallKind::Binary => {
            println!("Looks like a downloaded binary. Grab the latest from:");
            println!();
            println!("    https://github.com/webstonehq/tuxedo/releases/latest");
            println!();
            println!("...and replace the file above.");
        }
        InstallKind::Unknown => {
            println!("Could not detect the install method. Options:");
            println!();
            println!("    brew upgrade webstonehq/tap/tuxedo");
            println!("    cargo install --git https://github.com/webstonehq/tuxedo --force");
            println!("    https://github.com/webstonehq/tuxedo/releases/latest");
        }
    }
    Ok(())
}

/// Classify an executable path into an [`InstallKind`]. Exposed for tests.
pub fn detect_kind(exe: &Path) -> InstallKind {
    let s = exe.to_string_lossy();
    if s.contains("/Cellar/")
        || s.starts_with("/opt/homebrew/")
        || s.starts_with("/usr/local/Homebrew/")
        || s.contains("/homebrew/Cellar/")
        || s.contains("/linuxbrew/")
    {
        return InstallKind::Homebrew;
    }
    if s.contains("/.cargo/bin/") || s.contains("\\.cargo\\bin\\") {
        return InstallKind::Cargo;
    }
    // A bare /usr/local/bin/tuxedo could be either a Homebrew shim (older
    // macOS) or a manual download. Without more signal, treat it as a binary.
    if !s.is_empty() {
        return InstallKind::Binary;
    }
    InstallKind::Unknown
}

/// Spawn the background update check. Returns a receiver that yields exactly
/// one message — `Some(tag)` if a cached or freshly-fetched tag is available,
/// otherwise `None`. The receiver is dropped when the thread exits, so a
/// disconnect on `try_recv` means "give up, nothing's coming".
pub fn spawn_check() -> Receiver<Option<String>> {
    let (tx, rx) = mpsc::sync_channel::<Option<String>>(1);
    thread::spawn(move || {
        let result = check_for_update();
        let _ = tx.send(result);
    });
    rx
}

const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long to honor a cached *failure* (empty `tag`) before trying the
/// network again. Short enough to recover within an hour, long enough to
/// stop hammering GitHub once anonymous API calls have been rate-limited
/// (which is what burned us once during testing).
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const CURL_TIMEOUT_SECS: u64 = 5;
const RELEASE_URL: &str = "https://api.github.com/repos/webstonehq/tuxedo/releases/latest";

fn check_for_update() -> Option<String> {
    let cache_path = cache_path();
    let now = now_epoch();
    // Honor the cache for both success and failure. A non-empty tag within
    // CACHE_TTL is a success entry; an empty tag within NEGATIVE_CACHE_TTL
    // is a "we just tried and it failed, don't pummel GitHub again" marker.
    if let Some(p) = &cache_path
        && let Some((ts, tag)) = read_cache(p)
    {
        let age = now.saturating_sub(ts);
        if !tag.is_empty() && age < CACHE_TTL.as_secs() {
            return Some(tag);
        }
        if tag.is_empty() && age < NEGATIVE_CACHE_TTL.as_secs() {
            return None;
        }
    }
    // Cache is stale, missing, or expired-negative — try the network. Cache
    // either outcome so we don't retry on every launch when offline or
    // rate-limited.
    let tag = fetch_latest_body().and_then(|b| parse_tag_from_release_json(&b));
    if let Some(p) = &cache_path {
        let _ = write_cache(p, now, tag.as_deref().unwrap_or(""));
    }
    tag
}

fn fetch_latest_body() -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-m",
            &CURL_TIMEOUT_SECS.to_string(),
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            concat!("tuxedo/", env!("CARGO_PKG_VERSION")),
            RELEASE_URL,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Pull the `tag_name` value out of a GitHub release JSON payload. Doesn't
/// pull in a JSON parser — release payloads are well-formed enough that a
/// targeted string scan suffices, and a malformed payload simply returns
/// `None`. Exposed for unit testing.
pub fn parse_tag_from_release_json(body: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after_colon = &rest[colon + 1..];
    let q = after_colon.find('"')?;
    let after_q = &after_colon[q + 1..];
    let end = after_q.find('"')?;
    let tag = &after_q[..end];
    if tag.is_empty() {
        return None;
    }
    Some(tag.to_string())
}

/// True when `latest` is a strictly newer version than `current`, comparing
/// each dot-separated segment numerically. A leading `v` (e.g. `v2026.5.5`)
/// is stripped on both sides. Non-numeric segments fall back to lexicographic
/// comparison of that segment, so a future suffix like `2026.5.5-rc1` won't
/// crash — it just compares the strings.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let l = latest.trim_start_matches('v');
    let c = current.trim_start_matches('v');
    let mut li = l.split('.');
    let mut ci = c.split('.');
    loop {
        match (li.next(), ci.next()) {
            (None, None) => return false,
            (Some(a), None) => return a.parse::<u64>().is_ok_and(|n| n > 0) || !a.is_empty(),
            (None, Some(_)) => return false,
            (Some(a), Some(b)) => match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(x), Ok(y)) if x != y => return x > y,
                (Ok(_), Ok(_)) => continue,
                _ => match a.cmp(b) {
                    std::cmp::Ordering::Greater => return true,
                    std::cmp::Ordering::Less => return false,
                    std::cmp::Ordering::Equal => continue,
                },
            },
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    let base = xdg_cache_home()?;
    Some(base.join("tuxedo").join("latest_version.json"))
}

fn xdg_cache_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_CACHE_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache"))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read a cache file. Returns `(epoch_secs, tag)` on success. Format is a
/// tiny JSON-ish payload: `{"checked_at": <int>, "tag": "<str>"}`. An empty
/// `tag` is a sentinel for a cached failure (see [`NEGATIVE_CACHE_TTL`]);
/// callers distinguish success from failure on the returned string.
fn read_cache(path: &Path) -> Option<(u64, String)> {
    let body = std::fs::read_to_string(path).ok()?;
    let ts = scan_int(&body, "\"checked_at\"")?;
    let tag = scan_str(&body, "\"tag\"").unwrap_or_default();
    Some((ts, tag))
}

fn write_cache(path: &Path, now: u64, tag: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!("{{\"checked_at\": {now}, \"tag\": \"{tag}\"}}\n");
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn scan_int(body: &str, key: &str) -> Option<u64> {
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

fn scan_str(body: &str, key: &str) -> Option<String> {
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after_colon = &rest[colon + 1..];
    let q = after_colon.find('"')?;
    let after_q = &after_colon[q + 1..];
    let end = after_q.find('"')?;
    Some(after_q[..end].to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_kind_homebrew_paths() {
        assert_eq!(
            detect_kind(&PathBuf::from("/opt/homebrew/bin/tuxedo")),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/opt/homebrew/Cellar/tuxedo/2026.5.3/bin/tuxedo"
            )),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/usr/local/Cellar/tuxedo/2026.5.3/bin/tuxedo"
            )),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/home/linuxbrew/.linuxbrew/Cellar/tuxedo/2026.5.3/bin/tuxedo"
            )),
            InstallKind::Homebrew
        );
    }

    #[test]
    fn detect_kind_cargo_path() {
        assert_eq!(
            detect_kind(&PathBuf::from("/home/m/.cargo/bin/tuxedo")),
            InstallKind::Cargo
        );
    }

    #[test]
    fn detect_kind_falls_back_to_binary() {
        assert_eq!(
            detect_kind(&PathBuf::from("/usr/local/bin/tuxedo")),
            InstallKind::Binary
        );
        assert_eq!(
            detect_kind(&PathBuf::from("/tmp/tuxedo")),
            InstallKind::Binary
        );
    }

    #[test]
    fn is_newer_handles_calver_segments() {
        // Same version
        assert!(!is_newer("2026.5.3", "2026.5.3"));
        // Patch bump
        assert!(is_newer("2026.5.4", "2026.5.3"));
        assert!(!is_newer("2026.5.3", "2026.5.4"));
        // Crucially: numeric (not lex) compare on patches >= 10
        assert!(is_newer("2026.5.10", "2026.5.9"));
        assert!(!is_newer("2026.5.9", "2026.5.10"));
        // Month rollover
        assert!(is_newer("2026.10.1", "2026.9.5"));
        // Year rollover
        assert!(is_newer("2027.1.1", "2026.12.31"));
    }

    #[test]
    fn is_newer_strips_v_prefix() {
        assert!(is_newer("v2026.5.4", "2026.5.3"));
        assert!(is_newer("2026.5.4", "v2026.5.3"));
        assert!(!is_newer("v2026.5.3", "v2026.5.3"));
    }

    #[test]
    fn is_newer_handles_segment_count_mismatch() {
        // "2026.5" vs "2026.5.0" — equal in spirit, but with a non-zero suffix
        // the longer one is newer.
        assert!(is_newer("2026.5.1", "2026.5"));
        assert!(!is_newer("2026.5", "2026.5.1"));
    }

    #[test]
    fn parse_tag_extracts_first_tag_name() {
        let body = r#"{"url":"x","tag_name":"v2026.5.5","name":"2026.5.5"}"#;
        assert_eq!(
            parse_tag_from_release_json(body).as_deref(),
            Some("v2026.5.5")
        );
    }

    #[test]
    fn parse_tag_with_whitespace_and_extra_keys() {
        let body = r#"
        {
          "url": "x",
          "tag_name" : "2026.5.10" ,
          "draft": false
        }
        "#;
        assert_eq!(
            parse_tag_from_release_json(body).as_deref(),
            Some("2026.5.10")
        );
    }

    #[test]
    fn parse_tag_returns_none_on_missing_field() {
        let body = r#"{"name":"hi"}"#;
        assert!(parse_tag_from_release_json(body).is_none());
    }

    #[test]
    fn cache_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-update-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest_version.json");
        write_cache(&path, 1_700_000_000, "2026.5.5").unwrap();
        let (ts, tag) = read_cache(&path).unwrap();
        assert_eq!(ts, 1_700_000_000);
        assert_eq!(tag, "2026.5.5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_cache_none_on_missing_file() {
        assert!(read_cache(&PathBuf::from("/tmp/does-not-exist-xyzzy")).is_none());
    }

    #[test]
    fn cache_round_trip_empty_tag_is_negative_marker() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-update-neg-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest_version.json");
        // A failed check writes an empty tag.
        write_cache(&path, 1_700_000_000, "").unwrap();
        let (ts, tag) = read_cache(&path).unwrap();
        assert_eq!(ts, 1_700_000_000);
        assert_eq!(tag, "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
