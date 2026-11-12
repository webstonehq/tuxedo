use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::action::Action;
use crate::app::{App, DialogInputMode, Mode, View};
use crate::ui::dialog::draft_cursor_spans;

/// The current interaction owns the label; an underlying archive never masks
/// a help/palette/editor mode.
fn mode_label(app: &App) -> &'static str {
    match app.mode {
        Mode::Normal if app.view() == View::Archive => "ARCHIVE",
        Mode::Normal => "NORMAL",
        Mode::Insert => match app.draft.input_mode() {
            DialogInputMode::Normal => "Vim NORMAL",
            DialogInputMode::Insert => "Vim INSERT",
        },
        Mode::Search => "SEARCH",
        Mode::Visual => "TASK SELECT",
        Mode::Help => "HELP",
        Mode::Settings => "SETTINGS",
        Mode::PromptProject => "PROJECT",
        Mode::PromptContext => "CONTEXT",
        Mode::PickProject => "PICK +PROJECT",
        Mode::PickContext => "PICK @CONTEXT",
        Mode::PromptRenameProject => "RENAME +PROJECT",
        Mode::PromptRenameContext => "RENAME @CONTEXT",
        Mode::PickSavedFilter => "PICK FILTER",
        Mode::PromptSaveFilter => "SAVE FILTER",
        Mode::CommandPalette => "COMMAND",
        Mode::Share => "SHARE",
        Mode::PickTheme => "PICK THEME",
        Mode::Welcome => "WELCOME",
    }
}

fn selected_label(app: &App) -> Option<String> {
    (app.mode == Mode::Visual || !app.selection.is_empty())
        .then(|| format!("{} tasks selected", app.selection.len()))
}

/// Show only complete action hints, resolved in the task-list namespace.
fn action_hint(app: &App, action: Action, label: &str) -> Option<String> {
    app.keybinds
        .effective_keys(action)
        .first()
        .map(|key| format!("{key} {label}"))
}

