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

    /// Write a config file to an explicit path. Atomic via tmp-then-rename.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serialize(self);
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, body)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Resolve `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/config.toml`.
    /// Returns None only when neither XDG_CONFIG_HOME nor HOME is set.
    pub fn path() -> Option<PathBuf> {
        let base = xdg_config_home()?;
        Some(Self::path_in(&base))
    }

    /// Construct the config path under an explicit XDG-style base directory.
    /// Used by tests to avoid mutating process env.
    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxedo").join("config.toml")
    }
}

/// Resolve the XDG base config directory. Per the XDG Base Directory Spec,
/// `XDG_CONFIG_HOME` MUST be an absolute path; relative values are to be
/// ignored. We honor that and warn once so users debugging path resolution
/// can see why their relative override didn't take effect.
fn xdg_config_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_CONFIG_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        // Warn to stderr; this fires before the TUI takes over so it lands
        // in the user's terminal scrollback, not the alt-screen.
        eprintln!(
            "tuxedo: ignoring non-absolute XDG_CONFIG_HOME={:?} (per XDG spec)",
            p.display()
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config"))
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
        };
        let s = serialize(&c);
        let parsed = parse(&s);
        assert_eq!(parsed, c);
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
        };
        written.save_to(&path).expect("save should succeed");
        assert!(path.exists());
        let loaded = Config::load_from(&path);
        assert_eq!(loaded, written);
        let _ = fs::remove_dir_all(&base);
    }
}
