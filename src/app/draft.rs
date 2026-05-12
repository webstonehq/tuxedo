use super::App;
use super::draft_overlay::DraftOverlay;

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

#[derive(Debug, Default, Clone)]
pub struct DraftState {
    text: String,
    cursor: DraftCursor,
    autocomplete_selected: usize,
    autocomplete_suppressed: bool,
    /// Open metadata picker (slash menu, calendar, recurrence builder,
    /// priority chooser). At most one at a time. `None` is the default — the
    /// user is just editing text.
    overlay: Option<DraftOverlay>,
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

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = DraftCursor::zero();
        self.reset_autocomplete();
        self.overlay = None;
    }

    /// Replace the text and park the cursor at the end. Used when entering
    /// edit mode or otherwise seeding the input from existing text.
    pub fn set(&mut self, s: String) {
        self.cursor = DraftCursor::at_end(&s);
        self.text = s;
        self.reset_autocomplete();
        self.overlay = None;
    }

    pub fn insert_char(&mut self, c: char) {
        let pos = self.cursor.byte();
        self.text.insert(pos, c);
        self.cursor = DraftCursor(pos + c.len_utf8());
        self.reset_autocomplete();
    }

    pub fn backspace(&mut self) {
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        let prev = prev_char_boundary(&self.text, pos);
        self.text.drain(prev..pos);
        self.cursor = DraftCursor(prev);
        self.reset_autocomplete();
    }

    pub fn delete_forward(&mut self) {
        let pos = self.cursor.byte();
        if pos >= self.text.len() {
            return;
        }
        let next = next_char_boundary(&self.text, pos);
        self.text.drain(pos..next);
        self.reset_autocomplete();
    }

    pub fn move_left(&mut self) {
        let pos = self.cursor.byte();
        if pos == 0 {
            return;
        }
        self.cursor = DraftCursor(prev_char_boundary(&self.text, pos));
    }

    pub fn move_right(&mut self) {
        let pos = self.cursor.byte();
        if pos >= self.text.len() {
            return;
        }
        self.cursor = DraftCursor(next_char_boundary(&self.text, pos));
    }

    pub fn move_home(&mut self) {
        self.cursor = DraftCursor::zero();
    }

    pub fn move_end(&mut self) {
        self.cursor = DraftCursor::at_end(&self.text);
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
        self.text.replace_range(start..end, with);
        self.cursor = DraftCursor(start + with.len());
        self.autocomplete_selected = 0;
        self.autocomplete_suppressed = false;
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

/// App-level delegators. These keep the existing `app.draft_*()` call surface
/// intact for main.rs key handlers; the actual logic lives on `DraftState`.
impl App {
    pub fn draft_clear(&mut self) {
        self.draft.clear();
    }

    pub fn draft_set(&mut self, s: String) {
        self.draft.set(s);
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
    use crate::app::test_support::build_app;

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
}
