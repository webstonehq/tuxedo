//! Natural-language parser for the add-todo draft.
//!
//! When the user types prose into the add buffer ("Pay rent monthly on the
//! first, show 3 days before due, project home"), this module extracts the
//! structured todo.txt metadata so the caller can rewrite the buffer into
//! canonical form for the user to review.
//!
//! Pure logic — no I/O, no app state. The crate-level wiring lives in
//! `app::mutations::add_from_draft`.
//!
//! Detection (`looks_like_natural_language`) is intentionally conservative:
//! it returns `false` whenever the buffer already contains a `due:` / `rec:`
//! / `t:` token, which gives the rewrite pipeline trivial idempotency — a
//! second Enter on the canonical output falls through to the existing save
//! path.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::todo;

/// Structured fields extracted from a prose draft. Each `Option` field is
/// `None` when the user didn't say anything about that aspect; `Vec` fields
/// are empty for the same reason. `body` is the input with all recognized
/// phrases stripped and whitespace collapsed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedNl {
    pub body: String,
    pub due: Option<NaiveDate>,
    pub rec: Option<String>,
    pub threshold: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub priority: Option<char>,
}

/// Cheap heuristic gating the full parse. Returns `true` when the buffer
/// looks like prose worth interpreting. Two rules:
///
/// 1. The buffer must not already contain a `due:` / `rec:` / `t:` token —
///    that means the user (or a previous NL rewrite) already produced
///    canonical form, so leave it alone.
/// 2. The buffer must contain at least one trigger word (date words,
///    weekdays, months, recurrence vocabulary, `before`, `project`,
///    `context`, …).
pub fn looks_like_natural_language(text: &str) -> bool {
    if has_kv_token(text, "due") || has_kv_token(text, "rec") || has_kv_token(text, "t") {
        return false;
    }
    contains_trigger(text)
}

/// Main entry point. `today` resolves relative dates ("tomorrow", "the first
/// of the month"). Returns `None` when the parser couldn't extract anything
/// structured — the caller then falls through to the plain save path.
pub fn try_parse(text: &str, today: NaiveDate) -> Option<ParsedNl> {
    let mut scratch = Scratch::new(text);
    let mut parsed = ParsedNl::default();

    pass_leading_priority(&mut scratch, &mut parsed);
    pass_sigiled(&mut scratch, &mut parsed);
    pass_threshold(&mut scratch, &mut parsed);
    let weekday_hint = pass_recurrence(&mut scratch, &mut parsed);
    pass_date(&mut scratch, &mut parsed, today, weekday_hint);
    pass_project_context(&mut scratch, &mut parsed);
    pass_priority(&mut scratch, &mut parsed);

    parsed.body = scratch.remaining_cleaned();

    let extracted = parsed.due.is_some()
        || parsed.rec.is_some()
        || parsed.threshold.is_some()
        || !parsed.projects.is_empty()
        || !parsed.contexts.is_empty()
        || parsed.priority.is_some();
    if extracted { Some(parsed) } else { None }
}

/// Serialize a parsed result back to a canonical todo.txt line. Token order
/// is fixed: `(P) body +proj… @ctx… due:… rec:… t:…`. An empty body falls
/// back to `"todo"` so the result is always a well-formed task — the caller
/// is expected to flash a hint so the user knows to fix the body.
pub fn format_as_todo_txt(p: &ParsedNl) -> String {
    let mut out = String::new();
    if let Some(prio) = p.priority {
        out.push('(');
        out.push(prio);
        out.push(')');
        out.push(' ');
    }
    let body = p.body.trim();
    if body.is_empty() {
        out.push_str("todo");
    } else {
        out.push_str(body);
    }
    for proj in &p.projects {
        out.push_str(" +");
        out.push_str(proj);
    }
    for ctx in &p.contexts {
        out.push_str(" @");
        out.push_str(ctx);
    }
    if let Some(d) = p.due {
        out.push_str(" due:");
        out.push_str(&d.format("%Y-%m-%d").to_string());
    }
    if let Some(r) = &p.rec {
        out.push_str(" rec:");
        out.push_str(r);
    }
    if let Some(t) = &p.threshold {
        out.push_str(" t:");
        out.push_str(t);
    }
    out
}

// ---------------------------------------------------------------------------
// Trigger detection
// ---------------------------------------------------------------------------

fn has_kv_token(text: &str, key: &str) -> bool {
    for tok in text.split_whitespace() {
        if let Some((k, v)) = tok.split_once(':')
            && k == key
            && !v.is_empty()
        {
            return true;
        }
    }
    false
}

