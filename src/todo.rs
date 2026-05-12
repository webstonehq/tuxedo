use std::path::Path;

/// Why a line couldn't be parsed into a `Task`. Only `Empty` exists today —
/// the parser is permissive enough that almost anything else produces a
/// (possibly weird) `Task`. Kept as an enum so we can add reasons later
/// without changing every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseError::Empty => "empty",
        })
    }
}

/// Why a `+project` / `@context` mutation was rejected. `Invalid` covers
/// names that would break tokenization (whitespace, sigils, colons); `Parse`
/// would fire only if a constructed line failed to re-parse, which the
/// validators ensure cannot happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    Invalid,
    Parse(ParseError),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagError::Invalid => f.write_str("invalid name"),
            TagError::Parse(e) => write!(f, "{}", e),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub raw: String,
    pub done: bool,
    pub done_date: Option<String>,
    pub priority: Option<char>,
    pub created_date: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub due: Option<String>,
    /// Raw value of the `rec:` tag if present, e.g. `"+1m"` or `"3b"`. Stored
    /// as the unparsed string so a malformed value round-trips intact through
    /// `serialize` — only the spawn-on-complete code path needs to parse it.
    pub rec: Option<String>,
    /// Raw value of the `t:` (threshold) tag if present, e.g. `"2026-08-01"`
    /// or `"-3d"`. Stored unparsed for round-trip integrity; the visibility
    /// filter parses it on demand via `crate::threshold`.
    pub threshold: Option<String>,
}

pub fn parse_line(raw: &str) -> Result<Task, ParseError> {
    let line = raw.trim();
    if line.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut rest: &str = line;
    let mut done = false;
    let mut done_date: Option<String> = None;

    if let Some(stripped) = strip_prefix_x(rest) {
        done = true;
        rest = stripped;
        if let Some((date, after)) = take_iso_date_prefix(rest) {
            done_date = Some(date);
            rest = after;
        }
    }

    let mut priority: Option<char> = None;
    if let Some((c, after)) = take_priority_prefix(rest) {
        priority = Some(c);
        rest = after;
    }

    let mut created_date: Option<String> = None;
    if let Some((date, after)) = take_iso_date_prefix(rest) {
        created_date = Some(date);
        rest = after;
    }

    let projects = collect_tokens(rest, '+');
    let contexts = collect_tokens(rest, '@');
    let due = find_kv(rest, "due");
    let rec = find_kv(rest, "rec");
    let threshold = find_kv(rest, "t");

    Ok(Task {
        raw: line.to_string(),
        done,
        done_date,
        priority,
        created_date,
        projects,
        contexts,
        due,
        rec,
        threshold,
    })
}

fn strip_prefix_x(s: &str) -> Option<&str> {
    let mut chars = s.chars();
    if chars.next()? == 'x' {
        let rest = chars.as_str();
        if rest.starts_with(' ') || rest.starts_with('\t') {
            return Some(rest.trim_start());
        }
    }
    None
}

/// Strip a leading `YYYY-MM-DD` token. Returns `(date_string, rest)` only if
/// the prefix is a *real* calendar date — `9999-99-99` and other invalid
/// month/day combos are rejected so they don't poison sort/grouping code that
/// later trusts the value.
fn take_iso_date_prefix(s: &str) -> Option<(String, &str)> {
    let candidate = s.get(..10)?;
    if chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d").is_err() {
        return None;
    }
    if s.len() == 10 {
        return Some((candidate.to_string(), ""));
    }
    let bytes = s.as_bytes();
    if bytes[10] == b' ' || bytes[10] == b'\t' {
        return Some((candidate.to_string(), s[11..].trim_start()));
    }
    None
}

fn take_priority_prefix(s: &str) -> Option<(char, &str)> {
    let bytes = s.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && (bytes[3] == b' ' || bytes[3] == b'\t')
    {
        return Some((bytes[1] as char, s[4..].trim_start()));
    }
    None
}

fn collect_tokens(s: &str, sigil: char) -> Vec<String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if let Some(rest) = tok.strip_prefix(sigil)
            && !rest.is_empty()
        {
            out.push(rest.to_string());
        }
    }
    out
}

