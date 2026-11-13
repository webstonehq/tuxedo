use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::action::{Action, RecAction};
use crate::app::App;
use crate::theme::Theme;

struct Entry {
    keys: String,
    description: &'static str,
}

struct Section {
    title: &'static str,
    entries: Vec<Entry>,
}

fn key_label(keys: Vec<String>) -> String {
    keys.into_iter()
        .map(|key| match key.as_str() {
            "Down" => "↓".into(),
            "Up" => "↑".into(),
            "Left" => "←".into(),
            "Right" => "→".into(),
            _ => key,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn action(app: &App, action: Action, description: &'static str) -> Entry {
    let keys = key_label(app.keybinds.effective_keys(action));
    Entry {
        keys: if keys.is_empty() {
            "palette".into()
        } else {
            keys
        },
        description,
    }
}

fn pair(app: &App, first: Action, second: Action, description: &'static str) -> Entry {
    let first = action(app, first, "").keys;
    let second = action(app, second, "").keys;
    Entry {
        keys: format!("{first} / {second}"),
        description,
    }
}

fn sections(app: &App) -> (Vec<Section>, Vec<Section>) {
    use Action::*;
    let left = vec![
        Section {
            title: "NAVIGATION",
            entries: vec![
                pair(app, CursorDown, CursorUp, "next / previous task"),
                pair(app, CursorTop, CursorBottom, "first / last task"),
                pair(app, HalfPageDown, HalfPageUp, "page down / up"),
            ],
        },
        Section {
            title: "TASK EDITING",
            entries: vec![
                action(app, BeginAdd, "new task"),
                pair(app, BeginEdit, BeginEditInsert, "edit: normal / insert"),
                pair(app, ToggleComplete, Delete, "complete / delete"),
                pair(app, Reschedule, CyclePriority, "reschedule / priority"),
                pair(app, MoveTaskDown, MoveTaskUp, "move down / up"),
                pair(
                    app,
                    BeginPromptProject,
                    BeginPromptContext,
                    "project / context",
                ),
                pair(app, CopyLine, CopyBody, "copy line / body"),
                pair(app, OpenNote, CreateOrOpenNote, "open / create note"),
                action(app, Undo, "undo task change"),
            ],
        },
        Section {
            title: "TASK SELECTION",
            entries: vec![
                action(app, ToggleVisual, "enter / leave selection"),
                action(app, ToggleSelected, "toggle selected row"),
            ],
        },
    ];
    let right = vec![
        Section {
            title: "FILTER & VIEW",
            entries: vec![
                action(app, BeginSearch, "search tasks"),
                pair(app, PickProject, PickContext, "filter project / context"),
                pair(
                    app,
                    PickSavedFilter,
                    SaveCurrentFilter,
                    "saved filter: pick / save",
                ),
                action(app, CycleSort, "cycle sort"),
                pair(app, GoList, ToggleArchiveView, "list / archive view"),
                action(app, ArchiveCompleted, "archive completed"),
                pair(app, ToggleShowDone, ToggleShowFuture, "show done / future"),
            ],
        },
        Section {
            title: "LAYOUT & APPEARANCE",
            entries: vec![
                pair(app, ToggleLeftPane, ToggleRightPane, "filter / detail pane"),
                pair(app, OpenThemePicker, CycleDensity, "theme / density"),
                pair(
                    app,
                    ToggleLineNum,
                    ChangeWeekStart,
                    "line numbers / week start",
                ),
                action(app, CycleTheme, "cycle theme"),
            ],
        },
        Section {
            title: "SYSTEM",
            entries: vec![
                action(app, OpenCommandPalette, "command palette"),
                pair(app, OpenHelp, OpenSettings, "help / settings"),
                action(app, OpenShare, "phone capture QR"),
                action(app, EscapeStack, "cancel / clear"),
                action(app, Quit, "quit"),
            ],
        },
    ];
    (left, right)
}

fn details(app: &App) -> (Vec<Section>, Vec<Section>) {
    let fixed = |keys: &str, description| Entry {
        keys: keys.into(),
        description,
    };
    let left = vec![
        Section {
            title: "INLINE EDITOR · Vim default",
            entries: vec![
                fixed("Enter", "review / save draft"),
                fixed("Esc", "popup → normal → cancel"),
                fixed("h / l, ← / →", "move character"),
                fixed("w / b / e", "word motions (normal)"),
                fixed("i / a / A", "insert before / after / end"),
                fixed("x / dw / cw", "delete char / word / change"),
                fixed("Tab", "autocomplete (insert)"),
            ],
        },
        Section {
            title: "FORMAT · todo.txt",
            entries: vec![
                fixed("(A), YYYY-MM-DD", "priority, creation date"),
                fixed("+project, @context", "project and context tags"),
                fixed("due:YYYY-MM-DD", "due date; search due:+1w"),
                fixed("rec:Nu / rec:+Nu", "d/w/m/y/b; + anchors due"),
                fixed("x DATE BODY", "completed task"),
            ],
        },
    ];
    let recurrence = |action, description| Entry {
        keys: key_label(app.keybinds.effective_recurrence_keys(action)),
        description,
    };
    let right = vec![
        Section {
            title: "RECURRENCE · rec: picker",
            entries: vec![
                recurrence(RecAction::FocusNext, "next field"),
                recurrence(RecAction::FocusPrev, "previous field"),
                recurrence(RecAction::ValueNext, "increase value"),
                recurrence(RecAction::ValuePrev, "decrease value"),
                recurrence(RecAction::Accept, "save recurrence"),
                recurrence(RecAction::Cancel, "cancel recurrence"),
            ],
        },
        Section {
            title: "HELIX DRAFT · optional subset",
            entries: vec![
                fixed("E in settings", "profile for next task draft"),
                fixed("editing_profile", "config: vim (default) / helix"),
                fixed("h / l, ← / →", "move / extend one grapheme"),
                fixed("w / b / e", "next start / prev start / end"),
                fixed("Home / End", "draft start / end"),
                fixed("v", "toggle normal / extend"),
                fixed("% / x", "select whole task draft"),
                fixed(";", "collapse to active cursor"),
                fixed("d / c", "yank + delete / then insert"),
                fixed("i / a", "insert before / after selection"),
                fixed("I / A", "insert at absolute start / end"),
                fixed("y", "yank to app register"),
                fixed("p / P", "paste register after / before"),
                fixed("u / U", "undo / redo draft text"),
                fixed("Enter", "review / save draft"),
                fixed("Esc", "popup → normal → cancel"),
                fixed("Tab", "autocomplete (insert)"),
            ],
        },
    ];
    (left, right)
}

/// Wrap by terminal cells. Long custom bindings stay reachable rather than
/// disappearing behind the description or the neighboring column.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let word_width = Span::raw(word).width();
        if !row.is_empty() && used + 1 + word_width <= width {
            row.push(' ');
            row.push_str(word);
            used += 1 + word_width;
            continue;
        }
        if !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        for ch in word.chars() {
            let char_width = Span::raw(ch.to_string()).width();
            if used + char_width > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                used = 0;
            }
            row.push(ch);
            used += char_width;
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn section_rows(theme: &Theme, sections: &[Section], width: usize) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    let key_width = sections
        .iter()
        .flat_map(|section| &section.entries)
        .map(|entry| Span::raw(entry.keys.as_str()).width())
        .max()
        .unwrap_or(0)
        .min(22)
        .min(width.saturating_mul(2) / 5)
        .max(1);
    for section in sections {
        if !rows.is_empty() {
            rows.push(Line::raw(""));
        }
        for title in wrap(section.title, width) {
            rows.push(Line::styled(
                title,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if width < 16 {
            for entry in &section.entries {
                for row in wrap(&format!("{}: {}", entry.keys, entry.description), width) {
                    rows.push(Line::raw(row));
                }
            }
            continue;
        }
        let description_width = width.saturating_sub(key_width + 2).max(1);
        for entry in &section.entries {
            let keys = wrap(&entry.keys, key_width);
            let descriptions = wrap(entry.description, description_width);
            for i in 0..keys.len().max(descriptions.len()) {
                let key = keys.get(i).cloned().unwrap_or_default();
                let padding = key_width.saturating_sub(Span::raw(key.as_str()).width()) + 2;
                rows.push(Line::from(vec![
                    Span::styled(
                        key,
                        Style::default()
                            .fg(theme.context)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ".repeat(padding)),
                    Span::styled(
                        descriptions.get(i).cloned().unwrap_or_default(),
                        Style::default().fg(theme.fg),
                    ),
                ]));
            }
        }
    }
    rows
}

fn columns(theme: &Theme, left: &[Section], right: &[Section], width: usize) -> Vec<Line<'static>> {
    if width < 80 {
        let mut rows = section_rows(theme, left, width);
        rows.push(Line::raw(""));
        rows.extend(section_rows(theme, right, width));
        return rows;
    }
    let gutter = 4;
    let half = (width - gutter) / 2;
    let left = section_rows(theme, left, half);
    let right = section_rows(theme, right, width - half - gutter);
    (0..left.len().max(right.len()))
        .map(|i| {
            // A source line's style belongs only to its own column. Flatten
            // it into its spans before joining so a heading cannot color
            // the neighboring description, or lose its style on the right.
            let styled_spans = |line: &Line<'static>| {
                line.spans
                    .iter()
                    .map(|span| Span::styled(span.content.clone(), line.style.patch(span.style)))
                    .collect::<Vec<_>>()
            };
            let mut line = Line::from(left.get(i).map(styled_spans).unwrap_or_default());
            let padding = half.saturating_sub(line.width()) + gutter;
            line.spans.push(Span::raw(" ".repeat(padding)));
            if let Some(right) = right.get(i) {
                line.spans.extend(styled_spans(right));
            }
            line
        })
        .collect()
}

fn rows(app: &App, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let (left, right) = sections(app);
    let mut rows = columns(app.theme(), &left, &right, width);
    rows.push(Line::raw(""));
    rows.push(Line::styled(
        "─".repeat(width),
        Style::default().fg(app.theme().border),
    ));
    rows.push(Line::raw(""));
    let (left, right) = details(app);
    rows.extend(columns(app.theme(), &left, &right, width));
    rows.push(Line::raw(""));
    for note in wrap(
        "Helix is a fixed single-selection subset for task drafts only. Its register stays in memory across dialogs; it does not read the system clipboard. Draft undo is separate from task undo. Unsupported: multiple selections, counts, text objects, regex selection, macros, shell commands, multiline editing and editor-key remapping. I uses absolute draft start rather than first nonblank.",
        width,
    ) {
        rows.push(Line::styled(note, Style::default().fg(app.theme().dim)));
    }
    rows.push(Line::raw(""));
    for note in wrap(
        "Commas separate aliases; / pairs keys with actions. 'palette' means command palette only.",
        width,
    ) {
        rows.push(Line::styled(note, Style::default().fg(app.theme().dim)));
    }
    rows
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(" tuxedo · HELP ")
        .style(Style::default().bg(theme.panel).fg(theme.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Space separates the table from both borders and its scrolling controls.
    let inner = Rect::new(
        inner.x.saturating_add(u16::from(inner.width > 2)),
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let rows = rows(app, body.width);
    let max = rows
        .len()
        .saturating_sub(usize::from(body.height))
        .min(usize::from(u16::MAX - 1)) as u16;
    let scroll = app.help_scroll.get().min(max);
    app.help_scroll.set(scroll);
    frame.render_widget(Paragraph::new(rows).scroll((scroll, 0)), body);
    let controls = if footer.width >= 60 {
        format!(
            "j/k ↑↓ PgUp/PgDn scroll · Esc/?/q close · {}/{}",
            scroll + 1,
            max + 1
        )
    } else if footer.width >= 30 {
        "↑↓ scroll · Esc/?/q close".into()
    } else {
        "↑↓ · Esc close".into()
    };
    frame.render_widget(
        Paragraph::new(controls).style(Style::default().fg(theme.dim)),
        footer,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::palette::ENTRIES, config::Config, keybinds::KeyBindings};

    fn app() -> App {
        App::new(
            "/tmp/unused-help.txt".into(),
            "".into(),
            "2026-05-06".into(),
            Config::default(),
        )
    }

    #[test]
    fn wide_aliases_wrap_at_terminal_cell_boundaries() {
        let mut app = app();
        app.keybinds = KeyBindings::parse("[normal]\nopen_help = [\"界\", \"語\", \"文\"]");
        for row in rows(&app, 29) {
            assert!(row.width() <= 29, "{row:?}");
        }
    }

    #[test]
    fn overview_preserves_effective_keys_and_fits_wide_help() {
        let app = app();
        let (left, right) = sections(&app);
        let overview = columns(app.theme(), &left, &right, 95);
        assert!(overview.len() <= 26, "overview has {} rows", overview.len());
        let labels: String = left
            .iter()
            .chain(right.iter())
            .flat_map(|section| &section.entries)
            .map(|entry| entry.keys.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for entry in ENTRIES {
            for key in app.keybinds.effective_keys(entry.action) {
                assert!(
                    labels.contains(&key_label(vec![key.clone()])),
                    "missing {:?}: {key}",
                    entry.action
                );
            }
        }
        for row in overview {
            assert!(row.width() <= 95, "{row:?}");
        }
    }

    #[test]
    fn column_headers_keep_their_style_without_coloring_neighboring_actions() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = app();
        app.mode = crate::app::Mode::Help;
        let mut terminal = Terminal::new(TestBackend::new(110, 32)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        for (needle, heading) in [
            ("NAVIGATION", true),
            ("FILTER & VIEW", true),
            ("new task", false),
            ("command palette", false),
        ] {
            let (x, y) = (0..32)
                .find_map(|y| {
                    let row: String = (0..110).map(|x| buffer[(x, y)].symbol()).collect();
                    row.find(needle)
                        .map(|byte| (row[..byte].chars().count() as u16, y))
                })
                .expect("help heading or action is visible");
            let cell = &buffer[(x, y)];
            assert_eq!(
                cell.fg,
                if heading {
                    app.theme().accent
                } else {
                    app.theme().fg
                }
            );
            assert_eq!(cell.modifier.contains(Modifier::BOLD), heading, "{needle}");
        }
    }
}

#[cfg(test)]
mod helix_help_tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn supported_subset_and_departures_remain_reachable_at_required_widths() {
        let app = App::new(
            "/tmp/unused-helix-help.txt".into(),
            String::new(),
            "2026-11-13".into(),
            Config::default(),
        );
        for width in [72, 95] {
            let all_rows = rows(&app, width);
            assert!(all_rows.iter().all(|row| row.width() <= usize::from(width)));
            let text = all_rows
                .iter()
                .map(|row| {
                    row.spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(" ");
            for needle in [
                "HELIX DRAFT",
                "editing_profile",
                "Home / End",
                "u / U",
                "p / P",
                "I / A",
                "absolute",
                "single-selection",
                "Unsupported:",
            ] {
                assert!(text.contains(needle), "missing {needle} at {width}");
            }
        }
    }
}
