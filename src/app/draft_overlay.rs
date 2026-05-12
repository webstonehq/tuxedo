//! Metadata pickers that float over the add-task dialog. Triggered by `/` in
//! Insert mode, each overlay collects a specific piece of metadata (due date,
//! recurrence, priority, threshold) or routes back to the existing
//! autocomplete popup for projects/contexts, then writes the result into the
//! draft buffer using a small set of `apply_*` helpers.
//!
//! State lives on `DraftState::overlay`. The flow is:
//!
//! 1. User types `/` at BOL or after whitespace → `maybe_open_slash_menu`
//!    installs `DraftOverlay::SlashMenu` with `anchor` = position of the `/`.
//! 2. Filter text is `draft[anchor+1..cursor]`. Up/Down navigate, Enter accepts.
//! 3. On accept, `slash_accept` drains `draft[anchor..cursor]` and either:
//!    - opens a second overlay (Calendar, RecurrenceBuilder, PriorityChooser),
//!    - or inserts a sigil (`+`/`@`) and re-arms the autocomplete popup.
//! 4. The second overlay produces a value and `apply_*` splices it into the
//!    buffer — replacing the existing token of the same kind if present,
//!    otherwise appending.

use chrono::{Days, Months, NaiveDate};
use std::ops::Range;

use super::App;
use crate::recurrence::{self, RecSpec, RecUnit};
use crate::threshold::{self, ThresholdSpec};

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashKind {
    Due,
    Recurrence,
    Threshold,
    Priority,
    Project,
    Context,
}

#[derive(Debug, Clone, Copy)]
pub struct SlashEntry {
    pub label: &'static str,
    pub description: &'static str,
    pub cmd: &'static str,
    pub kind: SlashKind,
}

/// Order matches the mockup. The slash menu renders entries in this order
/// when the filter is empty; `slash_matches` re-sorts only via the filter.
pub const SLASH_ENTRIES: &[SlashEntry] = &[
    SlashEntry {
        label: "Due date",
        description: "when this needs doing",
        cmd: "/due",
        kind: SlashKind::Due,
    },
    SlashEntry {
        label: "Recurrence",
        description: "repeat after completing",
        cmd: "/rec",
        kind: SlashKind::Recurrence,
    },
    SlashEntry {
        label: "Threshold",
        description: "hide until this date",
        cmd: "/t",
        kind: SlashKind::Threshold,
    },
    SlashEntry {
        label: "Priority",
        description: "A · B · C",
        cmd: "/prio",
        kind: SlashKind::Priority,
    },
    SlashEntry {
        label: "+ Project",
        description: "attach to a project",
        cmd: "/proj",
        kind: SlashKind::Project,
    },
    SlashEntry {
        label: "@ Context",
        description: "tool, place or person",
        cmd: "/ctx",
        kind: SlashKind::Context,
    },
];

// ---------------------------------------------------------------------------
// Overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    /// Byte offset of the `/` in the draft. Filter text is `draft[anchor+1..cursor]`.
    pub anchor: usize,
    /// Index into the *filtered* entry list. Reset on every filter change.
    pub selected: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarTarget {
    Due,
    Threshold,
}

#[derive(Debug, Clone)]
pub struct CalendarState {
    pub target: CalendarTarget,
    pub focused: NaiveDate,
    /// Set when the picker was auto-triggered by typing `due:` / `t:` —
    /// records the byte offset of the leading key letter so accept can strip
    /// the empty literal before writing the chosen value. `None` for slash-
    /// menu opens; accept then uses `apply_kv` directly.
    pub anchor: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderField {
    Interval,
    Unit,
    Mode,
}

/// Single source of truth for the unit pill order. The renderer iterates this
/// same slice so a `rec:3b` (business-day) spec opened in the builder still
/// shows up as a selectable pill instead of being silently coerced to Week on
/// the next adjust.
pub const REC_UNIT_ORDER: &[RecUnit] = &[
    RecUnit::Day,
    RecUnit::BusinessDay,
    RecUnit::Week,
    RecUnit::Month,
    RecUnit::Year,
];

#[derive(Debug, Clone)]
pub struct RecurrenceBuilderState {
    pub interval: u32,
    pub unit: RecUnit,
    /// `true` writes `rec:+Nu` (strict — anchor on previous due), `false`
    /// writes `rec:Nu` (after-complete).
    pub strict: bool,
    pub field: BuilderField,
    /// See `CalendarState::anchor`.
    pub anchor: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct PriorityChooserState {
    /// 0=A, 1=B, 2=C, 3=clear.
    pub selected: u8,
}

#[derive(Debug, Clone)]
pub enum DraftOverlay {
    SlashMenu(SlashMenuState),
    Calendar(CalendarState),
    RecurrenceBuilder(RecurrenceBuilderState),
    PriorityChooser(PriorityChooserState),
}

/// Discriminator-only view of `DraftOverlay`, suitable for key-dispatch matches
/// that need to free the immutable borrow before calling `&mut App` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    SlashMenu,
    Calendar,
    RecurrenceBuilder,
    PriorityChooser,
}

impl DraftOverlay {
    pub fn kind(&self) -> OverlayKind {
        match self {
            DraftOverlay::SlashMenu(_) => OverlayKind::SlashMenu,
            DraftOverlay::Calendar(_) => OverlayKind::Calendar,
            DraftOverlay::RecurrenceBuilder(_) => OverlayKind::RecurrenceBuilder,
            DraftOverlay::PriorityChooser(_) => OverlayKind::PriorityChooser,
        }
    }
}

// ---------------------------------------------------------------------------
// Slash menu — open / filter / cancel / accept
// ---------------------------------------------------------------------------

impl App {
    /// After a `/` was just inserted into the draft, decide whether to open
    /// the slash menu. Triggers only at BOL or right after whitespace, so a
    /// `/` mid-URL (e.g. `https://example.com`) does not pop the menu.
    /// No-op when an overlay is already open.
    pub fn maybe_open_slash_menu(&mut self) {
        if self.draft.overlay().is_some() {
            return;
        }
        let text = self.draft.text();
        let cursor = self.draft.cursor();
        if cursor == 0 {
            return;
        }
        let slash_pos = cursor - 1;
        if text.as_bytes().get(slash_pos) != Some(&b'/') {
            return;
        }
        if slash_pos > 0 {
            let prev = super::draft::prev_char_boundary(text, slash_pos);
            let prev_char = text[prev..slash_pos].chars().next();
            if !matches!(prev_char, Some(c) if c.is_whitespace()) {
                return;
            }
        }
        self.draft
            .set_overlay(Some(DraftOverlay::SlashMenu(SlashMenuState {
                anchor: slash_pos,
                selected: 0,
            })));
    }