fn contains_trigger(text: &str) -> bool {
    let lower = ascii_lower(text);
    let words: Vec<&str> = lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '!' | '?')))
        .collect();

    const SINGLE_TRIGGERS: &[&str] = &[
        // date words
        "today",
        "tonight",
        "tomorrow",
        "yesterday",
        // weekdays
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "mon",
        "tue",
        "tues",
        "wed",
        "thu",
        "thurs",
        "fri",
        "sat",
        "sun",
        // recurrence
        "every",
        "each",
        "daily",
        "weekly",
        "biweekly",
        "monthly",
        "yearly",
        "annually",
        // prose markers
        "project",
        "proj",
        "context",
        "ctx",
        "priority",
        "before",
        "starting",
        "due",
        "by",
    ];

    for w in &words {
        if SINGLE_TRIGGERS.contains(w) {
            return true;
        }
        if parse_month(w).is_some() {
            return true;
        }
    }

    // Multi-word: "in N (day|week|month|year)s?"
    for i in 0..words.len() {
        if words[i] == "in" && i + 2 < words.len() {
            let n = words[i + 1]
                .parse::<u32>()
                .ok()
                .or_else(|| word_number(words[i + 1]));
            let unit = unit_char(words[i + 2]);
            if n.is_some() && unit.is_some() {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Scratch buffer: tracks consumed byte ranges across all passes.
// ---------------------------------------------------------------------------

struct Scratch<'a> {
    text: &'a str,
    /// ASCII-lowercased copy of `text`. Lowercasing only ASCII letters keeps
    /// byte indices aligned between `text` and `lower`, so a range valid in
    /// one is valid in the other.
    lower: String,
    consumed: Vec<bool>,
    /// Cached word ranges over the original text. Recomputed via
    /// `live_words()` each pass — cheap since inputs are short.
    word_cache: Vec<(usize, usize)>,
}

impl<'a> Scratch<'a> {
    fn new(text: &'a str) -> Self {
        let lower = ascii_lower(text);
        let consumed = vec![false; text.len()];
        let word_cache = compute_words(text);
        Self {
            text,
            lower,
            consumed,
            word_cache,
        }
    }

    /// Returns `true` if every byte in `[start, end)` is unconsumed. A
    /// fully-consumed word counts as gone for subsequent passes.
    fn is_live(&self, start: usize, end: usize) -> bool {
        (start..end).all(|i| !self.consumed.get(i).copied().unwrap_or(true))
    }

    fn mark(&mut self, start: usize, end: usize) {
        let end = end.min(self.consumed.len());
        for slot in self.consumed[start..end].iter_mut() {
            *slot = true;
        }
    }

    /// Lower-case slice with trailing punctuation stripped — what most pattern
    /// matchers want to compare against.
    fn word_lc(&self, range: (usize, usize)) -> &str {
        self.lower[range.0..range.1].trim_end_matches([',', '.', ';', ':', '!', '?'])
    }

    /// Original-case slice with trailing punctuation stripped — used when the
    /// extracted value needs to round-trip (e.g. tag names).
    fn word_orig(&self, range: (usize, usize)) -> &str {
        self.text[range.0..range.1].trim_end_matches([',', '.', ';', ':', '!', '?'])
    }

    /// Remaining body text after stripping consumed bytes and collapsing
    /// whitespace. Leading/trailing connector words ("and", "it's", …) are
    /// also dropped so the body reads cleanly.
    fn remaining_cleaned(&self) -> String {
        let mut buf = String::new();
        let mut prev_space = true;
        for (i, c) in self.text.char_indices() {
            let is_consumed = self.consumed.get(i).copied().unwrap_or(false);
            if is_consumed || c.is_whitespace() {
                if !prev_space {
                    buf.push(' ');
                    prev_space = true;
                }
            } else {
                buf.push(c);
                prev_space = false;
            }
        }
        let mut tokens: Vec<&str> = buf.split_whitespace().collect();
        let is_connector = |t: &str| {
            let cleaned = t
                .trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '!' | '?'))
                .to_ascii_lowercase();
            matches!(
                cleaned.as_str(),
                "and" | "or" | "but" | "it's" | "its" | "that" | "which" | ""
            )
        };
        while tokens.first().is_some_and(|t| is_connector(t)) {
            tokens.remove(0);
        }
        while tokens.last().is_some_and(|t| is_connector(t)) {
            tokens.pop();
        }
        let joined = tokens.join(" ");
        joined
            .trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':') || c.is_whitespace())
            .to_string()
    }
}

fn ascii_lower(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect()
}

fn compute_words(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
        last_end = i + c.len_utf8();
    }
    if let Some(st) = start {
        out.push((st, last_end));
    }
    out
}

// ---------------------------------------------------------------------------
// Pass 0: leading "(X) " priority prefix
// ---------------------------------------------------------------------------

/// Strip a leading `(X) ` priority token if the user typed canonical priority
/// syntax inside an otherwise prose buffer (e.g. `"(A) Buy milk tomorrow"`).
/// Without this, the `(A)` survives into the body — saving still works because
/// `todo::parse_line` strips it on re-parse, but `format_as_todo_txt` would
/// emit `"(A) Buy milk ..."` with the priority living *in the body*, and any
/// subsequent priority word in the prose would double up the prefix.
fn pass_leading_priority(scratch: &mut Scratch, p: &mut ParsedNl) {
    let bytes = scratch.text.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && bytes[3] == b' '
    {
        p.priority = Some(bytes[1] as char);
        scratch.mark(0, 4);
    }
}

// ---------------------------------------------------------------------------
// Pass 1: sigiled tokens (+proj, @ctx)
// ---------------------------------------------------------------------------

