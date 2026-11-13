use unicode_segmentation::UnicodeSegmentation;

use super::draft_overlay::DraftOverlay;
use super::{App, UNDO_LIMIT};
use std::collections::VecDeque;

/// Sub-mode for the edit/add dialog, mirroring vim's modal editing model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogInputMode {
    #[default]
    Insert,
    Normal,
    Extend,
}

impl DialogInputMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Insert => "Insert",
            Self::Normal => "Normal",
            Self::Extend => "Extend",
        }
    }
}

/// Fixed command namespace captured when a task dialog opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditingProfile {
    #[default]
    Vim,
    Helix,
}

impl EditingProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vim => "vim",
            Self::Helix => "helix",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Vim => "Vim",
            Self::Helix => "Helix subset",
        }
    }
}

/// Byte offset within a draft `String`. Construction enforces UTF-8 char-boundary
/// landing, so cursor positions can't be left mid-codepoint by direct assignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DraftCursor(usize);

impl DraftCursor {
    /// Clamp to the nearest char boundary at or before `byte` (or to `s.len()`).
    pub fn clamped(s: &str, byte: usize) -> Self {
        let mut b = byte.min(s.len());
        while b > 0 && !s.is_char_boundary(b) {
            b -= 1;
        }
        Self(b)
    }

    pub fn at_end(s: &str) -> Self {
        Self(s.len())
    }

    pub fn zero() -> Self {
        Self(0)
    }

    pub fn byte(self) -> usize {
        self.0
    }
}

/// The selection is part of an edit's state, but modes and overlays are not:
/// undo/redo runs in Normal mode without reviving stale popup ranges.
#[derive(Debug, Clone)]
struct DraftSnapshot {
    text: String,
    cursor: DraftCursor,
    anchor: DraftCursor,
}

#[derive(Debug, Default, Clone)]
pub struct DraftState {
    text: String,
    cursor: DraftCursor,
    anchor: DraftCursor,
    profile: EditingProfile,
    insert_after: bool,
    autocomplete_selected: usize,
    autocomplete_suppressed: bool,
    /// Open metadata picker (slash menu, calendar, recurrence builder,
    /// priority chooser). At most one at a time. `None` is the default — the
    /// user is just editing text.
    overlay: Option<DraftOverlay>,
    input_mode: DialogInputMode,
    undo: VecDeque<DraftSnapshot>,
    redo: Vec<DraftSnapshot>,
    /// Kept across draft openings; independent of OSC 52 task-list copying.
    register: String,
    /// The first text mutation in a compound picker operation records state.
    history_group_depth: usize,
    history_group_recorded: bool,
}