    /// After a `:` was just inserted, decide whether to auto-open a metadata
    /// picker. Mirrors `maybe_open_slash_menu`: triggers only when the chars
    /// immediately before the colon are one of the recognised keys (`due`,
    /// `t`, `rec`) and the char before *that* is whitespace or BOL. So
    /// `Recipe:` and `Mydue:` don't fire; ` due:` and `rec:` at BOL do.
    pub fn maybe_open_kv_overlay(&mut self) {
        if self.draft.overlay().is_some() {
            return;
        }
        let cursor = self.draft.cursor();
        if cursor == 0 {
            return;
        }
        let text = self.draft.text();
        if text.as_bytes().get(cursor - 1) != Some(&b':') {
            return;
        }
        let colon_pos = cursor - 1;
        // Try longest keys first so `rec` doesn't shadow a hypothetical `re`.
        for (key, kind) in [("rec", KvKind::Rec), ("due", KvKind::Due), ("t", KvKind::T)] {
            if let Some(key_start) = match_key_before(text, colon_pos, key) {
                match kind {
                    KvKind::Due => {
                        self.open_calendar_anchored(CalendarTarget::Due, Some(key_start));
                    }
                    KvKind::T => {
                        self.open_calendar_anchored(CalendarTarget::Threshold, Some(key_start));
                    }
                    KvKind::Rec => {
                        self.open_recurrence_builder_anchored(Some(key_start));
                    }
                }
                return;
            }
        }
    }

    /// Validate that the slash menu's anchor is still consistent with the
    /// current buffer. If the `/` was deleted or the cursor moved before it,
    /// drop the menu. Called after every text-edit key in handle_insert so
    /// the popup goes away when the user backspaces over its trigger.
    pub fn slash_menu_revalidate(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let text = self.draft.text();
        let still_slash = text.as_bytes().get(anchor) == Some(&b'/');
        let cursor_ok = self.draft.cursor() > anchor;
        if !still_slash || !cursor_ok {
            self.draft.set_overlay(None);
        }
    }

    /// Filter text typed after the `/`. Empty when the menu just opened.
    pub fn slash_filter(&self) -> &str {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return "";
        };
        let cursor = self.draft.cursor().min(self.draft.text().len());
        let start = (state.anchor + 1).min(cursor);
        &self.draft.text()[start..cursor]
    }

    /// Entries that match the current filter, in display order. Case-insensitive
    /// substring match against label and command. When the filter is empty,
    /// every entry passes.
    pub fn slash_matches(&self) -> Vec<&'static SlashEntry> {
        let filter = self.slash_filter().to_lowercase();
        SLASH_ENTRIES
            .iter()
            .filter(|e| {
                if filter.is_empty() {
                    return true;
                }
                e.label.to_lowercase().contains(&filter) || e.cmd.contains(&filter)
            })
            .collect()
    }

    /// Index of the currently-highlighted match, clamped to the filtered list.
    pub fn slash_selected(&self) -> usize {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return 0;
        };
        let n = self.slash_matches().len();
        if n == 0 { 0 } else { state.selected.min(n - 1) }
    }

    pub fn slash_step(&mut self, forward: bool) {
        let n = self.slash_matches().len();
        if n == 0 {
            return;
        }
        if let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay_mut() {
            let cur = state.selected.min(n - 1);
            state.selected = if forward {
                (cur + 1) % n
            } else {
                (cur + n - 1) % n
            };
        }
    }

    /// Cancel the slash menu and remove the `/filter` literal from the buffer.
    pub fn slash_cancel(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let cursor = self.draft.cursor();
        let end = cursor.max(anchor);
        self.draft.set_overlay(None);
        if anchor <= self.draft.text().len() && end <= self.draft.text().len() {
            self.draft.replace_token(anchor, end, "");
        }
    }

    /// Accept the highlighted entry. Removes the `/filter` literal and then
    /// either opens a second overlay (Due/Rec/Threshold/Priority) or inserts
    /// a sigil so the existing autocomplete popup takes over (Project/Context).
    pub fn slash_accept(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let cursor = self.draft.cursor();
        let matches = self.slash_matches();
        if matches.is_empty() {
            return;
        }
        let idx = state.selected.min(matches.len() - 1);
        let kind = matches[idx].kind;
        // Drop the `/filter` literal first so subsequent inserts land at the
        // anchor without colliding with leftover trigger chars.
        let end = cursor.max(anchor);
        self.draft.set_overlay(None);
        self.draft.replace_token(anchor, end, "");
        // Dispatch.
        match kind {
            SlashKind::Due => self.open_calendar(CalendarTarget::Due),
            SlashKind::Threshold => self.open_calendar(CalendarTarget::Threshold),
            SlashKind::Recurrence => self.open_recurrence_builder(),
            SlashKind::Priority => self.open_priority_chooser(),
            SlashKind::Project => self.insert_sigil_at_cursor('+'),
            SlashKind::Context => self.insert_sigil_at_cursor('@'),
        }
    }

    fn insert_sigil_at_cursor(&mut self, sigil: char) {
        let pos = self.draft.cursor();
        let needs_space = pos > 0
            && self
                .draft
                .text()
                .as_bytes()
                .get(pos - 1)
                .copied()
                .is_some_and(|b| !b.is_ascii_whitespace());
        let insert = if needs_space {
            format!(" {sigil}")
        } else {
            sigil.to_string()
        };
        self.draft.replace_token(pos, pos, &insert);
    }
}

