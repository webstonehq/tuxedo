use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::search::subseq_match_ci;
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
    let body_match_positions: Option<Vec<usize>> =
        opts.match_term.and_then(|n| subseq_match_ci(body, n));
    let body_start = body.as_ptr() as usize;
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
        let token = &rest[..tok_end];
        let token_offset = token.as_ptr() as usize - body_start;
        push_token_spans(
            &mut spans,
            token,
            token_offset,
            body_match_positions.as_deref(),
            task,
            opts,
            theme,
        );
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
    token_offset_in_body: usize,
    body_match_positions: Option<&[usize]>,
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

    // plain word — highlight each matched subsequence char inside this token.
    let base_color = if task.done { theme.done } else { theme.fg };
    let base_style = apply_dim(Style::default().fg(base_color), task.done);
    let hl_style = Style::default()
        .fg(theme.bg)
        .bg(theme.matched)
        .add_modifier(Modifier::BOLD);

    let token_end = token_offset_in_body + token.len();
    let mut local_positions = body_match_positions
        .into_iter()
        .flatten()
        .copied()
        .filter(|&p| p >= token_offset_in_body && p < token_end)
        .map(|p| p - token_offset_in_body)
        .peekable();

    if local_positions.peek().is_none() {
        spans.push(Span::styled(token, base_style));
        return;
    }

    let mut cursor = 0usize;
    for p in local_positions {
        if cursor < p {
            spans.push(Span::styled(&token[cursor..p], base_style));
        }
        let ch = token[p..]
            .chars()
            .next()
            .expect("match offset lands on a char boundary");
        let next = p + ch.len_utf8();
        spans.push(Span::styled(&token[p..next], hl_style));
        cursor = next;
    }
    if cursor < token.len() {
        spans.push(Span::styled(&token[cursor..], base_style));
    }
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
    fn build_line_highlights_subsequence_chars() {
        // "cade" is a subsequence of "Call dentist": C(0), a(1), D(5), e(6).
        // The renderer should emit highlighted single-char spans for those
        // positions, with the unmatched chars rendered in the base style.
        let task = parse_line("Call dentist").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: Some("cade"),
            today: "2026-05-06",
        };
        let line = build_line(&task, opts, &MUTED);
        let highlight_bg = MUTED.matched;
        let highlighted: String = line
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(highlight_bg))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted, "Cade");
    }
}
