//! The which-key menu: a bottom-anchored panel listing the continuations of
//! the currently armed chord leader. Appears once the leader has been held
//! for `which_key_delay` (see [`crate::whichkey`] for where the rows come
//! from) and disappears as soon as the chord resolves or is cancelled.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::whichkey::{MenuEntry, Registry};

/// Rows are laid out in columns so a leader with many continuations stays
/// short rather than eating the list. A column is at least this wide before
/// the panel drops to fewer of them.
const MIN_COL_W: u16 = 24;
const MAX_COLS: usize = 4;
/// Cap on body rows, so a heavily-bound leader can't cover the whole list.
const MAX_ROWS: u16 = 8;
/// Width of the right-aligned key cell and the `→ ` separator that follows.
const KEY_W: usize = 4;
const ARROW_W: usize = 2;

/// Panel geometry for `entries`, anchored to the bottom of `parent` (just
/// above the status bar, which the caller has already carved off).
pub fn area_for(parent: Rect, entries: &[MenuEntry]) -> Rect {
    let width = parent.width;
    let cols = columns(width, entries.len());
    let rows = entries.len().div_ceil(cols.max(1)) as u16;
    // +2 for the top and bottom border.
    let height = (rows.clamp(1, MAX_ROWS) + 2).min(parent.height);
    Rect {
        x: parent.x,
        y: parent.y + parent.height - height,
        width,
        height,
    }
}

/// How many columns fit `n` entries in `width`, keeping each at least
/// `MIN_COL_W` wide.
fn columns(width: u16, n: usize) -> usize {
    let by_width = (width / MIN_COL_W).max(1) as usize;
    by_width.min(MAX_COLS).min(n.max(1))
}

pub fn render(frame: &mut Frame, area: Rect, app: &App, leader: char, entries: &[MenuEntry]) {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                Registry::title(leader),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · Esc cancel ", Style::default().fg(theme.dim)),
        ]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols = columns(inner.width, entries.len());
    let rows = usize::from(MAX_ROWS.min(inner.height)).max(1);
    let col_w = usize::from(inner.width) / cols.max(1);

    // Column-major fill: reading down a column matches how the rows were
    // sorted by key, so `f` shows c, f, p, s top-to-bottom.
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        for col in 0..cols {
            let Some(entry) = entries.get(col * rows + row) else {
                continue;
            };
            spans.push(Span::styled(
                format!("{:>width$} ", entry.keys, width = KEY_W - 1),
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                "→ ",
                Style::default().fg(theme.dim).bg(theme.panel),
            ));
            // Clip the label to what's left of the column so two columns
            // never bleed into each other, then pad out to the column edge.
            let used = KEY_W + ARROW_W;
            let label = truncate(&entry.label, col_w.saturating_sub(used + 1));
            let drawn = used + label.chars().count();
            spans.push(Span::styled(
                label,
                Style::default().fg(theme.fg).bg(theme.panel),
            ));
            spans.push(Span::styled(
                " ".repeat(col_w.saturating_sub(drawn)),
                Style::default().bg(theme.panel),
            ));
        }
        if spans.is_empty() {
            break;
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

/// Clip `s` to `room` display columns, marking a cut with `…`.
fn truncate(s: &str, room: usize) -> String {
    if s.chars().count() <= room {
        return s.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(room.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<MenuEntry> {
        (0..n)
            .map(|i| MenuEntry {
                keys: i.to_string(),
                label: format!("action {i}"),
            })
            .collect()
    }

    #[test]
    fn columns_respect_width_and_entry_count() {
        // 100 cols / 24 = 4 columns available, but only 2 entries to place.
        assert_eq!(columns(100, 2), 2);
        assert_eq!(columns(100, 9), MAX_COLS);
        // A narrow pane still gets one column rather than dividing by zero.
        assert_eq!(columns(10, 5), 1);
        assert_eq!(columns(0, 0), 1);
    }

    #[test]
    fn panel_sits_at_the_bottom_and_grows_with_rows() {
        let parent = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 20,
        };
        // 30 cols → one column, so 3 entries stack into 3 rows + 2 borders.
        let r = area_for(parent, &entries(3));
        assert_eq!((r.width, r.height), (30, 5));
        assert_eq!(r.y, 15);
        assert_eq!(r.y + r.height, parent.y + parent.height);
    }

    #[test]
    fn panel_height_is_capped_and_fits_a_short_parent() {
        let parent = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 40,
        };
        let r = area_for(parent, &entries(50));
        assert_eq!(r.height, MAX_ROWS + 2);

        let squat = Rect {
            x: 0,
            y: 0,
            width: 30,
            height: 3,
        };
        assert_eq!(area_for(squat, &entries(50)).height, 3);
    }

    #[test]
    fn labels_are_clipped_with_an_ellipsis() {
        assert_eq!(truncate("filter by project", 8), "filter …");
        assert_eq!(truncate("short", 8), "short");
        assert_eq!(truncate("short", 0), "");
    }
}
