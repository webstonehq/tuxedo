use std::collections::BTreeMap;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
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

    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: "done.txt",
            file: "completed",
            count: app.archive.len(),
            sort: "completion-date",
            filter: None,
        },
    );

    let mut grouped: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, task) in app.archive.tasks().iter().enumerate() {
        let k = task.done_date.clone().unwrap_or_else(|| "unknown".into());
        grouped.entry(k).or_default().push(i);
    }
    let mut keys: Vec<_> = grouped.keys().cloned().collect();
    keys.sort_by(|a, b| b.cmp(a));

    let blank = super::density_blank_lines(app.prefs.density);
    let mut lines: Vec<Line> = Vec::new();
    for date in keys {
        let group = grouped
            .get(&date)
            .expect("invariant: date came from grouped.keys()");
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                date.clone(),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{} completed", group.len()),
                Style::default().fg(theme.dim),
            ),
        ]));
        for &i in group {
            let opts = task_row::RowOpts {
                idx_label: 0,
                cursor: false,
                multi_mode: false,
                multi_checked: false,
                selected: false,
                show_line_num: false,
                match_term: None,
                today: &app.today,
            };
            lines.push(task_row::build_line(&app.archive.tasks()[i], opts, theme));
        }
        for _ in 0..blank {
            lines.push(Line::raw(" "));
        }
    }

    if app.archive.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no completed tasks yet".to_string(),
            Style::default().fg(theme.dim),
        )));
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}
