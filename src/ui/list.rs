use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::app::{App, Mode};
use crate::ui::{header, task_row};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.bg));

    let [header_area, _spacer, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    let filter_label = header::filter_label(&app.filter);
    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: concat!(env!("CARGO_PKG_NAME"), " • ", env!("CARGO_PKG_VERSION")),
            file: &display_path(&app.file_path),
            count: app.visible_indices().len(),
            sort: app.sort_label(),
            filter: filter_label.as_deref(),
        },
    );

    if app.tasks.is_empty() {
        crate::ui::empty::render(frame, body_area, app);
        return;
    }

    let visible = app.visible_indices();
    let mut lines: Vec<Line> = Vec::new();

    if visible.is_empty() {
        lines.push(Line::from(ratatui::text::Span::styled(
            "   no tasks match".to_string(),
            Style::default().fg(theme.dim),
        )));
    } else {
        let blank = super::density_blank_lines(app.prefs.density);
        let last = visible.len().saturating_sub(1);
        for (i, &abs) in visible.iter().enumerate() {
            let task = &app.tasks[abs];
            let opts = task_row::RowOpts {
                idx_label: i,
                cursor: i == app.cursor && app.mode != Mode::Help && app.mode != Mode::Settings,
                multi_mode: app.mode == Mode::Visual,
                multi_checked: app.selection.is_selected(abs),
                selected: app.selection.is_selected(abs),
                show_line_num: app.prefs.layout.line_num,
                match_term: if app.filter.search.is_empty() {
                    None
                } else {
                    Some(&app.filter.search)
                },
                today: &app.today,
            };
            lines.push(task_row::build_line(task, opts, theme));
            if i != last {
                for _ in 0..blank {
                    lines.push(Line::raw(""));
                }
            }
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}

fn display_path(p: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rel) = p.strip_prefix(&home)
    {
        return format!("~/{}", rel.display());
    }
    p.display().to_string()
}
