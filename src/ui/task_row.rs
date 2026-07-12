use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
    let (mut spans, body, style) = build_line_parts(task, opts, theme);
    spans.extend(body);
    Line::from(spans).style(style)
}

/// Gutter spans, body spans, and row style for one task. The gutter/body
/// boundary is authoritative here — `build_lines` wraps the body and indents
/// continuations by the gutter's width, so the split must come from the same
/// place that emits the spans.
fn build_line_parts<'a>(
    task: &'a Task,
    opts: RowOpts<'a>,
    theme: &Theme,
) -> (Vec<Span<'a>>, Vec<Span<'a>>, Style) {
    let mut gutter: Vec<Span<'a>> = Vec::new();

    if opts.show_line_num {
        gutter.push(Span::styled(
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
        gutter.push(Span::styled(mark, Style::default().fg(c)));
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
    gutter.push(Span::styled(glyph, glyph_style));

    if task.done {
        gutter.push(Span::styled("    ", Style::default().fg(theme.done)));
    } else if let Some(p) = task.priority {
        gutter.push(Span::styled(
            format!("({}) ", p),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        gutter.push(Span::raw("    "));
    }

    let mut spans: Vec<Span<'a>> = Vec::new();
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
    (gutter, spans, line_style)
}

/// Minimum body columns required before wrapping engages. Below this the
/// pane is essentially all gutter, and wrapping would emit one (clipped,
/// invisible) column per line — strictly worse than the classic clipped
/// single line, which at least keeps the list compact.
const MIN_WRAP_BODY_COLS: usize = 4;

/// Build the lines for a task row. With `wrap_width: None` this is exactly
/// one line, identical to `build_line`. With `Some(width)` the body wraps at
/// word boundaries to `width` columns: continuation lines are indented to
/// the body's start column so the glyph/priority/line-number gutter stays
/// clean, and they carry the row's line style so cursor/selection
/// highlighting spans every wrapped line. Width accounting uses the same
/// unicode-width measurements as ratatui's renderer, so wrap math and cell
/// clipping can't disagree.
///
/// Rows that fit `width`, and panes too narrow to wrap usefully (see
/// `MIN_WRAP_BODY_COLS`), take an allocation-free fast path back to the
/// single-line form.
///
/// The detail pane keeps its own string-level `wrap_words` because it wraps
/// *unstyled* raw text before styling tokens; here the spans are already
/// styled (search-match highlighting can split a word into single-char
/// spans), so wrapping has to preserve span boundaries instead of
/// re-splitting a plain string.
pub fn build_lines<'a>(
    task: &'a Task,
    opts: RowOpts<'a>,
    theme: &Theme,
    wrap_width: Option<u16>,
) -> Vec<Line<'a>> {
    let (gutter, body, style) = build_line_parts(task, opts, theme);
    let single = |mut gutter: Vec<Span<'a>>, body: Vec<Span<'a>>| {
        gutter.extend(body);
        vec![Line::from(gutter).style(style)]
    };
    let Some(width) = wrap_width else {
        return single(gutter, body);
    };
    let total = usize::from(width);
    let indent: usize = gutter.iter().map(Span::width).sum();
    if total < indent + MIN_WRAP_BODY_COLS {
        return single(gutter, body);
    }
    let body_w: usize = body.iter().map(Span::width).sum();
    if indent + body_w <= total {
        return single(gutter, body);
    }
    wrap_spans(gutter, body, style, total, indent)
}

/// In-progress wrap state shared by `wrap_spans` and `hard_break`: the
/// completed lines, the line being filled, and its display width. `cur_w`
/// is the single source of truth — fit checks and the "line already shows
/// body text" test both derive from it, so the two functions can't drift
/// out of sync.
struct LineAcc<'a> {
    done: Vec<Line<'a>>,
    cur: Vec<Span<'a>>,
    cur_w: usize,
    avail: usize,
    indent: usize,
    style: Style,
}

impl<'a> LineAcc<'a> {
    fn new(gutter: Vec<Span<'a>>, avail: usize, indent: usize, style: Style) -> Self {
        Self {
            done: Vec::new(),
            cur: gutter,
            cur_w: indent,
            avail,
            indent,
            style,
        }
    }

    fn fits(&self, w: usize) -> bool {
        self.cur_w + w <= self.avail
    }

    /// Whether the current line holds anything beyond the gutter/indent.
    fn has_body(&self) -> bool {
        self.cur_w > self.indent
    }

    fn push(&mut self, span: Span<'a>, w: usize) {
        self.cur.push(span);
        self.cur_w += w;
    }

    /// Close the current line and open an indented continuation.
    fn break_line(&mut self) {
        self.done
            .push(Line::from(std::mem::take(&mut self.cur)).style(self.style));
        self.cur.push(indent_span(self.indent));
        self.cur_w = self.indent;
    }

    fn finish(mut self) -> Vec<Line<'a>> {
        self.done.push(Line::from(self.cur).style(self.style));
        self.done
    }
}

/// Greedy word-boundary wrap over pre-styled spans. `total_w` is the full
/// row width; `indent` is both the first line's initial occupancy (the
/// gutter) and the leading pad of every continuation line. Whitespace runs
/// at a break point are dropped, like any conventional word wrap; a word
/// wider than a whole continuation line is hard-broken at grapheme
/// boundaries. The caller guarantees `total_w >= indent + MIN_WRAP_BODY_COLS`.
fn wrap_spans<'a>(
    gutter: Vec<Span<'a>>,
    body: Vec<Span<'a>>,
    style: Style,
    total_w: usize,
    indent: usize,
) -> Vec<Line<'a>> {
    let mut acc = LineAcc::new(gutter, total_w, indent, style);
    // Whitespace is held back and only committed when a word follows it on
    // the same line, so line breaks swallow the separator instead of
    // leaking leading spaces onto continuations.
    let mut pending_ws: Vec<Span<'a>> = Vec::new();
    for atom in split_atoms(body) {
        match atom {
            Atom::Ws(span) => pending_ws.push(span),
            Atom::Word(word) => {
                let ws_w: usize = pending_ws.iter().map(Span::width).sum();
                let word_w: usize = word.iter().map(Span::width).sum();
                if acc.fits(ws_w + word_w) {
                    for s in pending_ws.drain(..) {
                        let w = s.width();
                        acc.push(s, w);
                    }
                    for s in word {
                        let w = s.width();
                        acc.push(s, w);
                    }
                } else if acc.indent + word_w <= acc.avail {
                    pending_ws.clear();
                    acc.break_line();
                    for s in word {
                        let w = s.width();
                        acc.push(s, w);
                    }
                } else {
                    // Over-long word. Keep the separator on this line when it
                    // fits; when it doesn't, break first — either way the
                    // word's head can never glue onto the previous word.
                    if acc.has_body() {
                        if acc.fits(ws_w) {
                            for s in pending_ws.drain(..) {
                                let w = s.width();
                                acc.push(s, w);
                            }
                        } else {
                            acc.break_line();
                        }
                    }
                    pending_ws.clear();
                    hard_break(&mut acc, word);
                }
            }
        }
    }
    acc.finish()
}