fn pass_sigiled(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    for (s, e) in words {
        if !scratch.is_live(s, e) {
            continue;
        }
        let tok = scratch.word_orig((s, e));
        if let Some(name) = tok.strip_prefix('+') {
            push_unique(&mut p.projects, name);
            scratch.mark(s, e);
        } else if let Some(name) = tok.strip_prefix('@') {
            push_unique(&mut p.contexts, name);
            scratch.mark(s, e);
        }
    }
}

fn push_unique(out: &mut Vec<String>, name: &str) {
    if name.is_empty() || !todo::is_valid_tag_name(name) {
        return;
    }
    if !out.iter().any(|x| x == name) {
        out.push(name.to_string());
    }
}

// ---------------------------------------------------------------------------
// Pass 2: threshold ("show N (day|week|month)s? before [the] [due [date]]")
// ---------------------------------------------------------------------------

fn pass_threshold(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    let mut i = 0;
    while i + 2 < words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            i += 1;
            continue;
        }
        let Some(n) = parse_number(scratch.word_lc(words[i])) else {
            i += 1;
            continue;
        };
        let Some(unit) = unit_char(scratch.word_lc(words[i + 1])) else {
            i += 1;
            continue;
        };
        // Only d/w/m for threshold (years are not in the t: grammar).
        if !matches!(unit, 'd' | 'w' | 'm') {
            i += 1;
            continue;
        }
        if scratch.word_lc(words[i + 2]) != "before" {
            i += 1;
            continue;
        }

        // Look backward for "show [the (todo|task|item)] [me|it]" preamble.
        let mut start_word = i;
        const PREAMBLE: &[&str] = &["show", "the", "todo", "task", "item", "me", "it"];
        let mut saw_show = false;
        while start_word > 0 {
            let w = scratch.word_lc(words[start_word - 1]);
            if PREAMBLE.contains(&w) {
                if w == "show" {
                    saw_show = true;
                }
                start_word -= 1;
            } else {
                break;
            }
        }
        if !saw_show {
            start_word = i;
        }

        // Look forward through "[the] [due] [date]".
        let mut end_word = i + 3;
        const TRAILERS: &[&str] = &["the", "due", "date"];
        while end_word < words.len() {
            let w = scratch.word_lc(words[end_word]);
            if TRAILERS.contains(&w) {
                end_word += 1;
            } else {
                break;
            }
        }

        let start_byte = words[start_word].0;
        let end_byte = words[end_word - 1].1;
        scratch.mark(start_byte, end_byte);
        p.threshold = Some(format!("-{n}{unit}"));
        return;
    }
}

fn parse_number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    word_number(s)
}

fn word_number(s: &str) -> Option<u32> {
    Some(match s {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        _ => return None,
    })
}

