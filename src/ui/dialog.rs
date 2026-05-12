use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{
    self, App, BuilderField, CalendarTarget, DraftOverlay, Mode, REC_UNIT_ORDER, TokenKind,
};
use crate::theme::Theme;

/// Classifier output: byte range + what kind of token lives there. Segments
/// cover the input contiguously and don't overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    Plain,
    Priority(char),
    Date,
    Project,
    Context,
    Due,
    KeyValue,
}

/// Walk a draft and tag each byte range with what it represents in the
/// todo.txt format. Used by the dialog to syntax-highlight what the user is
/// typing. Mirrors `todo::parse_line`'s grammar at the token level but
/// doesn't share code — the highlighter must keep up character-by-character
/// even on partially-typed input that the parser would reject.
pub(crate) fn classify_draft(s: &str) -> Vec<(std::ops::Range<usize>, SegmentKind)> {
    let mut out: Vec<(std::ops::Range<usize>, SegmentKind)> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional leading "x " marker (done) followed by an optional done-date.
    if bytes.len() >= 2 && bytes[0] == b'x' && bytes[1].is_ascii_whitespace() {
        out.push((0..1, SegmentKind::Plain));
        out.push((1..2, SegmentKind::Plain));
        i = 2;
        if let Some(end) = match_date(bytes, i) {
            out.push((i..end, SegmentKind::Date));
            i = end;
            if i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push((i..i + 1, SegmentKind::Plain));
                i += 1;
            }
        }
    }

    // Leading priority "(A)" through "(Z)".
    if let Some(end) = match_priority(bytes, i) {
        let pri_char = bytes[i + 1] as char;
        out.push((i..end, SegmentKind::Priority(pri_char)));
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            out.push((i..i + 1, SegmentKind::Plain));
            i += 1;
        }
    }

    // Optional creation date.
    if let Some(end) = match_date(bytes, i) {
        out.push((i..end, SegmentKind::Date));
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            out.push((i..i + 1, SegmentKind::Plain));
            i += 1;
        }
    }

    // Walk the rest as alternating whitespace runs and word tokens.
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push((start..i, SegmentKind::Plain));
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let word = &s[start..i];
        out.push((start..i, classify_word(word)));
    }

    out
}

fn match_priority(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.len() >= i + 3
        && bytes[i] == b'('
        && bytes[i + 1].is_ascii_uppercase()
        && bytes[i + 2] == b')'
    {
        Some(i + 3)
    } else {
        None
    }
}

fn match_date(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.len() < i + 10 {
        return None;
    }
    let d = |k: usize| bytes[i + k].is_ascii_digit();
    if d(0)
        && d(1)
        && d(2)
        && d(3)
        && bytes[i + 4] == b'-'
        && d(5)
        && d(6)
        && bytes[i + 7] == b'-'
        && d(8)
        && d(9)
    {
        Some(i + 10)
    } else {
        None
    }
}

fn classify_word(w: &str) -> SegmentKind {
    if w.starts_with('+') && w.len() > 1 {
        return SegmentKind::Project;
    }
    if w.starts_with('@') && w.len() > 1 {
        return SegmentKind::Context;
    }
    if let Some((k, v)) = w.split_once(':')
        && !v.is_empty()
        && is_kv_key(k)
    {
        if k == "due" {
            return SegmentKind::Due;
        }
        return SegmentKind::KeyValue;
    }
    SegmentKind::Plain
}

fn is_kv_key(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Syntax-highlighted draft with cursor inversion. Walks `classify_draft`
/// and emits one styled span per segment, splitting whichever segment
/// contains the cursor so its glyph stays readable with swapped fg/bg.
pub(crate) fn highlighted_draft_spans<'a>(
    draft: &'a str,
    cursor: usize,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let segments = classify_draft(draft);
    let cursor = cursor.min(draft.len());
    let mut out: Vec<Span<'a>> = Vec::new();

    for (range, kind) in segments {
        let style = segment_style(kind, theme);
        if cursor >= range.start && cursor < range.end {
            let before = &draft[range.start..cursor];
            let next = next_boundary(draft, cursor);
            let cursor_char = &draft[cursor..next];
            let after = &draft[next..range.end];
            if !before.is_empty() {
                out.push(Span::styled(before, style));
            }
            // Invert: glyph fg = panel bg, glyph bg = segment colour.
            let fg = style.fg.unwrap_or(theme.fg);
            let inv = Style::default().fg(theme.panel).bg(fg);
            out.push(Span::styled(cursor_char, inv));
            if !after.is_empty() {
                out.push(Span::styled(after, style));
            }
        } else {
            out.push(Span::styled(&draft[range.start..range.end], style));
        }
    }

    if cursor == draft.len() {
        out.push(Span::styled("█", Style::default().fg(theme.fg)));
    }
    out
}