/// Emit an over-long word across as many lines as needed, splitting at
/// grapheme-cluster boundaries so combining marks, variation selectors
/// (VS16 emoji), and ZWJ sequences are never torn apart. Each cluster is
/// measured string-level — the same way ratatui's renderer draws it — so
/// packed lines never overflow the pane. A cluster wider than the whole
/// body column is force-placed (and clipped) rather than looping.
fn hard_break<'a>(acc: &mut LineAcc<'a>, word: Vec<Span<'a>>) {
    for span in word {
        let mut chunk_start = 0usize;
        let mut chunk_w = 0usize;
        for (idx, g) in span.content.grapheme_indices(true) {
            let g_w = g.width();
            // Break before this cluster would overflow — but never emit an
            // empty body line, guaranteeing progress.
            if !acc.fits(chunk_w + g_w) && (acc.has_body() || chunk_start < idx) {
                if chunk_start < idx {
                    let piece = slice_span(&span, chunk_start, idx);
                    acc.push(piece, chunk_w);
                    chunk_start = idx;
                    chunk_w = 0;
                }
                acc.break_line();
            }
            chunk_w += g_w;
        }
        if chunk_start < span.content.len() {
            let end = span.content.len();
            let piece = slice_span(&span, chunk_start, end);
            acc.push(piece, chunk_w);
        }
    }
}

