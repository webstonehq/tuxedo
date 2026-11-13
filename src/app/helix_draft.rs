//! Single-selection commands for the optional task-draft editor.
use super::*;
use std::ops::Range;

impl DraftState {
    pub fn profile(&self) -> EditingProfile {
        self.profile
    }

    /// Start a fresh task draft. Generic prompts continue to use `set`.
    pub fn open(&mut self, text: String, mode: DialogInputMode, profile: EditingProfile) {
        self.set(text, mode);
        self.profile = profile;
        if profile == EditingProfile::Helix && mode != DialogInputMode::Insert {
            self.cursor = DraftCursor::zero();
        }
        self.anchor = self.cursor;
        self.normalize_selection();
    }

    pub fn anchor(&self) -> usize {
        self.anchor.byte()
    }

    /// Inclusive grapheme endpoints, exposed as a safe half-open byte range.
    /// The virtual insertion position after the text selects no character.
    pub fn selected_range(&self) -> Range<usize> {
        if self.input_mode == DialogInputMode::Insert {
            return self.anchor.byte().min(self.cursor())..self.anchor.byte().max(self.cursor());
        }
        let start = self.anchor.byte().min(self.cursor.byte());
        let end = grapheme_next(&self.text, self.anchor.byte().max(self.cursor.byte()));
        start..end
    }

    pub(super) fn normalize_selection(&mut self) {
        if self.profile != EditingProfile::Helix {
            return;
        }
        self.cursor = DraftCursor(if self.input_mode == DialogInputMode::Insert {
            grapheme_ceil(&self.text, self.cursor.byte())
        } else {
            grapheme_floor(&self.text, self.cursor.byte())
        });
        self.anchor = DraftCursor(grapheme_floor(&self.text, self.anchor.byte()));
    }

    pub fn helix_collapse(&mut self) {
        self.anchor = self.cursor;
    }

    pub fn helix_toggle_extend(&mut self) {
        if self.cursor() == self.text.len() && !self.text.is_empty() {
            self.cursor = DraftCursor(grapheme_prev(&self.text, self.cursor()));
            self.anchor = self.cursor;
        }
        self.input_mode = if self.input_mode == DialogInputMode::Extend {
            DialogInputMode::Normal
        } else {
            DialogInputMode::Extend
        };
    }

    pub fn helix_move(&mut self, right: bool) {
        let next = if right {
            // A real character remains selected at the draft's right boundary.
            let next = grapheme_next(&self.text, self.cursor());
            if next == self.text.len() && !self.text.is_empty() {
                grapheme_prev(&self.text, next)
            } else {
                next
            }
        } else {
            grapheme_prev(&self.text, self.cursor())
        };
        self.helix_land(next);
    }

    pub fn helix_boundary(&mut self, end: bool) {
        let pos = if end {
            grapheme_prev(&self.text, self.text.len())
        } else {
            0
        };
        self.helix_land(pos);
    }

    fn helix_land(&mut self, pos: usize) {
        self.cursor = DraftCursor(pos);
        if self.input_mode != DialogInputMode::Extend {
            self.anchor = self.cursor;
        }
    }

