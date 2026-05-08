use super::App;
use super::draft::prev_char_boundary;
use super::types::AUTOCOMPLETE_CAP;
use crate::todo::{self, Task, starts_with_iso_date, starts_with_priority};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Project,
    Context,
}

/// The `+project` / `@context` token currently under the input cursor, if any.
/// `start` is the byte offset of the sigil within the draft so callers can
/// slice and replace; `prefix` is the text already typed after the sigil.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveToken<'a> {
    pub kind: TokenKind,
    pub prefix: &'a str,
    pub start: usize,
}

/// Find the autocomplete token at `cursor`. Walks back through non-whitespace
/// chars to the start of the current word; returns `Some` only if that word
/// begins with `+` or `@`. Cursor on whitespace, on plain text, or at byte 0
/// of an empty draft yields `None`.
pub fn active_token(draft: &str, cursor: usize) -> Option<ActiveToken<'_>> {
    let cursor = cursor.min(draft.len());
    if cursor == 0 {
        return None;
    }
    let mut start = cursor;
    while start > 0 {
        let prev = prev_char_boundary(draft, start);
        let c = draft[prev..start].chars().next()?;
        if c.is_whitespace() {
            break;
        }
        start = prev;
    }
    if start >= cursor {
        return None;
    }
    let kind = match draft.as_bytes()[start] {
        b'+' => TokenKind::Project,
        b'@' => TokenKind::Context,
        _ => return None,
    };
    Some(ActiveToken {
        kind,
        prefix: &draft[start + 1..cursor],
        start,
    })
}

impl App {
    /// Whether the autocomplete popup should render and consume keys. False
    /// when there's no active token, the corpus has no matches, or the user
    /// has explicitly dismissed the popup with Esc.
    pub fn autocomplete_visible(&self) -> bool {
        !self.draft.autocomplete_suppressed() && !self.autocomplete_matches().is_empty()
    }

    /// Suggestions for the `+project` / `@context` token currently under the
    /// cursor. Empty when no token is active or the corpus has nothing
    /// matching the prefix. Sorted prefix-matches-first then contains-only,
    /// each group alphabetical, capped at 8.
    pub fn autocomplete_matches(&self) -> Vec<&str> {
        let Some(tok) = active_token(self.draft.text(), self.draft.cursor()) else {
            return Vec::new();
        };
        let prefix_lc = tok.prefix.to_lowercase();
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for t in &self.tasks {
            let source = match tok.kind {
                TokenKind::Project => &t.projects,
                TokenKind::Context => &t.contexts,
            };
            for s in source {
                seen.insert(s.as_str());
            }
        }
        let mut prefix_hits: Vec<&str> = Vec::new();
        let mut contains_hits: Vec<&str> = Vec::new();
        for s in seen {
            let lc = s.to_lowercase();
            if lc.starts_with(&prefix_lc) {
                prefix_hits.push(s);
            } else if !prefix_lc.is_empty() && lc.contains(&prefix_lc) {
                contains_hits.push(s);
            }
        }
        prefix_hits.extend(contains_hits);
        prefix_hits.truncate(AUTOCOMPLETE_CAP);
        prefix_hits
    }

    /// Move the autocomplete selection. No-op when no matches exist.
    pub fn autocomplete_step(&mut self, forward: bool) {
        let n = self.autocomplete_matches().len();
        self.draft.step_autocomplete(n, forward);
    }

    /// What `add_from_draft` would parse if the user hit Enter now: the
    /// trimmed draft, with today's date prepended when no priority/date/done
    /// marker is already present, run through `parse_line`. Returns None for
    /// an empty draft (no preview row to render).
    pub fn preview_parse(&self) -> Option<Result<Task, todo::ParseError>> {
        let trimmed = self.draft.text().trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut text = trimmed.to_string();
        if !starts_with_priority(&text) && !starts_with_iso_date(&text) && !text.starts_with("x ") {
            text = format!("{} {}", self.today, text);
        }
        Some(todo::parse_line(&text))
    }

