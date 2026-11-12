//! OSC 8 hyperlink overlay.
//!
//! Embedding OSC 8 escape sequences inside a [`ratatui::buffer::Cell`] symbol
//! breaks ratatui's diff: `Buffer::diff` calls `UnicodeWidthStr::width()` on the
//! symbol to compute `to_skip`, so the printable bytes of an escape sequence
//! (`]8;;...`) inflate the symbol's width and cause subsequent cells to be
//! omitted from the diff. The visible result is a one-cell gap after the
//! URL — see [ratatui#902](https://github.com/ratatui/ratatui/issues/902).
//!
//! Instead we keep every cell's symbol byte-identical to a normal render —
//! URL tokens just carry [`Modifier::UNDERLINED`] (plus their accent fg) — and
//! after `Terminal::draw` finishes we walk the rendered buffer, find each
//! contiguous run of underlined cells, and write the OSC 8 wrapper directly to
//! the backend. The URL text gets re-printed inside the wrapper so the
//! terminal tags each cell with the link without us having to fight the diff.
//! Terminals that don't understand OSC 8 ignore the sequences entirely and
//! the underline stays as the visible affordance.

use std::io::{self, Write};

use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

/// One run of underlined cells, captured for re-emission with OSC 8.
#[derive(Debug, Clone)]
pub struct UrlRun {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub modifier: Modifier,
}

/// Scan `buf` for horizontal runs of cells carrying [`Modifier::UNDERLINED`].
/// The visible text of each run is used as its link target — URL tokens are
/// the only thing that carries the underline modifier in this codebase, so the
/// run text *is* the URL. Style comes from the run's first cell; spans of one
/// URL token share a single style by construction.
pub fn collect(buf: &Buffer) -> Vec<UrlRun> {
    let area = buf.area;
    let mut runs = Vec::new();
    for y in area.top()..area.bottom() {
        let mut x = area.left();
        while x < area.right() {
            let underlined = buf
                .cell((x, y))
                .is_some_and(|c| c.modifier.contains(Modifier::UNDERLINED));
            if !underlined {
                x += 1;
                continue;
            }
            let start = x;
            // Style fields come from the run head — adjacent underlined cells
            // belong to the same span, so they share fg/bg/modifier.
            let (fg, bg, modifier) = buf
                .cell((start, y))
                .map(|c| (c.fg, c.bg, c.modifier))
                .unwrap_or((Color::Reset, Color::Reset, Modifier::empty()));
            let mut text = String::new();
            while x < area.right()
                && buf
                    .cell((x, y))
                    .is_some_and(|c| c.modifier.contains(Modifier::UNDERLINED))
            {
                if let Some(c) = buf.cell((x, y)) {
                    text.push_str(c.symbol());
                }
                x += 1;
            }
            runs.push(UrlRun {
                x: start,
                y,
                text,
                fg,
                bg,
                modifier,
            });
        }
    }
    runs
}

/// Write an OSC 8 overlay to `writer` for each captured run. For every run we
/// move the cursor to the run's screen position, restore its visual styling
/// (fg/bg/modifier — ratatui's last `Reset` emission cleared them), wrap the
/// run's text with OSC 8 open/close, and reset state so the next frame's diff
/// starts from a known baseline.
pub fn emit_overlay<W: Write>(writer: &mut W, runs: &[UrlRun]) -> io::Result<()> {
    for run in runs {
        // CUP is 1-indexed; ratatui cell coordinates are 0-indexed.
        write!(writer, "\x1b[{};{}H", run.y + 1, run.x + 1)?;
        writer.write_all(b"\x1b[0m")?;
        write_sgr_fg(writer, run.fg)?;
        write_sgr_bg(writer, run.bg)?;
        write_sgr_modifier(writer, run.modifier)?;
        write!(writer, "\x1b]8;;{}\x1b\\", run.text)?;
        writer.write_all(run.text.as_bytes())?;
        writer.write_all(b"\x1b]8;;\x1b\\")?;
        writer.write_all(b"\x1b[0m")?;
    }
    Ok(())
}