// ---------------------------------------------------------------------------
// Calendar
// ---------------------------------------------------------------------------

impl App {
    pub fn open_calendar(&mut self, target: CalendarTarget) {
        self.open_calendar_anchored(target, None);
    }

    /// Open the calendar with an optional trigger anchor. The anchor is set
    /// only when the user auto-triggered the picker by typing `due:` / `t:`
    /// directly — accept then strips that literal so `apply_kv` doesn't leave
    /// a duplicate empty token behind.
    pub fn open_calendar_anchored(&mut self, target: CalendarTarget, anchor: Option<usize>) {
        let existing = match target {
            CalendarTarget::Due => find_kv_value(self.draft.text(), "due"),
            CalendarTarget::Threshold => find_kv_value(self.draft.text(), "t"),
        };
        let focused = existing
            .and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
            .or_else(|| NaiveDate::parse_from_str(&self.today, "%Y-%m-%d").ok())
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date"));
        self.draft
            .set_overlay(Some(DraftOverlay::Calendar(CalendarState {
                target,
                focused,
                anchor,
            })));
    }

    pub fn calendar_state(&self) -> Option<&CalendarState> {
        match self.draft.overlay()? {
            DraftOverlay::Calendar(s) => Some(s),
            _ => None,
        }
    }

    pub fn calendar_move(&mut self, dx: i32, dy: i32) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let total_days = dx + dy * 7;
        let next = if total_days >= 0 {
            s.focused.checked_add_days(Days::new(total_days as u64))
        } else {
            s.focused
                .checked_sub_days(Days::new(total_days.unsigned_abs() as u64))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    pub fn calendar_set_relative(&mut self, days: i64) {
        let Some(today) = NaiveDate::parse_from_str(&self.today, "%Y-%m-%d").ok() else {
            return;
        };
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let next = if days >= 0 {
            today.checked_add_days(Days::new(days as u64))
        } else {
            today.checked_sub_days(Days::new(days.unsigned_abs()))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    pub fn calendar_add_months(&mut self, n: i32) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let next = if n >= 0 {
            s.focused.checked_add_months(Months::new(n as u32))
        } else {
            s.focused.checked_sub_months(Months::new(n.unsigned_abs()))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    /// Save the focused date into the draft and close the calendar.
    pub fn calendar_accept(&mut self) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay() else {
            return;
        };
        let target = s.target;
        let anchor = s.anchor;
        let date_str = s.focused.format("%Y-%m-%d").to_string();
        self.draft.set_overlay(None);
        let key = match target {
            CalendarTarget::Due => "due",
            CalendarTarget::Threshold => "t",
        };
        if let Some(a) = anchor {
            // Auto-trigger: remove the just-typed `KEY:` literal (and its
            // leading space) so apply_kv finds/replaces the canonical token
            // — either updating an existing one elsewhere or appending fresh.
            // Without this strip, retriggering on a line that already has a
            // `due:DATE` would leave two `due:` tokens.
            strip_trigger_literal(self, key, a);
        }
        self.apply_kv(key, Some(&date_str));
    }

    /// Clear the current value from the draft and close the calendar.
    pub fn calendar_clear(&mut self) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay() else {
            return;
        };
        let target = s.target;
        let anchor = s.anchor;
        self.draft.set_overlay(None);
        let key = match target {
            CalendarTarget::Due => "due",
            CalendarTarget::Threshold => "t",
        };
        if let Some(a) = anchor {
            strip_trigger_literal(self, key, a);
        }
        self.apply_kv(key, None);
    }

    /// Esc on the calendar. Leaves the buffer untouched — matches the `@`/`+`
    /// autocomplete model where Esc dismisses the popup but keeps any literal
    /// the user has typed so they can finish it by hand.
    pub fn calendar_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}

/// Remove the `KEY:` literal at `anchor` and its single leading space, if
/// any. Used by accept/clear paths when the picker was auto-triggered so the
/// user's typed prefix doesn't become a duplicate token after we write the
/// canonical one. No-op if the buffer no longer matches `KEY:` at `anchor`
/// (e.g. the user edited the line — defensive).
fn strip_trigger_literal(app: &mut App, key: &str, anchor: usize) {
    let key_with_colon = format!("{key}:");
    let text = app.draft.text();
    let end = anchor + key_with_colon.len();
    if end > text.len() || text[anchor..end] != key_with_colon {
        return;
    }
    let strip_start = if anchor > 0
        && text
            .as_bytes()
            .get(anchor - 1)
            .copied()
            .is_some_and(|b| b == b' ' || b == b'\t')
    {
        anchor - 1
    } else {
        anchor
    };
    app.draft.replace_token(strip_start, end, "");
}

// ---------------------------------------------------------------------------
// Recurrence builder
// ---------------------------------------------------------------------------

impl App {
    pub fn open_recurrence_builder(&mut self) {
        self.open_recurrence_builder_anchored(None);
    }

    pub fn open_recurrence_builder_anchored(&mut self, anchor: Option<usize>) {
        let existing = find_kv_value(self.draft.text(), "rec");
        let parsed = existing.as_deref().and_then(recurrence::parse_rec_spec);
        let mut state = match parsed {
            Some(spec) => RecurrenceBuilderState {
                interval: spec.n.max(1),
                unit: spec.unit,
                strict: spec.strict,
                field: BuilderField::Interval,
                anchor: None,
            },
            None => RecurrenceBuilderState {
                interval: 1,
                unit: RecUnit::Week,
                strict: false,
                field: BuilderField::Interval,
                anchor: None,
            },
        };
        state.anchor = anchor;
        self.draft
            .set_overlay(Some(DraftOverlay::RecurrenceBuilder(state)));
    }

    pub fn recurrence_state(&self) -> Option<&RecurrenceBuilderState> {
        match self.draft.overlay()? {
            DraftOverlay::RecurrenceBuilder(s) => Some(s),
            _ => None,
        }
    }

    pub fn recurrence_focus(&mut self, delta: i32) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay_mut() else {
            return;
        };
        let order = [
            BuilderField::Interval,
            BuilderField::Unit,
            BuilderField::Mode,
        ];
        let cur = order.iter().position(|f| *f == s.field).unwrap_or(0) as i32;
        let next = ((cur + delta).rem_euclid(order.len() as i32)) as usize;
        s.field = order[next];
    }

    /// Adjust the currently-focused field. `+1`/`-1` increments interval or
    /// cycles unit / mode. Interval clamps at 1 (no zero intervals — the
    /// recurrence parser rejects them).
    pub fn recurrence_adjust(&mut self, delta: i32) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay_mut() else {
            return;
        };
        match s.field {
            BuilderField::Interval => {
                let cur = s.interval as i32;
                s.interval = (cur + delta).max(1) as u32;
            }
            BuilderField::Unit => {
                let order = REC_UNIT_ORDER;
                let cur = order.iter().position(|u| *u == s.unit).unwrap_or(1) as i32;
                let n = order.len() as i32;
                let next = ((cur + delta).rem_euclid(n)) as usize;
                s.unit = order[next];
            }
            BuilderField::Mode => {
                s.strict = !s.strict;
            }
        }
    }

