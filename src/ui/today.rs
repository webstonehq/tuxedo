use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, GroupKey, Mode, TodayBucket};
use crate::theme::Theme;
use crate::ui::{header, task_row};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.bg));

    let [header_area, _sp, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    let agenda_count = app.visible_indices().len();
    let filter_label = header::filter_label(&app.filter);
    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: concat!(env!("CARGO_PKG_NAME"), " • ", env!("CARGO_PKG_VERSION")),
            file: "agenda · today",
            count: agenda_count,
            sort: "due",
            filter: filter_label.as_deref(),
        },
    );

    let blank = super::density_blank_lines(app.prefs.density);
    let canonical = [
        TodayBucket::Overdue,
        TodayBucket::Today,
        TodayBucket::Upcoming,
    ];

    // Per-bucket counts so the header can show "OVERDUE  3" without rescanning
    // mid-loop.
    let mut counts = [0usize; 3];
    for g in app.visible_groups() {
        if let GroupKey::TodayBucket(b) = g {
            counts[b.idx()] += 1;
        }
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut emitted = [false; 3];
    let mut last_bucket: Option<TodayBucket> = None;
    let cursor_active = app.mode != Mode::Help && app.mode != Mode::Settings;
    let needle: Option<&str> = if app.filter.search.is_empty() {
        None
    } else {
        Some(&app.filter.search)
    };

    let visible = app.visible_indices();
    let groups = app.visible_groups();

    for (i, (&abs, gk)) in visible.iter().zip(groups.iter()).enumerate() {
        let bucket = match gk {
            GroupKey::TodayBucket(b) => *b,
            _ => continue,
        };

        // Emit any earlier-canonical bucket that has zero items and hasn't
        // been seen yet, before showing the first present row of `bucket`.
        for &earlier in &canonical {
            if earlier.idx() >= bucket.idx() {
                break;
            }
            if !emitted[earlier.idx()] {
                push_blanks(&mut lines, blank);
                lines.push(section_header(theme, earlier, 0));
                lines.push(nothing_line(theme));
                emitted[earlier.idx()] = true;
            }
        }

        if last_bucket != Some(bucket) {
            push_blanks(&mut lines, blank);
            lines.push(section_header(theme, bucket, counts[bucket.idx()]));
            emitted[bucket.idx()] = true;
            last_bucket = Some(bucket);
        }

        let task = &app.tasks[abs];
        let opts = task_row::RowOpts {
            idx_label: i,
            cursor: i == app.cursor && cursor_active,
            multi_mode: app.mode == Mode::Visual,
            multi_checked: app.selection.is_selected(abs),
            selected: app.selection.is_selected(abs),
            show_line_num: app.prefs.layout.line_num,
            match_term: needle,
            today: &app.today,
        };
        lines.push(task_row::build_line(task, opts, theme));
    }

    // Trailing empty buckets — emit headers + "nothing" placeholder so the
    // user sees the full canonical list.
    for &b in &canonical {
        if !emitted[b.idx()] {
            push_blanks(&mut lines, blank);
            lines.push(section_header(theme, b, 0));
            lines.push(nothing_line(theme));
            emitted[b.idx()] = true;
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}

fn section_header<'a>(theme: &Theme, bucket: TodayBucket, count: usize) -> Line<'a> {
    let color = bucket_color(theme, bucket);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            bucket.label().to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{}", count), Style::default().fg(theme.dim)),
        Span::raw("  "),
        Span::styled("─".repeat(80), Style::default().fg(theme.border)),
    ])
}

fn bucket_color(theme: &Theme, bucket: TodayBucket) -> Color {
    match bucket {
        TodayBucket::Overdue => theme.overdue,
        TodayBucket::Today => theme.today,
        TodayBucket::Upcoming => theme.accent,
    }
}

fn nothing_line<'a>(theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        "   nothing".to_string(),
        Style::default().fg(theme.dim),
    ))
}

fn push_blanks(lines: &mut Vec<Line>, n: usize) {
    if lines.is_empty() {
        return;
    }
    for _ in 0..n {
        lines.push(Line::raw(" "));
    }
}