fn write_sgr_fg<W: Write>(w: &mut W, c: Color) -> io::Result<()> {
    match c {
        Color::Reset => Ok(()),
        Color::Rgb(r, g, b) => write!(w, "\x1b[38;2;{r};{g};{b}m"),
        Color::Indexed(i) => write!(w, "\x1b[38;5;{i}m"),
        Color::Black => writer_write(w, "\x1b[30m"),
        Color::Red => writer_write(w, "\x1b[31m"),
        Color::Green => writer_write(w, "\x1b[32m"),
        Color::Yellow => writer_write(w, "\x1b[33m"),
        Color::Blue => writer_write(w, "\x1b[34m"),
        Color::Magenta => writer_write(w, "\x1b[35m"),
        Color::Cyan => writer_write(w, "\x1b[36m"),
        Color::Gray => writer_write(w, "\x1b[37m"),
        Color::DarkGray => writer_write(w, "\x1b[90m"),
        Color::LightRed => writer_write(w, "\x1b[91m"),
        Color::LightGreen => writer_write(w, "\x1b[92m"),
        Color::LightYellow => writer_write(w, "\x1b[93m"),
        Color::LightBlue => writer_write(w, "\x1b[94m"),
        Color::LightMagenta => writer_write(w, "\x1b[95m"),
        Color::LightCyan => writer_write(w, "\x1b[96m"),
        Color::White => writer_write(w, "\x1b[97m"),
    }
}

fn write_sgr_bg<W: Write>(w: &mut W, c: Color) -> io::Result<()> {
    match c {
        Color::Reset => Ok(()),
        Color::Rgb(r, g, b) => write!(w, "\x1b[48;2;{r};{g};{b}m"),
        Color::Indexed(i) => write!(w, "\x1b[48;5;{i}m"),
        Color::Black => writer_write(w, "\x1b[40m"),
        Color::Red => writer_write(w, "\x1b[41m"),
        Color::Green => writer_write(w, "\x1b[42m"),
        Color::Yellow => writer_write(w, "\x1b[43m"),
        Color::Blue => writer_write(w, "\x1b[44m"),
        Color::Magenta => writer_write(w, "\x1b[45m"),
        Color::Cyan => writer_write(w, "\x1b[46m"),
        Color::Gray => writer_write(w, "\x1b[47m"),
        Color::DarkGray => writer_write(w, "\x1b[100m"),
        Color::LightRed => writer_write(w, "\x1b[101m"),
        Color::LightGreen => writer_write(w, "\x1b[102m"),
        Color::LightYellow => writer_write(w, "\x1b[103m"),
        Color::LightBlue => writer_write(w, "\x1b[104m"),
        Color::LightMagenta => writer_write(w, "\x1b[105m"),
        Color::LightCyan => writer_write(w, "\x1b[106m"),
        Color::White => writer_write(w, "\x1b[107m"),
    }
}

fn write_sgr_modifier<W: Write>(w: &mut W, m: Modifier) -> io::Result<()> {
    if m.contains(Modifier::BOLD) {
        writer_write(w, "\x1b[1m")?;
    }
    if m.contains(Modifier::DIM) {
        writer_write(w, "\x1b[2m")?;
    }
    if m.contains(Modifier::ITALIC) {
        writer_write(w, "\x1b[3m")?;
    }
    if m.contains(Modifier::UNDERLINED) {
        writer_write(w, "\x1b[4m")?;
    }
    if m.contains(Modifier::REVERSED) {
        writer_write(w, "\x1b[7m")?;
    }
    Ok(())
}

