//! Persisted UI preferences, located per the XDG Base Directory Specification.
//!
//! Path: `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/config.toml`
//!
//! Format: simple `key = value` lines. Lines starting with `#` and blank lines
//! are ignored. Unknown keys are ignored so older binaries won't choke on
//! newer files. Load failures fall back to defaults silently; save failures
//! print to stderr but never panic.

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::app::WeekStart;
use crate::app::{Density, Sort};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Config {
    pub theme: Option<String>,
    pub density: Option<Density>,
    pub sort: Option<Sort>,
    pub show_left: Option<bool>,
    pub show_right: Option<bool>,
    pub show_line_num: Option<bool>,
    pub show_status_bar: Option<bool>,
    pub show_done: Option<bool>,
    pub show_future: Option<bool>,
    /// Wrap long task rows onto continuation lines in the list and archive
    /// views instead of truncating them at the pane edge. Off by default so
    /// the dense single-line layout stays exactly as before.
    pub wrap_rows: Option<bool>,
    /// 64-character lowercase-hex token gating the in-TUI capture server.
    /// Persisted across sessions so phone bookmarks survive a relaunch.
    /// Stored on disk; only meaningful for LAN access, but flagged here
    /// so readers can decide whether to scrub it from shared dotfiles.
    pub share_token: Option<String>,
    /// Port the capture server binds to on first `s` press. Persisted
    /// so the same QR survives across sessions. If the port is taken on
    /// a future launch, the server falls back to an OS-assigned port
    /// and rewrites this field.
    pub share_port: Option<u16>,
    /// User-defined saved searches, as `(name, query)` pairs in file
    /// order. Serialized one-per-line as `filter.<name> = <query>`.
    /// The query is a `/`-search needle (subsequence match on the task
    /// body); see `App::save_current_filter_as`.
    pub filters: Vec<(String, String)>,
    /// Directory used by note actions for relative `note:<path>` tokens and
    /// generated task notes. Serialized as `notes_dir = ~/notes`.
    pub notes_dir: Option<String>,
    /// Metadata keys whose `key:value` tokens are omitted from rendered
    /// task rows (list + archive). The line is stored on disk untouched;
    /// this only affects display. Serialized as `hide_keys = a, b, c`.
    pub hidden_keys: Vec<String>,
    pub week_start: Option<WeekStart>,
}

impl Config {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    pub fn save(&self) -> io::Result<()> {
        let path =
            Self::path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config dir"))?;
        self.save_to(&path)
    }

    /// Read a config file from an explicit path. Missing or unreadable files
    /// fall back to defaults so callers don't need to distinguish first-run
    /// from corrupt files.
    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => parse(&s),
            Err(_) => Self::default(),
        }
    }

    /// Write a config file to an explicit path. Writes directly through a
    /// symlink when the path is one (preserving the link), otherwise uses
    /// atomic tmp-then-rename so concurrent writers don't clobber each other.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        let body = serialize(self);
        if path.is_symlink() {
            // Write directly through the symlink to preserve it.
            return fs::write(path, body);
        }
        // Atomic write: tmp-then-rename.
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let stem = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "config".to_string());
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_name = format!(".{stem}.tmp.{}.{}", std::process::id(), n);
        let tmp = path
            .parent()
            .map(|p| p.join(&tmp_name))
            .unwrap_or_else(|| PathBuf::from(&tmp_name));
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Resolve `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/config.toml`.
    /// Returns None only when neither XDG_CONFIG_HOME nor HOME is set.
    pub fn path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(Self::path_in(&base))
    }

    /// Load config from an explicit path, returning an error on read or parse
    /// failure instead of silently defaulting. Used by the hot-reload watcher
    /// so a corrupt file at runtime doesn't reset prefs to defaults.
    pub fn load_strict(path: &Path) -> Result<Self, String> {
        let s = fs::read_to_string(path).map_err(|e| format!("cannot read config: {e}"))?;
        Ok(parse(&s))
    }

    /// Construct the config path under an explicit XDG-style base directory.
    /// Used by tests to avoid mutating process env.
    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxedo").join("config.toml")
    }
}