impl DraftState {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor.byte()
    }

    pub fn autocomplete_index(&self) -> usize {
        self.autocomplete_selected
    }

    pub fn autocomplete_suppressed(&self) -> bool {
        self.autocomplete_suppressed
    }

    pub fn overlay(&self) -> Option<&DraftOverlay> {
        self.overlay.as_ref()
    }

    pub fn overlay_mut(&mut self) -> Option<&mut DraftOverlay> {
        self.overlay.as_mut()
    }

    pub fn set_overlay(&mut self, overlay: Option<DraftOverlay>) {
        self.overlay = overlay;
    }

    /// Presentation label for the current editor profile. Keep this separate
    /// from its mode so another editor can supply its own labels later.
    pub fn editor_profile_label(&self) -> &'static str {
        self.profile.label()
    }

    pub fn input_mode(&self) -> DialogInputMode {
        self.input_mode
    }

    pub fn set_input_mode(&mut self, mode: DialogInputMode) {
        self.input_mode = mode;
        self.normalize_selection();
    }

    pub fn clear(&mut self) {
        self.reset_history();
        self.text.clear();
        self.cursor = DraftCursor::zero();
        self.anchor = self.cursor;
        self.profile = EditingProfile::Vim;
        self.insert_after = false;
        self.reset_autocomplete();
        self.overlay = None;
        self.input_mode = DialogInputMode::Insert;
    }

    /// Replace the text and park the cursor at the end. Used when entering
    /// edit mode or otherwise seeding the input from existing text. The input
    /// sub-mode is the caller's choice — `App::draft_set` (`e`) lands in
    /// Normal mode, `App::draft_set_insert` (`i`) lands in Insert mode.
    pub fn set(&mut self, s: String, mode: DialogInputMode) {
        self.reset_history();
        self.cursor = DraftCursor::at_end(&s);
        self.anchor = self.cursor;
        self.profile = EditingProfile::Vim;
        self.insert_after = false;
        self.text = s;
        self.reset_autocomplete();
        self.overlay = None;
        self.input_mode = mode;
    }

    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor.byte();
        self.replace_token(pos, pos, c.encode_utf8(&mut [0; 4]));
        self.normalize_selection();
        self.reset_autocomplete();
    }

    pub fn backspace(&mut self) {
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        let prev = if self.profile == EditingProfile::Helix {
            helix_draft::grapheme_prev(&self.text, pos)
        } else {
            prev_char_boundary(&self.text, pos)
        };
        self.replace_token(prev, pos, "");
        self.normalize_selection();
        self.reset_autocomplete();
    }

    pub fn delete_forward(&mut self) {
        let pos = self.cursor.byte();
        if pos >= self.text.len() {
            return;
        }
        let next = if self.profile == EditingProfile::Helix {
            helix_draft::grapheme_next(&self.text, pos)
        } else {
            next_char_boundary(&self.text, pos)
        };
        self.replace_token(pos, next, "");
        self.normalize_selection();
        self.reset_autocomplete();
    }

    pub fn move_left(&mut self) {
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        self.cursor = DraftCursor(if self.profile == EditingProfile::Helix {
            helix_draft::grapheme_prev(&self.text, pos)
        } else {
            prev_char_boundary(&self.text, pos)
        });
        self.normalize_selection();
    }

    pub fn move_right(&mut self) {
        let pos = self.cursor.byte();
        if pos >= self.text.len() {
            return;
        }
        self.cursor = DraftCursor(if self.profile == EditingProfile::Helix {
            helix_draft::grapheme_next(&self.text, pos)
        } else {
            next_char_boundary(&self.text, pos)
        });
        self.normalize_selection();
    }

    pub fn move_home(&mut self) {
        self.cursor = DraftCursor::zero();
        self.normalize_selection();
    }

    pub fn move_end(&mut self) {
        self.cursor = DraftCursor::at_end(&self.text);
        self.normalize_selection();
    }

    /// Move to the start of the next word (`w`).
    pub fn move_word_forward(&mut self) {
        let s = &self.text;
        let mut pos = self.cursor.byte();
        while pos < s.len() && !s.as_bytes()[pos].is_ascii_whitespace() {
            pos = next_char_boundary(s, pos);
        }
        while pos < s.len() && s.as_bytes()[pos].is_ascii_whitespace() {
            pos = next_char_boundary(s, pos);
        }
        self.cursor = DraftCursor(pos);
        self.normalize_selection();
    }

    /// Move to the start of the current or previous word (`b`).
    pub fn move_word_backward(&mut self) {
        let s = &self.text;
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        let mut p = prev_char_boundary(s, pos);
        while p > 0 && s.as_bytes()[p].is_ascii_whitespace() {
            p = prev_char_boundary(s, p);
        }
        while p > 0 && !s.as_bytes()[prev_char_boundary(s, p)].is_ascii_whitespace() {
            p = prev_char_boundary(s, p);
        }
        self.cursor = DraftCursor(p);
        self.normalize_selection();
    }

    /// Delete from the cursor to the start of the next word (`dw`/`cw`).
    pub fn delete_word_forward(&mut self) {
        let start = self.cursor.byte();
        let s = &self.text;
        let mut end = start;
        while end < s.len() && !s.as_bytes()[end].is_ascii_whitespace() {
            end = next_char_boundary(s, end);
        }
        while end < s.len() && s.as_bytes()[end].is_ascii_whitespace() {
            end = next_char_boundary(s, end);
        }
        if end > start {
            self.replace_token(start, end, "");
            self.reset_autocomplete();
        }
    }

    /// Delete from the start of the previous word to the cursor (`Ctrl+W`).
    /// Whitespace-delimited, mirroring `move_word_backward` and matching
    /// readline's unix-word-rubout.
    pub fn delete_word_backward(&mut self) {
        let end = self.cursor.byte();
        if end == 0 {
            return;
        }
        let s = &self.text;
        let mut start = prev_char_boundary(s, end);
        // Skip whitespace immediately before the cursor, then the word itself.
        while start > 0 && s.as_bytes()[start].is_ascii_whitespace() {
            start = prev_char_boundary(s, start);
        }
        while start > 0 && !s.as_bytes()[prev_char_boundary(s, start)].is_ascii_whitespace() {
            start = prev_char_boundary(s, start);
        }
        self.replace_token(start, end, "");
        self.normalize_selection();
        self.reset_autocomplete();
    }

    /// Delete from the start of the line to the cursor (`Ctrl+U`).
    pub fn kill_to_start(&mut self) {
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        self.replace_token(0, pos, "");
        self.normalize_selection();
        self.reset_autocomplete();
    }

    /// Delete from the cursor to the end of the line (`Ctrl+K`). The cursor
    /// stays put.
    pub fn kill_to_end(&mut self) {
        let pos = self.cursor.byte();
        if pos >= self.text.len() {
            return;
        }
        self.replace_token(pos, self.text.len(), "");
        self.normalize_selection();
        self.reset_autocomplete();
    }

    /// Move to the end of the current or next word (`e`).
    pub fn move_word_end(&mut self) {
        let s = &self.text;
        let mut pos = self.cursor.byte();
        // Step off the current position first
        if pos < s.len() {
            pos = next_char_boundary(s, pos);
        }
        while pos < s.len() && s.as_bytes()[pos].is_ascii_whitespace() {
            pos = next_char_boundary(s, pos);
        }
        while pos < s.len() && {
            let next = next_char_boundary(s, pos);
            next < s.len() && !s.as_bytes()[next].is_ascii_whitespace()
        } {
            pos = next_char_boundary(s, pos);
        }
        self.cursor = DraftCursor(pos);
        self.normalize_selection();
    }

    /// Cycle the selected autocomplete match. `n` is the current match-list length.
    /// No-op when `n == 0`.
    pub fn step_autocomplete(&mut self, n: usize, forward: bool) {
        if n == 0 {
            return;
        }
        let cur = self.autocomplete_selected.min(n - 1);
        self.autocomplete_selected = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
    }

    /// Hide the popup until the next text mutation.
    pub fn suppress_autocomplete(&mut self) {
        self.autocomplete_suppressed = true;
    }

    /// Replace the byte range `[start, end)` with `with`, parking the cursor
    /// at `start + with.len()`. Used by `autocomplete_accept` to swap in a
    /// chosen suggestion. Caller guarantees `start` and `end` are char boundaries.
    pub fn replace_token(&mut self, start: usize, end: usize, with: &str) {
        let (start, end) = if self.profile == EditingProfile::Helix {
            (
                helix_draft::grapheme_floor(&self.text, start),
                helix_draft::grapheme_ceil(&self.text, end),
            )
        } else {
            (start, end)
        };
        if &self.text[start..end] != with {
            self.record_edit();
        }
        if self.profile == EditingProfile::Helix && self.input_mode == DialogInputMode::Insert {
            let a = self.anchor.byte();
            if a > end || (a == end && (!self.insert_after || start != end)) {
                self.anchor = DraftCursor(a - (end - start) + with.len());
            } else if a > start {
                self.anchor = DraftCursor(start + with.len());
            }
        }
        self.text.replace_range(start..end, with);
        self.cursor = DraftCursor(start + with.len());
        self.normalize_selection();
        self.autocomplete_selected = 0;
        self.autocomplete_suppressed = false;
    }

    pub fn register(&self) -> &str {
        &self.register
    }

    pub fn set_register(&mut self, text: String) {
        self.register = text;
    }

    fn snapshot(&self) -> DraftSnapshot {
        DraftSnapshot {
            text: self.text.clone(),
            cursor: DraftCursor(self.normal_endpoints().1),
            anchor: DraftCursor(self.normal_endpoints().0),
        }
    }

    /// Call immediately before an actual text mutation, including selections
    /// and paste. No-op edits must not discard the redo branch.
    fn record_edit(&mut self) {
        if self.profile != EditingProfile::Helix
            || (self.history_group_depth > 0 && self.history_group_recorded)
        {
            return;
        }
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(self.snapshot());
        self.redo.clear();
        if self.history_group_depth > 0 {
            self.history_group_recorded = true;
        }
    }

    /// Group the text replacements performed by one accepted picker into one
    /// edit. Nested groups are supported; callers must balance begin/end.
    pub fn begin_history_group(&mut self) {
        if self.history_group_depth == 0 {
            self.history_group_recorded = false;
        }
        self.history_group_depth += 1;
    }

    pub fn end_history_group(&mut self) {
        self.history_group_depth = self.history_group_depth.saturating_sub(1);
        if self.history_group_depth == 0 {
            self.history_group_recorded = false;
        }
    }

    fn restore_snapshot(&mut self, state: DraftSnapshot) {
        self.text = state.text;
        self.cursor = state.cursor;
        self.anchor = state.anchor;
        self.input_mode = DialogInputMode::Normal;
        self.normalize_selection();
        self.overlay = None;
        self.normalize_selection();
        self.reset_autocomplete();
    }

    pub fn undo(&mut self) -> bool {
        let Some(state) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push(self.snapshot());
        self.restore_snapshot(state);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(state) = self.redo.pop() else {
            return false;
        };
        self.undo.push_back(self.snapshot());
        self.restore_snapshot(state);
        true
    }

    fn reset_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.history_group_depth = 0;
        self.history_group_recorded = false;
    }

    fn reset_autocomplete(&mut self) {
        self.autocomplete_selected = 0;
        self.autocomplete_suppressed = false;
    }

    #[cfg(test)]
    pub(crate) fn force_cursor(&mut self, byte: usize) {
        self.cursor = DraftCursor::clamped(&self.text, byte);
    }
}

