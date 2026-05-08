use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::theme::Theme;
use crate::todo::Task;
use crate::ui::header;
use crate::ui::task_row;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.bg));

    let [header_area, _sp, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    let today = app.today.as_str();
    let mut overdue: Vec<&Task> = Vec::new();
    let mut due_today: Vec<&Task> = Vec::new();
    let mut upcoming: Vec<&Task> = Vec::new();
    for &i in app.visible_indices() {
        let t = &app.tasks[i];
        let Some(d) = t.due.as_deref() else { continue };
        match d.cmp(today) {
            std::cmp::Ordering::Less => overdue.push(t),
            std::cmp::Ordering::Equal => due_today.push(t),
            std::cmp::Ordering::Greater => upcoming.push(t),
        }
    }
    upcoming.sort_by(|a, b| a.due.cmp(&b.due));

    let agenda_count = overdue.len() + due_today.len() + upcoming.len();
    let filter_label = header::filter_label(&app.filter);
    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: env!("CARGO_PKG_NAME"),
            file: "agenda · today",
            count: agenda_count,
            sort: "due",
            filter: filter_label.as_deref(),
        },
    );

    let blank = super::density_blank_lines(app.prefs.density);
    let mut lines: Vec<Line> = Vec::new();
    section(&mut lines, theme, "OVERDUE", &overdue, theme.overdue, today);
    push_blanks(&mut lines, blank);
    section(&mut lines, theme, "TODAY", &due_today, theme.today, today);
    push_blanks(&mut lines, blank);
    section(
        &mut lines,
        theme,
        "UPCOMING",
        &upcoming,
        theme.accent,
        today,
    );

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}

fn section<'a>(
    lines: &mut Vec<Line<'a>>,
    theme: &Theme,
    label: &str,
    list: &[&'a Task],
    color: Color,
    today: &'a str,
) {
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{}", list.len()), Style::default().fg(theme.dim)),
        Span::raw("  "),
        Span::styled("─".repeat(80), Style::default().fg(theme.border)),
    ]));
    if list.is_empty() {
        lines.push(Line::from(Span::styled(
            "   nothing".to_string(),
            Style::default().fg(theme.dim),
        )));
        return;
    }
    for (i, t) in list.iter().enumerate() {
        let opts = task_row::RowOpts {
            idx_label: i,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today,
        };
        lines.push(task_row::build_line(t, opts, theme));
    }
}

fn push_blanks(lines: &mut Vec<Line>, n: usize) {
    for _ in 0..n {
        lines.push(Line::raw(" "));
    }
}
