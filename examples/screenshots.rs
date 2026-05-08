//! Generates the SVG screenshots embedded in the README.
//!
//! Usage:
//!     cargo run --example screenshots
//!
//! Renders each scene through ratatui's `TestBackend`, then walks the
//! resulting buffer and emits an SVG: one `<rect>` per horizontal bg run,
//! one `<text>` per non-blank cell. The themes use `Color::Rgb` exclusively
//! so colors come through faithfully.

use std::fs;
use std::path::{Path, PathBuf};

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use tuxedo::app::{App, Density, Mode, View};
use tuxedo::config::Config;
use tuxedo::sample;
use tuxedo::theme;
use tuxedo::ui;

const COLS: u16 = 130;
const ROWS: u16 = 32;
// Cell size in SVG user units. 13px font with these dims aligns acceptably
// in the common monospace fallback chain set on the <svg> root.
const CW: f32 = 8.0;
const CH: f32 = 16.0;

fn main() -> std::io::Result<()> {
    let out = PathBuf::from("docs/screenshots");
    fs::create_dir_all(&out)?;

    // Render every scene at Compact density so the screenshots stay
    // consistent and pack the most content per frame.
    let make = || {
        let mut app = App::new(
            PathBuf::from("/tmp/tuxedo-screenshots.txt"),
            sample::TODO_RAW.to_string(),
            "2026-05-06".to_string(),
            Config::default(),
        );
        app.prefs.density = Density::Compact;
        app
    };

    // 1. Default list view, fresh sample data, cursor on first task.
    save(&make(), &out.join("list.svg"))?;

    // 2. Today / agenda view — overdue · today · upcoming sections.
    let mut app = make();
    app.set_view(View::Today);
    save(&app, &out.join("today.svg"))?;

    // 3. Archive view — completed tasks grouped by completion date.
    let mut app = make();
    app.set_view(View::Archive);
    save(&app, &out.join("archive.svg"))?;

    // 4. Help overlay.
    let mut app = make();
    app.mode = Mode::Help;
    save(&app, &out.join("help.svg"))?;

    // 5. List with an active project filter — sidebar shows the selection.
    let mut app = make();
    app.set_project_filter(Some("work".to_string()));
    save(&app, &out.join("filter.svg"))?;

    // 6. List view in every built-in theme — for the README's themes section.
    for (i, t) in theme::ALL.iter().enumerate() {
        let mut app = make();
        app.prefs.set_theme_idx(i);
        let slug = t.name.to_lowercase().replace(' ', "-");
        save(&app, &out.join(format!("theme-{slug}.svg")))?;
    }

    println!("wrote screenshots to {}", out.display());
    Ok(())
}

fn save(app: &App, path: &Path) -> std::io::Result<()> {
    let backend = TestBackend::new(COLS, ROWS);
    let mut terminal = Terminal::new(backend).expect("terminal init");
    terminal.draw(|f| ui::draw(f, app)).expect("draw frame");
    let svg = render_svg(terminal.backend().buffer());
    fs::write(path, svg)
}

fn render_svg(buf: &Buffer) -> String {
    let cols = buf.area.width as usize;
    let rows = buf.area.height as usize;
    let total_w = cols as f32 * CW;
    let total_h = rows as f32 * CH;

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" \
         viewBox=\"0 0 {tw:.0} {th:.0}\" \
         width=\"{tw:.0}\" height=\"{th:.0}\" \
         font-family=\"ui-monospace, SFMono-Regular, Menlo, Consolas, monospace\" \
         font-size=\"13\">\n",
        tw = total_w,
        th = total_h,
    ));

    // Background pass: merge horizontal runs so we emit one <rect> per run.
    for y in 0..rows {
        let mut x = 0;
        while x < cols {
            let Some(bg) = bg_hex(buf[(x as u16, y as u16)].bg) else {
                x += 1;
                continue;
            };
            let start = x;
            x += 1;
            while x < cols && bg_hex(buf[(x as u16, y as u16)].bg).as_deref() == Some(bg.as_str()) {
                x += 1;
            }
            out.push_str(&format!(
                "<rect x=\"{rx:.1}\" y=\"{ry:.1}\" width=\"{rw:.1}\" height=\"{rh:.1}\" fill=\"{c}\"/>\n",
                rx = start as f32 * CW,
                ry = y as f32 * CH,
                rw = (x - start) as f32 * CW,
                rh = CH,
                c = bg,
            ));
        }
    }

    // Foreground pass: one <text> per non-blank cell. (Could batch by run
    // for size, but per-cell positioning is simpler and avoids monospace
    // metric guesswork.)
    for y in 0..rows {
        for x in 0..cols {
            let cell = &buf[(x as u16, y as u16)];
            let sym = cell.symbol();
            if sym.is_empty() || sym == " " {
                continue;
            }
            let fg = fg_hex(cell.fg).unwrap_or_else(|| "#cccccc".into());
            let weight = if cell.modifier.contains(Modifier::BOLD) {
                " font-weight=\"bold\""
            } else {
                ""
            };
            out.push_str(&format!(
                "<text x=\"{tx:.2}\" y=\"{ty:.2}\" fill=\"{fg}\"{weight}>{ch}</text>\n",
                tx = x as f32 * CW,
                ty = (y as f32 + 0.78) * CH,
                fg = fg,
                weight = weight,
                ch = escape(sym),
            ));
        }
    }

    out.push_str("</svg>\n");
    out
}

fn bg_hex(c: Color) -> Option<String> {
    if let Color::Rgb(r, g, b) = c {
        Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
    } else {
        None
    }
}

fn fg_hex(c: Color) -> Option<String> {
    if let Color::Rgb(r, g, b) = c {
        Some(format!("#{:02x}{:02x}{:02x}", r, g, b))
    } else {
        None
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
