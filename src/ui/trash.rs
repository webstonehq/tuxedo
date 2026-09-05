//! The Trash view: deleted tasks awaiting a second delete.
//!
//! Flat and in file order, unlike [`super::archive`] — trashed lines carry no
//! deletion-date tag to group by, which is the price of keeping them
//! byte-identical to how they appeared in `todo.txt`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Mode, View};
use crate::ui::{header, keep_cursor_visible, task_row};

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
            title: Some("trash.txt"),
            count: app.trash().len(),
            sort: "deletion-order",
            filter: None,
        },
    );

    let visible = app.visible_indices();
    let blank = super::density_blank_lines(app.prefs.density);
    let cursor_active = app.mode != Mode::Help && app.mode != Mode::Settings;

    if visible.is_empty() {
        let para = Paragraph::new(vec![Line::from(Span::styled(
            "   trash is empty".to_string(),
            Style::default().fg(theme.dim),
        ))])
        .style(Style::default().bg(theme.bg).fg(theme.fg));
        frame.render_widget(para, body_area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_line: Option<usize> = None;

    for (i, &abs) in visible.iter().enumerate() {
        if i > 0 {
            for _ in 0..blank {
                lines.push(Line::raw(" "));
            }
        }
        let task = &app.trash().tasks()[abs];
        let opts = task_row::RowOpts {
            idx_label: i,
            cursor: i == app.cursor && cursor_active,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: app.prefs.layout.line_num,
            match_term: None,
            today: app.today(),
            hidden_keys: &app.prefs.hidden_keys,
        };
        if i == app.cursor {
            cursor_line = Some(lines.len());
        }
        lines.push(task_row::build_line(task, opts, theme));
    }

    let scroll_cell = &app.view_scroll[View::Trash.idx()];
    let scroll = keep_cursor_visible(
        scroll_cell.get(),
        cursor_line,
        body_area.height,
        lines.len(),
    );
    scroll_cell.set(scroll);

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .scroll((scroll, 0));
    frame.render_widget(para, body_area);
}