/// Find the value of `key:value` for a specific key. Returns the first hit;
/// later duplicates are ignored.
fn find_kv(s: &str, key: &str) -> Option<String> {
    for tok in s.split_whitespace() {
        if let Some((k, v)) = tok.split_once(':')
            && is_valid_key(k)
            && !v.is_empty()
            && k == key
        {
            return Some(v.to_string());
        }
    }
    None
}

fn is_valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn parse_file(s: &str) -> Vec<Task> {
    s.lines().filter_map(|line| parse_line(line).ok()).collect()
}

pub fn serialize(tasks: &[Task]) -> String {
    let mut out = String::new();
    for t in tasks {
        out.push_str(&t.raw);
        out.push('\n');
    }
    out
}

/// Atomically write `body` to `path` (write to .tmp sibling, rename).
pub fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Task {
    /// Mark this task complete as of `today`. No-op if already done.
    /// The serialized line follows todo.txt convention: `x DONE CREATED BODY`,
    /// where `BODY` has had any leading priority/created-date stripped. If the
    /// task carried no creation date, `today` is used so the line stays well-
    /// formed.
    pub fn mark_done(&mut self, today: &str) -> Result<(), ParseError> {
        if self.done {
            return Ok(());
        }
        let created = self
            .created_date
            .clone()
            .unwrap_or_else(|| today.to_string());
        let body = body_after_priority(&self.raw);
        let new_raw = format!("x {today} {created} {body}");
        self.replace_from_raw(&new_raw)
    }

    /// Reverse `mark_done`: drop the leading `x ` and the done-date token.
    /// Priority that was stripped at completion time is not recovered — the
    /// user can re-set it after un-archiving.
    pub fn unmark_done(&mut self) -> Result<(), ParseError> {
        if !self.done {
            return Ok(());
        }
        let after_x = self.raw.strip_prefix("x ").unwrap_or(&self.raw);
        let body = if self.done_date.is_some() {
            // mark_done emits "x DONE_DATE CREATED BODY". Drop the 10-char
            // date plus its trailing space.
            let bytes = after_x.as_bytes();
            if bytes.len() >= 11 && (bytes[10] == b' ' || bytes[10] == b'\t') {
                after_x[11..].trim_start().to_string()
            } else {
                after_x.to_string()
            }
        } else {
            after_x.to_string()
        };
        self.replace_from_raw(&body)
    }

    /// Set or clear this task's priority. The priority byte is replaced in
    /// place at the start of the line; nothing else changes.
    pub fn set_priority(&mut self, priority: Option<char>) -> Result<(), ParseError> {
        let body = strip_priority(&self.raw);
        let new_raw = match priority {
            Some(p) => format!("({p}) {body}"),
            None => body.to_string(),
        };
        self.replace_from_raw(&new_raw)
    }

    /// Cycle priority A → B → C → none → A. Returns the new value (for the
    /// caller to flash). Behaves like `set_priority` w.r.t. the line format.
    pub fn cycle_priority(&mut self) -> Result<Option<char>, ParseError> {
        let next = match self.priority {
            None => Some('A'),
            Some('A') => Some('B'),
            Some('B') => Some('C'),
            Some(_) => None,
        };
        self.set_priority(next)?;
        Ok(next)
    }

    /// Append `+name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the project was already present.
    pub fn add_project(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '+', |t| &t.projects)
    }

    /// Append `@name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the context was already present.
    pub fn add_context(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '@', |t| &t.contexts)
    }

    /// Remove every `@name` token from the line. Returns `Ok(true)` if any
    /// was removed, `Ok(false)` if the context was absent.
    pub fn remove_context(&mut self, name: &str) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if !self.contexts.iter().any(|c| c == name) {
            return Ok(false);
        }
        let needle = format!("@{name}");
        let new_raw = self
            .raw
            .split_whitespace()
            .filter(|tok| *tok != needle)
            .collect::<Vec<_>>()
            .join(" ");
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    fn add_tag(
        &mut self,
        name: &str,
        sigil: char,
        existing: impl Fn(&Task) -> &Vec<String>,
    ) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if existing(self).iter().any(|x| x == name) {
            return Ok(false);
        }
        let new_raw = format!("{} {sigil}{name}", self.raw.trim_end());
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    /// Re-parse `raw` and overwrite self. Only mutates on success, so a
    /// failed parse leaves the task untouched.
    fn replace_from_raw(&mut self, raw: &str) -> Result<(), ParseError> {
        *self = parse_line(raw)?;
        Ok(())
    }
}

