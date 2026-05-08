use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{self, App, Mode, TokenKind};
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