    /// Helix word motions select the traversed range. Punctuation is its own
    /// word class; `w` includes following whitespace, `e` stops before it.
    pub fn helix_word(&mut self, command: char) {
        let backwards = command == 'b';
        let cursor = self.cursor();
        let range = self.selected_range();
        if (backwards && cursor == 0)
            || (!backwards && range.end == self.text.len() && self.anchor() <= cursor)
        {
            return;
        }
        let mut anchor = if backwards {
            grapheme_next(&self.text, cursor)
        } else {
            cursor
        };
        let mut head = if backwards {
            cursor
        } else {
            grapheme_next(&self.text, cursor)
        };
        let head_start = head;
        let mut previous = if backwards {
            self.text[head..].graphemes(true).next()
        } else {
            self.text[..head].graphemes(true).next_back()
        };
        loop {
            let next_pos = if backwards {
                grapheme_prev(&self.text, head)
            } else {
                head
            };
            let next = if backwards {
                if head == 0 {
                    None
                } else {
                    self.text[next_pos..head].graphemes(true).next()
                }
            } else {
                self.text[head..].graphemes(true).next()
            };
            let Some(next) = next else {
                break;
            };
            let boundary = previous.is_none_or(|prev| {
                word_class(prev) != word_class(next)
                    && if command == 'w' {
                        word_class(next) != 0
                    } else {
                        word_class(prev) != 0
                    }
            });
            if boundary {
                if head == head_start {
                    anchor = head;
                } else {
                    break;
                }
            }
            previous = Some(next);
            head = if backwards {
                next_pos
            } else {
                grapheme_next(&self.text, head)
            };
        }
        let (new_anchor, new_cursor) = if head < anchor {
            (grapheme_prev(&self.text, anchor), head)
        } else {
            (anchor, grapheme_prev(&self.text, head))
        };
        if self.input_mode != DialogInputMode::Extend {
            self.anchor = DraftCursor(new_anchor);
        }
        self.cursor = DraftCursor(new_cursor);
        self.normalize_selection();
    }

    pub fn helix_select_all(&mut self) {
        self.anchor = DraftCursor::zero();
        self.cursor = DraftCursor(grapheme_prev(&self.text, self.text.len()));
    }

    /// Insertion before/after the whole selection, irrespective of direction.
    pub fn helix_insert(&mut self, after: bool) {
        let range = self.selected_range();
        self.cursor = DraftCursor(if after { range.end } else { range.start });
        self.anchor = DraftCursor(if after { range.start } else { range.end });
        self.insert_after = after;
        self.input_mode = DialogInputMode::Insert;
    }

    pub fn helix_insert_boundary(&mut self, end: bool) {
        self.cursor = DraftCursor(if end { self.text.len() } else { 0 });
        self.anchor = self.cursor;
        self.insert_after = false;
        self.input_mode = DialogInputMode::Insert;
    }

    pub(super) fn normal_endpoints(&self) -> (usize, usize) {
        if self.profile != EditingProfile::Helix || self.input_mode != DialogInputMode::Insert {
            return (self.anchor.byte(), self.cursor());
        }
        if self.insert_after {
            (self.anchor.byte(), grapheme_prev(&self.text, self.cursor()))
        } else if self.anchor.byte() > self.cursor() {
            (grapheme_prev(&self.text, self.anchor.byte()), self.cursor())
        } else {
            (self.cursor(), self.cursor())
        }
    }

    /// Preserve the insertion selection; append restores its active block cursor.
    pub fn helix_normal(&mut self) {
        let (anchor, cursor) = self.normal_endpoints();
        self.anchor = DraftCursor(anchor);
        self.cursor = DraftCursor(cursor);
        self.input_mode = DialogInputMode::Normal;
        self.normalize_selection();
    }

    pub fn helix_yank(&mut self) {
        self.register = self.text[self.selected_range()].to_string();
        if self.input_mode == DialogInputMode::Extend {
            self.input_mode = DialogInputMode::Normal;
        }
    }

    pub fn helix_delete(&mut self, change: bool) {
        let range = self.selected_range();
        if !range.is_empty() {
            self.helix_yank();
            self.record_edit();
            self.text.replace_range(range.clone(), "");
            self.cursor = DraftCursor(range.start);
            self.anchor = self.cursor;
            self.reset_autocomplete();
        }
        self.insert_after = false;
        self.input_mode = if change {
            DialogInputMode::Insert
        } else {
            DialogInputMode::Normal
        };
        if !change && self.cursor() == self.text.len() {
            self.cursor = DraftCursor(grapheme_prev(&self.text, self.cursor()));
            self.anchor = self.cursor;
        }
        self.normalize_selection();
    }