#[path = "helix_draft.rs"]
mod helix_draft;

/// App-level delegators. These keep the existing `app.draft_*()` call surface
/// intact for main.rs key handlers; the actual logic lives on `DraftState`.
impl App {
    pub fn draft_clear(&mut self) {
        self.draft.clear();
    }

    /// Seed the edit dialog and open it in Normal mode (`e`), so vim users can
    /// navigate before changing anything.
    pub fn draft_set(&mut self, s: String) {
        self.draft.set(s, DialogInputMode::Normal);
    }

    /// Seed the edit dialog and open it in Insert mode (`i`), for immediate
    /// typing without the vim modal step.
    pub fn draft_set_insert(&mut self, s: String) {
        self.draft.set(s, DialogInputMode::Insert);
    }

    pub fn draft_insert_char(&mut self, c: char) {
        self.draft.insert_char(c);
    }

    pub fn draft_backspace(&mut self) {
        self.draft.backspace();
    }

    pub fn draft_delete_forward(&mut self) {
        self.draft.delete_forward();
    }

    pub fn draft_left(&mut self) {
        self.draft.move_left();
    }

    pub fn draft_right(&mut self) {
        self.draft.move_right();
    }

    pub fn draft_home(&mut self) {
        self.draft.move_home();
    }

    pub fn draft_end(&mut self) {
        self.draft.move_end();
    }