enum Atom<'a> {
    Ws(Span<'a>),
    Word(Vec<Span<'a>>),
}

/// Split spans into alternating whitespace / word atoms. Adjacent non-space
/// runs from *different* spans merge into one word (search-match highlighting
/// splits a single word into per-char spans; a break inside it would be
/// wrong), while each whitespace run stays its own atom.
fn split_atoms(body: Vec<Span<'_>>) -> Vec<Atom<'_>> {
    let mut atoms: Vec<Atom> = Vec::new();
    for span in body {
        let content = span.content.as_ref();
        let mut rest = 0usize;
        while rest < content.len() {
            let s = &content[rest..];
            let is_ws = s
                .chars()
                .next()
                .expect("rest < len guarantees a char")
                .is_whitespace();
            let run_len = s
                .find(|c: char| c.is_whitespace() != is_ws)
                .unwrap_or(s.len());
            let piece = slice_span(&span, rest, rest + run_len);
            if is_ws {
                atoms.push(Atom::Ws(piece));
            } else if let Some(Atom::Word(word)) = atoms.last_mut() {
                word.push(piece);
            } else {
                atoms.push(Atom::Word(vec![piece]));
            }
            rest += run_len;
        }
    }
    atoms
}

/// Sub-slice a span, preserving its style. Borrowed content stays borrowed
/// (the common case — body spans point into `task.raw`); owned content is
/// re-allocated for the slice.
fn slice_span<'a>(span: &Span<'a>, start: usize, end: usize) -> Span<'a> {
    match &span.content {
        std::borrow::Cow::Borrowed(s) => Span::styled(&s[start..end], span.style),
        std::borrow::Cow::Owned(s) => Span::styled(s[start..end].to_string(), span.style),
    }
}

