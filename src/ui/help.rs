use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;
use crate::theme::Theme;

type Section = (&'static str, &'static [(&'static str, &'static str)]);

const NAVIGATION: Section = (
    "NAVIGATION",
    &[
        ("j / ↓", "next task"),
        ("k / ↑", "previous task"),
        ("gg", "first task"),
        ("G", "last task"),
        ("Ctrl-d / Ctrl-u", "page down / up"),
    ],
);

const EDITING: Section = (
    "EDITING",
    &[
        ("a", "add task"),
        ("e / i", "edit current line"),
        ("x", "toggle complete"),
        ("dd", "delete task"),
        ("p", "cycle priority A→B→C→·"),
        ("c", "add/remove context"),
        ("+", "add project"),
        ("u", "undo"),
    ],
);

const VIEW: Section = (
    "VIEW",
    &[
        ("/", "fuzzy search"),
        ("fp", "filter by project"),
        ("fc", "filter by context"),
        ("s", "cycle sort"),
        ("v", "visual / multi-select"),
        ("t", "today view"),
        ("A", "archive completed"),
        ("H", "show done in list"),
        ("[", "toggle filter pane"),
        ("]", "toggle detail pane"),
        ("T", "cycle theme"),
        ("D", "cycle density"),
        ("L", "toggle line numbers"),
    ],
);

const SYSTEM: Section = (
    "SYSTEM",
    &[("?", "this help"), (",", "settings"), ("q", "quit")],
);

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
            Span::styled(
                " · keybindings ".to_string(),
                Style::default().fg(theme.dim),
            ),
        ]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);

    let left_lines = render_sections(theme, &[NAVIGATION, EDITING]);
    let right_lines = render_sections(theme, &[VIEW, SYSTEM]);
    let bg = Style::default().bg(theme.panel).fg(theme.fg);
    frame.render_widget(Paragraph::new(left_lines).style(bg), left);
    frame.render_widget(Paragraph::new(right_lines).style(bg), right);
}

fn render_sections<'a>(theme: &Theme, sections: &[Section]) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    for (title, items) in sections {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                title.to_string(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        for (k, d) in *items {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    pad_str(k, 18),
                    Style::default()
                        .fg(theme.context)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(d.to_string(), Style::default().fg(theme.fg)),
            ]));
        }
        lines.push(Line::raw(" "));
    }
    lines
}

fn pad_str(s: &str, w: usize) -> String {
    let len = s.chars().count();
    if len >= w {
        s.to_string()
    } else {
        let mut o = s.to_string();
        o.push_str(&" ".repeat(w - len));
        o
    }
}
