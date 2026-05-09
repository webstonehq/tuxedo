use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;
use crate::todo::Task;
use crate::ui::task_row::{due_label, due_token_style};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.panel));

    let task = app.cur_task();
    // Wrap to the actual pane width minus 1-char left padding and 1-char
    // safety margin on the right. Floor at 16 so a tiny pane still wraps.
    let wrap_w = (area.width as usize).saturating_sub(2).max(16);
    let lines = build_lines(theme, task, &app.today, wrap_w);
    let para = Paragraph::new(lines).style(Style::default().bg(theme.panel).fg(theme.fg));
    frame.render_widget(para, area);
}

fn build_lines<'a>(
    theme: &Theme,
    task: Option<&'a Task>,
    today: &'a str,
    wrap_w: usize,
) -> Vec<Line<'a>> {
    let mut rows: Vec<Line> = Vec::new();
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " DETAIL",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let Some(t) = task else {
        rows.push(line_panel(
            theme,
            vec![Span::styled(" (no task)", Style::default().fg(theme.dim))],
        ));
        return rows;
    };

    let priority_value = if let Some(p) = t.priority {
        Span::styled(
            format!("({p})"),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" priority  ", Style::default().fg(theme.dim)),
            priority_value,
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" created   ", Style::default().fg(theme.dim)),
            Span::styled(
                t.created_date.as_deref().unwrap_or("—"),
                Style::default().fg(theme.fg),
            ),
        ],
    ));
    if let Some(due) = &t.due {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" due       ", Style::default().fg(theme.dim)),
                Span::styled(due.as_str(), Style::default().fg(theme.fg)),
                Span::raw("  "),
                Span::styled(due_label(due, today), Style::default().fg(theme.overdue)),
            ],
        ));
    }
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" projects  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.projects
                    .iter()
                    .map(|p| format!("+{p}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.project),
            ),
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" contexts  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.contexts
                    .iter()
                    .map(|c| format!("@{c}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.context),
            ),
        ],
    ));
    if t.done {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" done      ", Style::default().fg(theme.dim)),
                Span::styled(
                    t.done_date.as_deref().unwrap_or(""),
                    Style::default().fg(theme.done),
                ),
            ],
        ));
    }
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " RAW",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let mut state = RawWalk::default();
    for chunk in wrap_words(&t.raw, wrap_w) {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        let mut words = chunk.into_iter();
        if let Some(first) = words.next() {
            spans.push(style_raw_token(first, t, today, theme, &mut state));
        }
        for w in words {
            spans.push(Span::raw(" "));
            spans.push(style_raw_token(w, t, today, theme, &mut state));
        }
        rows.push(line_panel(theme, spans));
    }
    rows
}

#[derive(Default)]
struct RawWalk {
    done_marker_consumed: bool,
    priority_consumed: bool,
}

fn style_raw_token<'a>(
    token: &'a str,
    task: &Task,
    today: &str,
    theme: &Theme,
    state: &mut RawWalk,
) -> Span<'a> {
    if task.done && !state.done_marker_consumed {
        state.done_marker_consumed = true;
        if token == "x" {
            return Span::styled(token, Style::default().fg(theme.done));
        }
    }
    if !state.priority_consumed
        && let Some(p) = task.priority
        && token.len() == 3
        && token.as_bytes()[0] == b'('
        && token.as_bytes()[1] == p as u8
        && token.as_bytes()[2] == b')'
    {
        state.priority_consumed = true;
        return Span::styled(
            token,
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(rest) = token.strip_prefix("due:") {
        return Span::styled(token, due_token_style(task.done, rest, today, theme));
    }
    if token.len() > 1 && token.starts_with('+') {
        return Span::styled(token, Style::default().fg(theme.project));
    }
    if token.len() > 1 && token.starts_with('@') {
        return Span::styled(token, Style::default().fg(theme.context));
    }
    Span::styled(token, Style::default().fg(theme.fg))
}

fn line_panel<'a>(theme: &Theme, spans: Vec<Span<'a>>) -> Line<'a> {
    Line::from(spans).style(Style::default().bg(theme.panel))
}

/// Wrap `s` to roughly `width` graphemes, returning each output line as a
/// vector of borrowed words. Borrowing avoids the per-frame `String` alloc
/// that the previous `Vec<String>` form forced on every render.
fn wrap_words(s: &str, width: usize) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut acc: Vec<&str> = Vec::new();
    let mut acc_len = 0;
    for word in s.split_whitespace() {
        let wlen = word.chars().count();
        let extra = if acc.is_empty() { 0 } else { 1 };
        if acc_len + wlen + extra > width && !acc.is_empty() {
            out.push(std::mem::take(&mut acc));
            acc_len = 0;
        }
        if !acc.is_empty() {
            acc_len += 1;
        }
        acc.push(word);
        acc_len += wlen;
    }
    if !acc.is_empty() {
        out.push(acc);
    }
    out
}