    pub fn recurrence_accept(&mut self) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay() else {
            return;
        };
        let value = format_rec_value(s);
        let anchor = s.anchor;
        self.draft.set_overlay(None);
        if let Some(a) = anchor {
            strip_trigger_literal(self, "rec", a);
        }
        self.apply_kv("rec", Some(&value));
    }

    pub fn recurrence_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}

// ---------------------------------------------------------------------------
// Priority chooser
// ---------------------------------------------------------------------------

impl App {
    pub fn open_priority_chooser(&mut self) {
        let existing = find_priority(self.draft.text());
        let selected = match existing {
            Some('A') => 0,
            Some('B') => 1,
            Some('C') => 2,
            _ => 0,
        };
        self.draft
            .set_overlay(Some(DraftOverlay::PriorityChooser(PriorityChooserState {
                selected,
            })));
    }

    pub fn priority_state(&self) -> Option<&PriorityChooserState> {
        match self.draft.overlay()? {
            DraftOverlay::PriorityChooser(s) => Some(s),
            _ => None,
        }
    }

    pub fn priority_step(&mut self, forward: bool) {
        let Some(DraftOverlay::PriorityChooser(s)) = self.draft.overlay_mut() else {
            return;
        };
        let n: i32 = 4; // A, B, C, clear
        let cur = s.selected as i32;
        let next = (cur + if forward { 1 } else { -1 }).rem_euclid(n);
        s.selected = next as u8;
    }

    pub fn priority_accept(&mut self) {
        let Some(DraftOverlay::PriorityChooser(s)) = self.draft.overlay() else {
            return;
        };
        let pri = match s.selected {
            0 => Some('A'),
            1 => Some('B'),
            2 => Some('C'),
            _ => None,
        };
        self.draft.set_overlay(None);
        self.apply_priority(pri);
    }

    pub fn priority_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}

// ---------------------------------------------------------------------------
// apply_* — write metadata back into the draft buffer
// ---------------------------------------------------------------------------

impl App {
    /// Insert/replace/remove a `key:value` token in the draft. `value = None`
    /// removes any existing token with that key. Existing tokens are replaced
    /// in place to preserve the user's body-text layout; new tokens append
    /// with a leading space.
    pub(super) fn apply_kv(&mut self, key: &str, value: Option<&str>) {
        let existing = find_kv_token_range(self.draft.text(), key);
        match (existing, value) {
            (Some(range), Some(v)) => {
                let replacement = format!("{key}:{v}");
                self.draft
                    .replace_token(range.start, range.end, &replacement);
            }
            (Some(range), None) => {
                // Delete the token. Drop one leading space if present so we
                // don't leave "  " mid-line.
                let leading_space = range.start > 0
                    && self
                        .draft
                        .text()
                        .as_bytes()
                        .get(range.start - 1)
                        .copied()
                        .is_some_and(|b| b == b' ' || b == b'\t');
                let start = if leading_space {
                    range.start - 1
                } else {
                    range.start
                };
                self.draft.replace_token(start, range.end, "");
            }
            (None, Some(v)) => {
                let (cur_len, needs_space) = {
                    let text = self.draft.text();
                    let needs_space = !text.is_empty()
                        && !text
                            .as_bytes()
                            .last()
                            .copied()
                            .is_some_and(|b| b == b' ' || b == b'\t');
                    (text.len(), needs_space)
                };
                let insert = if needs_space {
                    format!(" {key}:{v}")
                } else {
                    format!("{key}:{v}")
                };
                self.draft.replace_token(cur_len, cur_len, &insert);
            }
            (None, None) => {}
        }
    }

