use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel: Color,
    pub border: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub cursor: Color,
    #[allow(dead_code)]
    pub selection: Color,
    pub statusbar: Color,
    pub status_fg: Color,
    pub mode_fg: Color,
    pub mode_bg: Color,
    pub pri_a: Color,
    pub pri_b: Color,
    pub pri_c: Color,
    pub pri_d: Color,
    pub pri_other: Color,
    pub project: Color,
    pub context: Color,
    pub due: Color,
    pub overdue: Color,
    pub today: Color,
    pub done: Color,
    pub selected: Color,
    pub matched: Color,
}

impl Theme {
    pub fn priority_color(&self, p: char) -> Color {
        match p {
            'A' => self.pri_a,
            'B' => self.pri_b,
            'C' => self.pri_c,
            'D' => self.pri_d,
            _ => self.pri_other,
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

pub const MUTED: Theme = Theme {
    name: "Muted Slate",
    bg: rgb(0x1a, 0x1d, 0x23),
    panel: rgb(0x1f, 0x23, 0x2b),
    border: rgb(0x2a, 0x2f, 0x38),
    fg: rgb(0xc8, 0xcc, 0xd4),
    dim: rgb(0x6b, 0x72, 0x80),
    accent: rgb(0x8a, 0xa9, 0xc9),
    cursor: rgb(0x3a, 0x41, 0x50),
    selection: rgb(0x2f, 0x39, 0x47),
    statusbar: rgb(0x25, 0x2a, 0x33),
    status_fg: rgb(0xa8, 0xb0, 0xbc),
    mode_fg: rgb(0x1a, 0x1d, 0x23),
    mode_bg: rgb(0x8a, 0xa9, 0xc9),
    pri_a: rgb(0xe0, 0x7a, 0x7a),
    pri_b: rgb(0xd4, 0xb0, 0x6a),
    pri_c: rgb(0x7a, 0xa6, 0x7a),
    pri_d: rgb(0x7a, 0x9e, 0xc9),
    pri_other: rgb(0x9a, 0x8f, 0xc4),
    project: rgb(0x7f, 0xb3, 0xa8),
    context: rgb(0xc8, 0x9a, 0x6e),
    due: rgb(0xd4, 0xb0, 0x6a),
    overdue: rgb(0xe0, 0x7a, 0x7a),
    today: rgb(0xe0, 0x7a, 0x7a),
    done: rgb(0x5a, 0x62, 0x70),
    selected: rgb(0x2f, 0x39, 0x47),
    matched: rgb(0xd4, 0xb0, 0x6a),
};

pub const DAWN: Theme = Theme {
    name: "Dawn",
    bg: rgb(0xfa, 0xf6, 0xf0),
    panel: rgb(0xf3, 0xed, 0xe2),
    border: rgb(0xe0, 0xd6, 0xc4),
    fg: rgb(0x3d, 0x35, 0x28),
    dim: rgb(0x8a, 0x7e, 0x6a),
    accent: rgb(0xa3, 0x5d, 0x3a),
    cursor: rgb(0xe8, 0xde, 0xc8),
    selection: rgb(0xed, 0xe0, 0xc8),
    statusbar: rgb(0xed, 0xe2, 0xcc),
    status_fg: rgb(0x5a, 0x4f, 0x3d),
    mode_fg: rgb(0xfa, 0xf6, 0xf0),
    mode_bg: rgb(0xa3, 0x5d, 0x3a),
    pri_a: rgb(0xb8, 0x48, 0x3a),
    pri_b: rgb(0xa3, 0x72, 0x2a),
    pri_c: rgb(0x5a, 0x7a, 0x3a),
    pri_d: rgb(0x3a, 0x6a, 0x8a),
    pri_other: rgb(0x7a, 0x4a, 0x8a),
    project: rgb(0x3a, 0x7a, 0x6a),
    context: rgb(0xa3, 0x5d, 0x3a),
    due: rgb(0xa3, 0x72, 0x2a),
    overdue: rgb(0xb8, 0x48, 0x3a),
    today: rgb(0xb8, 0x48, 0x3a),
    done: rgb(0xa8, 0x9a, 0x82),
    selected: rgb(0xed, 0xe0, 0xc8),
    matched: rgb(0xa3, 0x72, 0x2a),
};

pub const NORD: Theme = Theme {
    name: "Nord",
    bg: rgb(0x2e, 0x34, 0x40),
    panel: rgb(0x3b, 0x42, 0x52),
    border: rgb(0x43, 0x4c, 0x5e),
    fg: rgb(0xd8, 0xde, 0xe9),
    dim: rgb(0x6c, 0x76, 0x86),
    accent: rgb(0x88, 0xc0, 0xd0),
    cursor: rgb(0x43, 0x4c, 0x5e),
    selection: rgb(0x43, 0x4c, 0x5e),
    statusbar: rgb(0x3b, 0x42, 0x52),
    status_fg: rgb(0xd8, 0xde, 0xe9),
    mode_fg: rgb(0x2e, 0x34, 0x40),
    mode_bg: rgb(0x88, 0xc0, 0xd0),
    pri_a: rgb(0xbf, 0x61, 0x6a),
    pri_b: rgb(0xeb, 0xcb, 0x8b),
    pri_c: rgb(0xa3, 0xbe, 0x8c),
    pri_d: rgb(0x81, 0xa1, 0xc1),
    pri_other: rgb(0xb4, 0x8e, 0xad),
    project: rgb(0xa3, 0xbe, 0x8c),
    context: rgb(0xd0, 0x87, 0x70),
    due: rgb(0xeb, 0xcb, 0x8b),
    overdue: rgb(0xbf, 0x61, 0x6a),
    today: rgb(0xbf, 0x61, 0x6a),
    done: rgb(0x4c, 0x56, 0x6a),
    selected: rgb(0x43, 0x4c, 0x5e),
    matched: rgb(0xeb, 0xcb, 0x8b),
};

pub const MATRIX: Theme = Theme {
    name: "Matrix",
    bg: rgb(0x0a, 0x12, 0x0a),
    panel: rgb(0x0f, 0x1a, 0x0f),
    border: rgb(0x1a, 0x2a, 0x1a),
    fg: rgb(0x7f, 0xcc, 0x7f),
    dim: rgb(0x3f, 0x6a, 0x3f),
    accent: rgb(0x9f, 0xff, 0x9f),
    cursor: rgb(0x1a, 0x2e, 0x1a),
    selection: rgb(0x1f, 0x3a, 0x1f),
    statusbar: rgb(0x0f, 0x1a, 0x0f),
    status_fg: rgb(0x7f, 0xcc, 0x7f),
    mode_fg: rgb(0x0a, 0x12, 0x0a),
    mode_bg: rgb(0x9f, 0xff, 0x9f),
    pri_a: rgb(0xff, 0x8c, 0x8c),
    pri_b: rgb(0xff, 0xd6, 0x6e),
    pri_c: rgb(0x9f, 0xff, 0x9f),
    pri_d: rgb(0x7f, 0xd0, 0xff),
    pri_other: rgb(0xcf, 0x9f, 0xff),
    project: rgb(0x9f, 0xff, 0x9f),
    context: rgb(0xff, 0xb5, 0x6e),
    due: rgb(0xff, 0xd6, 0x6e),
    overdue: rgb(0xff, 0x8c, 0x8c),
    today: rgb(0xff, 0x8c, 0x8c),
    done: rgb(0x3f, 0x6a, 0x3f),
    selected: rgb(0x1f, 0x3a, 0x1f),
    matched: rgb(0xff, 0xd6, 0x6e),
};

pub const TERMINAL: Theme = Theme {
    name: "Terminal",
    bg: Color::Reset,
    panel: Color::Reset,
    border: Color::DarkGray,
    fg: Color::Reset,
    dim: Color::DarkGray,
    accent: Color::Cyan,
    cursor: Color::DarkGray,
    selection: Color::DarkGray,
    statusbar: Color::Reset,
    status_fg: Color::Reset,
    mode_fg: Color::Black,
    mode_bg: Color::Cyan,
    pri_a: Color::Red,
    pri_b: Color::Yellow,
    pri_c: Color::Green,
    pri_d: Color::Blue,
    pri_other: Color::Magenta,
    project: Color::Green,
    context: Color::Yellow,
    due: Color::Yellow,
    overdue: Color::Red,
    today: Color::Red,
    done: Color::DarkGray,
    selected: Color::DarkGray,
    matched: Color::Yellow,
};

pub const BUILT_IN: &[&Theme] = &[&MUTED, &DAWN, &NORD, &MATRIX, &TERMINAL];

static REGISTRY: OnceLock<Vec<&'static Theme>> = OnceLock::new();

/// Install the runtime theme registry. Built-in themes are prepended in their
/// canonical order, followed by `user_themes` in the order supplied. Each
/// user `Theme` is `Box::leak`ed to satisfy the `&'static Theme` contract
/// used throughout the UI; themes live for the whole program lifetime so the
/// leak is bounded and intentional.
///
/// Must be called once at startup, before any call to `all()` that needs to
/// see user themes. Subsequent calls are no-ops (the first init wins) so test
/// binaries with multiple integration tests don't race.
pub fn init(user_themes: Vec<Theme>) {
    let mut v: Vec<&'static Theme> = BUILT_IN.to_vec();
    for t in user_themes {
        v.push(Box::leak(Box::new(t)));
    }
    let _ = REGISTRY.set(v);
}

/// All themes available for selection, in cycle order. If `init` was never
/// called (unit tests, examples), falls back to the built-in set only.
pub fn all() -> &'static [&'static Theme] {
    REGISTRY.get_or_init(|| BUILT_IN.to_vec()).as_slice()
}

/// Resolve `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/themes`. Returns None
/// only when neither XDG_CONFIG_HOME nor HOME is set.
pub fn themes_dir() -> Option<PathBuf> {
    Some(crate::xdg::config_home()?.join("tuxedo").join("themes"))
}

/// Read every `*.toml` file in `dir`, parse it as a theme, and return the
/// successfully-loaded themes plus one warning per skipped file. Files are
/// processed in sorted filename order so the cycle order is deterministic.
///
/// Skip conditions (each produces a warning):
/// - file can't be read
/// - file doesn't contain all 26 required fields, or has an unparseable color
/// - theme `name` collides with a built-in or an already-loaded user theme
///
/// A non-existent directory is treated as "no themes" (no warning) — this is
/// the first-run case.
pub fn load_user_themes(dir: &Path) -> (Vec<Theme>, Vec<String>) {
    let mut themes = Vec::new();
    let mut warnings = Vec::new();

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (themes, warnings),
        Err(e) => {
            warnings.push(format!("themes dir: {e}"));
            return (themes, warnings);
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|r| r.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    files.sort();

    let mut seen_names: Vec<String> = BUILT_IN.iter().map(|t| t.name.to_string()).collect();

    for path in files {
        let display = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let body = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("themes/{display}: {e}"));
                continue;
            }
        };
        match parse_theme(&body) {
            Ok(theme) => {
                if seen_names.iter().any(|n| n == theme.name) {
                    warnings.push(format!(
                        "themes/{display}: name '{}' already in use, skipped",
                        theme.name
                    ));
                    continue;
                }
                seen_names.push(theme.name.to_string());
                themes.push(theme);
            }
            Err(e) => warnings.push(format!("themes/{display}: {e}")),
        }
    }

