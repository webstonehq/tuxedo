//! Full-frame snapshot tests for every major mode/view.
//!
//! Each scene renders the real `ui::draw` into a fixed-size `TestBackend` and
//! emits two snapshots:
//!
//! * `*_text` — the visible character grid. Catches layout, content, and
//!   widget-placement regressions.
//! * `*_styled` — the same grid with inline `{fg=#hex bg=#hex mod=…}` tags.
//!   Catches styling regressions (priority colors, due-date buckets, cursor
//!   highlight, dim, bold) that the plain-text view would miss.
//!
//! Run `cargo insta review` after intentional UI changes to accept new
//! snapshots, or `INSTA_UPDATE=auto cargo test --test snapshots` to bulk-accept
//! during local iteration.

use std::path::PathBuf;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use tuxedo::app::{App, Density, Mode, View};
use tuxedo::config::Config;
use tuxedo::sample;
use tuxedo::ui;

const COLS: u16 = 100;
const ROWS: u16 = 32;

/// File path used in every fixture. Hard-coded (not `temp_dir()`) so the
/// header line that displays it stays byte-identical across runs and
/// machines. The file is never actually written; `App::new` only stores it.
const FIXTURE_PATH: &str = "/tmp/tuxedo-snapshot.txt";

/// Config-file path for the settings-overlay fixture. Hard-coded for the
/// same reason as `FIXTURE_PATH`: `Config::path()` resolves `$HOME` at
/// runtime, which would otherwise bake the author's home directory into
/// the snapshot and break on any other machine (CI included).
const FIXTURE_CONFIG_PATH: &str = "/tmp/tuxedo-snapshot.toml";

fn make_app() -> App {
    let mut app = App::new(
        PathBuf::from(FIXTURE_PATH),
        sample::TODO_RAW.to_string(),
        "2026-05-06".to_string(),
        Config::default(),
    );
    app.config_path = Some(PathBuf::from(FIXTURE_CONFIG_PATH));
    // Compact density keeps each scene dense and stable: blank-line counts
    // shift with density, which would churn snapshots without adding signal.
    app.prefs.density = Density::Compact;
    app
}

fn render(app: &App) -> Buffer {
    let backend = TestBackend::new(COLS, ROWS);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    terminal.draw(|f| ui::draw(f, app)).expect("draw frame");
    terminal.backend().buffer().clone()
}