fn unit_char(s: &str) -> Option<char> {
    Some(match s {
        "day" | "days" => 'd',
        "week" | "weeks" => 'w',
        "month" | "months" => 'm',
        "year" | "years" => 'y',
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Pass 3: recurrence
// ---------------------------------------------------------------------------

fn pass_recurrence(scratch: &mut Scratch, p: &mut ParsedNl) -> Option<Weekday> {
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let standalone = match w {
            "daily" => Some(("+1d".to_string(), 1, None)),
            "weekly" => Some(("+1w".to_string(), 1, None)),
            "biweekly" => Some(("+2w".to_string(), 1, None)),
            "monthly" => Some(("+1m".to_string(), 1, None)),
            "yearly" | "annually" => Some(("+1y".to_string(), 1, None)),
            _ => None,
        };
        let (rec, count, wh) = if let Some(s) = standalone {
            s
        } else if w == "every" || w == "each" {
            match parse_every_phrase(scratch, &words, i) {
                Some(v) => v,
                None => continue,
            }
        } else {
            continue;
        };
        let start_byte = words[i].0;
        let end_byte = words[i + count - 1].1;
        scratch.mark(start_byte, end_byte);
        p.rec = Some(rec);
        return wh;
    }
    None
}

/// Parse `every <...>` starting at index `i`. Returns `(rec_value, word_count, weekday_hint)`
/// on success. `word_count` includes `every` itself.
fn parse_every_phrase(
    scratch: &Scratch,
    words: &[(usize, usize)],
    i: usize,
) -> Option<(String, usize, Option<Weekday>)> {
    if i + 1 >= words.len() {
        return None;
    }
    let w1 = scratch.word_lc(words[i + 1]);

    if w1 == "weekday" {
        return Some(("+1b".to_string(), 2, None));
    }

    if w1 == "business" {
        if i + 2 < words.len() {
            let w2 = scratch.word_lc(words[i + 2]);
            if w2 == "day" || w2 == "days" {
                return Some(("+1b".to_string(), 3, None));
            }
        }
        return None;
    }

    if w1 == "other" {
        if i + 2 >= words.len() {
            return None;
        }
        let w2 = scratch.word_lc(words[i + 2]);
        if let Some(wd) = parse_weekday(w2) {
            return Some(("+2w".to_string(), 3, Some(wd)));
        }
        let unit = match w2 {
            "day" | "days" => 'd',
            "week" | "weeks" => 'w',
            "month" | "months" => 'm',
            "year" | "years" => 'y',
            _ => return None,
        };
        return Some((format!("+2{unit}"), 3, None));
    }

    if let Some(wd) = parse_weekday(w1) {
        return Some(("+1w".to_string(), 2, Some(wd)));
    }

    if let Some(n) = parse_number(w1) {
        if i + 2 >= words.len() {
            return None;
        }
        let unit = match scratch.word_lc(words[i + 2]) {
            "day" | "days" => 'd',
            "week" | "weeks" => 'w',
            "month" | "months" => 'm',
            "year" | "years" => 'y',
            _ => return None,
        };
        return Some((format!("+{n}{unit}"), 3, None));
    }

    let unit = match w1 {
        "day" => 'd',
        "week" => 'w',
        "month" => 'm',
        "year" => 'y',
        _ => return None,
    };
    Some((format!("+1{unit}"), 2, None))
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Pass 4: date
// ---------------------------------------------------------------------------

fn pass_date(
    scratch: &mut Scratch,
    p: &mut ParsedNl,
    today: NaiveDate,
    weekday_hint: Option<Weekday>,
) {
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        if let Some((date, count)) = match_date_at(scratch, &words, i, today) {
            let start_byte = words[i].0;
            let end_byte = words[i + count - 1].1;
            scratch.mark(start_byte, end_byte);
            p.due = Some(date);
            return;
        }
    }
    if p.due.is_none()
        && let Some(wd) = weekday_hint
        && let Some(d) = next_weekday(today, wd, true)
    {
        p.due = Some(d);
    }
}

/// Try every supported date phrase starting at `words[i]`. Returns the
/// resolved date and the number of words to consume.
fn match_date_at(
    scratch: &Scratch,
    words: &[(usize, usize)],
    i: usize,
    today: NaiveDate,
) -> Option<(NaiveDate, usize)> {
    let w = scratch.word_lc(words[i]);

    if let Ok(d) = NaiveDate::parse_from_str(w, "%Y-%m-%d") {
        return Some((d, 1));
    }

    if w == "today" || w == "tonight" {
        return Some((today, 1));
    }
    if w == "tomorrow" {
        return Some((today.checked_add_days(Days::new(1))?, 1));
    }
    if w == "yesterday" {
        return Some((today.checked_sub_days(Days::new(1))?, 1));
    }

    // Marker words that introduce a date phrase: "due April 15", "on Friday",
    // "by the 15th", "starting Friday", "before December 5". The marker is
    // consumed along with the date so it doesn't survive into the body. Any
    // "before" still standing at this point has already been ignored by the
    // threshold pass (which would have consumed "N <unit> before [trailers]").
    if matches!(w, "starting" | "on" | "due" | "by" | "before")
        && let Some((d, count)) = next_alive_match(scratch, words, i + 1, today)
    {
        return Some((d, 1 + count));
    }

    if (w == "this" || w == "next")
        && i + 1 < words.len()
        && let Some(wd) = parse_weekday(scratch.word_lc(words[i + 1]))
    {
        let strict = w == "next";
        if let Some(d) = next_weekday(today, wd, strict) {
            return Some((d, 2));
        }
    }

    if let Some(wd) = parse_weekday(w)
        && let Some(d) = next_weekday(today, wd, false)
    {
        return Some((d, 1));
    }

    // "in N <unit>s?"
    if w == "in"
        && i + 2 < words.len()
        && let Some(n) = parse_number(scratch.word_lc(words[i + 1]))
    {
        let unit = scratch.word_lc(words[i + 2]);
        if let Some(d) = advance_from(today, n, unit) {
            return Some((d, 3));
        }
    }

    // "N <unit>s? from (now|today)"
    if let Some(n) = parse_number(w)
        && i + 3 < words.len()
    {
        let unit = scratch.word_lc(words[i + 1]);
        let from = scratch.word_lc(words[i + 2]);
        let nowt = scratch.word_lc(words[i + 3]);
        if from == "from"
            && (nowt == "now" || nowt == "today")
            && let Some(d) = advance_from(today, n, unit)
        {
            return Some((d, 4));
        }
    }

    // "MONTH D[ord]?(, YYYY)?"
    if let Some(month) = parse_month(w)
        && i + 1 < words.len()
        && let Some(day) = parse_day_ordinal(scratch.word_lc(words[i + 1]))
    {
        let (year, consumed) = match try_parse_year(scratch, words, i + 2) {
            Some(y) => (y, 3),
            None => (today.year(), 2),
        };
        if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
            let rolled = if consumed == 2 && d < today {
                NaiveDate::from_ymd_opt(year + 1, month, day).unwrap_or(d)
            } else {
                d
            };
            return Some((rolled, consumed));
        }
    }

    // "D[ord] (of)? MONTH(, YYYY)?"
    if let Some(day) = parse_day_ordinal(w) {
        let mut j = i + 1;
        if j < words.len() && scratch.word_lc(words[j]) == "of" {
            j += 1;
        }
        if j < words.len()
            && let Some(month) = parse_month(scratch.word_lc(words[j]))
        {
            let (year, year_extra) = match try_parse_year(scratch, words, j + 1) {
                Some(y) => (y, 1),
                None => (today.year(), 0),
            };
            let consumed = j - i + 1 + year_extra;
            if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
                let rolled = if year_extra == 0 && d < today {
                    NaiveDate::from_ymd_opt(year + 1, month, day).unwrap_or(d)
                } else {
                    d
                };
                return Some((rolled, consumed));
            }
        }
    }

    // "the (Nth|first|...) (of (the|next) month)?"
    if w == "the"
        && i + 1 < words.len()
        && let Some(day) = parse_day_ordinal(scratch.word_lc(words[i + 1]))
    {
        let (date, consumed) = resolve_ordinal_month_phrase(scratch, words, i + 2, today, day);
        return Some((date, 2 + consumed));
    }

    // "(first|1st) of (the|next) month"
    if (w == "first" || w == "1st") && i + 3 < words.len() {
        let w1 = scratch.word_lc(words[i + 1]);
        let w2 = scratch.word_lc(words[i + 2]);
        let w3 = scratch.word_lc(words[i + 3]);
        if w1 == "of" && (w2 == "the" || w2 == "next") && w3 == "month" {
            let next_month = w2 == "next";
            let target = if next_month {
                today.checked_add_months(Months::new(1))?
            } else {
                today
            };
            if let Some(d) = NaiveDate::from_ymd_opt(target.year(), target.month(), 1) {
                let rolled = if !next_month && d < today {
                    today
                        .checked_add_months(Months::new(1))
                        .and_then(|n| NaiveDate::from_ymd_opt(n.year(), n.month(), 1))
                        .unwrap_or(d)
                } else {
                    d
                };
                return Some((rolled, 4));
            }
        }
    }

    None
}