    /// Replace, prepend, or remove the leading `(X) ` priority token.
    pub(super) fn apply_priority(&mut self, priority: Option<char>) {
        let has_priority = {
            let bytes = self.draft.text().as_bytes();
            bytes.len() >= 4
                && bytes[0] == b'('
                && bytes[1].is_ascii_uppercase()
                && bytes[2] == b')'
                && bytes[3] == b' '
        };
        match (has_priority, priority) {
            (true, Some(p)) => {
                // Replace just the letter inside the parens — keeps cursor
                // semantics simple and avoids touching the trailing space.
                self.draft.replace_token(1, 2, &p.to_string());
            }
            (true, None) => {
                self.draft.replace_token(0, 4, "");
            }
            (false, Some(p)) => {
                let prefix = format!("({p}) ");
                self.draft.replace_token(0, 0, &prefix);
            }
            (false, None) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum KvKind {
    Due,
    T,
    Rec,
}

/// True when `text[colon_pos - key.len() .. colon_pos] == key` and the char
/// before that range is whitespace or BOL. Returns the byte offset where the
/// key starts so the caller can record it as the trigger anchor.
fn match_key_before(text: &str, colon_pos: usize, key: &str) -> Option<usize> {
    if colon_pos < key.len() {
        return None;
    }
    let key_start = colon_pos - key.len();
    // `key_start` is a byte arithmetic — if the char immediately before the
    // colon is multi-byte, it may land mid-codepoint and `text.get` returns
    // None instead of panicking on a direct slice.
    if text.get(key_start..colon_pos) != Some(key) {
        return None;
    }
    if key_start == 0 {
        return Some(key_start);
    }
    let prev = super::draft::prev_char_boundary(text, key_start);
    let prev_char = text.get(prev..key_start).and_then(|s| s.chars().next())?;
    if prev_char.is_whitespace() {
        Some(key_start)
    } else {
        None
    }
}

/// Byte range of the first whitespace-delimited token with the form
/// `key:<non-empty value>`. Returns `None` when no such token exists.
fn find_kv_token_range(text: &str, key: &str) -> Option<Range<usize>> {
    let needle = format!("{key}:");
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let tok = &text[start..i];
        if let Some(rest) = tok.strip_prefix(&needle)
            && !rest.is_empty()
        {
            return Some(start..i);
        }
    }
    None
}

/// Value of the first `key:value` token, if any. Wraps `find_kv_token_range`
/// for the common "look up the existing value" call sites.
fn find_kv_value(text: &str, key: &str) -> Option<String> {
    let range = find_kv_token_range(text, key)?;
    let tok = &text[range];
    tok.split_once(':').map(|(_, v)| v.to_string())
}

/// Leading priority letter, if the line starts with `(X) `.
fn find_priority(text: &str) -> Option<char> {
    let bytes = text.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && bytes[3] == b' '
    {
        Some(bytes[1] as char)
    } else {
        None
    }
}

/// Format a builder state as the value portion of a `rec:` token (e.g. `1w`,
/// `+2m`). Used by `recurrence_accept` and by the live preview line.
pub fn format_rec_value(state: &RecurrenceBuilderState) -> String {
    let prefix = if state.strict { "+" } else { "" };
    let unit = match state.unit {
        RecUnit::Day => "d",
        RecUnit::BusinessDay => "b",
        RecUnit::Week => "w",
        RecUnit::Month => "m",
        RecUnit::Year => "y",
    };
    format!("{prefix}{}{unit}", state.interval)
}

/// "Next occurrence" for the recurrence-builder preview line. Computed via
/// the same `recurrence::advance` used by the completion-spawn path, anchored
/// on the app's `today` so the value is identical to what the user would see
/// after marking a task done now.
pub fn recurrence_next_preview(state: &RecurrenceBuilderState, today: &str) -> Option<NaiveDate> {
    let spec = RecSpec {
        strict: state.strict,
        n: state.interval,
        unit: state.unit,
    };
    let date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    recurrence::advance(date, &spec)
}

/// Resolved threshold date for the "→ shows on {date}" hint in the calendar.
/// Mirrors `threshold::resolve` against `(due, today)` so the hint matches the
/// actual visibility filter. Unused today; reserved for future hint copy.
#[allow(dead_code)]
pub fn threshold_preview(value: &str, due: Option<&str>, today: &str) -> Option<NaiveDate> {
    let spec = threshold::parse_threshold(value)?;
    if let ThresholdSpec::Absolute(d) = spec {
        return Some(d);
    }
    threshold::resolve(&spec, due, Some(today))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn apply_kv_appends_when_absent() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.apply_kv("due", Some("2026-05-12"));
        assert_eq!(app.draft.text(), "Buy milk due:2026-05-12");
    }

    #[test]
    fn apply_kv_replaces_when_present() {
        let mut app = build_app("");
        app.draft_set("Buy milk due:2026-05-01 +groceries".into());
        app.apply_kv("due", Some("2026-05-12"));
        assert_eq!(app.draft.text(), "Buy milk due:2026-05-12 +groceries");
    }

    #[test]
    fn apply_kv_clear_removes_token_and_space() {
        let mut app = build_app("");
        app.draft_set("Buy milk due:2026-05-01 +groceries".into());
        app.apply_kv("due", None);
        // Leading space before `due:` is also dropped so we don't leave "  ".
        assert_eq!(app.draft.text(), "Buy milk +groceries");
    }

    #[test]
    fn apply_kv_clear_when_absent_is_noop() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.apply_kv("due", None);
        assert_eq!(app.draft.text(), "Buy milk");
    }