    (themes, warnings)
}

/// Parse a theme file body. The `name` field becomes the display name and is
/// leaked into a `&'static str` so the resulting `Theme` matches built-ins
/// structurally. All 26 color fields are required; missing or unparseable
/// fields produce a one-line error that names the offending field.
fn parse_theme(s: &str) -> Result<Theme, String> {
    let mut name: Option<String> = None;
    let mut colors: std::collections::BTreeMap<&'static str, Color> =
        std::collections::BTreeMap::new();

    for (lineno, raw) in s.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        let v = unquote(v.trim());
        if k == "name" {
            if v.is_empty() {
                return Err(format!("line {}: empty name", lineno + 1));
            }
            name = Some(v.to_string());
            continue;
        }
        if let Some(field) = match_color_field(k) {
            let color = parse_color(v).ok_or_else(|| {
                format!(
                    "line {}: field '{field}' has invalid color '{v}' (expected #rrggbb or reset/transparent)",
                    lineno + 1
                )
            })?;
            colors.insert(field, color);
        }
        // unknown keys are ignored for forward compatibility
    }

    let name = name.ok_or_else(|| "missing field 'name'".to_string())?;

    let get = |field: &'static str| -> Result<Color, String> {
        colors
            .get(field)
            .copied()
            .ok_or_else(|| format!("missing field '{field}'"))
    };

    let leaked_name: &'static str = Box::leak(name.into_boxed_str());

    Ok(Theme {
        name: leaked_name,
        bg: get("bg")?,
        panel: get("panel")?,
        border: get("border")?,
        fg: get("fg")?,
        dim: get("dim")?,
        accent: get("accent")?,
        cursor: get("cursor")?,
        selection: get("selection")?,
        statusbar: get("statusbar")?,
        status_fg: get("status_fg")?,
        mode_fg: get("mode_fg")?,
        mode_bg: get("mode_bg")?,
        pri_a: get("pri_a")?,
        pri_b: get("pri_b")?,
        pri_c: get("pri_c")?,
        pri_d: get("pri_d")?,
        pri_other: get("pri_other")?,
        project: get("project")?,
        context: get("context")?,
        due: get("due")?,
        overdue: get("overdue")?,
        today: get("today")?,
        done: get("done")?,
        selected: get("selected")?,
        matched: get("matched")?,
    })
}

