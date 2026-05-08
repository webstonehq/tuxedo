use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;
use crate::todo::{Task, body_after_priority};

#[derive(Clone, Copy, Default)]
pub struct RowOpts<'a> {
    pub idx_label: usize,
    pub cursor: bool,
    pub multi_mode: bool,
    pub multi_checked: bool,
    pub selected: bool,
    pub show_line_num: bool,
    pub match_term: Option<&'a str>,
    pub today: &'a str,
}

pub fn build_line<'a>(task: &'a Task, opts: RowOpts<'a>, theme: &Theme) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();

    if opts.show_line_num {
        spans.push(Span::styled(
            format!("{:>3} ", opts.idx_label + 1),
            Style::default().fg(theme.dim),
        ));
    }
    if opts.multi_mode {
        let mark = if opts.multi_checked { "[x] " } else { "[ ] " };
        let c = if opts.multi_checked {
            theme.accent
        } else {
            theme.dim
        };
        spans.push(Span::styled(mark, Style::default().fg(c)));
    }

    // status glyph + priority box
    let glyph = if task.done {
        "✓ "
    } else if opts.cursor {
        "▸ "
    } else {
        "  "
    };
    let glyph_color = if task.done { theme.done } else { theme.accent };
    let mut glyph_style = Style::default().fg(glyph_color);
    if opts.cursor {
        glyph_style = glyph_style.add_modifier(Modifier::BOLD);
    }
    spans.push(Span::styled(glyph, glyph_style));

    if task.done {
        spans.push(Span::styled("    ", Style::default().fg(theme.done)));
    } else if let Some(p) = task.priority {
        spans.push(Span::styled(
            format!("({}) ", p),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("    "));
    }

    // body — walk &str slices instead of collecting Vec<char>. Spans borrow
    // straight from `task.raw`, so most rows allocate only for the format!()
    // calls above.
    let body = body_after_priority(&task.raw);
    let mut rest = body;
    while !rest.is_empty() {
        let ws_end = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        if ws_end > 0 {
            spans.push(Span::raw(&rest[..ws_end]));
            rest = &rest[ws_end..];
        }
        if rest.is_empty() {
            break;
        }
        let tok_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        push_token_spans(&mut spans, &rest[..tok_end], task, opts, theme);
        rest = &rest[tok_end..];
    }
    let line_style = if opts.cursor {
        Style::default().bg(theme.cursor)
    } else if opts.selected {
        Style::default().bg(theme.selected)
    } else {
        Style::default()
    };
    Line::from(spans).style(line_style)
}

fn push_token_spans<'a>(
    spans: &mut Vec<Span<'a>>,
    token: &'a str,
    task: &Task,
    opts: RowOpts<'a>,
    theme: &Theme,
) {
    if let Some(c) = sigil_token_color(token, task, theme) {
        spans.push(Span::styled(token, Style::default().fg(c)));
        return;
    }
    if let Some(rest) = token.strip_prefix("due:") {
        spans.push(Span::styled(
            token,
            due_token_style(task.done, rest, opts.today, theme),
        ));
        return;
    }
    // generic key:value (lowercase key)
    if let Some((k, _v)) = token.split_once(':')
        && !k.is_empty()
        && k.chars()
            .next()
            .expect("invariant: !k.is_empty() guarded above")
            .is_ascii_lowercase()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        spans.push(Span::styled(token, Style::default().fg(theme.dim)));
        return;
    }

    // plain word — apply match-term highlight
    let base_color = if task.done { theme.done } else { theme.fg };
    let dim_done = task.done;
    if let Some(needle) = opts.match_term
        && let Some((start, end)) = match_range_ci(token, needle)
    {
        let before = &token[..start];
        let mid = &token[start..end];
        let after = &token[end..];
        if !before.is_empty() {
            spans.push(Span::styled(
                before,
                apply_dim(Style::default().fg(base_color), dim_done),
            ));
        }
        spans.push(Span::styled(
            mid,
            Style::default()
                .fg(theme.bg)
                .bg(theme.matched)
                .add_modifier(Modifier::BOLD),
        ));
        if !after.is_empty() {
            spans.push(Span::styled(
                after,
                apply_dim(Style::default().fg(base_color), dim_done),
            ));
        }
        return;
    }
    spans.push(Span::styled(
        token,
        apply_dim(Style::default().fg(base_color), dim_done),
    ));
}

fn sigil_token_color(token: &str, task: &Task, theme: &Theme) -> Option<Color> {
    if !token.starts_with('+') && !token.starts_with('@') {
        return None;
    }
    if task.done {
        return Some(theme.done);
    }
    if token.starts_with('+') {
        Some(theme.project)
    } else {
        Some(theme.context)
    }
}

fn apply_dim(style: Style, dim: bool) -> Style {
    if dim {
        style.add_modifier(Modifier::DIM)
    } else {
        style
    }
}

#[derive(Copy, Clone)]
enum DueStatus {
    Overdue,
    Today,
    Soon,
    Later,
    None,
}

