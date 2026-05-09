use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Mode, View};
use crate::ui::dialog::draft_cursor_spans;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let mut mode_label: std::borrow::Cow<'static, str> = match app.mode {
        Mode::Normal => "NORMAL".into(),
        Mode::Insert => "INSERT".into(),
        Mode::Search => "SEARCH".into(),
        Mode::Visual => "VISUAL".into(),
        Mode::Help => "HELP".into(),
        Mode::Settings => "SETTINGS".into(),
        Mode::PromptProject => "PROJECT".into(),
        Mode::PromptContext => "CONTEXT".into(),
        Mode::PickProject => "PICK +PROJECT".into(),
        Mode::PickContext => "PICK @CONTEXT".into(),
    };
    if matches!(app.view, View::Today) {
        mode_label = "AGENDA".into();
    } else if matches!(app.view, View::Archive) {
        mode_label = "ARCHIVE".into();
    }
    if let Some(f) = app.flash_active() {
        mode_label = format!("{mode_label} · {f}").into();
    }

    let hint = match app.mode {
        Mode::Insert => "Enter save · Esc cancel",
        Mode::Visual => "space toggle · x complete · dd delete · Esc cancel",
        Mode::Help => "? close help",
        Mode::Settings => "Esc back",
        Mode::PromptProject => "type +project name · Enter save · Esc cancel",
        Mode::PromptContext => "type @context name · Enter toggle · Esc cancel",
        Mode::PickProject => "j/k or ↑↓ cycle projects · Enter keep · Esc clear",
        Mode::PickContext => "j/k or ↑↓ cycle contexts · Enter keep · Esc clear",
        _ => "j/k · a add · x done · / search · ? help · u undo · q quit",
    };

    let mut right_parts = Vec::new();
    if matches!(app.view, View::Archive) {
        right_parts.push(format!("{} archived", app.archive.len()));
    } else {
        right_parts.push(format!("{} open", app.visible_indices().len()));
    }
    if !app.selection.is_empty() {
        right_parts.push(format!("{} selected", app.selection.len()));
    }
    right_parts.push(app.today.clone());
    right_parts.push(concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")).to_string());
    let right_text = right_parts.join(" · ");

    // Append a chord indicator (e.g. " g…") so two-key sequences like gg/dd/fp
    // give visible feedback on the first press. Only shown while armed.
    let chord_suffix = app
        .chord
        .active()
        .map(|c| format!(" {c}…"))
        .unwrap_or_default();
    // Layout: mode chip on left, hint in middle, right text right-aligned.
    let chip_text = format!(" {mode_label}{chord_suffix} ");
    let chip_w = chip_text.chars().count() as u16;
    let right_w = right_text.chars().count() as u16 + 1;
    let middle_w = area.width.saturating_sub(chip_w).saturating_sub(right_w);

    let [chip_area, mid_area, right_area] = Layout::horizontal([
        Constraint::Length(chip_w),
        Constraint::Length(middle_w),
        Constraint::Length(right_w),
    ])
    .areas(area);

    let chip = Paragraph::new(Span::styled(
        chip_text,
        Style::default()
            .bg(theme.mode_bg)
            .fg(theme.mode_fg)
            .add_modifier(Modifier::BOLD),
    ))
    .style(Style::default().bg(theme.statusbar));
    frame.render_widget(chip, chip_area);

    let mid_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(hint, Style::default().fg(theme.status_fg)),
    ])
    .style(Style::default().bg(theme.statusbar));
    frame.render_widget(
        Paragraph::new(mid_line).style(Style::default().bg(theme.statusbar)),
        mid_area,
    );

    let right_line = Line::from(Span::styled(
        format!("{right_text} "),
        Style::default().fg(theme.dim),
    ))
    .style(Style::default().bg(theme.statusbar));
    frame.render_widget(
        Paragraph::new(right_line)
            .style(Style::default().bg(theme.statusbar))
            .right_aligned(),
        right_area,
    );
}

pub fn render_command_line(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let visible_count = app.visible_indices().len();
    let suggestion = format!("  {visible_count} matches · Enter accept · Esc cancel");
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            "/",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    spans.extend(draft_cursor_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme.fg,
        theme.bg,
    ));
    spans.push(Span::styled(suggestion, Style::default().fg(theme.dim)));
    let line = Line::from(spans).style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.bg)),
        area,
    );
}