/// True if `s` begins with a `(X) ` priority token.
pub fn starts_with_priority(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' '
}

/// True if `s` begins with a `YYYY-MM-DD` token (followed by EOL or whitespace
/// is not required here — callers use this as a hint, not a tokenizer).
pub fn starts_with_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

/// Strip a leading `(X) ` priority token if present, otherwise return the
/// input unchanged.
pub fn strip_priority(raw: &str) -> &str {
    let b = raw.as_bytes();
    if b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' ' {
        return &raw[4..];
    }
    raw
}

/// A project/context name is valid if non-empty and contains no characters
/// that would break the todo.txt tokenization: whitespace splits a tag in
/// half, and `+`/`@`/`:` collide with the format's own sigils.
pub fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| !c.is_whitespace() && c != '+' && c != '@' && c != ':')
}

pub fn body_after_priority(raw: &str) -> &str {
    let mut s = raw;
    if let Some(stripped) = strip_prefix_x(s) {
        s = stripped;
        if let Some((_, after)) = take_iso_date_prefix(s) {
            s = after;
        }
    }
    if let Some((_, after)) = take_priority_prefix(s) {
        s = after;
    }
    if let Some((_, after)) = take_iso_date_prefix(s) {
        s = after;
    }
    s
}

/// Description text only: strip the leading `x `, done/created dates, and
/// priority via `body_after_priority`, then drop every `+project`,
/// `@context`, and `key:value` token from what remains. Whitespace between
/// surviving words collapses to single spaces. Returns an owned `String`
/// because we're filtering tokens, not slicing a prefix.
pub fn body_only(raw: &str) -> String {
    body_after_priority(raw)
        .split_whitespace()
        .filter(|tok| !is_meta_token(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_meta_token(tok: &str) -> bool {
    if let Some(rest) = tok.strip_prefix('+')
        && !rest.is_empty()
    {
        return true;
    }
    if let Some(rest) = tok.strip_prefix('@')
        && !rest.is_empty()
    {
        return true;
    }
    if let Some((k, v)) = tok.split_once(':')
        && is_valid_key(k)
        && !v.is_empty()
    {
        return true;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_empty_returns_err() {
        assert!(matches!(parse_line(""), Err(ParseError::Empty)));
        assert!(matches!(parse_line("   "), Err(ParseError::Empty)));
        assert!(matches!(parse_line("\n"), Err(ParseError::Empty)));
    }

    #[test]
    fn parse_line_simple_input_returns_ok() {
        assert!(parse_line("Hello").is_ok());
    }

    #[test]
    fn parse_error_displays_human_message() {
        assert_eq!(format!("{}", ParseError::Empty), "empty");
    }

    #[test]
    fn parses_line_starting_with_non_ascii_after_single_byte() {
        // Regression: `take_iso_date_prefix` used byte indexing (`&s[..10]`)
        // after a byte-length check, which panicked when byte 10 fell inside
        // a multi-byte UTF-8 character. Triggered by tasks like the one
        // below, where '2' is 1 byte and the following Cyrillic chars are
        // 2 bytes each, putting byte 10 inside 'с'.
        let t = parse_line("2Написать задачи на день due:2026-05-11 rec:+1d").unwrap();
        assert_eq!(t.created_date, None);
        assert_eq!(t.due.as_deref(), Some("2026-05-11"));
        assert_eq!(t.rec.as_deref(), Some("+1d"));
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        // `9999-99-99` is well-formed lexically but not a real date —
        // earlier versions accepted it and let the bogus value flow into
        // sort/grouping code as a string. The parser now refuses.
        let t = parse_line("9999-99-99 not a date").unwrap();
        assert_eq!(t.created_date, None);
        assert!(t.raw.starts_with("9999-99-99"));
    }

    #[test]
    fn parses_priority_and_dates() {
        let t = parse_line("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08").unwrap();
        assert_eq!(t.priority, Some('A'));
        assert_eq!(t.created_date.as_deref(), Some("2026-04-28"));
        assert_eq!(t.due.as_deref(), Some("2026-05-08"));
        assert_eq!(t.projects, vec!["health"]);
        assert_eq!(t.contexts, vec!["phone"]);
        assert!(!t.done);
        assert_eq!(t.rec, None);
    }

    #[test]
    fn parses_rec_tag() {
        let t = parse_line("2026-05-09 Pay rent due:2026-05-15 rec:+1m").unwrap();
        assert_eq!(t.rec.as_deref(), Some("+1m"));
        assert_eq!(t.due.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn parses_absolute_threshold_tag() {
        let t = parse_line("2026-04-01 Renew passport t:2026-08-01 +personal").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("2026-08-01"));
    }

    #[test]
    fn parses_relative_threshold_tag() {
        let t = parse_line("Pay rent due:2026-06-01 t:-3d +finance").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("-3d"));
        assert_eq!(t.due.as_deref(), Some("2026-06-01"));
    }

    #[test]
    fn body_only_strips_threshold_token() {
        // The "no chip" rendering choice relies on body_only filtering `t:`
        // out via is_meta_token. Asserting it here so a future change to
        // is_valid_key can't regress this without an explicit test failure.
        assert_eq!(
            body_only("2026-04-01 Renew passport t:2026-08-01 +personal"),
            "Renew passport",
        );
        assert_eq!(
            body_only("Pay rent due:2026-06-01 t:-3d +finance"),
            "Pay rent",
        );
    }

    #[test]
    fn parses_completed() {
        let t = parse_line("x 2026-05-05 2026-05-01 Submit expense report +work @laptop").unwrap();
        assert!(t.done);
        assert_eq!(t.done_date.as_deref(), Some("2026-05-05"));
        assert_eq!(t.created_date.as_deref(), Some("2026-05-01"));
        assert_eq!(t.projects, vec!["work"]);
    }

    #[test]
    fn parses_all_sample_lines() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        assert_eq!(parsed.len(), 19);
        let done = parsed.iter().filter(|t| t.done).count();
        assert_eq!(done, 3);
        let with_due = parsed.iter().filter(|t| t.due.is_some()).count();
        assert_eq!(with_due, 7);
        let with_rec = parsed.iter().filter(|t| t.rec.is_some()).count();
        assert_eq!(with_rec, 1);
        let with_threshold = parsed.iter().filter(|t| t.threshold.is_some()).count();
        assert_eq!(with_threshold, 1);
    }

    #[test]
    fn body_strips_metadata() {
        let raw = "(A) 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw), "Hello world");
        let raw2 = "x 2026-05-05 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw2), "Hello world");
    }

    #[test]
    fn body_only_drops_tags_and_kv_pairs() {
        // Plain description survives unchanged.
        assert_eq!(body_only("Hello world"), "Hello world");
        // Priority + creation date prefix are stripped, +project / @context /
        // due:... are filtered out, words collapse to single spaces.
        assert_eq!(
            body_only("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08"),
            "Call dentist",
        );
        // Completed lines lose `x` + done date + creation date as well.
        assert_eq!(
            body_only("x 2026-05-05 2026-05-01 Submit expense report +work @laptop"),
            "Submit expense report",
        );
        // Sigils inside a word (not at the start of a token) are not tags
        // and must be preserved.
        assert_eq!(body_only("email a+b@example.com"), "email a+b@example.com");
        // Lone sigils with no name are not valid tags either.
        assert_eq!(body_only("type @ then context"), "type @ then context");
        // Unknown key:value tokens still drop — todo.txt treats any
        // alphanumeric `key:value` as an extension, so we mirror that.
        assert_eq!(body_only("backup id:abc-123 nightly"), "backup nightly");
    }

    #[test]
    fn round_trip_preserves_raw() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        let serialized = serialize(&parsed);
        let reparsed = parse_file(&serialized);
        assert_eq!(parsed.len(), reparsed.len());
        for (a, b) in parsed.iter().zip(reparsed.iter()) {
            assert_eq!(a.raw, b.raw);
        }
    }
}