fn segment_style(kind: SegmentKind, theme: &Theme) -> Style {
    let (color, bold) = match kind {
        SegmentKind::Plain => (theme.fg, false),
        SegmentKind::Priority(p) => (theme.priority_color(p), true),
        SegmentKind::Date => (theme.dim, false),
        SegmentKind::Project => (theme.project, false),
        SegmentKind::Context => (theme.context, false),
        SegmentKind::Due => (theme.due, false),
        SegmentKind::KeyValue => (theme.dim, false),
    };
    let s = Style::default().fg(color);
    if bold {
        s.add_modifier(Modifier::BOLD)
    } else {
        s
    }
}

fn next_boundary(s: &str, i: usize) -> usize {
    let len = s.len();
    let mut j = (i + 1).min(len);
    while j < len && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

/// Render `draft` with the insertion point highlighted at byte offset `cursor`.
/// When the cursor sits past the last char, append a block glyph; otherwise the
/// character under the cursor is drawn with swapped fg/bg so it stays readable.
pub fn draft_cursor_spans<'a>(
    draft: &'a str,
    cursor: usize,
    fg: Color,
    bg: Color,
) -> Vec<Span<'a>> {
    let cursor = cursor.min(draft.len());
    let before = &draft[..cursor];
    let after = &draft[cursor..];
    let mut iter = after.char_indices();
    if let Some((_, _)) = iter.next() {
        let next = iter.next().map(|(i, _)| i).unwrap_or(after.len());
        let cursor_char = &after[..next];
        let rest = &after[next..];
        vec![
            Span::styled(before, Style::default().fg(fg)),
            Span::styled(cursor_char, Style::default().fg(bg).bg(fg)),
            Span::styled(rest, Style::default().fg(fg)),
        ]
    } else {
        vec![
            Span::styled(before, Style::default().fg(fg)),
            Span::styled("█", Style::default().fg(fg)),
        ]
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let title = if app.selection.editing().is_some() {
        " EDIT TASK "
    } else {
        " ADD TASK "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [_p1, input_area, preview_area, _p2, hint_area, _p3] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Split the input row into a fixed prefix ("  › ") and a scrollable
    // content area. Without this, long drafts get clipped at the dialog's
    // right edge — including the cursor itself, so the user can't see what
    // they're typing. The prefix never scrolls; the content paragraph offsets
    // horizontally to keep the cursor onscreen.
    const PREFIX_W: u16 = 4;
    let [prefix_area, content_area] =
        Layout::horizontal([Constraint::Length(PREFIX_W), Constraint::Min(0)]).areas(input_area);

    let prefix_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "› ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().bg(theme.panel));
    frame.render_widget(
        Paragraph::new(prefix_line).style(Style::default().bg(theme.panel)),
        prefix_area,
    );

    let content_line = Line::from(highlighted_draft_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme,
    ))
    .style(Style::default().bg(theme.panel));
    let cursor = app.draft.cursor().min(app.draft.text().len());
    let cursor_col = app.draft.text()[..cursor].chars().count();
    let avail = content_area.width as usize;
    // Pin the cursor to the rightmost visible column whenever it would
    // otherwise overflow. Stateless: when the cursor moves left of the
    // viewport, scroll naturally drops back to 0.
    let scroll_x = if avail == 0 {
        0
    } else {
        cursor_col.saturating_sub(avail.saturating_sub(1)) as u16
    };
    frame.render_widget(
        Paragraph::new(content_line)
            .style(Style::default().bg(theme.panel))
            .scroll((0, scroll_x)),
        content_area,
    );

    let preview = preview_line(app);
    frame.render_widget(
        Paragraph::new(preview).style(Style::default().bg(theme.panel)),
        preview_area,
    );

    let hint = hint_line(theme);
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().bg(theme.panel)),
        hint_area,
    );
}