/// Continuation-line pad. Real gutters are at most 14 columns (line number
/// 4 + visual checkbox 4 + glyph 2 + priority 4), so the static pad is
/// allocation-free in practice; the fallback keeps pathological indents
/// correct anyway.
fn indent_span<'a>(indent: usize) -> Span<'a> {
    const PAD: &str = "                                ";
    if indent <= PAD.len() {
        Span::raw(&PAD[..indent])
    } else {
        Span::raw(" ".repeat(indent))
    }
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

    /// Base opts for the wrap tests: no gutter extras, no cursor.
    fn wrap_opts<'a>() -> RowOpts<'a> {
        RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
        }
    }

    /// Display width of a built line (sum of span widths).
    fn line_width(line: &Line) -> usize {
        line.spans.iter().map(Span::width).sum()
    }

    /// All span text of a line, concatenated.
    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wrap_splits_long_body_at_word_boundaries() {
        let task = parse_line("Call the dentist about the appointment tomorrow").unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(24));
        assert!(lines.len() > 1, "expected multiple lines, got {lines:?}");
        for l in &lines {
            assert!(line_width(l) <= 24, "line overflows: {:?}", line_text(l));
        }
        let joined: String = lines.iter().map(line_text).collect::<Vec<_>>().join(" ");
        for word in ["Call", "dentist", "appointment", "tomorrow"] {
            assert!(joined.contains(word), "missing {word:?} in {joined:?}");
        }
        // No word was split: every line after the gutter/indent starts and
        // ends on a word boundary from the original body.
        assert!(lines[1].spans[0].content.chars().all(|c| c == ' '));
    }

    #[test]
    fn wrap_hard_breaks_unbroken_word() {
        let long = format!("x{}", "a".repeat(60));
        let task = parse_line(&long).unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(line_width(l) <= 20, "line overflows: {:?}", line_text(l));
        }
        let total_a: usize = lines
            .iter()
            .map(|l| line_text(l).chars().filter(|&c| c == 'a').count())
            .sum();
        assert_eq!(total_a, 60, "hard break must not drop characters");
    }

    #[test]
    fn wrap_keeps_separator_before_hard_broken_word() {
        let long = format!("foo {}", "a".repeat(60));
        let task = parse_line(&long).unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        assert!(
            line_text(&lines[0]).contains("foo a"),
            "separator between 'foo' and the long word must survive: {:?}",
            line_text(&lines[0]),
        );
    }

    #[test]
    fn wrap_measures_wide_chars_by_display_width() {
        // Each CJK char is 2 columns; gutter is 6. "日本語" (6) fits the
        // first line at width 14 but "テスト" (6, +1 separator) does not.
        let task = parse_line("日本語 テスト").unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(14));
        assert_eq!(lines.len(), 2, "{lines:?}");
        for l in &lines {
            assert!(line_width(l) <= 14, "line overflows: {:?}", line_text(l));
        }
        assert!(line_text(&lines[0]).contains("日本語"));
        assert!(line_text(&lines[1]).contains("テスト"));
    }

    #[test]
    fn wrap_exact_boundary_width_stays_single_line() {
        // Gutter (6) + "aaa bb" (6) lands exactly on width 12.
        let task = parse_line("aaa bb").unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(12));
        assert_eq!(lines.len(), 1, "{lines:?}");
        // One column narrower must wrap.
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(11));
        assert_eq!(lines.len(), 2, "{lines:?}");
    }

    #[test]
    fn wrap_strips_hidden_keys_before_measuring() {
        let h = vec!["uid".to_string()];
        let mut opts = wrap_opts();
        opts.hidden_keys = &h;
        // Without stripping, uid:aaaaaaaaaaaaaaaaaaaa would force a second
        // line; with it the visible body fits in one.
        let task = parse_line("short uid:aaaaaaaaaaaaaaaaaaaa task").unwrap();
        let lines = build_lines(&task, opts, &MUTED, Some(24));
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(!line_text(&lines[0]).contains("uid:"));
    }

    #[test]
    fn wrap_continuation_lines_carry_cursor_style() {
        let task = parse_line("Call the dentist about the appointment tomorrow").unwrap();
        let mut opts = wrap_opts();
        opts.cursor = true;
        let lines = build_lines(&task, opts, &MUTED, Some(24));
        assert!(lines.len() > 1);
        for l in &lines {
            assert_eq!(
                l.style.bg,
                Some(MUTED.cursor),
                "every wrapped line keeps the cursor background",
            );
        }
    }

    #[test]
    fn wrap_continuation_indent_matches_gutter_width() {
        let task = parse_line("(A) Call the dentist about the appointment tomorrow").unwrap();
        let mut opts = wrap_opts();
        opts.show_line_num = true;
        let lines = build_lines(&task, opts, &MUTED, Some(28));
        assert!(lines.len() > 1);
        // Gutter: "  1 " (4) + glyph (2) + "(A) " (4) = 10 columns.
        let indent = &lines[1].spans[0];
        assert_eq!(indent.content.as_ref(), " ".repeat(10));
    }

    #[test]
    fn wrap_wide_width_matches_single_line_content() {
        let task = parse_line("(B) Buy milk +groceries @errands due:2026-05-10").unwrap();
        let single = build_line(&task, wrap_opts(), &MUTED);
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(200));
        assert_eq!(lines.len(), 1);
        assert_eq!(line_text(&lines[0]), line_text(&single));
        assert_eq!(lines[0].style, single.style);
    }

    #[test]
    fn wrap_hard_break_keeps_every_scalar_in_zwj_emoji() {
        // Per-char width sums overestimate ZWJ ligature width, so breaks land
        // early — the invariant is "never overflow, never lose a scalar".
        let fam = "👨‍👩‍👧‍👦".repeat(12);
        let task = parse_line(&fam).unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        let joined: String = lines.iter().map(line_text).collect();
        let scalars_in: usize = fam.chars().count();
        let scalars_out = joined.chars().filter(|c| !matches!(c, ' ')).count();
        assert_eq!(scalars_out, scalars_in, "no scalar lost");
    }

    #[test]
    fn wrap_never_orphans_combining_marks_at_line_start() {
        let word = "e\u{301}".repeat(40); // e + COMBINING ACUTE
        let task = parse_line(&word).unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        for l in &lines[1..] {
            let text = line_text(l);
            let first_nonspace = text.chars().find(|c| *c != ' ');
            assert_ne!(
                first_nonspace,
                Some('\u{301}'),
                "line must not start with a combining mark: {text:?}"
            );
        }
    }

    #[test]
    fn wrap_falls_back_to_single_line_in_degenerate_pane() {
        // Width (4) is narrower than the gutter (6) plus MIN_WRAP_BODY_COLS:
        // wrapping would emit one clipped, invisible column per line, so the
        // row must fall back to the classic clipped single line instead.
        let task = parse_line("some words here to wrap").unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(4));
        assert_eq!(lines.len(), 1, "{lines:?}");
        let single = build_line(&task, wrap_opts(), &MUTED);
        assert_eq!(line_text(&lines[0]), line_text(&single));
    }

    #[test]
    fn wrap_never_fuses_words_across_hard_break() {
        // Regression: word 1 fills the line to one column short of the pane
        // edge; word 2 needs a hard break. The separator must not be
        // swallowed while the same line keeps filling — that rendered
        // "aaaaaaaaaaaaac" with the words glued together.
        let task = parse_line("aaaaaaaaaaaaa cccccccccccccccc").unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        assert!(lines.len() > 1);
        for l in &lines {
            let text = line_text(l);
            assert!(
                !text.contains("ac"),
                "words fused across the hard break: {text:?}",
            );
            assert!(line_width(l) <= 20, "line overflows: {text:?}");
        }
        let total_c: usize = lines
            .iter()
            .map(|l| line_text(l).chars().filter(|&c| c == 'c').count())
            .sum();
        assert_eq!(total_c, 16, "hard break must not drop characters");
    }

    #[test]
    fn wrap_hard_break_measures_vs16_emoji_string_level() {
        // Regression: "❤\u{FE0F}" (heart + variation selector) is 1 column
        // when its scalars are measured one by one but 2 columns as a
        // cluster — which is how ratatui draws it. Char-level accounting
        // packed twice as many per line and the overflow was clipped
        // invisibly at the pane edge.
        let word = "\u{2764}\u{FE0F}".repeat(20);
        let task = parse_line(&word).unwrap();
        let lines = build_lines(&task, wrap_opts(), &MUTED, Some(20));
        assert!(lines.len() > 1);
        let mut hearts = 0usize;
        for l in &lines {
            assert!(
                line_width(l) <= 20,
                "line renders wider than the pane: {:?}",
                line_text(l),
            );
            hearts += line_text(l).chars().filter(|&c| c == '\u{2764}').count();
        }
        assert_eq!(hearts, 20, "no cluster may be clipped away");
    }

    #[test]
    fn wrap_fully_hidden_body_stays_single_line() {
        let h = vec!["uid".to_string()];
        let mut opts = wrap_opts();
        opts.hidden_keys = &h;
        let task = parse_line("uid:abc-123").unwrap();
        let lines = build_lines(&task, opts, &MUTED, Some(20));
        assert_eq!(lines.len(), 1);
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
