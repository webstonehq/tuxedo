use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let w = 56u16.min(area.width.saturating_sub(4));
    let h = 16u16.min(area.height.saturating_sub(2));
    let r = super::centered_in(area, w, h);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.bg))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "tuxedo",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
        .style(Style::default().bg(theme.bg));
    let inner = block.inner(r);
    frame.render_widget(block, r);

    let shortcuts: &[(&str, &str)] = &[
        ("n", "add a task"),
        ("?", "show all keybindings"),
        (",", "settings"),
        ("q", "quit"),
    ];

    let mut lines: Vec<Line> = Vec::new();
    if inner.width >= super::logo::WIDTH {
        lines.extend(super::logo::centered_lines(theme, inner.width));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        "  no tasks yet — let's get started".to_string(),
        Style::default().fg(theme.fg),
    )));
    lines.push(Line::raw(""));
    for (key, label) in shortcuts {
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(
                pad_key(key, 4),
                Style::default()
                    .fg(theme.context)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(label.to_string(), Style::default().fg(theme.fg)),
        ]));
    }
    lines.push(Line::raw(""));
    let mut hint_spans = vec![
        Span::raw("   "),
        Span::styled("format: ".to_string(), Style::default().fg(theme.dim)),
    ];
    hint_spans.extend(super::dialog::format_hint_spans(theme));
    lines.push(Line::from(hint_spans));

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, inner);
}

fn pad_key(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.to_string()
    } else {
        let mut o = s.to_string();
        o.push_str(&" ".repeat(w - len));
        o
    }
}