const COLOR_FIELDS: &[&str] = &[
    "bg",
    "panel",
    "border",
    "fg",
    "dim",
    "accent",
    "cursor",
    "selection",
    "statusbar",
    "status_fg",
    "mode_fg",
    "mode_bg",
    "pri_a",
    "pri_b",
    "pri_c",
    "pri_d",
    "pri_other",
    "project",
    "context",
    "due",
    "overdue",
    "today",
    "done",
    "selected",
    "matched",
];

fn match_color_field(k: &str) -> Option<&'static str> {
    COLOR_FIELDS.iter().copied().find(|f| *f == k)
}

/// Parse a color value. Accepts `#rrggbb` hex or the keywords `reset` /
/// `transparent` (both map to `Color::Reset`, inheriting the terminal bg).
fn parse_color(s: &str) -> Option<Color> {
    if s.eq_ignore_ascii_case("reset") || s.eq_ignore_ascii_case("transparent") {
        return Some(Color::Reset);
    }
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_THEME: &str = "\
name = Solarized Dark
bg = #002b36
panel = #073642
border = #586e75
fg = #93a1a1
dim = #657b83
accent = #b58900
cursor = #073642
selection = #073642
statusbar = #073642
status_fg = #93a1a1
mode_fg = #002b36
mode_bg = #b58900
pri_a = #dc322f
pri_b = #b58900
pri_c = #859900
pri_d = #268bd2
pri_other = #6c71c4
project = #859900
context = #cb4b16
due = #b58900
overdue = #dc322f
today = #dc322f
done = #586e75
selected = #073642
matched = #b58900
";

    #[test]
    fn parse_color_accepts_lower_and_upper_hex() {
        assert_eq!(parse_color("#1a2b3c"), Some(Color::Rgb(0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_color("#1A2B3C"), Some(Color::Rgb(0x1a, 0x2b, 0x3c)));
        assert_eq!(parse_color("#000000"), Some(Color::Rgb(0, 0, 0)));
        assert_eq!(parse_color("#ffffff"), Some(Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn parse_color_rejects_bad_shapes() {
        assert_eq!(parse_color("1a2b3c"), None); // no #
        assert_eq!(parse_color("#fff"), None); // 3 chars
        assert_eq!(parse_color("#xyzxyz"), None); // non-hex
        assert_eq!(parse_color("#1a2b3"), None); // 5 chars
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("#"), None);
    }

    #[test]
    fn parse_theme_full_roundtrip() {
        let t = parse_theme(FULL_THEME).expect("parse should succeed");
        assert_eq!(t.name, "Solarized Dark");
        assert_eq!(t.bg, Color::Rgb(0x00, 0x2b, 0x36));
        assert_eq!(t.matched, Color::Rgb(0xb5, 0x89, 0x00));
        // priority_color stays wired through the parsed theme
        assert_eq!(t.priority_color('A'), Color::Rgb(0xdc, 0x32, 0x2f));
        assert_eq!(t.priority_color('Z'), Color::Rgb(0x6c, 0x71, 0xc4));
    }

    #[test]
    fn parse_theme_missing_name() {
        let body = FULL_THEME.replace("name = Solarized Dark\n", "");
        let err = parse_theme(&body).expect_err("should reject missing name");
        assert!(err.contains("name"), "error should mention 'name': {err}");
    }

    #[test]
    fn parse_theme_missing_field_names_it() {
        let body = FULL_THEME.replace("accent = #b58900\n", "");
        let err = parse_theme(&body).expect_err("should reject missing accent");
        assert!(err.contains("accent"), "error should name 'accent': {err}");
    }

    #[test]
    fn parse_theme_bad_color_names_field() {
        let body = FULL_THEME.replace("accent = #b58900", "accent = not-a-color");
        let err = parse_theme(&body).expect_err("should reject bad color");
        assert!(err.contains("accent"), "error should name 'accent': {err}");
    }

    #[test]
    fn parse_theme_quoted_name_unquoted() {
        let body = FULL_THEME.replace("name = Solarized Dark", "name = \"Solarized Dark\"");
        let t = parse_theme(&body).expect("parse should succeed");
        assert_eq!(t.name, "Solarized Dark");
    }

    #[test]
    fn parse_theme_ignores_unknown_keys_and_comments() {
        let body = format!("# hello\nbogus = whatever\n{FULL_THEME}");
        let t = parse_theme(&body).expect("parse should succeed");
        assert_eq!(t.name, "Solarized Dark");
    }

    fn unique_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tuxedo-themes-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn terminal_theme_uses_terminal_palette() {
        assert_eq!(TERMINAL.bg, Color::Reset);
        assert_eq!(TERMINAL.panel, Color::Reset);
        assert_eq!(TERMINAL.fg, Color::Reset);
        assert_eq!(TERMINAL.pri_a, Color::Red);
        assert!(
            BUILT_IN.iter().any(|t| t.name == TERMINAL.name),
            "TERMINAL should be registered in BUILT_IN"
        );
    }

    #[test]
    fn load_user_themes_missing_dir_is_silent() {
        let dir = unique_dir("missing");
        let (themes, warnings) = load_user_themes(&dir);
        assert!(themes.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_user_themes_mixes_good_and_bad() {
        let dir = unique_dir("mixed");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(dir.join("a-good.toml"), FULL_THEME).expect("write good");
        fs::write(dir.join("b-broken.toml"), "name = Broken\nbg = #001122\n")
            .expect("write broken");
        // a non-toml file is ignored entirely (not even a warning)
        fs::write(dir.join("notes.md"), "ignore me").expect("write notes");

        let (themes, warnings) = load_user_themes(&dir);
        assert_eq!(themes.len(), 1);
        assert_eq!(themes[0].name, "Solarized Dark");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("b-broken.toml"),
            "warning should name the file: {}",
            warnings[0]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_user_themes_skips_collision_with_builtin() {
        let dir = unique_dir("collision");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");
        // Rename a parsed-fine theme to collide with the built-in Nord.
        let body = FULL_THEME.replace("name = Solarized Dark", "name = Nord");
        fs::write(dir.join("nord-clone.toml"), body).expect("write clone");

        let (themes, warnings) = load_user_themes(&dir);
        assert!(themes.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("Nord") && warnings[0].contains("already in use"),
            "warning should explain collision: {}",
            warnings[0]
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_user_themes_processes_in_sorted_order() {
        let dir = unique_dir("order");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");
        fs::write(
            dir.join("z-last.toml"),
            FULL_THEME.replace("name = Solarized Dark", "name = ZTheme"),
        )
        .expect("write z");
        fs::write(
            dir.join("a-first.toml"),
            FULL_THEME.replace("name = Solarized Dark", "name = ATheme"),
        )
        .expect("write a");

        let (themes, _) = load_user_themes(&dir);
        assert_eq!(themes.len(), 2);
        assert_eq!(themes[0].name, "ATheme");
        assert_eq!(themes[1].name, "ZTheme");

        let _ = fs::remove_dir_all(&dir);
    }
}