    /// Returns false for an empty register so the controller can explain the no-op.
    pub fn helix_paste(&mut self, before: bool) -> bool {
        if self.register.is_empty() {
            return false;
        }
        let range = self.selected_range();
        let pos = if before { range.start } else { range.end };
        self.record_edit();
        self.text.insert_str(pos, &self.register);
        self.anchor = DraftCursor(grapheme_floor(&self.text, pos));
        self.cursor = DraftCursor(grapheme_prev(&self.text, pos + self.register.len()));
        self.input_mode = DialogInputMode::Normal;
        self.normalize_selection();
        self.reset_autocomplete();
        true
    }
}

pub(super) fn grapheme_floor(s: &str, byte: usize) -> usize {
    if byte >= s.len() {
        return s.len();
    }
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .take_while(|&i| i <= byte)
        .last()
        .unwrap_or(0)
}

pub(super) fn grapheme_ceil(s: &str, byte: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i >= byte)
        .unwrap_or(s.len())
}

pub(super) fn grapheme_prev(s: &str, byte: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .take_while(|&i| i < byte)
        .last()
        .unwrap_or(0)
}

pub(super) fn grapheme_next(s: &str, byte: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i > byte)
        .unwrap_or(s.len())
}

fn word_class(grapheme: &str) -> u8 {
    let c = grapheme.chars().next().unwrap_or(' ');
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str) -> DraftState {
        let mut d = DraftState::default();
        d.open(text.into(), DialogInputMode::Normal, EditingProfile::Helix);
        d
    }
    fn selected(d: &DraftState) -> &str {
        &d.text()[d.selected_range()]
    }

    // Official Helix keymap/usage and helix-core movement.rs (25.07.1):
    // https://docs.helix-editor.com/keymap.html#movement
    // w selects through trailing spaces; e selects through the word end.
    #[test]
    fn word_sequences_pin_selected_spans_and_active_cursor() {
        let mut d = draft("one two");
        d.helix_word('w');
        assert_eq!((selected(&d), d.anchor(), d.cursor()), ("one ", 0, 3));
        d.helix_word('b');
        assert_eq!((selected(&d), d.anchor(), d.cursor()), ("one ", 3, 0));
        d.helix_word('w');
        d.helix_word('w');
        assert_eq!((selected(&d), d.anchor(), d.cursor()), ("two", 4, 6));
        d.helix_word('w');
        assert_eq!(selected(&d), "two");
        let mut d = draft("one two");
        d.helix_word('e');
        assert_eq!((selected(&d), d.cursor()), ("one", 2));
        d.helix_word('e');
        assert_eq!((selected(&d), d.anchor(), d.cursor()), (" two", 3, 6));
        let mut d = draft("one, two");
        d.helix_word('w');
        assert_eq!(selected(&d), "one");
        d.helix_word('w');
        assert_eq!((selected(&d), d.cursor()), (", ", 4));
        let mut d = draft("   one two");
        d.helix_word('w');
        assert_eq!(selected(&d), "   ");
        let mut d = draft("   one two");
        d.helix_word('e');
        assert_eq!((selected(&d), d.cursor()), ("   one", 5));
    }

    #[test]
    fn grapheme_selection_extends_reverses_and_edits_without_splitting() {
        let mut d = draft("e\u{301}👩‍💻z");
        assert_eq!(selected(&d), "e\u{301}");
        d.helix_move(true);
        assert_eq!(selected(&d), "👩‍💻");
        d.helix_toggle_extend();
        d.helix_move(true);
        assert_eq!(selected(&d), "👩‍💻z");
        d.helix_move(false);
        d.helix_move(false);
        assert_eq!(selected(&d), "e\u{301}👩‍💻");
        assert!(d.anchor() > d.cursor());
        d.helix_delete(true);
        assert_eq!(d.text(), "z");
        assert_eq!(d.register(), "e\u{301}👩‍💻");
        assert_eq!(d.input_mode(), DialogInputMode::Insert);
        d.insert_char('!');
        d.helix_normal();
        assert!(d.undo());
        assert!(d.undo());
        assert_eq!(selected(&d), "e\u{301}👩‍💻");
        assert!(d.anchor() > d.cursor());
        assert!(d.redo());
        assert_eq!(d.text(), "z");
    }

    #[test]
    fn yank_paste_register_crosses_dialogs_and_empty_is_safe() {
        let mut d = draft("word");
        assert!(!d.helix_paste(false));
        d.helix_select_all();
        d.helix_yank();
        d.open("!".into(), DialogInputMode::Normal, EditingProfile::Helix);
        assert!(d.helix_paste(false));
        assert_eq!(d.text(), "!word");
        assert_eq!(selected(&d), "word");
        assert!(d.undo());
        assert_eq!(d.text(), "!");
        assert!(d.helix_paste(true));
        assert_eq!(d.text(), "word!");
        d.helix_delete(false);
        assert_eq!(d.text(), "!");
        assert_eq!(d.register(), "word");
        d.open("".into(), DialogInputMode::Normal, EditingProfile::Helix);
        d.helix_move(true);
        d.helix_word('b');
        d.helix_delete(false);
        assert_eq!(d.selected_range(), 0..0);
        assert!(d.helix_paste(false));
        assert_eq!(d.text(), "word");
    }

    #[test]
    fn absolute_insert_boundaries_and_joined_clusters_are_safe() {
        for text in ["  alpha", "", "   "] {
            let mut d = draft(text);
            d.helix_insert_boundary(false);
            d.insert_char('X');
            assert_eq!(d.text(), format!("X{text}"));
            d.helix_insert_boundary(true);
            d.insert_char('!');
            assert!(d.text().ends_with('!'));
        }
        let mut d = draft("e\u{301}z");
        d.helix_insert(false);
        d.insert_char('a');
        d.move_right();
        d.backspace();
        assert_eq!(d.text(), "az");
        let mut d = draft("\u{301}z");
        d.helix_insert(false);
        d.insert_char('e');
        assert_eq!(d.cursor(), "e\u{301}".len());
        d.backspace();
        assert_eq!(d.text(), "z");
    }

    #[test]
    fn insertion_preserves_verified_forward_and_reverse_word_selections() {
        for reverse in [false, true] {
            for after in [false, true] {
                let mut d = draft("one two");
                d.helix_word('w');
                if reverse {
                    std::mem::swap(&mut d.anchor, &mut d.cursor);
                }
                d.helix_insert(after);
                d.insert_char('!');
                d.helix_normal();
                if after {
                    assert_eq!(
                        (d.text(), selected(&d), d.anchor(), d.cursor()),
                        ("one !two", "one !", 0, 4)
                    );
                } else {
                    assert_eq!(
                        (d.text(), selected(&d), d.anchor(), d.cursor()),
                        ("!one two", "one ", 4, 1)
                    );
                }
            }
        }
    }

    #[test]
    fn fresh_insert_draft_does_not_inherit_append_cursor_restoration() {
        let mut d = draft("abc");
        d.helix_insert(true);
        d.open("xy".into(), DialogInputMode::Insert, EditingProfile::Helix);
        d.insert_char('!');
        d.helix_normal();
        assert_eq!((d.text(), d.anchor(), d.cursor()), ("xy!", 3, 3));
    }

    #[test]
    fn insertion_uses_selection_edges_in_either_direction() {
        for reverse in [false, true] {
            let mut d = draft("abc");
            d.helix_select_all();
            if reverse {
                std::mem::swap(&mut d.anchor, &mut d.cursor);
            }
            d.helix_insert(false);
            assert_eq!(d.cursor(), 0);
            d.helix_normal();
            d.helix_select_all();
            if reverse {
                std::mem::swap(&mut d.anchor, &mut d.cursor);
            }
            d.helix_insert(true);
            assert_eq!(d.cursor(), 3);
            d.insert_char('!');
            d.helix_normal();
            assert_eq!(selected(&d), "abc!");
        }
    }
}