/// Recurse into the date matcher at `i`, skipping over any consumed words.
/// Used by the `starting`/`on` wrappers so they can prefix a real date phrase.
fn next_alive_match(
    scratch: &Scratch,
    words: &[(usize, usize)],
    mut i: usize,
    today: NaiveDate,
) -> Option<(NaiveDate, usize)> {
    while i < words.len() && !scratch.is_live(words[i].0, words[i].1) {
        i += 1;
    }
    if i >= words.len() {
        return None;
    }
    match_date_at(scratch, words, i, today)
}

fn try_parse_year(scratch: &Scratch, words: &[(usize, usize)], i: usize) -> Option<i32> {
    if i >= words.len() {
        return None;
    }
    // `word_lc` already strips trailing punctuation, which handles the
    // common "April 15, 2026" shape (the comma sticks to "15,").
    let y: i32 = scratch.word_lc(words[i]).parse().ok()?;
    if (1900..=9999).contains(&y) {
        Some(y)
    } else {
        None
    }
}

fn resolve_ordinal_month_phrase(
    scratch: &Scratch,
    words: &[(usize, usize)],
    j: usize,
    today: NaiveDate,
    day: u32,
) -> (NaiveDate, usize) {
    // After the ordinal: optional "of (the|next) month".
    let mut extra = 0;
    let mut next_month = false;
    if j < words.len() && scratch.word_lc(words[j]) == "of" {
        if j + 2 < words.len() {
            let w1 = scratch.word_lc(words[j + 1]);
            let w2 = scratch.word_lc(words[j + 2]);
            if (w1 == "the" || w1 == "next") && w2 == "month" {
                if w1 == "next" {
                    next_month = true;
                }
                extra = 3;
            }
        }
        if extra == 0 && j + 1 < words.len() && scratch.word_lc(words[j + 1]) == "month" {
            extra = 2;
        }
    }

    let target = if next_month {
        today.checked_add_months(Months::new(1)).unwrap_or(today)
    } else {
        today
    };
    let candidate = NaiveDate::from_ymd_opt(target.year(), target.month(), day);
    let resolved = match candidate {
        Some(d) if !next_month && d < today => today
            .checked_add_months(Months::new(1))
            .and_then(|n| NaiveDate::from_ymd_opt(n.year(), n.month(), day))
            .unwrap_or(d),
        Some(d) => d,
        None => today,
    };
    (resolved, extra)
}

fn advance_from(today: NaiveDate, n: u32, unit: &str) -> Option<NaiveDate> {
    let unit_char = unit_char(unit)?;
    match unit_char {
        'd' => today.checked_add_days(Days::new(u64::from(n))),
        'w' => today.checked_add_days(Days::new(u64::from(n) * 7)),
        'm' => today.checked_add_months(Months::new(n)),
        'y' => today.checked_add_months(Months::new(n.checked_mul(12)?)),
        _ => None,
    }
}

/// Next occurrence of `target` weekday. With `strict = true`, today is
/// skipped (so "every monday" on a Monday rolls forward by 7 days).
fn next_weekday(today: NaiveDate, target: Weekday, strict: bool) -> Option<NaiveDate> {
    let cur = today.weekday().num_days_from_monday();
    let tgt = target.num_days_from_monday();
    let mut diff = (tgt + 7 - cur) % 7;
    if diff == 0 && strict {
        diff = 7;
    }
    today.checked_add_days(Days::new(u64::from(diff)))
}