fn hints(app: &App) -> Vec<String> {
    match app.mode {
        Mode::Normal if app.view() == View::Archive => [
            (Action::OpenHelp, "help"),
            (Action::ToggleArchiveView, "list"),
            (Action::ToggleComplete, "restore"),
            (Action::Delete, "delete"),
            (Action::Quit, "quit"),
        ]
        .into_iter()
        .filter_map(|(action, label)| action_hint(app, action, label))
        .collect(),
        Mode::Normal => [
            (Action::OpenHelp, "help"),
            (Action::BeginAdd, "new"),
            (Action::ToggleComplete, "done"),
            (Action::BeginSearch, "search"),
            (Action::Undo, "undo"),
            (Action::Quit, "quit"),
        ]
        .into_iter()
        .filter_map(|(action, label)| action_hint(app, action, label))
        .collect(),
        Mode::Visual => [
            (Action::EscapeStack, "cancel"),
            (Action::ToggleSelected, "toggle"),
            (Action::ToggleComplete, "complete"),
            (Action::Delete, "delete"),
        ]
        .into_iter()
        .filter_map(|(action, label)| action_hint(app, action, label))
        .collect(),
        // Draft and metadata controls live beside their active input.
        Mode::Insert => Vec::new(),
        Mode::Help => vec!["Esc close · j/k scroll".into()],
        Mode::Settings => vec!["Esc back".into()],
        Mode::PromptProject
        | Mode::PromptSaveFilter
        | Mode::PromptRenameProject
        | Mode::PromptRenameContext => vec!["Enter save · Esc cancel".into()],
        Mode::PromptContext => vec!["Enter toggle · Esc cancel".into()],
        Mode::PickProject | Mode::PickContext => vec![
            "Enter keep · Esc clear".into(),
            "j/k cycle · r rename".into(),
        ],
        Mode::PickSavedFilter => vec!["Enter keep · Esc revert".into(), "j/k cycle".into()],
        Mode::CommandPalette => vec!["Enter run · Esc cancel".into()],
        Mode::Share => vec!["any key dismisses".into()],
        Mode::PickTheme => vec!["Enter keep · Esc revert".into()],
        Mode::Welcome => vec!["c create · s sample · q quit".into()],
        Mode::Search => Vec::new(),
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let label = if area.width < 40 && app.mode == Mode::Visual {
        "TASKS"
    } else {
        mode_label(app)
    };
    let mut spans = vec![Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(theme.mode_fg)
            .bg(theme.mode_bg)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(selected) = selected_label(app) {
        let text = if area.width < 20 {
            format!("{} sel", app.selection.len())
        } else if area.width < 40 {
            format!("{} selected", app.selection.len())
        } else {
            format!("· {selected}")
        };
        spans.push(Span::styled(text, Style::default().fg(theme.accent)));
    }
    let mut used: usize = spans.iter().map(Span::width).sum();
    let mut append = |text: String, color| {
        let text = format!(" · {text}");
        let width = Span::raw(text.as_str()).width();
        if used + width <= usize::from(area.width) {
            used += width;
            spans.push(Span::styled(text, Style::default().fg(color)));
        }
    };
    // Chords and optional hints cannot squeeze out selected-task counts.
    if let Some(leader) = app.chord.active() {
        append(format!("{leader}…"), theme.accent);
    }
    for hint in hints(app) {
        append(hint, theme.status_fg);
    }
    if app.prefs.layout.status_bar {
        let count = if app.view() == View::Archive {
            format!("{} archived", app.archive().len())
        } else {
            format!("{} open", app.visible_indices().len())
        };
        append(count, theme.dim);
        append(app.today().into(), theme.dim);
        append(
            concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")).into(),
            theme.dim,
        );
        if let Some(tag) = app.update_available() {
            append(format!("↑ {tag} (tuxedo update)"), theme.accent);
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.statusbar)),
        area,
    );
}

/// Search owns two rows: input/cursor and count/controls. Notices have a
/// separate surface, so external-change feedback cannot replace the query.
pub fn render_command_line(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let theme = app.theme();
    let prefix = if area.width >= 20 {
        "SEARCH / "
    } else if area.width >= 4 {
        "/ "
    } else {
        ""
    };
    let prefix_w = prefix.len().min(usize::from(area.width)) as u16;
    let input = Rect::new(
        area.x + prefix_w,
        area.y,
        area.width.saturating_sub(prefix_w),
        1,
    );
    frame.render_widget(
        Paragraph::new(prefix).style(Style::default().fg(theme.accent)),
        Rect::new(area.x, area.y, prefix_w, 1),
    );
    let (draft, cursor) =
        crate::ui::dialog::visible_draft_window(app.draft.text(), app.draft.cursor(), input.width);
    let spans = draft_cursor_spans(draft, cursor, theme.fg, theme.bg);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.bg)),
        input,
    );
    if area.height > 1 {
        let count = app.visible_indices().len();
        let (summary, controls) = if area.width < 32 {
            let selected = if app.selection.is_empty() {
                String::new()
            } else {
                format!("/{}sel", app.selection.len())
            };
            (format!("{count} hits{selected}"), " ↵OK Esc×")
        } else {
            let selected = if app.selection.is_empty() {
                String::new()
            } else {
                format!(" · {} selected", app.selection.len())
            };
            let summary = format!("{count} matches{selected}");
            let controls = if Span::raw(summary.as_str()).width() + 27 <= usize::from(area.width) {
                " · Enter accept · Esc cancel"
            } else if selected.is_empty() {
                " · Enter OK · Esc cancel"
            } else {
                " ↵OK Esc×"
            };
            (summary, controls)
        };
        frame.render_widget(
            Paragraph::new(format!("{summary}{controls}"))
                .style(Style::default().fg(theme.dim).bg(theme.bg)),
            Rect::new(area.x, area.y + 1, area.width, 1),
        );
    }
}

pub fn contextual(app: &App) -> bool {
    app.prefs.layout.status_bar
        || app.mode == Mode::Search
        || app.mode == Mode::Visual
        || !app.selection.is_empty()
        || app.chord.active().is_some()
}

/// Prewrap once with the same rows used for sizing and drawing. Estimating
/// from character count loses words when wrapping leaves partially empty rows.
fn notice_lines(notice: &str, width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let width = usize::from(width);
    let text = format!("Notice: {notice}");
    let mut lines = Vec::new();
    let mut row = String::new();
    let mut columns = 0;
    for word in text.split_whitespace() {
        let word_width = Span::raw(word).width();
        if !row.is_empty() && columns + 1 + word_width <= width {
            row.push(' ');
            row.push_str(word);
            columns += 1 + word_width;
            continue;
        }
        if !row.is_empty() {
            lines.push(Line::raw(std::mem::take(&mut row)));
            columns = 0;
        }
        for ch in word.chars() {
            let char_width = Span::raw(ch.to_string()).width();
            if columns + char_width > width && !row.is_empty() {
                lines.push(Line::raw(std::mem::take(&mut row)));
                columns = 0;
            }
            row.push(ch);
            columns += char_width;
        }
    }
    if !row.is_empty() {
        lines.push(Line::raw(row));
    }
    lines
}

pub fn notice_height(app: &App, width: u16) -> u16 {
    app.flash_active()
        .map(|notice| notice_lines(notice, width).len().min(usize::from(u16::MAX)) as u16)
        .unwrap_or(0)
}

pub fn render_notice(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(notice) = app.flash_active() {
        frame.render_widget(
            Paragraph::new(notice_lines(notice, area.width)).style(
                Style::default()
                    .fg(app.theme().accent)
                    .bg(app.theme().panel),
            ),
            area,
        );
    }
}