    pub fn draft_word_forward(&mut self) {
        self.draft.move_word_forward();
    }

    pub fn draft_word_backward(&mut self) {
        self.draft.move_word_backward();
    }

    pub fn draft_word_end(&mut self) {
        self.draft.move_word_end();
    }

    pub fn draft_delete_word_forward(&mut self) {
        self.draft.delete_word_forward();
    }

    pub fn draft_delete_word_backward(&mut self) {
        self.draft.delete_word_backward();
    }

    pub fn draft_kill_to_start(&mut self) {
        self.draft.kill_to_start();
    }

    pub fn draft_kill_to_end(&mut self) {
        self.draft.kill_to_end();
    }
}

pub(super) fn prev_char_boundary(s: &str, i: usize) -> usize {
    let mut j = i.saturating_sub(1);
    while j > 0 && !s.is_char_boundary(j) {
        j -= 1;
    }
    j
}

fn next_char_boundary(s: &str, i: usize) -> usize {
    let len = s.len();
    let mut j = (i + 1).min(len);
    while j < len && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::DialogInputMode;
    use crate::app::test_support::build_app;

    #[test]
    fn draft_set_opens_in_normal_mode() {
        // `e` seeds the edit dialog in Normal mode for vim-style navigation.
        let mut app = build_app("");
        app.draft_set("hello".into());
        assert_eq!(app.draft.input_mode(), DialogInputMode::Normal);
    }

    #[test]
    fn draft_set_insert_opens_in_insert_mode() {
        // `i` seeds the edit dialog in Insert mode for immediate typing.
        let mut app = build_app("");
        app.draft_set_insert("hello".into());
        assert_eq!(app.draft.input_mode(), DialogInputMode::Insert);
    }

    #[test]
    fn draft_left_right_navigates_within_text() {
        let mut app = build_app("");
        app.draft_set("hello".into());
        assert_eq!(app.draft.cursor(), 5);
        app.draft_left();
        app.draft_left();
        assert_eq!(app.draft.cursor(), 3);
        app.draft_insert_char('X');
        assert_eq!(app.draft.text(), "helXlo");
        assert_eq!(app.draft.cursor(), 4);
        app.draft_right();
        app.draft_right();
        // Already at end; further right is a no-op.
        app.draft_right();
        assert_eq!(app.draft.cursor(), app.draft.text().len());
    }

    #[test]
    fn draft_backspace_deletes_before_cursor() {
        let mut app = build_app("");
        app.draft_set("abc".into());
        app.draft_left();
        // Cursor between 'b' and 'c'; backspace removes 'b'.
        app.draft_backspace();
        assert_eq!(app.draft.text(), "ac");
        assert_eq!(app.draft.cursor(), 1);
    }

    #[test]
    fn draft_delete_forward_removes_char_at_cursor() {
        let mut app = build_app("");
        app.draft_set("abc".into());
        app.draft_home();
        app.draft_delete_forward();
        assert_eq!(app.draft.text(), "bc");
        assert_eq!(app.draft.cursor(), 0);
    }

    #[test]
    fn draft_handles_multibyte_chars_on_char_boundaries() {
        // "café" — 'é' is two bytes (U+00E9 = 0xC3 0xA9).
        let mut app = build_app("");
        app.draft_set("café".into());
        assert_eq!(app.draft.cursor(), 5);
        app.draft_left();
        // Cursor must land on a char boundary (before 'é', at byte 3).
        assert_eq!(app.draft.cursor(), 3);
        app.draft_backspace();
        assert_eq!(app.draft.text(), "caé");
    }

    #[test]
    fn draft_delete_word_backward_removes_prior_word() {
        let mut app = build_app("");
        app.draft_set("hello world".into());
        // Cursor parks at end (11) after `set`.
        app.draft_delete_word_backward();
        assert_eq!(app.draft.text(), "hello ");
        assert_eq!(app.draft.cursor(), 6);
    }

    #[test]
    fn draft_delete_word_backward_eats_trailing_space_then_word() {
        let mut app = build_app("");
        app.draft_set("hello world ".into());
        app.draft_delete_word_backward();
        assert_eq!(app.draft.text(), "hello ");
        assert_eq!(app.draft.cursor(), 6);
    }

    #[test]
    fn draft_delete_word_backward_at_start_is_noop() {
        let mut app = build_app("");
        app.draft_set("hello".into());
        app.draft_home();
        app.draft_delete_word_backward();
        assert_eq!(app.draft.text(), "hello");
        assert_eq!(app.draft.cursor(), 0);
    }

    #[test]
    fn draft_delete_word_backward_respects_multibyte_boundary() {
        // "café com" — 'é' is two bytes; the deleted word must start on a
        // char boundary so the surviving "café " stays valid UTF-8.
        let mut app = build_app("");
        app.draft_set("café com".into());
        app.draft_delete_word_backward();
        assert_eq!(app.draft.text(), "café ");
    }

    #[test]
    fn draft_kill_to_start_removes_text_before_cursor() {
        let mut app = build_app("");
        app.draft_set("hello world".into());
        app.draft.force_cursor(6); // between "hello " and "world"
        app.draft_kill_to_start();
        assert_eq!(app.draft.text(), "world");
        assert_eq!(app.draft.cursor(), 0);
    }

    #[test]
    fn draft_kill_to_start_at_start_is_noop() {
        let mut app = build_app("");
        app.draft_set("hello".into());
        app.draft_home();
        app.draft_kill_to_start();
        assert_eq!(app.draft.text(), "hello");
        assert_eq!(app.draft.cursor(), 0);
    }

    #[test]
    fn draft_kill_to_end_removes_text_after_cursor() {
        let mut app = build_app("");
        app.draft_set("hello world".into());
        app.draft.force_cursor(5); // right after "hello"
        app.draft_kill_to_end();
        assert_eq!(app.draft.text(), "hello");
        assert_eq!(app.draft.cursor(), 5);
    }

    #[test]
    fn draft_kill_to_end_at_end_is_noop() {
        let mut app = build_app("");
        app.draft_set("hello".into());
        // Cursor already at end after `set`.
        app.draft_kill_to_end();
        assert_eq!(app.draft.text(), "hello");
        assert_eq!(app.draft.cursor(), 5);
    }
}