fn parse_month(s: &str) -> Option<u32> {
    Some(match s {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

fn parse_day_ordinal(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        if (1..=31).contains(&n) {
            return Some(n);
        }
        return None;
    }
    if s.len() >= 3 {
        let (num, suffix) = s.split_at(s.len() - 2);
        if matches!(suffix, "st" | "nd" | "rd" | "th")
            && let Ok(n) = num.parse::<u32>()
            && (1..=31).contains(&n)
        {
            return Some(n);
        }
    }
    Some(match s {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Pass 5: project / context prose
// ---------------------------------------------------------------------------

fn pass_project_context(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    let mut i = 0;
    while i < words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            i += 1;
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let is_project = w == "project" || w == "proj";
        let is_context = w == "context" || w == "ctx";
        if !is_project && !is_context {
            i += 1;
            continue;
        }
        // Find the next live word as the name.
        let mut name_idx = i + 1;
        while name_idx < words.len() && !scratch.is_live(words[name_idx].0, words[name_idx].1) {
            name_idx += 1;
        }
        if name_idx >= words.len() {
            i += 1;
            continue;
        }
        let name = scratch.word_orig(words[name_idx]).to_string();
        if !todo::is_valid_tag_name(&name) {
            i += 1;
            continue;
        }
        // Walk back over connector words ("and", "part", "of", "for", "in", "it's", "the").
        const CONNECTORS: &[&str] = &[
            "and", "or", "part", "of", "for", "in", "the", "it's", "its", "a", "an",
        ];
        let mut start_word = i;
        while start_word > 0 {
            let prev_range = words[start_word - 1];
            if !scratch.is_live(prev_range.0, prev_range.1) {
                break;
            }
            let prev = scratch.word_lc(prev_range);
            if CONNECTORS.contains(&prev) {
                start_word -= 1;
            } else {
                break;
            }
        }
        let end_byte = words[name_idx].1;
        scratch.mark(words[start_word].0, end_byte);
        if is_project {
            push_unique(&mut p.projects, &name);
        } else {
            push_unique(&mut p.contexts, &name);
        }
        i = name_idx + 1;
    }
}

// ---------------------------------------------------------------------------
// Pass 6: priority words
// ---------------------------------------------------------------------------

fn pass_priority(scratch: &mut Scratch, p: &mut ParsedNl) {
    if p.priority.is_some() {
        return;
    }
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let prio = match w {
            "high" | "highest" if next_lc(scratch, &words, i + 1) == Some("priority") => {
                Some(('A', 2))
            }
            "medium" | "med" if next_lc(scratch, &words, i + 1) == Some("priority") => {
                Some(('B', 2))
            }
            "low" if next_lc(scratch, &words, i + 1) == Some("priority") => Some(('C', 2)),
            "priority" => match next_lc(scratch, &words, i + 1) {
                Some("a") => Some(('A', 2)),
                Some("b") => Some(('B', 2)),
                Some("c") => Some(('C', 2)),
                Some("high") | Some("highest") => Some(('A', 2)),
                Some("medium") | Some("med") => Some(('B', 2)),
                Some("low") => Some(('C', 2)),
                _ => None,
            },
            _ => None,
        };
        if let Some((c, count)) = prio {
            scratch.mark(words[i].0, words[i + count - 1].1);
            p.priority = Some(c);
            return;
        }
    }
}

fn next_lc<'a>(scratch: &'a Scratch, words: &[(usize, usize)], i: usize) -> Option<&'a str> {
    words.get(i).map(|r| scratch.word_lc(*r))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn detection_skips_already_tokenized() {
        assert!(!looks_like_natural_language("Buy milk due:2026-05-10"));
        assert!(!looks_like_natural_language("Task rec:+1w"));
        assert!(!looks_like_natural_language("Hidden t:-3d"));
    }

    #[test]
    fn detection_skips_plain_words() {
        assert!(!looks_like_natural_language("Buy milk"));
        assert!(!looks_like_natural_language("(A) Buy milk"));
        assert!(!looks_like_natural_language("Buy milk +groceries @store"));
    }

    #[test]
    fn detection_fires_on_triggers() {
        assert!(looks_like_natural_language("Buy milk tomorrow"));
        assert!(looks_like_natural_language("Pay rent monthly"));
        assert!(looks_like_natural_language("Submit timesheet every friday"));
        assert!(looks_like_natural_language("Meeting in 3 days"));
        assert!(looks_like_natural_language("Call mom on tuesday"));
    }

    #[test]
    fn parses_user_example() {
        let today = d("2026-05-11");
        let input = "Pay rent monthly on the first of the month, show the todo 3 days before the due date. It's part of project home and context bank";
        let parsed = try_parse(input, today).unwrap();
        assert_eq!(parsed.body, "Pay rent");
        assert_eq!(parsed.due, Some(d("2026-06-01")));
        assert_eq!(parsed.rec.as_deref(), Some("+1m"));
        assert_eq!(parsed.threshold.as_deref(), Some("-3d"));
        assert_eq!(parsed.projects, vec!["home".to_string()]);
        assert_eq!(parsed.contexts, vec!["bank".to_string()]);
        assert_eq!(parsed.priority, None);
    }

    #[test]
    fn formats_user_example_canonically() {
        let today = d("2026-05-11");
        let input = "Pay rent monthly on the first of the month, show the todo 3 days before the due date. It's part of project home and context bank";
        let parsed = try_parse(input, today).unwrap();
        let out = format_as_todo_txt(&parsed);
        assert_eq!(out, "Pay rent +home @bank due:2026-06-01 rec:+1m t:-3d");
    }

    #[test]
    fn parses_buy_milk_tomorrow() {
        let today = d("2026-05-11");
        let parsed = try_parse("Buy milk tomorrow", today).unwrap();
        assert_eq!(parsed.body, "Buy milk");
        assert_eq!(parsed.due, Some(d("2026-05-12")));
        assert_eq!(parsed.rec, None);
        assert_eq!(parsed.threshold, None);
    }

    #[test]
    fn parses_call_mom_every_week_starting_friday() {
        let today = d("2026-05-11"); // Monday
        let parsed = try_parse(
            "Call mom every week starting Friday for project family",
            today,
        )
        .unwrap();
        assert_eq!(parsed.body, "Call mom");
        assert_eq!(parsed.rec.as_deref(), Some("+1w"));
        assert_eq!(parsed.due, Some(d("2026-05-15"))); // next Friday
        assert_eq!(parsed.projects, vec!["family".to_string()]);
    }

    #[test]
    fn parses_annual_review_due_april_15() {
        let today = d("2026-05-11");
        let parsed = try_parse("Annual review due April 15 +work @office", today).unwrap();
        // "Annual" stays in body: we only treat "annually" as a recurrence
        // trigger ("Annual review" reads as an adjective). "due" is consumed
        // as the date marker so it doesn't survive into the body.
        assert_eq!(parsed.body, "Annual review");
        assert_eq!(parsed.due, Some(d("2027-04-15"))); // April 15 already past this year
        assert_eq!(parsed.projects, vec!["work".to_string()]);
        assert_eq!(parsed.contexts, vec!["office".to_string()]);
        assert_eq!(parsed.rec, None);
    }

    #[test]
    fn date_marker_words_are_consumed() {
        // "due", "by", "on", "starting", "before" preceding a date are
        // consumed alongside the date phrase — none survive in the body.
        let today = d("2026-05-11");
        for input in [
            "Pay rent due Friday",
            "Pay rent by Friday",
            "Pay rent on Friday",
            "Pay rent before Friday",
            "Pay rent starting Friday",
        ] {
            let parsed =
                try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
            assert_eq!(parsed.body, "Pay rent", "input: {input:?}");
            assert!(parsed.due.is_some(), "input: {input:?}");
        }
    }

    #[test]
    fn dangling_before_with_no_date_extracts_nothing() {
        // "before" alone (no following date) is a trigger but yields no
        // extraction — caller falls through and saves as plain prose.
        let today = d("2026-05-11");
        assert!(try_parse("Pay rent before payday", today).is_none());
    }

    #[test]
    fn parses_every_other_friday_show_one_day_before() {
        let today = d("2026-05-11");
        let parsed = try_parse(
            "Submit timesheet every other friday show 1 day before",
            today,
        )
        .unwrap();
        assert_eq!(parsed.body, "Submit timesheet");
        assert_eq!(parsed.rec.as_deref(), Some("+2w"));
        assert_eq!(parsed.threshold.as_deref(), Some("-1d"));
        assert_eq!(parsed.due, Some(d("2026-05-15")));
    }

    #[test]
    fn idempotent_on_canonical_form() {
        let today = d("2026-05-11");
        let parsed = try_parse(
            "Pay rent monthly on the first, show 3 days before due, project home",
            today,
        )
        .unwrap();
        let canonical = format_as_todo_txt(&parsed);
        // Detection should refuse to re-parse the canonical form.
        assert!(!looks_like_natural_language(&canonical));
    }

    #[test]
    fn first_of_the_month_rolls_forward() {
        let today = d("2026-05-11");
        let parsed = try_parse("Pay rent on the first of the month", today).unwrap();
        assert_eq!(parsed.due, Some(d("2026-06-01")));
    }

    #[test]
    fn every_monday_on_a_monday_picks_next_week() {
        let today = d("2026-05-11"); // Monday
        let parsed = try_parse("Standup every monday", today).unwrap();
        assert_eq!(parsed.rec.as_deref(), Some("+1w"));
        assert_eq!(parsed.due, Some(d("2026-05-18")));
    }

    #[test]
    fn daily_standup_has_rec_no_due() {
        let today = d("2026-05-11");
        let parsed = try_parse("daily standup", today).unwrap();
        assert_eq!(parsed.body, "standup");
        assert_eq!(parsed.rec.as_deref(), Some("+1d"));
        assert_eq!(parsed.due, None);
    }

    #[test]
    fn business_day_recurrence() {
        let today = d("2026-05-11");
        let parsed = try_parse("Standup every business day", today).unwrap();
        assert_eq!(parsed.rec.as_deref(), Some("+1b"));
        assert_eq!(parsed.body, "Standup");
    }

    #[test]
    fn empty_body_falls_back_to_todo() {
        let today = d("2026-05-11");
        let parsed = try_parse("every monday", today).unwrap();
        let out = format_as_todo_txt(&parsed);
        assert!(out.starts_with("todo "));
        assert!(out.contains("rec:+1w"));
    }

    #[test]
    fn multiple_projects_collected() {
        let today = d("2026-05-11");
        let parsed = try_parse(
            "Plan offsite tomorrow for project home and project rentals",
            today,
        )
        .unwrap();
        assert_eq!(
            parsed.projects,
            vec!["home".to_string(), "rentals".to_string()]
        );
    }

    #[test]
    fn invalid_project_name_left_in_body() {
        // "project two words" has "two" as the candidate name. Valid tag name
        // (no spaces in "two"), so we'd actually consume "project two" — the
        // bare word "words" remains. This is the documented behavior.
        let today = d("2026-05-11");
        let parsed = try_parse("Refactor tomorrow project two words", today).unwrap();
        assert_eq!(parsed.projects, vec!["two".to_string()]);
        assert!(parsed.body.contains("words"));
    }

    #[test]
    fn sigiled_tokens_collected() {
        let today = d("2026-05-11");
        let parsed = try_parse("Buy milk tomorrow +groceries @store", today).unwrap();
        assert_eq!(parsed.projects, vec!["groceries".to_string()]);
        assert_eq!(parsed.contexts, vec!["store".to_string()]);
        assert_eq!(parsed.body, "Buy milk");
    }

    #[test]
    fn priority_high_priority_maps_to_a() {
        let today = d("2026-05-11");
        let parsed = try_parse("Fix bug high priority tomorrow", today).unwrap();
        assert_eq!(parsed.priority, Some('A'));
        assert_eq!(parsed.due, Some(d("2026-05-12")));
        assert_eq!(parsed.body, "Fix bug");
    }

    #[test]
    fn leading_priority_prefix_is_recognized() {
        // "(A) " at the head of the buffer sets priority and is stripped from
        // the body. Without this pass, the body would carry the prefix and
        // format_as_todo_txt would emit "(A) (A) Buy milk ..." if the prose
        // also mentioned priority.
        let today = d("2026-05-11");
        let parsed = try_parse("(A) Buy milk tomorrow", today).unwrap();
        assert_eq!(parsed.priority, Some('A'));
        assert_eq!(parsed.body, "Buy milk");
        assert_eq!(parsed.due, Some(d("2026-05-12")));
        assert_eq!(format_as_todo_txt(&parsed), "(A) Buy milk due:2026-05-12");
    }

    #[test]
    fn leading_priority_does_not_double_up_with_prose() {
        // If both the prefix and a prose priority phrase are present, the
        // prefix wins and the prose pass is short-circuited so the output
        // doesn't carry two "(X) " heads.
        let today = d("2026-05-11");
        let parsed = try_parse("(B) Fix bug high priority tomorrow", today).unwrap();
        assert_eq!(parsed.priority, Some('B'));
        let out = format_as_todo_txt(&parsed);
        assert_eq!(out.matches("(B)").count(), 1);
        assert!(!out.contains("(A)"));
    }

    #[test]
    fn try_parse_returns_none_when_nothing_extracted() {
        let today = d("2026-05-11");
        // No triggers, no extraction — try_parse returns None and the caller
        // falls through to the plain save path.
        assert!(try_parse("Hello world", today).is_none());
        // Trigger fires ("every") but the recurrence phrase is unrecognizable,
        // and no other pass finds anything — extraction is still empty.
        assert!(try_parse("every gnarbax", today).is_none());
    }

    #[test]
    fn rec_values_are_recurrence_module_compatible() {
        // Cross-check: every emitted rec: value must round-trip through the
        // recurrence parser the rest of the app uses. Catches drift if either
        // parser's grammar changes.
        let today = d("2026-05-11");
        for input in [
            "every day standup",
            "weekly review",
            "every monday meeting",
            "every 3 weeks haircut",
            "every other friday",
            "every business day check inbox",
            "yearly taxes",
            "biweekly retro",
        ] {
            let parsed =
                try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
            let rec = parsed.rec.unwrap_or_else(|| panic!("no rec for {input:?}"));
            assert!(
                crate::recurrence::parse_rec_spec(&rec).is_some(),
                "rec value {rec:?} from {input:?} failed recurrence::parse_rec_spec"
            );
        }
    }

    #[test]
    fn threshold_values_are_threshold_module_compatible() {
        let today = d("2026-05-11");
        for input in [
            "Task due tomorrow show 3 days before due",
            "Task due tomorrow 2 weeks before due",
            "Task due tomorrow show 1 month before",
        ] {
            let parsed =
                try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
            let t = parsed
                .threshold
                .unwrap_or_else(|| panic!("no threshold for {input:?}"));
            assert!(
                crate::threshold::parse_threshold(&t).is_some(),
                "t value {t:?} from {input:?} failed threshold::parse_threshold"
            );
        }
    }
}
