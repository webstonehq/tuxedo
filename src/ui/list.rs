use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, DisplayRow, GroupKey, ListDueBucket, Mode, View};
use crate::core::filter;
use crate::theme::Theme;
use crate::ui::{header, keep_cursor_visible, task_row};

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
            title: Some(&display_path(&app.file_path)),
            // title: None,
            // file: &display_path(&app.file_path),
            count: app.visible_indices().len(),
            hidden: app.hidden_by_collapse(),
            sort: app.sort_label(),
            filter: filter_label.as_deref(),
        },
    );

    if app.tasks().is_empty() {
        // The welcome overlay covers the same ground on first run; drawing the
        // empty-state card underneath stacks two boxes on top of each other.
        if app.mode != Mode::Welcome {
            crate::ui::empty::render(frame, body_area, app);
        }
        return;
    }

    // Highlight only the free-text part — a `due:` term filters by date and
    // never appears verbatim in the body.
    let resolved_needle = (!app.filter.search.is_empty())
        .then(|| filter::resolve_needle(&app.filter.search, app.today()));
    let match_term: Option<&str> = resolved_needle
        .as_ref()
        .and_then(|n| (!n.text.is_empty()).then_some(n.text.as_str()));

    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_line = None;
    let mut first_member_header = None;
    let mut last_header_line = 0;
    if app.visible_indices().is_empty() {
        lines.push(Line::from(Span::styled(
            "   no tasks match",
            Style::default().fg(theme.dim),
        )));
    } else {
        let blank = super::density_blank_lines(app.prefs.density);
        let mut task_number = 0;
        for (row, entry) in app.display_rows().iter().enumerate() {
            let focused = row == app.cursor && app.mode != Mode::Help && app.mode != Mode::Settings;
            match entry {
                DisplayRow::Header { key, count } => {
                    if !lines.is_empty() {
                        push_blanks(&mut lines, blank);
                    }
                    if row == app.cursor {
                        cursor_line = Some(lines.len());
                    }
                    last_header_line = lines.len();
                    lines.push(group_header(
                        theme,
                        key,
                        *count,
                        focused,
                        app.group_collapsed(key),
                    ));
                }
                DisplayRow::Task(abs) => {
                    if row == app.cursor {
                        cursor_line = Some(lines.len());
                        if row > 0
                            && matches!(app.display_rows()[row - 1], DisplayRow::Header { .. })
                        {
                            first_member_header = Some(last_header_line);
                        }
                    }
                    let opts = task_row::RowOpts {
                        idx_label: task_number,
                        cursor: focused,
                        multi_mode: app.effective_mode() == Mode::Visual,
                        multi_checked: app.selection.is_selected(*abs),
                        selected: app.selection.is_selected(*abs),
                        show_line_num: app.prefs.layout.line_num,
                        match_term,
                        today: app.today(),
                        hidden_keys: &app.prefs.hidden_keys,
                    };
                    lines.push(task_row::build_line(&app.tasks()[*abs], opts, theme));
                    task_number += 1;
                    if row + 1 < app.display_rows().len()
                        && app.prefs.group_by.effective(app.prefs.sort) == crate::app::GroupBy::None
                    {
                        push_blanks(&mut lines, blank);
                    }
                }
            }
        }
    }

    let scroll_cell = &app.view_scroll[View::List.idx()];
    let mut scroll = keep_cursor_visible(
        scroll_cell.get(),
        cursor_line,
        body_area.height,
        lines.len(),
    );
    if let (Some(header), Some(cursor)) = (first_member_header, cursor_line)
        && cursor - header < usize::from(body_area.height)
    {
        scroll = scroll.min(header as u16);
    }
    scroll_cell.set(scroll);

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme.bg).fg(theme.fg))
        .scroll((scroll, 0));
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

fn group_header<'a>(
    theme: &Theme,
    gk: &GroupKey,
    count: usize,
    focused: bool,
    collapsed: bool,
) -> Line<'a> {
    let (label, color) = match gk {
        GroupKey::ListPriority(Some(c)) => (format!("PRIORITY {c}"), theme.priority_color(*c)),
        GroupKey::ListPriority(None) => ("NO PRIORITY".to_string(), theme.dim),
        GroupKey::ListDue(b) => (b.label().to_string(), due_bucket_color(theme, *b)),
        // Defensive fallthrough — not produced under List view.
        GroupKey::ArchiveDate(d) => (d.clone(), theme.accent),
        GroupKey::None => (String::new(), theme.fg),
        GroupKey::Done => ("DONE".to_string(), theme.dim),
    };

    let char_count = count.to_string().len() + label.len();
    let divider_length = 80 - char_count;

    let line = Line::from(vec![
        Span::raw(if focused { "▶" } else { " " }),
        Span::styled(
            if collapsed { "[+] " } else { "[-] " },
            Style::default().fg(color),
        ),
        Span::styled(format!("({})", count), Style::default().fg(theme.dim)),
        Span::raw("  "),
        Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            "─".repeat(divider_length),
            Style::default().fg(theme.border),
        ),
    ]);
    if focused {
        line.style(Style::default().bg(theme.cursor))
    } else {
        line
    }
}

fn due_bucket_color(theme: &Theme, b: ListDueBucket) -> Color {
    match b {
        ListDueBucket::Overdue => theme.overdue,
        ListDueBucket::Today => theme.today,
        ListDueBucket::ThisWeek => theme.accent,
        ListDueBucket::NextWeek => theme.accent,
        ListDueBucket::Later => theme.accent,
        ListDueBucket::NoDue => theme.dim,
    }
}

fn push_blanks(lines: &mut Vec<Line>, n: usize) {
    for _ in 0..n {
        lines.push(Line::raw(" "));
    }
}