#[cfg(test)]
mod history_tests {
    // Add inside draft.rs's existing tests module (or adapt helper to child module).
    fn helix_history_draft(text: &str) -> super::DraftState {
        let mut draft = super::DraftState::default();
        draft.set(text.into(), super::DialogInputMode::Insert);
        draft.profile = super::EditingProfile::Helix;
        draft
    }

    #[test]
    fn helix_history_is_bounded_and_redoes_each_edit() {
        let mut draft = helix_history_draft("");
        for _ in 0..55 {
            draft.insert_char('a');
        }
        assert_eq!(draft.undo.len(), 50);
        for _ in 0..50 {
            assert!(draft.undo());
        }
        assert!(!draft.undo());
        assert_eq!(draft.text(), "aaaaa");
        for _ in 0..50 {
            assert!(draft.redo());
        }
        assert!(!draft.redo());
        assert_eq!(draft.text().len(), 55);
    }

    #[test]
    fn helix_history_noop_preserves_redo_new_edit_discards_it() {
        let mut draft = helix_history_draft("a");
        draft.insert_char('b');
        assert!(draft.undo());
        draft.replace_token(0, 1, "a");
        assert!(draft.redo());
        assert_eq!(draft.text(), "ab");
        assert!(draft.undo());
        draft.replace_token(0, 1, "c");
        assert!(!draft.redo());
        assert_eq!(draft.text(), "c");
    }

