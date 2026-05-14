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
        ("n", "new task"),
        ("e / i", "edit current line"),
        ("x", "toggle complete"),
        ("dd", "delete task"),
        ("p", "cycle priority A→B→C→·"),
        ("c", "add/remove context"),
        ("+", "add project"),
        ("yy", "copy line to clipboard"),
        ("yb", "copy body only"),
        ("u", "undo"),
    ],
);

const VIEW: Section = (
    "VIEW",
    &[
        ("/", "fuzzy search"),
        ("fp", "filter by project"),
        ("fc", "filter by context"),
        ("S", "cycle sort"),
        ("v", "visual / multi-select"),
        ("l", "list view"),
        ("a", "archive view"),
        ("A", "archive completed"),
        ("H", "show done in list"),
        ("F", "show future in list"),
        ("[ / ]", "toggle filter / detail"),
        ("T", "cycle theme"),
        ("D", "cycle density"),
        ("L", "toggle line numbers"),
    ],
);

const SYSTEM: Section = (
    "SYSTEM",
    &[
        (": / Ctrl-P", "command palette"),
        ("s", "share capture QR"),
        ("? / ,", "help / settings"),
        ("q", "quit"),
    ],
);

const FORMAT: Section = (
    "FORMAT",
    &[
        ("(A)", "priority A-Z"),
        ("YYYY-MM-DD", "creation / done date"),
        ("+project", "project tag(s)"),
        ("@context", "context tag(s)"),
        ("due:YYYY-MM-DD", "due date"),
        ("rec:Nu", "recur (u in d/w/m/y/b)"),
        ("rec:+Nu", "strict: anchor on due:"),
        ("x DATE BODY", "completed task prefix"),
    ],
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
            Span::styled(" · help ".to_string(), Style::default().fg(theme.dim)),
        ]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let bg = Style::default().bg(theme.panel).fg(theme.fg);

    // Keybindings (top, two columns) — divider — Format (bottom, two columns).
    // Each half splits sections across left/right; the last section in each
    // column drops its trailing blank so the divider lands tight.
    let kb_lines = two_columns(theme, inner.width, &[NAVIGATION, EDITING], &[VIEW, SYSTEM]);
    let kb_height = u16::try_from(kb_lines.len()).unwrap_or(u16::MAX);

    let (fmt_left, fmt_right) = FORMAT.1.split_at(FORMAT.1.len().div_ceil(2));
    let fmt_left_section: Section = (FORMAT.0, fmt_left);
    let fmt_right_section: Section = ("", fmt_right);
    let fmt_lines = two_columns(
        theme,
        inner.width,
        &[fmt_left_section],
        &[fmt_right_section],
    );

    let [kb_area, divider, fmt_area] = Layout::vertical([
        Constraint::Length(kb_height),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    frame.render_widget(Paragraph::new(kb_lines).style(bg), kb_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(usize::from(divider.width)),
            Style::default().fg(theme.border),
        )))
        .style(bg),
        divider,
    );
    frame.render_widget(Paragraph::new(fmt_lines).style(bg), fmt_area);
}

/// Render `left` and `right` section lists side-by-side. Each side gets half
/// the available width; rows are zipped so column heights stay aligned. The
/// trailing blank that `render_sections` adds after every section is dropped
/// from the last section in each column — visually that just means the column
/// ends flush with its final entry rather than carrying dead space below.
fn two_columns<'a>(
    theme: &Theme,
    total_width: u16,
    left: &[Section],
    right: &[Section],
) -> Vec<Line<'a>> {
    let left_lines = render_sections_trimmed(theme, left);
    let right_lines = render_sections_trimmed(theme, right);
    let rows = left_lines.len().max(right_lines.len());
    let half = usize::from(total_width / 2);
    let mut out: Vec<Line> = Vec::with_capacity(rows);
    for i in 0..rows {
        let mut spans: Vec<Span> = Vec::new();
        let left_spans: Vec<Span> = left_lines.get(i).map_or_else(Vec::new, |l| l.spans.clone());
        let left_width: usize = left_spans.iter().map(|s| s.content.chars().count()).sum();
        spans.extend(left_spans);
        if left_width < half {
            spans.push(Span::raw(" ".repeat(half - left_width)));
        }
        if let Some(r) = right_lines.get(i) {
            spans.extend(r.spans.clone());
        }
        out.push(Line::from(spans));
    }
    out
}

fn render_sections_trimmed<'a>(theme: &Theme, sections: &[Section]) -> Vec<Line<'a>> {
    let mut lines = render_sections(theme, sections);
    // Drop the trailing blank that `render_sections` appends after the last
    // section so columns end flush.
    if matches!(lines.last(), Some(line) if line_is_blank(line)) {
        lines.pop();
    }
    lines
}

fn line_is_blank(line: &Line) -> bool {
    line.spans
        .iter()
        .all(|s| s.content.chars().all(|c| c == ' '))
}

fn render_sections<'a>(theme: &Theme, sections: &[Section]) -> Vec<Line<'a>> {
    let mut lines: Vec<Line> = Vec::new();
    for (title, items) in sections {
        // An empty title means "this is a continuation column, skip the
        // header row" — used to align the right half of a 2-col section
        // (e.g. FORMAT) with the left half that owns the header.
        if title.is_empty() {
            lines.push(Line::raw(" "));
        } else {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    (*title).to_string(),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        for (k, d) in *items {
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    pad_str(k, 18),
                    Style::default()
                        .fg(theme.context)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled((*d).to_string(), Style::default().fg(theme.fg)),
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