fn preview_line<'a>(app: &App) -> Line<'a> {
    let theme = app.theme();
    let parsed = match app.preview_parse() {
        Some(r) => r,
        None => return Line::raw("").style(Style::default().bg(theme.panel)),
    };
    let mut spans: Vec<Span<'a>> = vec![Span::raw("  ")];
    match parsed {
        Ok(t) => {
            spans.push(Span::styled("ok ", Style::default().fg(theme.dim)));
            if let Some(p) = t.priority {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
                spans.push(Span::styled(
                    format!("pri {p} "),
                    Style::default()
                        .fg(theme.priority_color(p))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if let Some(d) = t.due {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
                spans.push(Span::styled(
                    format!("due {d} "),
                    Style::default().fg(theme.due),
                ));
            }
            let np = t.projects.len();
            let nc = t.contexts.len();
            if np + nc > 0 {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
            }
            if np > 0 {
                spans.push(Span::styled(
                    format!("{np} +"),
                    Style::default().fg(theme.dim),
                ));
                spans.push(Span::styled(
                    if np == 1 { "project " } else { "projects " },
                    Style::default().fg(theme.project),
                ));
            }
            if nc > 0 {
                spans.push(Span::styled(
                    format!("{nc} @"),
                    Style::default().fg(theme.dim),
                ));
                spans.push(Span::styled(
                    if nc == 1 { "context" } else { "contexts" },
                    Style::default().fg(theme.context),
                ));
            }
        }
        Err(e) => {
            spans.push(Span::styled(
                "err ",
                Style::default()
                    .fg(theme.overdue)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!("{e}"), Style::default().fg(theme.dim)));
        }
    }
    Line::from(spans).style(Style::default().bg(theme.panel))
}

pub fn render_prompt(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let (sigil, label) = match app.mode {
        Mode::PromptProject => ("+", " ADD PROJECT "),
        Mode::PromptContext => ("@", " TOGGLE CONTEXT "),
        _ => return,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            label,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [_p, input_area, _p2] = Layout::vertical([
        Constraint::Length(0),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            sigil,
            Style::default()
                .fg(if sigil == "+" {
                    theme.project
                } else {
                    theme.context
                })
                .add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(draft_cursor_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme.fg,
        theme.panel,
    ));
    let line = Line::from(spans).style(Style::default().bg(theme.panel));
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.panel)),
        input_area,
    );
}

/// Colored example tokens illustrating the todo.txt format.
/// Used by both the empty state and the add/edit dialog so they stay in sync.
pub fn format_hint_spans<'a>(theme: &Theme) -> Vec<Span<'a>> {
    use ratatui::style::Modifier;
    vec![
        Span::styled(
            "(A) ",
            Style::default()
                .fg(theme.pri_a)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Buy milk ", Style::default().fg(theme.fg)),
        Span::styled("+shop ", Style::default().fg(theme.project)),
        Span::styled("@home ", Style::default().fg(theme.context)),
        Span::styled("due:2026-05-12", Style::default().fg(theme.due)),
    ]
}

/// Floating suggestion popup anchored just below the add/edit dialog.
/// `dlg` is the dialog rect we're attached to; `screen` is the full frame
/// area, used to keep the popup on-screen when the dialog is near the bottom
/// or right edge. No-op when the popup is hidden.
pub fn render_autocomplete(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    if !app.autocomplete_visible() {
        return;
    }
    let matches = app.autocomplete_matches();
    if matches.is_empty() {
        return;
    }
    let theme = app.theme();
    let kind = match app::active_token(app.draft.text(), app.draft.cursor()) {
        Some(t) => t.kind,
        None => return,
    };
    let (sigil, sigil_color) = match kind {
        TokenKind::Project => ('+', theme.project),
        TokenKind::Context => ('@', theme.context),
    };

    let longest = matches.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    // +3 = leading space, sigil, trailing space.
    let popup_w: u16 = (((longest as u16).saturating_add(3)).max(16)).min(dlg.width.max(16));
    let popup_h: u16 = matches.len() as u16;

    // Anchor below the dialog, aligned to the input prefix ("  › " = 4 cols).
    let mut popup_x = dlg.x + 4;
    let mut popup_y = dlg.y + dlg.height;
    // Keep on-screen when the dialog hugs the bottom/right edge.
    let max_x = screen.x + screen.width.saturating_sub(popup_w);
    let max_y = screen.y + screen.height.saturating_sub(popup_h);
    if popup_x > max_x {
        popup_x = max_x;
    }
    if popup_y > max_y {
        popup_y = max_y;
    }

    let area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_w,
        height: popup_h,
    };
    frame.render_widget(Clear, area);

    let selected = app.draft.autocomplete_index().min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let is_sel = i == selected;
            let bg = if is_sel { theme.accent } else { theme.panel };
            let fg = if is_sel { theme.bg } else { theme.fg };
            Line::from(vec![
                Span::styled(
                    format!(" {}", sigil),
                    Style::default()
                        .fg(sigil_color)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{} ", s), Style::default().fg(fg).bg(bg)),
            ])
            .style(Style::default().bg(bg))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        area,
    );
}

/// Anchor a popup `popup_w` × `popup_h` cells just below `dlg`, clamping into
/// `screen` so it stays visible at the bottom/right edges. Mirrors the
/// `render_autocomplete` placement code so every overlay floats in the same
/// place.
fn anchor_below_dialog(dlg: Rect, screen: Rect, popup_w: u16, popup_h: u16) -> Rect {
    let mut popup_x = dlg.x + 4;
    let mut popup_y = dlg.y + dlg.height;
    let max_x = screen.x + screen.width.saturating_sub(popup_w);
    let max_y = screen.y + screen.height.saturating_sub(popup_h);
    if popup_x > max_x {
        popup_x = max_x;
    }
    if popup_y > max_y {
        popup_y = max_y;
    }
    Rect {
        x: popup_x,
        y: popup_y,
        width: popup_w,
        height: popup_h,
    }
}

/// Dispatch to the right per-overlay render function. Returns true when an
/// overlay rendered, so the caller can skip the regular autocomplete popup.
pub fn render_overlay(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) -> bool {
    match app.draft.overlay() {
        Some(DraftOverlay::SlashMenu(_)) => {
            render_slash_menu(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::Calendar(_)) => {
            render_calendar(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::RecurrenceBuilder(_)) => {
            render_recurrence_builder(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::PriorityChooser(_)) => {
            render_priority_chooser(frame, dlg, screen, app);
            true
        }
        None => false,
    }
}

fn render_slash_menu(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let matches = app.slash_matches();
    if matches.is_empty() {
        return;
    }
    let selected = app.slash_selected();

    // Width: longest label + spacer + longest description + spacer + cmd.
    let label_w = matches
        .iter()
        .map(|e| e.label.chars().count())
        .max()
        .unwrap_or(0);
    let desc_w = matches
        .iter()
        .map(|e| e.description.chars().count())
        .max()
        .unwrap_or(0);
    let cmd_w = matches
        .iter()
        .map(|e| e.cmd.chars().count())
        .max()
        .unwrap_or(0);
    let content_w = label_w + 4 + desc_w + 4 + cmd_w + 4; // padding/spacers
    // Wider than the dialog on purpose so the footer hint fits — anchor
    // placement clamps to the screen edge below.
    let popup_w: u16 = (content_w as u16).max(60).min(screen.width.max(40));
    // Title row + entries + spacer + footer + 2 borders.
    let popup_h: u16 = matches.len() as u16 + 5;

    let area = anchor_below_dialog(dlg, screen, popup_w, popup_h);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "ATTACH METADATA",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    for (i, entry) in matches.iter().enumerate() {
        let is_sel = i == selected;
        let bg = if is_sel { theme.cursor } else { theme.panel };
        let label_style = if is_sel {
            Style::default()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(bg)
        };
        let desc_style = Style::default().fg(theme.dim).bg(bg);
        let cmd_style = Style::default().fg(theme.dim).bg(bg);

        // Right-align the /cmd by padding between description and cmd.
        let label_w_pad = label_w + 2;
        let desc_w_pad = desc_w + 2;
        let total = inner.width as usize;
        let used = 2 + label_w_pad + desc_w_pad + entry.cmd.chars().count() + 1;
        let pad = total.saturating_sub(used);

        let label_padded = pad_to(entry.label, label_w_pad);
        let desc_padded = pad_to(entry.description, desc_w_pad);
        lines.push(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(label_padded, label_style),
                Span::styled(desc_padded, desc_style),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(entry.cmd.to_string(), cmd_style),
                Span::styled(" ", Style::default().bg(bg)),
            ])
            .style(Style::default().bg(bg)),
        );
    }
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(slash_footer(theme));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn pad_to(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        s.to_string()
    } else {
        let mut out = String::with_capacity(s.len() + (width - n));
        out.push_str(s);
        for _ in n..width {
            out.push(' ');
        }
        out
    }
}

fn slash_footer<'a>(theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "↑↓",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" pick · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Enter",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" insert · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Esc",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" dismiss · type to filter", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel))
}

fn render_calendar(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.calendar_state() else {
        return;
    };
    let popup_w: u16 = 50u16.min(screen.width.max(40));
    let popup_h: u16 = 13;
    let area = anchor_below_dialog(dlg, screen, popup_w, popup_h);
    frame.render_widget(Clear, area);
    let label = match state.target {
        CalendarTarget::Due => "DUE",
        CalendarTarget::Threshold => "THRESHOLD",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    use chrono::{Datelike, NaiveDate};
    let focused = state.focused;
    let today = NaiveDate::parse_from_str(&app.today, "%Y-%m-%d").ok();
    let first_of_month =
        NaiveDate::from_ymd_opt(focused.year(), focused.month(), 1).unwrap_or(focused);
    // Sunday-leading week: weekday().num_days_from_sunday() ∈ 0..7.
    let lead = first_of_month.weekday().num_days_from_sunday() as i64;
    let days_in_month = days_in_month(focused.year(), focused.month());

    let mut lines: Vec<Line> = Vec::new();
    // Header: « Month YYYY »
    let header = Line::from(vec![
        Span::raw("  "),
        Span::styled("‹  ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("{} {}", month_name(focused.month()), focused.year()),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ›", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel));
    lines.push(header);
    // Weekday row.
    let dow = Line::from(vec![
        Span::raw("  "),
        Span::styled(" S  M  T  W  T  F  S ", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel));
    lines.push(dow);
    // Up to 6 week rows. Break before any all-blank row so February-style
    // months don't leave a trailing empty week.
    let mut day = 1i64;
    for _week in 0..6 {
        if day - lead >= days_in_month as i64 {
            break;
        }
        let mut spans: Vec<Span> = vec![Span::raw("  ")];
        for col in 0..7 {
            let pos = day - lead + col;
            if pos < 0 || pos >= days_in_month as i64 {
                spans.push(Span::styled("    ", Style::default().bg(theme.panel)));
            } else {
                let n = (pos + 1) as u32;
                let cell = NaiveDate::from_ymd_opt(focused.year(), focused.month(), n);
                let is_today = today == cell;
                let is_focus = focused.day() == n;
                let mut style = Style::default().fg(theme.fg).bg(theme.panel);
                if is_today {
                    style = style.fg(theme.today);
                }
                if is_focus {
                    style = style.bg(theme.cursor).add_modifier(Modifier::BOLD);
                }
                spans.push(Span::styled(format!(" {:>2} ", n), style));
            }
        }
        lines.push(Line::from(spans).style(Style::default().bg(theme.panel)));
        day += 7;
    }
    // Spacer + focused-date label.
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format_focused(focused),
                Style::default().fg(theme.due).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    lines.push(calendar_footer(theme));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn calendar_footer<'a>(theme: &Theme) -> Line<'a> {
    let chip = |k: &'static str, label: &'static str| -> Vec<Span<'a>> {
        vec![
            Span::styled(
                k,
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {label} "), Style::default().fg(theme.dim)),
        ]
    };
    let mut spans = vec![Span::raw("  ")];
    spans.extend(chip("t", "today"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("T", "tmw"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("w", "+1w"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("m", "+1mo"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("x", "clear"));
    Line::from(spans).style(Style::default().bg(theme.panel))
}

fn render_recurrence_builder(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.recurrence_state() else {
        return;
    };
    let popup_w: u16 = 60;
    let popup_h: u16 = 9;
    let area = anchor_below_dialog(dlg, screen, popup_w, popup_h);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            " ↻ REPEAT ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let pill = |label: &str, focused: bool, theme: &Theme| -> Span<'static> {
        let bg = if focused { theme.cursor } else { theme.panel };
        let fg = theme.fg;
        let m = if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        Span::styled(
            format!(" {label} "),
            Style::default().fg(fg).bg(bg).add_modifier(m),
        )
    };

    let interval_focus = state.field == BuilderField::Interval;
    let unit_focus = state.field == BuilderField::Unit;
    let mode_focus = state.field == BuilderField::Mode;

    // every {N} day/business/week/month/year — single source of truth in
    // REC_UNIT_ORDER so the cycle and render can't drift apart.
    let mut every_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("every ", Style::default().fg(theme.dim)),
        pill(&state.interval.to_string(), interval_focus, theme),
        Span::raw("  "),
    ];
    for unit in REC_UNIT_ORDER.iter().copied() {
        let sel = state.unit == unit;
        let label = match unit {
            crate::recurrence::RecUnit::Day => "day",
            crate::recurrence::RecUnit::Week => "week",
            crate::recurrence::RecUnit::Month => "month",
            crate::recurrence::RecUnit::Year => "year",
            crate::recurrence::RecUnit::BusinessDay => "business",
        };
        let focused = unit_focus && sel;
        let style = if sel {
            Style::default()
                .fg(theme.fg)
                .bg(if focused { theme.accent } else { theme.cursor })
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim).bg(theme.panel)
        };
        every_spans.push(Span::styled(format!(" {label} "), style));
    }
    let line1 = Line::from(every_spans).style(Style::default().bg(theme.panel));

    // mode  strict / after-complete    next: ...
    let strict_label = " strict ";
    let after_label = " after-complete ";
    let mode_bg_strict = if state.strict {
        theme.cursor
    } else {
        theme.panel
    };
    let mode_bg_after = if !state.strict {
        theme.cursor
    } else {
        theme.panel
    };
    let mode_emph_strict = if mode_focus && state.strict {
        theme.accent
    } else {
        mode_bg_strict
    };
    let mode_emph_after = if mode_focus && !state.strict {
        theme.accent
    } else {
        mode_bg_after
    };
    let mut line2_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("mode  ", Style::default().fg(theme.dim)),
        Span::styled(
            strict_label,
            Style::default()
                .fg(theme.fg)
                .bg(mode_emph_strict)
                .add_modifier(if state.strict {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::raw(" "),
        Span::styled(
            after_label,
            Style::default()
                .fg(theme.fg)
                .bg(mode_emph_after)
                .add_modifier(if !state.strict {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
    ];
    let next = crate::app::recurrence_next_preview(state, &app.today)
        .map(format_focused)
        .unwrap_or_else(|| "—".into());
    let next_label = format!("next: {next}");
    // Measure the already-built left side instead of hardcoding a width that
    // silently drifts when the mode-line copy changes. The `+ 2` keeps a
    // 2-cell margin from the right border when there's room.
    let left_width: usize = line2_spans.iter().map(|s| s.content.chars().count()).sum();
    let next_pad =
        (inner.width as usize).saturating_sub(left_width + next_label.chars().count() + 2);
    line2_spans.push(Span::styled(
        " ".repeat(next_pad),
        Style::default().bg(theme.panel),
    ));
    line2_spans.push(Span::styled(next_label, Style::default().fg(theme.dim)));
    let line2 = Line::from(line2_spans).style(Style::default().bg(theme.panel));

    let value = crate::app::format_rec_value(state);
    let line_preview = Line::from(vec![
        Span::raw("  "),
        Span::styled("→ writes ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("rec:{value}"),
            Style::default().fg(theme.due).add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().bg(theme.panel));

    let lines = vec![
        Line::raw("").style(Style::default().bg(theme.panel)),
        line1,
        Line::raw("").style(Style::default().bg(theme.panel)),
        line2,
        Line::raw("").style(Style::default().bg(theme.panel)),
        line_preview,
        Line::raw("").style(Style::default().bg(theme.panel)),
        rec_footer(theme),
    ];

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn rec_footer<'a>(theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "hjkl",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" move · ", Style::default().fg(theme.dim)),
        Span::styled(
            "+/-",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" adjust · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Enter",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" save · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Esc",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" cancel", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel))
}

fn render_priority_chooser(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.priority_state() else {
        return;
    };
    let popup_w: u16 = 24;
    let popup_h: u16 = 8;
    let area = anchor_below_dialog(dlg, screen, popup_w, popup_h);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            " PRIORITY ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: [(u8, &str, ratatui::style::Color); 4] = [
        (0, "(A)", theme.pri_a),
        (1, "(B)", theme.pri_b),
        (2, "(C)", theme.pri_c),
        (3, "clear", theme.dim),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (i, label, color) in rows {
        let is_sel = state.selected == i;
        let bg = if is_sel { theme.cursor } else { theme.panel };
        let m = if is_sel {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        lines.push(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(
                    label.to_string(),
                    Style::default().fg(color).bg(bg).add_modifier(m),
                ),
                Span::styled(
                    " ".repeat((inner.width as usize).saturating_sub(2 + label.chars().count())),
                    Style::default().bg(bg),
                ),
            ])
            .style(Style::default().bg(bg)),
        );
    }
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "jk",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" move · ", Style::default().fg(theme.dim)),
            Span::styled(
                "Enter",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" set", Style::default().fg(theme.dim)),
        ])
        .style(Style::default().bg(theme.panel)),
    );

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn days_in_month(year: i32, month: u32) -> u32 {
    use chrono::NaiveDate;
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1);
    let first_this = NaiveDate::from_ymd_opt(year, month, 1);
    match (first_next, first_this) {
        (Some(n), Some(t)) => (n - t).num_days() as u32,
        _ => 30,
    }
}

fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "?",
    }
}

fn format_focused(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let dow = match d.weekday() {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    };
    let mon = match d.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "?",
    };
    format!("{dow} {mon} {}", d.day())
}

fn hint_line<'a>(theme: &Theme) -> Line<'a> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled("format: ", Style::default().fg(theme.dim)),
    ];
    spans.extend(format_hint_spans(theme));
    Line::from(spans).style(Style::default().bg(theme.panel))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use crate::app::{App, Mode};
    use crate::config::Config;

    /// Pull just the rows immediately below the centered Insert dialog where
    /// the popup floats — avoids matching against the sidebar / status bar
    /// content that contains the same project / context names.
    fn popup_region_text(buf: &Buffer) -> String {
        // Mirror the dialog placement in `ui::draw`: 8 rows tall, centered.
        // The popup begins at dlg.y + dlg.height and is up to 8 rows tall.
        let rows = buf.area.height;
        let cols = buf.area.width;
        let dlg_h: u16 = 8;
        let dlg_y = (rows.saturating_sub(dlg_h)) / 2;
        let popup_top = dlg_y + dlg_h;
        let popup_bottom = (popup_top + 8).min(rows);
        let mut out = String::new();
        for y in popup_top..popup_bottom {
            for x in 0..cols {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn build_insert_app(seed: &str, draft: &str) -> App {
        let path = std::env::temp_dir().join(format!(
            "tuxedo-dialog-test-{}-{}.txt",
            std::process::id(),
            seed.len(),
        ));
        std::fs::write(&path, seed).unwrap();
        let mut app = App::new(
            path,
            seed.to_string(),
            "2026-05-06".to_string(),
            Config::default(),
        );
        app.mode = Mode::Insert;
        app.draft_set(draft.to_string());
        app
    }

    #[test]
    fn classify_plain_text_is_all_plain() {
        let r = super::classify_draft("Hello world");
        assert!(
            r.iter()
                .all(|(_, k)| matches!(k, super::SegmentKind::Plain))
        );
        let mut prev = 0;
        for (range, _) in &r {
            assert_eq!(range.start, prev);
            prev = range.end;
        }
        assert_eq!(prev, "Hello world".len());
    }

    #[test]
    fn classify_priority_at_start() {
        let r = super::classify_draft("(A) Hello");
        assert!(matches!(r[0].1, super::SegmentKind::Priority('A')));
        assert_eq!(r[0].0, 0..3);
    }

    #[test]
    fn classify_creation_date() {
        let r = super::classify_draft("2026-05-01 Hello");
        assert!(matches!(r[0].1, super::SegmentKind::Date));
        assert_eq!(r[0].0, 0..10);
    }

    #[test]
    fn classify_project_token() {
        let s = "Hello +work";
        let r = super::classify_draft(s);
        let proj = r
            .iter()
            .find(|(_, k)| matches!(k, super::SegmentKind::Project))
            .unwrap();
        assert_eq!(&s[proj.0.clone()], "+work");
    }

    #[test]
    fn classify_context_token() {
        let s = "Hello @home";
        let r = super::classify_draft(s);
        let ctx = r
            .iter()
            .find(|(_, k)| matches!(k, super::SegmentKind::Context))
            .unwrap();
        assert_eq!(&s[ctx.0.clone()], "@home");
    }

    #[test]
    fn classify_due_keyvalue() {
        let s = "Hello due:2026-05-15";
        let r = super::classify_draft(s);
        let due = r
            .iter()
            .find(|(_, k)| matches!(k, super::SegmentKind::Due))
            .unwrap();
        assert_eq!(&s[due.0.clone()], "due:2026-05-15");
    }

    #[test]
    fn classify_other_keyvalue() {
        let s = "Hello rec:1w";
        let r = super::classify_draft(s);
        let kv = r
            .iter()
            .find(|(_, k)| matches!(k, super::SegmentKind::KeyValue))
            .unwrap();
        assert_eq!(&s[kv.0.clone()], "rec:1w");
    }

    #[test]
    fn classify_full_line_covers_all_bytes() {
        let s = "(A) 2026-05-01 Buy milk +shop @home due:2026-05-12";
        let r = super::classify_draft(s);
        let mut prev = 0;
        for (range, _) in &r {
            assert_eq!(range.start, prev);
            prev = range.end;
        }
        assert_eq!(prev, s.len());
        assert!(matches!(r[0].1, super::SegmentKind::Priority('A')));
    }

    #[test]
    fn classify_done_marker_then_date() {
        let s = "x 2026-05-05 thing";
        let r = super::classify_draft(s);
        let date_seg = r
            .iter()
            .find(|(_, k)| matches!(k, super::SegmentKind::Date))
            .unwrap();
        assert_eq!(&s[date_seg.0.clone()], "2026-05-05");
    }

    #[test]
    fn classify_lone_sigil_stays_plain() {
        // A bare "+" or "@" with no following text shouldn't get a sigil
        // colour — it's just a character the user is mid-typing.
        let s = "Foo + bar";
        let r = super::classify_draft(s);
        let plus = r
            .iter()
            .find(|(range, _)| &s[range.clone()] == "+")
            .expect("lone + should appear as its own segment");
        assert!(matches!(plus.1, super::SegmentKind::Plain));
    }

    /// Pull the dialog's interior rows (between the borders) — preview lives
    /// on row 3 of the inner area in the current layout.
    fn dialog_inner_text(buf: &Buffer) -> String {
        let rows = buf.area.height;
        let cols = buf.area.width;
        let dlg_h: u16 = 9;
        let dlg_y = (rows.saturating_sub(dlg_h)) / 2;
        let mut out = String::new();
        for y in dlg_y..(dlg_y + dlg_h) {
            for x in 0..cols {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn input_row_scrolls_to_keep_cursor_visible_for_long_draft() {
        // A draft longer than the dialog's content area must scroll
        // horizontally so the tail (where the cursor sits) stays visible —
        // otherwise the user can't see what they're typing past the right
        // edge.
        let tail = "ZZSCROLLTAIL";
        let draft = format!("{}{}", "x".repeat(80), tail);
        let app = build_insert_app("plain\n", &draft);
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        // Dialog is 8 rows tall, centered in a 30-row area; input lives on
        // the second inner row (top border + 1 row padding + input).
        let dlg_y = (30u16 - 8) / 2;
        let input_y = dlg_y + 2;
        let mut row = String::new();
        for x in 0..80 {
            row.push_str(buf[(x, input_y)].symbol());
        }
        assert!(
            row.contains(tail),
            "input row should scroll so the cursor end ({tail}) stays visible:\n{row}"
        );
    }

    #[test]
    fn preview_line_shows_priority_chip() {
        let app = build_insert_app("plain\n", "(A) Buy milk");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let text = dialog_inner_text(terminal.backend().buffer());
        assert!(text.contains("ok"), "preview should say 'ok'\n{text}");
        assert!(
            text.contains("pri A"),
            "preview should show 'pri A'\n{text}"
        );
    }

    #[test]
    fn preview_line_blank_when_draft_empty() {
        let app = build_insert_app("plain\n", "");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let text = dialog_inner_text(terminal.backend().buffer());
        // No "ok" or "err" badge when draft is empty.
        assert!(
            !text.contains("ok "),
            "empty draft should not render preview\n{text}"
        );
        assert!(
            !text.contains("err "),
            "empty draft should not render preview\n{text}"
        );
    }

    #[test]
    fn autocomplete_popup_renders_project_matches() {
        let app = build_insert_app(
            "(A) 2026-05-01 a +work\n(A) 2026-05-01 b +health\n",
            "Foo +",
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("health"),
            "expected 'health' in popup\n{popup}"
        );
        assert!(popup.contains("work"), "expected 'work' in popup\n{popup}");
    }

    #[test]
    fn autocomplete_popup_hidden_when_no_token() {
        // A draft with no `+` / `@` token at the cursor should leave the
        // popup region empty even if the corpus has projects.
        let app = build_insert_app("(A) 2026-05-01 a +uniqueprojname\n", "plain text");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            !popup.contains("uniqueprojname"),
            "popup region should not list corpus when no active token\n{popup}"
        );
    }

    #[test]
    fn autocomplete_popup_filters_by_context_kind() {
        let app = build_insert_app("(A) 2026-05-01 a +uniqueprojname @uniquecontext\n", "Foo @");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("uniquecontext"),
            "expected context value in popup\n{popup}"
        );
        assert!(
            !popup.contains("uniqueprojname"),
            "context popup must not list projects\n{popup}"
        );
    }
}