    #[test]
    fn helix_history_restores_reverse_unicode_selection() {
        let mut draft = helix_history_draft("e\u{301}👩‍💻z");
        draft.input_mode = super::DialogInputMode::Normal;
        draft.cursor = super::DraftCursor::zero();
        draft.anchor = super::DraftCursor::clamped(draft.text(), "e\u{301}".len());
        let original_cursor = draft.cursor;
        let original_anchor = draft.anchor;
        let end = draft.text().len();
        draft.replace_token(0, end, "replacement");
        assert!(draft.undo());
        assert_eq!(draft.text(), "e\u{301}👩‍💻z");
        assert_eq!(draft.cursor, original_cursor);
        assert_eq!(draft.anchor, original_anchor);
        assert_eq!(draft.input_mode(), super::DialogInputMode::Normal);
        assert!(draft.redo());
        assert_eq!(draft.text(), "replacement");
    }

    #[test]
    fn helix_register_outlives_drafts_but_history_does_not() {
        let mut draft = helix_history_draft("first");
        draft.set_register("👩‍💻".into());
        draft.insert_char('!');
        assert!(draft.undo());
        assert_eq!(draft.register(), "👩‍💻");
        draft.clear();
        assert!(!draft.redo());
        assert!(!draft.undo());
        assert_eq!(draft.register(), "👩‍💻");
        draft.set("second".into(), super::DialogInputMode::Normal);
        assert!(!draft.undo());
        assert_eq!(draft.register(), "👩‍💻");
    }

    #[test]
    fn helix_compound_replacements_are_one_history_edit() {
        let mut draft = helix_history_draft("buy milk");
        draft.begin_history_group();
        draft.replace_token(0, 0, "(A) ");
        let end = draft.text().len();
        draft.replace_token(end, end, " due:2026-11-14");
        draft.end_history_group();
        assert_eq!(draft.undo.len(), 1);
        assert!(draft.undo());
        assert_eq!(draft.text(), "buy milk");
        assert!(draft.redo());
        assert_eq!(draft.text(), "(A) buy milk due:2026-11-14");
    }

    #[test]
    fn helix_navigation_and_register_changes_do_not_record_text_history() {
        let mut draft = helix_history_draft("word");
        draft.move_home();
        draft.move_right();
        draft.set_register("o".into());
        draft.set_input_mode(super::DialogInputMode::Normal);
        assert!(!draft.undo());
        assert_eq!(draft.text(), "word");
    }
}
