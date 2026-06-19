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
    /// `key:value` tokens whose key is in this list are omitted from the
    /// rendered body. Empty (the common case) means render everything,
    /// byte-for-byte as before.
    pub hidden_keys: &'a [String],
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
    let body = body_after_priority(&task.clean_raw);
    let body_match_positions: Option<Vec<usize>> =
        opts.match_term.and_then(|n| subseq_match_ci(body, n));
    let body_start = body.as_ptr() as usize;
    let mut rest = body;
    // Whether any visible body token has been emitted yet. Drives the
    // hidden-token branch's whitespace fix-up so a skipped token never
    // leaves a leading, trailing, or doubled space. When `hidden_keys`
    // is empty the branch is never entered and output is byte-identical
    // to before.
    let mut emitted_body_token = false;
    while !rest.is_empty() {
        let ws_end = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        let pushed_ws = ws_end > 0;
        if pushed_ws {
            spans.push(Span::raw(&rest[..ws_end]));
            rest = &rest[ws_end..];
        }
        if rest.is_empty() {
            break;
        }
        let tok_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..tok_end];
        if is_hidden_kv(token, opts.hidden_keys) {
            // Drop the separator we just emitted for this token...
            if pushed_ws {
                spans.pop();
            }
            rest = &rest[tok_end..];
            // ...and if nothing visible precedes it, also swallow the
            // following whitespace run so the next token doesn't inherit
            // an orphan leading space.
            if !emitted_body_token {
                let n = rest
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(rest.len());
                rest = &rest[n..];
            }
            continue;
        }
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
        emitted_body_token = true;
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
    // URLs are picked off before the generic key:value branch — `http:` would
    // otherwise classify as a lowercase key and steal the underline + accent
    // styling that doubles as the OSC 8 hyperlink marker (see `ui::hyperlinks`).
    if is_url_token(token) {
        spans.push(Span::styled(token, url_token_style(task.done, theme)));
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

/// True when `token` is a `key:value` pair whose key (case-insensitively)
/// appears in `hidden_keys`. Empty list short-circuits so the common path
/// stays allocation- and comparison-free.
fn is_hidden_kv(token: &str, hidden_keys: &[String]) -> bool {
    if hidden_keys.is_empty() {
        return false;
    }
    match token.split_once(':') {
        Some((k, v)) if !k.is_empty() && !v.is_empty() => {
            hidden_keys.iter().any(|h| h.eq_ignore_ascii_case(k))
        }
        _ => false,
    }
}

pub(crate) fn is_url_token(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
}

pub(crate) fn url_token_style(task_done: bool, theme: &Theme) -> Style {
    let color = if task_done { theme.done } else { theme.accent };
    let mut style = Style::default()
        .fg(color)
        .add_modifier(Modifier::UNDERLINED);
    if task_done {
        style = style.add_modifier(Modifier::DIM);
    }
    style
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
            hidden_keys: &[],
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
            hidden_keys: &[],
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

    /// Render `raw` and return the body text (all span content joined,
    /// fixed glyph/priority prefix trimmed). Tasks here carry no priority
    /// and aren't done, so the prefix is pure leading whitespace and the
    /// "no leading body space" invariant makes `trim_start` exact.
    fn body_text(raw: &str, hidden: &[String]) -> String {
        let task = parse_line(raw).unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: hidden,
        };
        let line = build_line(&task, opts, &MUTED);
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .trim_start()
            .to_string()
    }

    #[test]
    fn hidden_key_in_middle_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(
            body_text("Call dentist uid:abc-123 @phone +health", &h),
            "Call dentist @phone +health",
        );
    }

    #[test]
    fn hidden_key_at_start_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("uid:abc-123 Call dentist", &h), "Call dentist");
    }

    #[test]
    fn hidden_key_at_end_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("Call dentist uid:abc-123", &h), "Call dentist");
    }

    #[test]
    fn adjacent_hidden_keys_collapse_to_single_space() {
        let h = vec!["uid".to_string(), "sync".to_string()];
        assert_eq!(body_text("Call uid:a sync:b dentist", &h), "Call dentist",);
    }

    #[test]
    fn hidden_key_match_is_case_insensitive() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("Call UID:abc done", &h), "Call done");
    }

    #[test]
    fn empty_hidden_list_renders_everything_unchanged() {
        assert_eq!(
            body_text("Call dentist uid:abc @phone +health", &[]),
            "Call dentist uid:abc @phone +health",
        );
    }

    #[test]
    fn url_token_is_underlined_and_accented() {
        // The underline modifier is the sentinel `ui::hyperlinks::linkify`
        // looks for. If this test fails, OSC 8 hyperlinks silently stop being
        // emitted — break it intentionally only when changing the marker.
        let task = parse_line("See https://example.com for details").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
        };
        let line = build_line(&task, opts, &MUTED);
        let url_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "https://example.com")
            .expect("URL token rendered as its own span");
        assert!(
            url_span.style.add_modifier.contains(Modifier::UNDERLINED),
            "URL span must carry Modifier::UNDERLINED; got {:?}",
            url_span.style,
        );
        assert_eq!(url_span.style.fg, Some(MUTED.accent));
    }

    #[test]
    fn url_token_not_classified_as_key_value() {
        // Without the URL branch in front of the generic key:value branch,
        // `http:` would split into ("http", "//example.com") and render with
        // the dim key-value style instead of the accent + underline.
        let task = parse_line("note http://example.com").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
        };
        let line = build_line(&task, opts, &MUTED);
        let url_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "http://example.com")
            .expect("URL span");
        assert_ne!(
            url_span.style.fg,
            Some(MUTED.dim),
            "URL must not pick up the dim key-value color",
        );
    }

    #[test]
    fn non_listed_key_not_hidden() {
        let h = vec!["uid".to_string()];
        // `due:` stays; only configured keys are dropped.
        assert_eq!(
            body_text("Pay rent due:2026-05-15 uid:x", &h),
            "Pay rent due:2026-05-15",
        );
    }
}