fn writer_write<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    w.write_all(s.as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn paint(buf: &mut Buffer, x: u16, y: u16, text: &str, style: Style) {
        buf.set_string(x, y, text, style);
    }

    #[test]
    fn collect_finds_underlined_run_with_style() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        paint(&mut buf, 0, 0, "see ", Style::default());
        paint(
            &mut buf,
            4,
            0,
            "https://example.com",
            Style::default()
                .fg(Color::Rgb(0x8a, 0xa9, 0xc9))
                .add_modifier(Modifier::UNDERLINED),
        );

        let runs = collect(&buf);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.x, 4);
        assert_eq!(run.y, 0);
        assert_eq!(run.text, "https://example.com");
        assert_eq!(run.fg, Color::Rgb(0x8a, 0xa9, 0xc9));
        assert!(run.modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn collect_returns_empty_when_no_underlined_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        paint(&mut buf, 0, 0, "plain text", Style::default());
        assert!(collect(&buf).is_empty());
    }

    #[test]
    fn collect_splits_multiple_runs_on_one_row() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let url_style = Style::default().add_modifier(Modifier::UNDERLINED);
        paint(&mut buf, 0, 0, "a ", Style::default());
        paint(&mut buf, 2, 0, "http://x", url_style);
        paint(&mut buf, 10, 0, " ", Style::default());
        paint(&mut buf, 11, 0, "http://y", url_style);

        let runs = collect(&buf);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].text, "http://x");
        assert_eq!(runs[1].text, "http://y");
    }

    #[test]
    fn emit_overlay_emits_osc8_wrapper_with_text_and_styles() {
        let runs = vec![UrlRun {
            x: 4,
            y: 2,
            text: "https://example.com".to_string(),
            fg: Color::Rgb(0x8a, 0xa9, 0xc9),
            bg: Color::Reset,
            modifier: Modifier::UNDERLINED,
        }];
        let mut out: Vec<u8> = Vec::new();
        emit_overlay(&mut out, &runs).unwrap();
        let s = String::from_utf8(out).unwrap();

        // 1-indexed CUP at (col=5, row=3) for the 0-indexed (4, 2).
        assert!(s.contains("\x1b[3;5H"), "MoveTo missing or wrong: {s:?}");
        // OSC 8 open with the URL as parameter, followed by the visible text,
        // then the closer — without that triple the terminal never enters or
        // leaves hyperlink mode.
        assert!(s.contains("\x1b]8;;https://example.com\x1b\\"));
        assert!(s.contains("https://example.com\x1b]8;;\x1b\\"));
        // Fg colour preserved as 24-bit SGR so the re-printed text matches
        // what ratatui originally rendered.
        assert!(s.contains("\x1b[38;2;138;169;201m"));
        // Underline modifier applied.
        assert!(s.contains("\x1b[4m"));
        // Trailing SGR reset returns the terminal to a clean baseline so the
        // next frame's diff doesn't inherit stray attributes.
        assert!(s.ends_with("\x1b[0m"));
    }

    #[test]
    fn emit_overlay_with_no_runs_writes_nothing() {
        let mut out: Vec<u8> = Vec::new();
        emit_overlay(&mut out, &[]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn end_to_end_render_keeps_url_visible_intact() {
        // Regression: an earlier version mutated cell symbols to carry the
        // OSC 8 prefix, which inflated `Cell::symbol().width()` and caused
        // `Buffer::diff` to skip subsequent cells. The visible URL came back
        // with a one-character gap. Driving a full `ui::draw` here would
        // require the App fixture; we mimic the relevant invariant by
        // asserting that `collect` finds the URL text *exactly* as painted
        // and `emit_overlay` round-trips it byte-for-byte.
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        let url_style = Style::default()
            .fg(Color::Rgb(0x88, 0xc0, 0xd0))
            .add_modifier(Modifier::UNDERLINED);
        paint(&mut buf, 0, 0, "https://seaquel.app", url_style);

        let runs = collect(&buf);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].text, "https://seaquel.app");

        let mut out: Vec<u8> = Vec::new();
        emit_overlay(&mut out, &runs).unwrap();
        let s = String::from_utf8(out).unwrap();
        // The URL appears twice — once inside the OSC 8 parameter, once as the
        // visible text reprinted between the open and close sequences.
        assert_eq!(s.matches("https://seaquel.app").count(), 2);
    }
}