fn due_status(due: &str, today: &str) -> DueStatus {
    if due.len() != 10 || today.len() != 10 {
        return DueStatus::None;
    }
    match due.cmp(today) {
        std::cmp::Ordering::Less => DueStatus::Overdue,
        std::cmp::Ordering::Equal => DueStatus::Today,
        std::cmp::Ordering::Greater => {
            // within 2 days?
            let d = day_diff(due, today).unwrap_or(99);
            if d <= 2 {
                DueStatus::Soon
            } else {
                DueStatus::Later
            }
        }
    }
}

fn day_diff(a: &str, b: &str) -> Option<i64> {
    let to_ymd = |s: &str| -> Option<(i32, u32, u32)> {
        let y = s.get(0..4)?.parse().ok()?;
        let mo = s.get(5..7)?.parse().ok()?;
        let d = s.get(8..10)?.parse().ok()?;
        Some((y, mo, d))
    };
    let (ay, am, ad) = to_ymd(a)?;
    let (by, bm, bd) = to_ymd(b)?;
    let da = chrono::NaiveDate::from_ymd_opt(ay, am, ad)?;
    let db = chrono::NaiveDate::from_ymd_opt(by, bm, bd)?;
    Some(da.signed_duration_since(db).num_days())
}

/// Find the first case-insensitive occurrence of `needle` in `token`,
/// returning a byte range in `token` that always lands on char boundaries.
///
/// The naive `token.to_lowercase().find(...)` then byte-slice approach is
/// broken whenever a codepoint's lowercase has a different UTF-8 length
/// (e.g. Turkish `İ` lowercases to `i` + combining dot, gaining a byte).
/// Slicing the original token at offsets derived from the lowercased
/// string then panics on the non-boundary cut.
pub(crate) fn match_range_ci(token: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let n_lower = needle.to_lowercase();
    let starts: Vec<usize> = token.char_indices().map(|(i, _)| i).collect();
    for &start in &starts {
        let mut remaining = n_lower.as_str();
        for (off, ch) in token[start..].char_indices() {
            let lower: String = ch.to_lowercase().collect();
            let Some(rest) = remaining.strip_prefix(lower.as_str()) else {
                break;
            };
            remaining = rest;
            if remaining.is_empty() {
                return Some((start, start + off + ch.len_utf8()));
            }
        }
    }
    None
}

pub(crate) fn due_token_style(task_done: bool, due: &str, today: &str, theme: &Theme) -> Style {
    let status = due_status(due, today);
    let c = if task_done {
        theme.done
    } else {
        match status {
            DueStatus::Overdue => theme.overdue,
            DueStatus::Today => theme.today,
            DueStatus::Soon => theme.due,
            DueStatus::Later | DueStatus::None => theme.dim,
        }
    };
    let mut style = Style::default().fg(c);
    if matches!(status, DueStatus::Overdue | DueStatus::Today) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

pub fn due_label(due: &str, today: &str) -> String {
    if let Some(d) = day_diff(due, today) {
        if d < 0 {
            return if d == -1 {
                "overdue 1d".into()
            } else {
                format!("overdue {}d", -d)
            };
        }
        if d == 0 {
            return "today".into();
        }
        if d == 1 {
            return "tomorrow".into();
        }
        if d < 7 {
            return format!("in {}d", d);
        }
    }
    due.to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::MUTED;
    use crate::todo::parse_line;

    #[test]
    fn build_line_does_not_panic_on_unicode_with_match_term() {
        // Regression: the previous lowercase-find-then-byte-slice approach
        // panics here. "İ".to_lowercase() = "i" + combining dot (3 bytes vs
        // 2 in the original), so the match offset derived from the
        // lowercased string lands off a char boundary in the source token.
        let task = parse_line("İa").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: Some("a"),
            today: "2026-05-06",
        };
        // Build must not panic; we don't assert on the rendered spans.
        let _ = build_line(&task, opts, &MUTED);
    }

    #[test]
    fn match_range_ascii_finds_substring_case_insensitive() {
        assert_eq!(match_range_ci("Hello", "ell"), Some((1, 4)));
        assert_eq!(match_range_ci("HELLO", "ell"), Some((1, 4)));
        assert_eq!(match_range_ci("hello", "ELL"), Some((1, 4)));
    }

    #[test]
    fn match_range_returns_token_byte_range_for_unicode() {
        // "Café" byte layout: C(0) a(1) f(2) é(3..5). Matching "fé" must
        // return (2, 5) — pointing at the original token's bytes, on char
        // boundaries — not a position derived from the lowercased copy.
        assert_eq!(match_range_ci("Café", "fé"), Some((2, 5)));
        assert_eq!(match_range_ci("Café", "FÉ"), Some((2, 5)));
    }

    #[test]
    fn match_range_empty_or_missing_returns_none() {
        assert_eq!(match_range_ci("anything", ""), None);
        assert_eq!(match_range_ci("Hello", "xyz"), None);
    }
}