    #[test]
    fn apply_kv_appends_to_empty_buffer_without_leading_space() {
        let mut app = build_app("");
        app.apply_kv("due", Some("2026-05-12"));
        assert_eq!(app.draft.text(), "due:2026-05-12");
    }

    #[test]
    fn apply_priority_prepends_when_absent() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.apply_priority(Some('A'));
        assert_eq!(app.draft.text(), "(A) Buy milk");
    }

    #[test]
    fn apply_priority_replaces_when_present() {
        let mut app = build_app("");
        app.draft_set("(A) Buy milk".into());
        app.apply_priority(Some('B'));
        assert_eq!(app.draft.text(), "(B) Buy milk");
    }

    #[test]
    fn apply_priority_clears_when_present() {
        let mut app = build_app("");
        app.draft_set("(A) Buy milk".into());
        app.apply_priority(None);
        assert_eq!(app.draft.text(), "Buy milk");
    }

    #[test]
    fn apply_priority_clear_when_absent_is_noop() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.apply_priority(None);
        assert_eq!(app.draft.text(), "Buy milk");
    }

    #[test]
    fn find_kv_token_range_skips_non_matching_prefix() {
        // `rec:1w` must not be picked up when looking for `re` — token has to
        // start with the exact `key:` prefix.
        assert!(find_kv_token_range("Hi rec:1w", "re").is_none());
        // But the exact key matches.
        assert!(find_kv_token_range("Hi rec:1w", "rec").is_some());
    }

    #[test]
    fn find_kv_token_range_picks_first_only() {
        // Mirrors `todo::find_kv` — first wins. The replacement target is
        // therefore the first token.
        let r = find_kv_token_range("a due:2026-01-01 b due:2026-02-02", "due").unwrap();
        assert_eq!(&"a due:2026-01-01 b due:2026-02-02"[r], "due:2026-01-01");
    }

    #[test]
    fn format_rec_value_emits_strict_prefix() {
        let s = RecurrenceBuilderState {
            interval: 2,
            unit: RecUnit::Month,
            strict: true,
            field: BuilderField::Interval,
            anchor: None,
        };
        assert_eq!(format_rec_value(&s), "+2m");
        let s2 = RecurrenceBuilderState {
            interval: 1,
            unit: RecUnit::Week,
            strict: false,
            field: BuilderField::Interval,
            anchor: None,
        };
        assert_eq!(format_rec_value(&s2), "1w");
    }

    #[test]
    fn slash_menu_opens_at_bol() {
        let mut app = build_app("");
        app.mode = crate::app::Mode::Insert;
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        assert!(matches!(
            app.draft.overlay(),
            Some(DraftOverlay::SlashMenu(_))
        ));
    }

    #[test]
    fn slash_menu_opens_after_whitespace() {
        let mut app = build_app("");
        app.draft_set("Hi ".into());
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        assert!(matches!(
            app.draft.overlay(),
            Some(DraftOverlay::SlashMenu(_))
        ));
    }

    #[test]
    fn slash_menu_does_not_open_mid_word() {
        // `https:/...` — the `/` follows `:` which isn't whitespace.
        let mut app = build_app("");
        app.draft_set("https:".into());
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        assert!(app.draft.overlay().is_none());
    }

    #[test]
    fn slash_menu_filter_narrows_entries() {
        let mut app = build_app("");
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        // Typing `du` narrows to "Due date".
        app.draft_insert_char('d');
        app.draft_insert_char('u');
        let matches = app.slash_matches();
        assert!(matches.iter().any(|e| e.kind == SlashKind::Due));
        assert!(matches.iter().all(|e| e.kind == SlashKind::Due));
    }

    #[test]
    fn slash_menu_revalidates_when_slash_deleted() {
        let mut app = build_app("");
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        assert!(app.draft.overlay().is_some());
        app.draft_backspace();
        app.slash_menu_revalidate();
        assert!(app.draft.overlay().is_none());
    }

    #[test]
    fn slash_cancel_removes_trigger_text() {
        let mut app = build_app("");
        app.draft_set("Hi ".into());
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        app.draft_insert_char('d');
        app.draft_insert_char('u');
        app.slash_cancel();
        assert_eq!(app.draft.text(), "Hi ");
        assert!(app.draft.overlay().is_none());
    }

    #[test]
    fn slash_accept_due_opens_calendar() {
        let mut app = build_app("");
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        app.slash_accept();
        // Default selection is the first entry — Due date. Calendar should open.
        assert!(matches!(
            app.draft.overlay(),
            Some(DraftOverlay::Calendar(_))
        ));
        // The `/` literal is gone.
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn slash_accept_proj_inserts_sigil_and_no_overlay() {
        let mut app = build_app("");
        app.draft_set("Buy milk ".into());
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        // Filter to /proj.
        app.draft_insert_char('p');
        app.draft_insert_char('r');
        app.draft_insert_char('o');
        app.slash_accept();
        assert!(app.draft.overlay().is_none());
        assert!(app.draft.text().ends_with('+'));
    }

    #[test]
    fn calendar_accept_writes_due_token() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.open_calendar(CalendarTarget::Due);
        // Default focus = today (2026-05-06 from test_support).
        app.calendar_accept();
        assert!(app.draft.overlay().is_none());
        assert_eq!(app.draft.text(), "Buy milk due:2026-05-06");
    }

    #[test]
    fn calendar_clear_removes_existing_due() {
        let mut app = build_app("");
        app.draft_set("Buy milk due:2026-05-12".into());
        app.open_calendar(CalendarTarget::Due);
        app.calendar_clear();
        assert_eq!(app.draft.text(), "Buy milk");
    }

    #[test]
    fn calendar_reopens_focused_on_existing_value() {
        let mut app = build_app("");
        app.draft_set("Buy milk due:2026-07-04".into());
        app.open_calendar(CalendarTarget::Due);
        let s = app.calendar_state().unwrap();
        assert_eq!(s.focused, NaiveDate::from_ymd_opt(2026, 7, 4).unwrap());
    }

    #[test]
    fn recurrence_accept_writes_rec_token() {
        let mut app = build_app("");
        app.draft_set("Water plants".into());
        app.open_recurrence_builder();
        // Default = 1, Week, after-complete → "rec:1w".
        app.recurrence_accept();
        assert_eq!(app.draft.text(), "Water plants rec:1w");
    }

    #[test]
    fn recurrence_adjust_interval_clamps_at_one() {
        let mut app = build_app("");
        app.open_recurrence_builder();
        app.recurrence_adjust(-10);
        let s = app.recurrence_state().unwrap();
        assert_eq!(s.interval, 1);
    }

    #[test]
    fn recurrence_strict_mode_emits_plus_prefix() {
        let mut app = build_app("");
        app.draft_set("Pay rent".into());
        app.open_recurrence_builder();
        app.recurrence_focus(2); // Interval -> Mode (skipping Unit)
        app.recurrence_adjust(1); // toggle strict
        let s = app.recurrence_state().unwrap();
        assert!(s.strict);
        app.recurrence_accept();
        assert_eq!(app.draft.text(), "Pay rent rec:+1w");
    }

    #[test]
    fn priority_accept_writes_pri_token() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        app.open_priority_chooser();
        // selected=0 → A.
        app.priority_accept();
        assert_eq!(app.draft.text(), "(A) Buy milk");
    }

    #[test]
    fn priority_clear_removes_existing() {
        let mut app = build_app("");
        app.draft_set("(A) Buy milk".into());
        app.open_priority_chooser();
        app.priority_step(false); // 0 -> 3 (clear)
        app.priority_accept();
        assert_eq!(app.draft.text(), "Buy milk");
    }

    #[test]
    fn typing_due_colon_opens_calendar_with_anchor() {
        let mut app = build_app("");
        app.draft_set("Buy milk ".into());
        for c in "due:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        let s = app.calendar_state().expect("calendar should be open");
        assert_eq!(s.target, CalendarTarget::Due);
        // Anchor points at the `d` of `due:`.
        assert_eq!(s.anchor, Some("Buy milk ".len()));
    }

    #[test]
    fn typing_t_colon_opens_threshold_calendar() {
        let mut app = build_app("");
        app.draft_set("Pay rent ".into());
        for c in "t:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        let s = app.calendar_state().expect("calendar should be open");
        assert_eq!(s.target, CalendarTarget::Threshold);
        assert_eq!(s.anchor, Some("Pay rent ".len()));
    }

    #[test]
    fn typing_rec_colon_opens_recurrence_builder() {
        let mut app = build_app("");
        app.draft_set("Water plants ".into());
        for c in "rec:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        let s = app.recurrence_state().expect("builder should be open");
        assert_eq!(s.anchor, Some("Water plants ".len()));
    }

    #[test]
    fn kv_trigger_fires_at_bol() {
        let mut app = build_app("");
        for c in "due:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        let s = app.calendar_state().expect("calendar should be open");
        assert_eq!(s.anchor, Some(0));
    }

    #[test]
    fn kv_trigger_does_not_panic_on_multibyte_prefix() {
        // Regression: `match_key_before` used byte arithmetic and then sliced
        // `text[key_start..colon_pos]` directly. If the char immediately
        // before `due`/`rec` is multi-byte, `key_start` lands mid-codepoint
        // and the slice would panic. With `text.get` it just returns `None`
        // and the overlay stays closed.
        for prefix in ["réc", "ñdue", "✓rec"] {
            let mut app = build_app("");
            app.draft_set(prefix.to_string());
            app.draft_insert_char(':');
            // Must not panic; must not open an overlay (boundary char before
            // the key is not whitespace).
            app.maybe_open_kv_overlay();
            assert!(
                app.draft.overlay().is_none(),
                "overlay must stay closed for prefix {prefix:?}",
            );
        }
    }

    #[test]
    fn recurrence_builder_preserves_business_day_unit() {
        // Regression: opening the builder on an existing `rec:3b` used to put
        // BusinessDay outside the unit cycle, so adjusting any other field
        // silently coerced it to Week on the next +/-. Now BusinessDay is in
        // REC_UNIT_ORDER and round-trips intact.
        let mut app = build_app("");
        app.draft_set("Submit timesheet rec:3b".into());
        app.open_recurrence_builder();
        let s = app.recurrence_state().expect("builder open");
        assert_eq!(s.unit, RecUnit::BusinessDay);
        // Cycle the Mode field — the unit must not move.
        app.recurrence_focus(2); // Interval -> Mode
        app.recurrence_adjust(1); // toggle strict
        let s = app.recurrence_state().expect("builder still open");
        assert_eq!(
            s.unit,
            RecUnit::BusinessDay,
            "unit must survive a Mode toggle"
        );
        app.recurrence_accept();
        assert_eq!(app.draft.text(), "Submit timesheet rec:+3b");
    }

    #[test]
    fn recurrence_unit_cycle_includes_business_day() {
        // Stepping through the unit cycle must reach BusinessDay.
        let mut app = build_app("");
        app.open_recurrence_builder();
        app.recurrence_focus(1); // Interval -> Unit
        let order_len = REC_UNIT_ORDER.len();
        let mut seen: Vec<RecUnit> = Vec::with_capacity(order_len);
        for _ in 0..order_len {
            let s = app.recurrence_state().expect("builder open");
            seen.push(s.unit);
            app.recurrence_adjust(1);
        }
        assert!(seen.contains(&RecUnit::BusinessDay));
    }

    #[test]
    fn kv_trigger_does_not_fire_mid_word() {
        // `Recipe:` ends with `e:`, not a recognised key, and the boundary
        // before `due` etc. isn't whitespace either — must not pop.
        let mut app = build_app("");
        app.draft_set("Recipe".into());
        app.draft_insert_char(':');
        app.maybe_open_kv_overlay();
        assert!(app.draft.overlay().is_none());

        // `Mydue:` — `due` appears but the char before is `y`, not whitespace.
        let mut app2 = build_app("");
        app2.draft_set("Mydue".into());
        app2.draft_insert_char(':');
        app2.maybe_open_kv_overlay();
        assert!(app2.draft.overlay().is_none());

        // `let:` — single-letter `t:` test variant. Char before `t` is `e`,
        // not whitespace.
        let mut app3 = build_app("");
        app3.draft_set("let".into());
        app3.draft_insert_char(':');
        app3.maybe_open_kv_overlay();
        assert!(app3.draft.overlay().is_none());
    }

    #[test]
    fn kv_trigger_accept_strips_literal_and_appends() {
        let mut app = build_app("");
        app.draft_set("Buy milk ".into());
        for c in "due:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        app.calendar_accept();
        // The empty `due:` we typed is stripped; the canonical token sits at
        // the end. Exactly one `due:` in the result.
        assert_eq!(app.draft.text(), "Buy milk due:2026-05-06");
        assert_eq!(app.draft.text().matches("due:").count(), 1);
    }

    #[test]
    fn kv_trigger_accept_updates_existing_due() {
        // Re-triggering on a line that already has `due:DATE` means "change
        // the date" — the existing token gets the new value and the empty
        // literal we just typed disappears.
        let mut app = build_app("");
        app.draft_set("Buy milk due:2026-04-01 +groceries".into());
        app.draft_insert_char(' ');
        for c in "due:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        let s = app.calendar_state().expect("calendar should be open");
        // Focused date should be the existing value.
        assert_eq!(s.focused, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap());
        // User picks today (the `t` shortcut) → date jumps to 2026-05-06.
        app.calendar_set_relative(0);
        app.calendar_accept();
        assert_eq!(
            app.draft.text(),
            "Buy milk due:2026-05-06 +groceries",
            "existing due: should be updated and the empty trigger removed",
        );
        assert_eq!(app.draft.text().matches("due:").count(), 1);
    }

    #[test]
    fn kv_trigger_cancel_leaves_literal_in_buffer() {
        // Esc behaves like @/+ autocomplete: the typed `due:` stays so the
        // user can finish the date by hand if they want.
        let mut app = build_app("");
        app.draft_set("Buy milk ".into());
        for c in "due:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        app.calendar_cancel();
        assert_eq!(app.draft.text(), "Buy milk due:");
        assert!(app.draft.overlay().is_none());
    }

    #[test]
    fn kv_trigger_rec_accept_writes_canonical_token() {
        let mut app = build_app("");
        app.draft_set("Water plants ".into());
        for c in "rec:".chars() {
            app.draft_insert_char(c);
        }
        app.maybe_open_kv_overlay();
        // Default builder = 1 week after-complete.
        app.recurrence_accept();
        assert_eq!(app.draft.text(), "Water plants rec:1w");
        assert_eq!(app.draft.text().matches("rec:").count(), 1);
    }

    #[test]
    fn end_to_end_flow_writes_full_task() {
        // Simulates the full add-task flow with the slash menu:
        //   type body → `/` → /due → calendar T → Enter → save.
        // The final task line must round-trip through `parse_line` with the
        // expected metadata fields populated.
        let mut app = build_app("");
        app.mode = crate::app::Mode::Insert;
        for c in "Schedule team offsite".chars() {
            app.draft_insert_char(c);
        }
        app.draft_insert_char(' ');
        app.draft_insert_char('/');
        app.maybe_open_slash_menu();
        // Default selection is "Due date".
        app.slash_accept();
        assert!(matches!(
            app.draft.overlay(),
            Some(DraftOverlay::Calendar(_))
        ));
        // T = tomorrow → 2026-05-07 (today is 2026-05-06 in test_support).
        app.calendar_set_relative(1);
        app.calendar_accept();
        assert!(app.draft.overlay().is_none());
        assert_eq!(app.draft.text(), "Schedule team offsite due:2026-05-07");

        // Saving runs through `parse_line` and prepends today as creation
        // date. After save the task list grows by one with the expected fields.
        app.add_from_draft();
        let task = app.tasks().last().expect("task added");
        assert_eq!(task.due.as_deref(), Some("2026-05-07"));
        assert_eq!(task.created_date.as_deref(), Some("2026-05-06"));
        assert!(task.raw.contains("Schedule team offsite"));
    }
}
