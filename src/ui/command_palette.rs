use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::app::palette::{self, ENTRIES};
use crate::theme::Theme;
use crate::ui::dialog::draft_cursor_spans;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "tuxedo",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · command ", Style::default().fg(theme.dim)),
        ]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let bg = Style::default().bg(theme.panel).fg(theme.fg);

    let hits = app.command_palette.hits();
    // Footer reserved (1 row) + divider (1 row); the rest is for input + list.
    let [input_area, divider_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let mut input_spans = vec![
        Span::raw(" "),
        Span::styled(
            ">",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    input_spans.extend(draft_cursor_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme.fg,
        theme.panel,
    ));
    let summary = if app.draft.text().is_empty() {
        String::new()
    } else {
        format!("  {} matches", hits.len())
    };
    input_spans.push(Span::styled(summary, Style::default().fg(theme.dim)));
    frame.render_widget(
        Paragraph::new(Line::from(input_spans).style(bg)).style(bg),
        input_area,
    );

    frame.render_widget(
        Paragraph::new(
            Line::from(Span::styled(
                "─".repeat(usize::from(divider_area.width)),
                Style::default().fg(theme.border),
            ))
            .style(bg),
        )
        .style(bg),
        divider_area,
    );

    let list_h = usize::from(list_area.height);
    if hits.is_empty() {
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled("no matches", Style::default().fg(theme.dim)),
        ])
        .style(bg);
        frame.render_widget(Paragraph::new(line).style(bg), list_area);
    } else {
        let cursor = app.command_palette.cursor.min(hits.len() - 1);
        // Window the list so the cursor stays on-screen.
        let start = if cursor < list_h {
            0
        } else {
            cursor + 1 - list_h
        };
        let end = (start + list_h).min(hits.len());
        let lines: Vec<Line> = hits[start..end]
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let abs = start + i;
                let is_sel = abs == cursor;
                let entry = ENTRIES[hit.entry_idx];
                render_row(entry, &hit.match_positions, is_sel, list_area.width, theme)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(bg), list_area);
    }

    let footer = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "↑↓ / Ctrl-N/P",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" navigate · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Enter",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" run · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Esc",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" cancel", Style::default().fg(theme.dim)),
    ])
    .style(bg);
    frame.render_widget(Paragraph::new(footer).style(bg), footer_area);
}

fn render_row<'a>(
    entry: palette::PaletteEntry,
    match_positions: &[usize],
    selected: bool,
    width: u16,
    theme: &Theme,
) -> Line<'a> {
    // Layout per row: " ▶ <label, match-highlighted, padded>   <keys dimmed> "
    // Selected rows get a cursor mark, bold label, and accent-tinted bg.
    let bg = if selected { theme.cursor } else { theme.panel };
    let label_fg = theme.fg;
    let dim_fg = theme.dim;
    let hl_style = Style::default()
        .fg(theme.bg)
        .bg(theme.matched)
        .add_modifier(Modifier::BOLD);
    let label_style = if selected {
        Style::default()
            .fg(label_fg)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(label_fg).bg(bg)
    };

    let mut spans: Vec<Span<'a>> = Vec::new();
    spans.push(Span::styled(
        if selected { " ▶ " } else { "   " },
        Style::default()
            .fg(if selected { theme.accent } else { bg })
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    ));

    // Label with per-byte match highlighting. `entry.label` is `&'static
    // str`, so all of these slices are `&'static str` and the spans borrow
    // them — no allocation per row.
    let label = entry.label;
    let mut cursor = 0usize;
    for &p in match_positions {
        if p < cursor || p >= label.len() {
            continue;
        }
        if cursor < p {
            spans.push(Span::styled(&label[cursor..p], label_style));
        }
        // Match offsets land on char boundaries (see `subseq_match_ci`).
        let ch_len = label[p..].chars().next().map(char::len_utf8).unwrap_or(1);
        spans.push(Span::styled(&label[p..p + ch_len], hl_style));
        cursor = p + ch_len;
    }
    if cursor < label.len() {
        spans.push(Span::styled(&label[cursor..], label_style));
    }

    // Pad to right-align the keys.
    let label_cols = label.chars().count() + 3; // 3 = sigil width above
    let keys_cols = entry.keys.chars().count() + 1; // trailing space
    let total = usize::from(width);
    if label_cols + keys_cols < total {
        spans.push(Span::styled(
            " ".repeat(total - label_cols - keys_cols),
            Style::default().bg(bg),
        ));
    }
    spans.push(Span::styled(entry.keys, Style::default().fg(dim_fg).bg(bg)));
    spans.push(Span::styled(" ", Style::default().bg(bg)));

    Line::from(spans).style(Style::default().bg(bg))
}