    /// Replace the active token with the currently selected match. No-op when
    /// no token is active or no matches exist. Replaces the *whole* token at
    /// the cursor (sigil to next whitespace), so accepting `work` while the
    /// cursor sits inside `+wor` produces `+work`, not `+workr`.
    pub fn autocomplete_accept(&mut self) {
        let Some(tok) = active_token(self.draft.text(), self.draft.cursor()) else {
            return;
        };
        let matches = self.autocomplete_matches();
        if matches.is_empty() {
            return;
        }
        let idx = self.draft.autocomplete_index().min(matches.len() - 1);
        let chosen = matches[idx].to_string();
        let token_start = tok.start;
        let after_sigil = &self.draft.text()[token_start + 1..];
        let end_offset = after_sigil
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(after_sigil.len());
        let end = token_start + 1 + end_offset;
        self.draft.replace_token(token_start + 1, end, &chosen);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn active_token_empty_draft_is_none() {
        assert!(active_token("", 0).is_none());
    }

    #[test]
    fn active_token_plain_text_is_none() {
        assert!(active_token("Hello world", 5).is_none());
        assert!(active_token("Hello world", 11).is_none());
        assert!(active_token("Hello world", 0).is_none());
    }

    #[test]
    fn active_token_after_trailing_space_is_none() {
        // Cursor right after a space — no active token even if a sigil sits
        // earlier on the line. Typing past a token completes it.
        assert!(active_token("foo ", 4).is_none());
        assert!(active_token("+work ", 6).is_none());
    }

    #[test]
    fn active_token_right_after_plus_sigil() {
        let t = active_token("Foo +", 5).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "");
        assert_eq!(t.start, 4);
    }

    #[test]
    fn active_token_right_after_at_sigil() {
        let t = active_token("Foo @", 5).unwrap();
        assert_eq!(t.kind, TokenKind::Context);
        assert_eq!(t.prefix, "");
        assert_eq!(t.start, 4);
    }

