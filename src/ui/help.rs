use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::RecAction;
use crate::app::App;
use crate::app::palette::ENTRIES;

/// Build width-bounded rows before scrolling, so every alias remains reachable
/// even when a binding list is longer than the terminal's viewport.
fn rows(app: &App, width: u16) -> Vec<Line<'static>> {
    let theme = app.theme();
    let mut rows = Vec::new();
    let mut section = |title: &str, entries: Vec<String>| {
        rows.push(Line::styled(
            title.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        let width = usize::from(width).max(1);
        for entry in entries {
            // Each row is independently bounded, including in tiny panes.
            let mut row = String::new();
            let mut columns = 0;
            for ch in entry.chars() {
                let char_width = Span::raw(ch.to_string()).width();
                if columns + char_width > width && !row.is_empty() {
                    rows.push(Line::raw(std::mem::take(&mut row)));
                    columns = 0;
                }
                row.push(ch);
                columns += char_width;
            }
            if !row.is_empty() {
                rows.push(Line::raw(row));
            }
        }
        rows.push(Line::raw(""));
    };
    section(
        "TASK LIST · effective bindings",
        ENTRIES
            .iter()
            .map(|entry| {
                let keys = app.keybinds.keys_label(entry.action);
                format!(
                    " {}: {}",
                    if keys.is_empty() { "unbound" } else { &keys },
                    entry.label
                )
            })
            .collect(),
    );
    section(
        "INLINE EDITOR · fixed controls",
        vec![
            " Enter: save · Esc: insert → normal → cancel".into(),
            " Normal: h/l or arrows move · w/b/e words · i/a/A insert".into(),
            " Normal: x delete character · dw delete word · cw change word".into(),
            " Insert: type text · arrows move · Tab autocomplete".into(),
        ],
    );
    section(
        "RECURRENCE (rec:) · effective bindings",
        [
            (RecAction::FocusNext, "next field"),
            (RecAction::FocusPrev, "previous field"),
            (RecAction::ValueNext, "increase value"),
            (RecAction::ValuePrev, "decrease value"),
            (RecAction::Accept, "save recurrence"),
            (RecAction::Cancel, "cancel recurrence"),
        ]
        .into_iter()
        .map(|(action, label)| {
            let keys = app.keybinds.recurrence_label(action);
            format!(
                " {}: {label}",
                if keys.is_empty() { "unbound" } else { &keys }
            )
        })
        .collect(),
    );
    section(
        "FORMAT",
        vec![
            " (A): priority A-Z · YYYY-MM-DD: creation / done date".into(),
            " +project · @context · due:YYYY-MM-DD (due:+1w in search)".into(),
            " rec:Nu: recur (d/w/m/y/b) · rec:+Nu: anchor on due date".into(),
            " x DATE BODY: completed task".into(),
        ],
    );
    rows
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title(" tuxedo · HELP ")
        .style(Style::default().bg(theme.panel).fg(theme.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let rows = rows(app, body.width);
    let max = rows
        .len()
        .saturating_sub(usize::from(body.height))
        .min(usize::from(u16::MAX)) as u16;
    let scroll = app.help_scroll.get().min(max);
    app.help_scroll.set(scroll);
    frame.render_widget(Paragraph::new(rows).scroll((scroll, 0)), body);
    frame.render_widget(
        Paragraph::new(format!(
            "j/k ↑↓ PgUp/PgDn scroll · Esc/?/q close · {}/{}",
            scroll + 1,
            max + 1
        ))
        .style(Style::default().fg(theme.dim)),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, keybinds::KeyBindings};
    #[test]
    fn wide_aliases_wrap_at_terminal_cell_boundaries() {
        let mut app = App::new(
            "/tmp/unused-help.txt".into(),
            "".into(),
            "2026-05-06".into(),
            Config::default(),
        );
        app.keybinds = KeyBindings::parse("[normal]\nopen_help = [\"界\", \"語\", \"文\"]");
        // Entry rows include each alias without clipping a wide character.
        for row in rows(&app, 29).into_iter().filter(|row| {
            row.spans.iter().any(|s| {
                s.content.starts_with(' ')
                    || s.content.contains('界')
                    || s.content.contains('語')
                    || s.content.contains('文')
            })
        }) {
            assert!(row.width() <= 29, "{row:?}");
        }
    }
}