/// Flatten a buffer to a plain character grid. Trailing whitespace per row is
/// preserved so width regressions show up as missing/extra padding columns.
fn buffer_to_text(buf: &Buffer) -> String {
    let cols = buf.area.width;
    let rows = buf.area.height;
    let mut out = String::with_capacity(usize::from(rows) * usize::from(cols + 1));
    for y in 0..rows {
        for x in 0..cols {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

/// Flatten a buffer to text with inline style tags. Adjacent cells sharing
/// the same style are collapsed into one run; default styles are omitted.
///
/// Format: `{fg=#xxxxxx bg=#xxxxxx mod=bold,dim}…{/}`. Either attribute is
/// dropped when it's `Color::Reset`. The closing `{/}` only appears when the
/// run was non-default.
fn buffer_to_styled(buf: &Buffer) -> String {
    let cols = buf.area.width;
    let rows = buf.area.height;
    let mut out = String::new();

    for y in 0..rows {
        let mut x = 0u16;
        let mut current: Option<StyleKey> = None;
        while x < cols {
            let cell = &buf[(x, y)];
            let key = StyleKey::from_cell(cell);
            if Some(&key) != current.as_ref() {
                if current.as_ref().is_some_and(|k| !k.is_default()) {
                    out.push_str("{/}");
                }
                if !key.is_default() {
                    push_open_tag(&mut out, &key);
                }
                current = Some(key);
            }
            out.push_str(escape(cell.symbol()).as_str());
            x += 1;
        }
        if current.as_ref().is_some_and(|k| !k.is_default()) {
            out.push_str("{/}");
        }
        out.push('\n');
    }
    out
}

#[derive(Clone, PartialEq, Eq)]
struct StyleKey {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

impl StyleKey {
    fn from_cell(cell: &ratatui::buffer::Cell) -> Self {
        Self {
            fg: cell.fg,
            bg: cell.bg,
            modifier: cell.modifier,
        }
    }

    fn is_default(&self) -> bool {
        matches!(self.fg, Color::Reset)
            && matches!(self.bg, Color::Reset)
            && self.modifier.is_empty()
    }
}

fn push_open_tag(out: &mut String, key: &StyleKey) {
    out.push('{');
    let mut first = true;
    if !matches!(key.fg, Color::Reset) {
        out.push_str("fg=");
        out.push_str(&color_repr(key.fg));
        first = false;
    }
    if !matches!(key.bg, Color::Reset) {
        if !first {
            out.push(' ');
        }
        out.push_str("bg=");
        out.push_str(&color_repr(key.bg));
        first = false;
    }
    if !key.modifier.is_empty() {
        if !first {
            out.push(' ');
        }
        out.push_str("mod=");
        out.push_str(&modifier_repr(key.modifier));
    }
    out.push('}');
}

fn color_repr(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        Color::Reset => "reset".into(),
        // Themes are RGB-only today; keep a fallback so a future ANSI color
        // still produces a stable, readable token instead of `Debug` noise.
        other => format!("{:?}", other).to_lowercase(),
    }
}

fn modifier_repr(m: Modifier) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if m.contains(Modifier::BOLD) {
        parts.push("bold");
    }
    if m.contains(Modifier::DIM) {
        parts.push("dim");
    }
    if m.contains(Modifier::ITALIC) {
        parts.push("italic");
    }
    if m.contains(Modifier::UNDERLINED) {
        parts.push("underlined");
    }
    if m.contains(Modifier::REVERSED) {
        parts.push("reversed");
    }
    if m.contains(Modifier::SLOW_BLINK) {
        parts.push("slow_blink");
    }
    if m.contains(Modifier::RAPID_BLINK) {
        parts.push("rapid_blink");
    }
    if m.contains(Modifier::CROSSED_OUT) {
        parts.push("crossed_out");
    }
    if m.contains(Modifier::HIDDEN) {
        parts.push("hidden");
    }
    parts.join(",")
}

/// Escape brace literals so they don't collide with our `{tag}` syntax.
fn escape(s: &str) -> String {
    s.replace('{', "{{").replace('}', "}}")
}

/// Snapshot both the text grid and the styled grid for the given scene.
/// Uses two separate insta calls so a layout-only change doesn't force a
/// styling review (and vice versa).
fn snapshot_app(name: &str, app: &App) {
    let buf = render(app);
    insta::assert_snapshot!(format!("{name}_text"), buffer_to_text(&buf));
    insta::assert_snapshot!(format!("{name}_styled"), buffer_to_styled(&buf));
}

// ---------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------

#[test]
fn list_default() {
    snapshot_app("list_default", &make_app());
}

#[test]
fn list_with_search() {
    let mut app = make_app();
    app.set_search("work".to_string());
    snapshot_app("list_with_search", &app);
}

#[test]
fn list_with_project_filter() {
    let mut app = make_app();
    app.set_project_filter(Some("work".to_string()));
    snapshot_app("list_with_project_filter", &app);
}

#[test]
fn list_grouped_by_due() {
    let mut app = make_app();
    // Default sort is Priority (groups by priority bucket); cycle once to
    // exercise the Due grouping path which has different bucket logic.
    app.cycle_sort();
    snapshot_app("list_grouped_by_due", &app);
}

#[test]
fn list_no_sidebars() {
    let mut app = make_app();
    app.prefs.layout.left = false;
    app.prefs.layout.right = false;
    snapshot_app("list_no_sidebars", &app);
}

#[test]
fn archive_view() {
    let mut app = make_app();
    app.set_view(View::Archive);
    snapshot_app("archive_view", &app);
}

#[test]
fn help_overlay() {
    let mut app = make_app();
    app.mode = Mode::Help;
    snapshot_app("help_overlay", &app);
}

#[test]
fn settings_overlay() {
    let mut app = make_app();
    app.mode = Mode::Settings;
    snapshot_app("settings_overlay", &app);
}

#[test]
fn insert_dialog() {
    let mut app = make_app();
    app.mode = Mode::Insert;
    app.draft_set("(A) Buy milk +groceries @errands due:2026-05-10".to_string());
    snapshot_app("insert_dialog", &app);
}

#[test]
fn empty_state() {
    let mut app = App::new(
        PathBuf::from(FIXTURE_PATH),
        String::new(),
        "2026-05-06".to_string(),
        Config::default(),
    );
    app.prefs.density = Density::Compact;
    app.prefs.layout.left = false;
    app.prefs.layout.right = false;
    snapshot_app("empty_state", &app);
}