    #[test]
    fn active_token_mid_project() {
        // "Foo +wor" — cursor between 'o' and 'r' (byte 7).
        let t = active_token("Foo +wor", 7).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "wo");
        assert_eq!(t.start, 4);
    }

    #[test]
    fn active_token_at_end_of_project() {
        let t = active_token("Foo +work", 9).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "work");
        assert_eq!(t.start, 4);
    }

    #[test]
    fn active_token_sigil_at_start_of_draft() {
        let t = active_token("+work", 5).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "work");
        assert_eq!(t.start, 0);
    }

    #[test]
    fn active_token_handles_multibyte_after_sigil() {
        // 'é' is two bytes (U+00E9 = 0xC3 0xA9). Make sure prefix slicing
        // doesn't split a char.
        let s = "Hi +café";
        let t = active_token(s, s.len()).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "café");
    }

    #[test]
    fn active_token_handles_multibyte_before_sigil() {
        // Multi-byte char in surrounding text shouldn't confuse the walk.
        let s = "café +wo";
        let t = active_token(s, s.len()).unwrap();
        assert_eq!(t.kind, TokenKind::Project);
        assert_eq!(t.prefix, "wo");
    }

    #[test]
    fn autocomplete_matches_empty_when_no_token() {
        let mut app = build_app("a +work\n");
        app.draft_set("Hello world".into());
        assert!(app.autocomplete_matches().is_empty());
    }

    #[test]
    fn autocomplete_matches_empty_when_corpus_empty() {
        let mut app = build_app("plain task\n");
        app.draft_set("Foo +".into());
        assert!(app.autocomplete_matches().is_empty());
    }

    #[test]
    fn autocomplete_matches_filters_by_prefix() {
        let mut app = build_app("a +work\nb +health\nc +home\n");
        app.draft_set("Foo +ho".into());
        let m = app.autocomplete_matches();
        assert!(m.contains(&"home"));
        assert!(!m.contains(&"work"));
    }

    #[test]
    fn autocomplete_matches_prefix_ranks_above_contains() {
        let mut app = build_app("a +outdoor\nb +work\n");
        app.draft_set("Foo +o".into());
        let m = app.autocomplete_matches();
        let pos_outdoor = m.iter().position(|s| *s == "outdoor").unwrap();
        // "work" doesn't contain "o" with prefix match, but the *contains*
        // pass picks it up because "work" contains 'o'. Outdoor (prefix) must
        // appear before work (contains-only).
        if let Some(pw) = m.iter().position(|s| *s == "work") {
            assert!(pos_outdoor < pw);
        }
    }

    #[test]
    fn autocomplete_matches_case_insensitive() {
        let mut app = build_app("a +Work\n");
        app.draft_set("Foo +wo".into());
        assert!(app.autocomplete_matches().contains(&"Work"));
    }

    #[test]
    fn autocomplete_matches_caps_at_eight() {
        let raw: String = (0..20).map(|i| format!("a +p{:02}\n", i)).collect();
        let mut app = build_app(&raw);
        app.draft_set("Foo +p".into());
        assert_eq!(app.autocomplete_matches().len(), 8);
    }

    #[test]
    fn autocomplete_matches_dedups_across_tasks() {
        let mut app = build_app("a +work\nb +work\nc +work\n");
        app.draft_set("Foo +".into());
        let m = app.autocomplete_matches();
        assert_eq!(m.iter().filter(|s| **s == "work").count(), 1);
    }

    #[test]
    fn autocomplete_matches_context_kind() {
        let mut app = build_app("a +work @home\n");
        app.draft_set("Foo @".into());
        let m = app.autocomplete_matches();
        assert!(m.contains(&"home"));
        assert!(!m.contains(&"work"));
    }

    #[test]
    fn autocomplete_step_wraps_forward_and_back() {
        let mut app = build_app("a +foo\nb +bar\nc +baz\n");
        app.draft_set("X +".into());
        // Matches sort alphabetically: bar, baz, foo (since none start with "")
        // — actually all match the empty prefix as prefix matches.
        assert_eq!(app.draft.autocomplete_index(), 0);
        app.autocomplete_step(true);
        assert_eq!(app.draft.autocomplete_index(), 1);
        app.autocomplete_step(true);
        assert_eq!(app.draft.autocomplete_index(), 2);
        app.autocomplete_step(true);
        assert_eq!(app.draft.autocomplete_index(), 0); // wrap forward
        app.autocomplete_step(false);
        assert_eq!(app.draft.autocomplete_index(), 2); // wrap backward
    }

    #[test]
    fn autocomplete_step_noop_when_no_matches() {
        let mut app = build_app("plain\n");
        app.draft_set("X +".into());
        // No matches — step shouldn't blow up or move past 0.
        app.autocomplete_step(true);
        assert_eq!(app.draft.autocomplete_index(), 0);
    }

    #[test]
    fn autocomplete_accept_replaces_partial_token_at_end() {
        let mut app = build_app("a +work\n");
        app.draft_set("Call dentist +wo".into());
        app.autocomplete_accept();
        assert_eq!(app.draft.text(), "Call dentist +work");
        assert_eq!(app.draft.cursor(), app.draft.text().len());
    }

    #[test]
    fn autocomplete_accept_replaces_whole_token_when_cursor_mid() {
        // Modern-editor convention: accepting a suggestion replaces the entire
        // word the cursor is inside, not just the prefix.
        let mut app = build_app("a +work\n");
        app.draft_set("Call +wor extra".into());
        // Position cursor between 'o' and 'r' (byte 7).
        app.draft.force_cursor(7);
        app.autocomplete_accept();
        assert_eq!(app.draft.text(), "Call +work extra");
        assert_eq!(app.draft.cursor(), "Call +work".len());
    }

    #[test]
    fn autocomplete_accept_no_token_is_noop() {
        let mut app = build_app("a +work\n");
        app.draft_set("Plain text".into());
        let original = app.draft.text().to_string();
        app.autocomplete_accept();
        assert_eq!(app.draft.text(), original);
    }

    #[test]
    fn autocomplete_accept_no_matches_is_noop() {
        let mut app = build_app("a\n"); // no projects in corpus
        app.draft_set("Hi +foo".into());
        let original = app.draft.text().to_string();
        app.autocomplete_accept();
        assert_eq!(app.draft.text(), original);
    }

    #[test]
    fn preview_parse_empty_returns_none() {
        let mut app = build_app("");
        assert!(app.preview_parse().is_none());
        app.draft_set("   ".into());
        assert!(app.preview_parse().is_none());
    }

    #[test]
    fn preview_parse_prepends_today_for_bare_text() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        let r = app.preview_parse().unwrap().unwrap();
        assert_eq!(r.created_date.as_deref(), Some("2026-05-06"));
    }

    #[test]
    fn preview_parse_does_not_prepend_when_priority_present() {
        let mut app = build_app("");
        app.draft_set("(A) Buy milk".into());
        let r = app.preview_parse().unwrap().unwrap();
        assert_eq!(r.priority, Some('A'));
        assert!(r.created_date.is_none());
    }

    #[test]
    fn preview_parse_does_not_prepend_when_date_present() {
        let mut app = build_app("");
        app.draft_set("2026-05-01 Old task".into());
        let r = app.preview_parse().unwrap().unwrap();
        assert_eq!(r.created_date.as_deref(), Some("2026-05-01"));
    }

    #[test]
    fn autocomplete_visible_reflects_matches_and_suppression() {
        let mut app = build_app("a +work\n");
        app.draft_set("X +".into());
        assert!(app.autocomplete_visible());

        // Esc-equivalent: setting the flag hides the popup without changing
        // the underlying matches.
        app.draft.suppress_autocomplete();
        assert!(!app.autocomplete_visible());
        assert!(!app.autocomplete_matches().is_empty());

        // Mutating the draft re-arms the popup.
        app.draft_insert_char('w');
        assert!(!app.draft.autocomplete_suppressed());
        assert!(app.autocomplete_visible());
    }

    #[test]
    fn autocomplete_visible_false_with_no_token() {
        let mut app = build_app("a +work\n");
        app.draft_set("Plain text".into());
        assert!(!app.autocomplete_visible());
    }

    #[test]
    fn draft_mutation_resets_autocomplete_selected() {
        // When the user types a character, the popup's match list usually
        // changes; keeping `selected` pointing at "the second item" leaves
        // the highlight on a different suggestion than before.
        let mut app = build_app("a +foo +bar\n");
        app.draft_set("X +".into());
        app.autocomplete_step(true);
        assert_eq!(app.draft.autocomplete_index(), 1);
        app.draft_insert_char('b');
        assert_eq!(app.draft.autocomplete_index(), 0);

        // Backspace also resets.
        app.autocomplete_step(true);
        app.draft_backspace();
        assert_eq!(app.draft.autocomplete_index(), 0);

        // Delete-forward also resets.
        app.draft_set("X +b".into());
        app.draft_home();
        app.autocomplete_step(true);
        app.draft_delete_forward();
        assert_eq!(app.draft.autocomplete_index(), 0);
    }

    #[test]
    fn autocomplete_accept_uses_selected_index() {
        let mut app = build_app("a +alpha +beta +gamma\n");
        app.draft_set("X +".into());
        // Default selected = 0 → "alpha".
        app.autocomplete_step(true); // selected = 1 → "beta"
        app.autocomplete_accept();
        assert_eq!(app.draft.text(), "X +beta");
    }
}