fn parse(s: &str) -> Config {
    let mut c = Config::default();
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = unquote(v.trim());
        match k {
            "theme" => c.theme = Some(v.to_string()),
            "density" => c.density = v.parse().ok(),
            "sort" => c.sort = v.parse().ok(),
            "show_left" => c.show_left = parse_bool(v),
            "show_right" => c.show_right = parse_bool(v),
            "show_line_num" => c.show_line_num = parse_bool(v),
            "show_status_bar" => c.show_status_bar = parse_bool(v),
            "show_done" => c.show_done = parse_bool(v),
            "show_future" => c.show_future = parse_bool(v),
            "wrap_rows" => c.wrap_rows = parse_bool(v),
            // Reject anything that isn't a valid hex token so we don't
            // carry forward a corrupt value that the server would later
            // refuse to compare against.
            "share_token" if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) => {
                c.share_token = Some(v.to_ascii_lowercase());
            }
            "share_port" => c.share_port = v.parse().ok(),
            "notes_dir" if !v.trim().is_empty() => c.notes_dir = Some(v.to_string()),
            // Comma-separated key list; surrounding whitespace trimmed and
            // empty entries (trailing/double comma) dropped so a hand-
            // edited line is forgiving.
            "hide_keys" => {
                c.hidden_keys = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "week_start" => c.week_start = v.parse().ok(),
            // Saved searches: `filter.<name> = <query>`. The name is the
            // (trimmed) text after the `filter.` prefix; the query is the
            // (unquoted) value, which may itself contain `=`. A repeated
            // name collapses to one entry, last value wins, position of
            // the first occurrence kept — matching `upsert`'s semantics.
            _ if k
                .strip_prefix("filter.")
                .is_some_and(|n| !n.trim().is_empty()) =>
            {
                let name = k.strip_prefix("filter.").expect("checked above").trim();
                match c.filters.iter_mut().find(|(n, _)| n.as_str() == name) {
                    Some((_, q)) => *q = v.to_string(),
                    None => c.filters.push((name.to_string(), v.to_string())),
                }
            }
            _ => {} // forward-compatible: ignore unknowns
        }
    }
    c
}

fn serialize(c: &Config) -> String {
    let mut out = String::from("# tuxedo config\n");
    // writeln! against a String is infallible; the unwrap can never fire.
    if let Some(v) = &c.theme {
        let _ = writeln!(out, "theme = {v}");
    }
    if let Some(v) = c.density {
        let _ = writeln!(out, "density = {v}");
    }
    if let Some(v) = c.sort {
        let _ = writeln!(out, "sort = {v}");
    }
    if let Some(v) = c.show_left {
        let _ = writeln!(out, "show_left = {v}");
    }
    if let Some(v) = c.show_right {
        let _ = writeln!(out, "show_right = {v}");
    }
    if let Some(v) = c.show_line_num {
        let _ = writeln!(out, "show_line_num = {v}");
    }
    if let Some(v) = c.show_status_bar {
        let _ = writeln!(out, "show_status_bar = {v}");
    }
    if let Some(v) = c.show_done {
        let _ = writeln!(out, "show_done = {v}");
    }
    if let Some(v) = c.show_future {
        let _ = writeln!(out, "show_future = {v}");
    }
    if let Some(v) = c.wrap_rows {
        let _ = writeln!(out, "wrap_rows = {v}");
    }
    if let Some(v) = &c.share_token {
        let _ = writeln!(out, "share_token = {v}");
    }
    if let Some(v) = c.share_port {
        let _ = writeln!(out, "share_port = {v}");
    }
    for (name, query) in &c.filters {
        let _ = writeln!(out, "filter.{name} = {query}");
    }
    if let Some(v) = &c.notes_dir {
        let _ = writeln!(out, "notes_dir = {v}");
    }
    if !c.hidden_keys.is_empty() {
        let _ = writeln!(out, "hide_keys = {}", c.hidden_keys.join(", "));
    }
    if let Some(v) = c.week_start {
        let _ = writeln!(out, "week_start = {v}");
    }
    out
}

fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let c = Config {
            theme: Some("Nord".into()),
            density: Some(Density::Cozy),
            sort: Some(Sort::Due),
            show_left: Some(false),
            show_right: Some(true),
            show_line_num: Some(false),
            show_status_bar: Some(true),
            show_done: Some(true),
            show_future: Some(true),
            wrap_rows: Some(true),
            share_token: Some("a".repeat(64)),
            share_port: Some(18080),
            filters: vec![
                ("weekly".into(), "report".into()),
                ("waiting".into(), "@waiting due=2026".into()),
            ],
            notes_dir: Some("~/notes".into()),
            hidden_keys: vec!["uid".into(), "sync".into()],
            week_start: Some(WeekStart::Sunday),
        };

        let s = serialize(&c);
        let parsed = parse(&s);
        assert_eq!(parsed, c);
    }

    #[test]
    fn filters_round_trip() {
        let c = Config {
            filters: vec![
                ("weekly".into(), "report".into()),
                // Query may contain '=' — split_once('=') only splits the
                // first '=', so the key/value boundary stays unambiguous.
                ("waiting".into(), "due=2026".into()),
            ],
            ..Config::default()
        };
        let s = serialize(&c);
        let parsed = parse(&s);
        assert_eq!(parsed.filters, c.filters);
    }

    #[test]
    fn filter_lines_parsed_and_others_ignored() {
        let s = "theme = Dawn\nfilter.weekly = report\nbogus = 42\nfilter.waiting = @waiting\n";
        let c = parse(s);
        assert_eq!(c.theme.as_deref(), Some("Dawn"));
        assert_eq!(
            c.filters,
            vec![
                ("weekly".to_string(), "report".to_string()),
                ("waiting".to_string(), "@waiting".to_string()),
            ]
        );
    }

    #[test]
    fn filter_name_trimmed_on_parse() {
        // A hand-edited `filter.  weekly  = report` must yield the same
        // name an in-app save would, so the picker/panel don't show a
        // padded duplicate.
        let c = parse("filter.  weekly  = report\n");
        assert_eq!(
            c.filters,
            vec![("weekly".to_string(), "report".to_string())]
        );
    }

    #[test]
    fn duplicate_filter_lines_dedup_last_wins_in_place() {
        // Two `filter.weekly` lines (hand-edited config) collapse to one
        // entry, last value wins, keeping the first occurrence's position.
        let c = parse("filter.weekly = first\nfilter.other = x\nfilter.weekly = second\n");
        assert_eq!(
            c.filters,
            vec![
                ("weekly".to_string(), "second".to_string()),
                ("other".to_string(), "x".to_string()),
            ]
        );
    }

    #[test]
    fn hide_keys_parsed_trimmed_and_round_trips() {
        // Comma-separated, surrounding whitespace trimmed, empty entries
        // dropped (trailing comma / double comma from a hand-edited file).
        let c = parse("hide_keys = uid ,  sync ,,\n");
        assert_eq!(c.hidden_keys, vec!["uid".to_string(), "sync".to_string()]);
        // serialize -> parse must reproduce the same list.
        let reparsed = parse(&serialize(&c));
        assert_eq!(reparsed.hidden_keys, c.hidden_keys);
        // Absent key leaves the list empty, not None/garbage.
        assert!(parse("theme = Dawn\n").hidden_keys.is_empty());
    }

    #[test]
    fn rejects_malformed_share_token() {
        // Too short, non-hex, uppercase OK only after normalization to
        // lower; whitespace inside. Each of these must be dropped so a
        // hand-edited config doesn't carry forward an invalid token.
        let s = "share_token = abcd\nshare_port = notanumber\n";
        let c = parse(s);
        assert_eq!(c.share_token, None);
        assert_eq!(c.share_port, None);
    }

    #[test]
    fn share_token_normalized_to_lowercase() {
        let s = format!("share_token = {}\n", "F".repeat(64));
        let c = parse(&s);
        assert_eq!(c.share_token.as_deref(), Some("f".repeat(64).as_str()));
    }

    #[test]
    fn unknown_keys_ignored() {
        let s = "theme = Dawn\nbogus = 42\nshow_left = false\n";
        let c = parse(s);
        assert_eq!(c.theme.as_deref(), Some("Dawn"));
        assert_eq!(c.show_left, Some(false));
        assert_eq!(c.density, None);
    }

    #[test]
    fn comments_and_blanks_skipped() {
        let s = "# header\n\n  # indented comment\ntheme = Matrix\n";
        let c = parse(s);
        assert_eq!(c.theme.as_deref(), Some("Matrix"));
    }

    #[test]
    fn quoted_values_unquoted() {
        let s = "theme = \"Muted Slate\"\n";
        let c = parse(s);
        assert_eq!(c.theme.as_deref(), Some("Muted Slate"));
    }

    #[test]
    fn parses_bool_aliases() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    /// Exercise the on-disk save/load round-trip via an explicit base path,
    /// so the test doesn't mutate process env (which is `unsafe` and races
    /// every other test that reads env, regardless of XDG_CONFIG_HOME).
    #[test]
    fn save_then_load_via_explicit_path() {
        let base = std::env::temp_dir().join(format!(
            "tuxedo-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        let path = Config::path_in(&base);
        assert!(path.starts_with(&base));
        assert!(path.ends_with("tuxedo/config.toml"));

        let written = Config {
            theme: Some("Dawn".into()),
            density: Some(Density::Compact),
            sort: Some(Sort::File),
            show_left: Some(false),
            show_right: Some(false),
            show_line_num: Some(true),
            show_status_bar: Some(false),
            show_done: Some(true),
            show_future: Some(false),
            wrap_rows: Some(false),
            share_token: None,
            share_port: None,
            filters: vec![("errand".into(), "@errand".into())],
            notes_dir: Some("/tmp/notes".into()),
            hidden_keys: vec!["uid".into()],
            week_start: Some(WeekStart::Sunday),
        };
        written.save_to(&path).expect("save should succeed");
        assert!(path.exists());
        let loaded = Config::load_from(&path);
        assert_eq!(loaded, written);
        let _ = fs::remove_dir_all(&base);
    }
}
