//! User-configurable keybindings for every mode and overlay.
//!
//! Path: `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/keybinds.toml`
//!
//! # Format
//!
//! One `[table]` per keyboard context, whose keys are action names in
//! snake_case and whose values are a key string or an array of them:
//!
//! ```toml
//! [normal]
//! open_help = "F1"
//! begin_add = ["N", "Ctrl-n"]
//!
//! [calendar]
//! accept = "Space"
//! ```
//!
//! Lines before any header belong to `[normal]`, preserving the original
//! header-less format. Every context has its own namespace: an overlay owns
//! the keyboard while it is open, so `q` may mean one thing in `[normal]`
//! and another in `[pick]` without colliding.
//!
//! # Removing a default
//!
//! Custom bindings are consulted before the built-ins, but a built-in a
//! config never mentions still fires. Two keys change that, and are
//! recognized in every table:
//!
//! ```toml
//! [normal]
//! unbind = ["x", "dd"]      # these keys now do nothing at all
//!
//! [calendar]
//! replace_defaults = true   # ignore every built-in in this table
//! ```
//!
//! `replace_defaults` drops the built-in *action* bindings only. Contexts
//! with a text field (`[search]`, `[prompt]`, `[dialog_insert]`) still type
//! ordinary characters into the buffer, so a config cannot leave the user
//! unable to write.
//!
//! # Chords
//!
//! A two-key chord (`"gg"`, `"f p"`) may be bound in any table whose
//! context threads a [`Chord`] through — `[normal]` and `[dialog]`. Tables
//! without one have no leader state to arm, so a chord there is dropped at
//! parse time rather than bound to a key that could never fire.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::app::Chord;

/// Config keys that configure the table itself rather than name an action.
const UNBIND_KEY: &str = "unbind";
const REPLACE_KEY: &str = "replace_defaults";

/// An action enum that can be driven from a `keybinds.toml` table.
///
/// Implementors are the per-context vocabularies in [`crate::action`]:
/// [`Action`] for `[normal]`, `RecAction` for `[recurrence]`, and so on.
pub trait KeyAction: Copy {
    /// Table name in `keybinds.toml`.
    const TABLE: &'static str;
    /// Map a snake_case action name to a variant.
    fn from_keybind_name(name: &str) -> Option<Self>;
}

/// What the binding layer decided about a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<A> {
    /// A binding matched — run this action.
    Run(A),
    /// The binding layer consumed the key: it armed a chord leader, or the
    /// key is explicitly unbound. The handler must do nothing with it.
    Swallow,
    /// Nothing claimed the key. Handlers with a text field insert it; the
    /// rest ignore it.
    Fallthrough,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    tables: BTreeMap<String, Table>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Table {
    bindings: Vec<RawBinding>,
    /// Keys the built-ins must not see.
    unbound: Vec<KeyPress>,
    /// Drop every built-in binding in this table.
    replace: bool,
}

/// A parsed line, with the action still in string form. Names are resolved
/// against [`KeyAction::from_keybind_name`] at lookup time, which keeps the
/// parser free of any knowledge of the individual action vocabularies.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawBinding {
    action: String,
    first: KeyPress,
    second: Option<KeyPress>,
}

impl KeyBindings {
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(_) => Self::default(),
        }
    }

    pub fn parse(s: &str) -> Self {
        let mut bindings = Self::default();
        // No table header yet means `[normal]`, preserving the original
        // header-less format.
        let mut section = Action::TABLE.to_string();
        for raw_line in s.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = table_name(line) {
                section = name.to_ascii_lowercase();
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let table = bindings.tables.entry(section.clone()).or_default();
            match name.as_str() {
                REPLACE_KEY => {
                    table.replace = matches!(unquote(value.trim()), Some("true"));
                }
                UNBIND_KEY => {
                    for key_text in parse_value_strings(value) {
                        // An unbound chord blocks its leader outright: there
                        // is no way to say "the second key of this pair does
                        // nothing" that a user could reason about.
                        if let Some(keys) = parse_key_sequence(&key_text) {
                            table.unbound.push(keys[0].clone());
                        }
                    }
                }
                _ => {
                    for key_text in parse_value_strings(value) {
                        if let Some(binding) = RawBinding::parse(&name, &key_text) {
                            push(&mut table.bindings, binding);
                        }
                    }
                }
            }
        }
        bindings
    }

    /// Resolve `key` in `A`'s table, falling back to `builtin` for anything
    /// the config does not claim.
    ///
    /// This is the entry point for every context *without* chords. A chord
    /// configured in such a table is ignored: there is no leader state to
    /// arm, so binding it would swallow the leader key forever. See
    /// [`Self::resolve_chorded`] for `[normal]` and `[dialog]`.
    pub fn resolve<A: KeyAction>(
        &self,
        key: KeyEvent,
        builtin: impl FnOnce(KeyEvent) -> Option<A>,
    ) -> Outcome<A> {
        self.resolve_inner(key, None, |key, _| builtin(key))
    }

    /// Resolve `key` in `A`'s table with two-key chord support.
    ///
    /// `chord` carries the leader state, and is handed to `builtin` as well
    /// so the built-in chords (`gg`, `dd`, `dw`) read the same leader the
    /// configured ones do.
    pub fn resolve_chorded<A: KeyAction>(
        &self,
        key: KeyEvent,
        chord: &mut Chord,
        builtin: impl FnOnce(KeyEvent, &mut Chord) -> Option<A>,
    ) -> Outcome<A> {
        self.resolve_inner(key, Some(chord), |key, chord| {
            builtin(key, chord.expect("chord passed above"))
        })
    }

    /// All precedence rules live here, so they hold everywhere by
    /// construction: a completed chord beats a single key, `unbind` beats a
    /// configured binding, a configured binding beats the built-in, and
    /// `replace_defaults` suppresses the built-in entirely.
    fn resolve_inner<A: KeyAction>(
        &self,
        key: KeyEvent,
        mut chord: Option<&mut Chord>,
        builtin: impl FnOnce(KeyEvent, Option<&mut Chord>) -> Option<A>,
    ) -> Outcome<A> {
        if let Some(table) = self.tables.get(A::TABLE) {
            // A completed chord wins over everything: its leader is already
            // armed, so no single-key reading of this keystroke applies.
            if let Some(chord) = chord.as_deref_mut()
                && let Some(action) = table.match_chord::<A>(key, chord)
            {
                return Outcome::Run(action);
            }
            if table.unbound.iter().any(|k| k.matches(key)) {
                if let Some(chord) = chord.as_deref_mut() {
                    chord.clear();
                }
                return Outcome::Swallow;
            }
            if let Some(action) = table.match_single::<A>(key) {
                if let Some(chord) = chord.as_deref_mut() {
                    chord.clear();
                }
                return Outcome::Run(action);
            }
            // Not a complete binding, but the first key of a configured
            // chord: arm the leader and wait for the second press. Only in a
            // context that has a leader to arm — elsewhere the chord is dead
            // config and the key falls through as if it were unbound.
            if let Some(chord) = chord.as_deref_mut()
                && table.arm_leader::<A>(key, chord)
            {
                return Outcome::Swallow;
            }
            if table.replace {
                return Outcome::Fallthrough;
            }
        }

        match builtin(key, chord) {
            Some(action) => Outcome::Run(action),
            None => Outcome::Fallthrough,
        }
    }

    /// Every two-key chord configured for `[normal]`, as
    /// `(leader, second-key label, action)`. Feeds the which-key menu so a
    /// rebound chord is advertised under the key the user actually presses.
    pub fn chords(&self) -> Vec<(char, String, Action)> {
        let Some(table) = self.tables.get(Action::TABLE) else {
            return Vec::new();
        };
        table
            .bindings
            .iter()
            .filter_map(|binding| {
                let second = binding.second.as_ref()?;
                let leader = binding.first.leader_char()?;
                let action = Action::from_keybind_name(&binding.action)?;
                Some((leader, second.label(), action))
            })
            .collect()
    }

    /// Normal-mode actions the user has bound to a key of their own, chord
    /// or not, plus the keys they have unbound. The which-key menu uses this
    /// to drop built-in rows that no longer apply.
    pub fn bound_actions(&self) -> Vec<Action> {
        let Some(table) = self.tables.get(Action::TABLE) else {
            return Vec::new();
        };
        let mut out: Vec<Action> = Vec::new();
        for binding in &table.bindings {
            if let Some(action) = Action::from_keybind_name(&binding.action)
                && !out.contains(&action)
            {
                out.push(action);
            }
        }
        out
    }

    /// True when `A`'s table opts out of its built-in bindings entirely.
    /// The which-key menu reads this so it stops advertising built-in chords
    /// that no longer fire.
    pub fn replaces_defaults<A: KeyAction>(&self) -> bool {
        self.tables
            .get(A::TABLE)
            .map(|t| t.replace)
            .unwrap_or(false)
    }

    pub fn path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(Self::path_in(&base))
    }

    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxedo").join("keybinds.toml")
    }
}

impl Table {
    /// A binding whose leader is armed and whose second key is `key`.
    fn match_chord<A: KeyAction>(&self, key: KeyEvent, chord: &mut Chord) -> Option<A> {
        for binding in &self.bindings {
            let (Some(second), Some(leader)) =
                (binding.second.as_ref(), binding.first.leader_char())
            else {
                continue;
            };
            if chord.active() == Some(leader)
                && second.matches(key)
                && let Some(action) = A::from_keybind_name(&binding.action)
            {
                chord.clear();
                return Some(action);
            }
        }
        None
    }

    fn match_single<A: KeyAction>(&self, key: KeyEvent) -> Option<A> {
        self.bindings
            .iter()
            .filter(|b| b.second.is_none() && b.first.matches(key))
            .find_map(|b| A::from_keybind_name(&b.action))
    }

    /// Arm `key` as a leader if some binding in this table starts with it.
    fn arm_leader<A: KeyAction>(&self, key: KeyEvent, chord: &mut Chord) -> bool {
        for binding in &self.bindings {
            if binding.second.is_some()
                && binding.first.matches(key)
                && A::from_keybind_name(&binding.action).is_some()
                && let Some(leader) = binding.first.leader_char()
            {
                chord.arm(leader);
                return true;
            }
        }
        false
    }
}

/// Append `binding`, dropping any earlier one on the same keys so the last
/// line in the file wins.
fn push(bindings: &mut Vec<RawBinding>, binding: RawBinding) {
    bindings
        .retain(|existing| existing.first != binding.first || existing.second != binding.second);
    bindings.push(binding);
}

impl RawBinding {
    fn parse(action: &str, text: &str) -> Option<Self> {
        let keys = parse_key_sequence(text)?;
        match keys.as_slice() {
            [first] => Some(Self {
                action: action.to_string(),
                first: first.clone(),
                second: None,
            }),
            [first, second] if first.leader_char().is_some() => Some(Self {
                action: action.to_string(),
                first: first.clone(),
                second: Some(second.clone()),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyPress {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyPress {
    fn matches(&self, key: KeyEvent) -> bool {
        let code = normalized_code(key.code, key.modifiers);
        self.code == code && self.modifiers == normalized_modifiers(code, key.modifiers)
    }

    /// Display form for the which-key menu: `"p"`, `"Ctrl-n"`, `"Enter"`.
    fn label(&self) -> String {
        let mut out = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            out.push_str("Ctrl-");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            out.push_str("Alt-");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            out.push_str("Shift-");
        }
        match self.code {
            KeyCode::Char(' ') => out.push_str("Space"),
            KeyCode::Char(c) => out.push(c),
            KeyCode::F(n) => out.push_str(&format!("F{n}")),
            other => out.push_str(&format!("{other}")),
        }
        out
    }

    fn leader_char(&self) -> Option<char> {
        if self.modifiers == KeyModifiers::NONE
            && let KeyCode::Char(c) = self.code
        {
            Some(c)
        } else {
            None
        }
    }
}

fn parse_key_sequence(text: &str) -> Option<Vec<KeyPress>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.split_whitespace().count() > 1 {
        let keys: Option<Vec<KeyPress>> = text.split_whitespace().map(parse_key).collect();
        return keys.filter(|keys| keys.len() <= 2);
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() == 2
        && chars.iter().all(|c| !c.is_whitespace())
        && !text.contains('-')
        && !text.contains('+')
        && parse_named_key(text).is_none()
    {
        return Some(vec![
            KeyPress {
                code: KeyCode::Char(chars[0]),
                modifiers: KeyModifiers::NONE,
            },
            KeyPress {
                code: KeyCode::Char(chars[1]),
                modifiers: KeyModifiers::NONE,
            },
        ]);
    }
    parse_key(text).map(|key| vec![key])
}

fn parse_key(text: &str) -> Option<KeyPress> {
    if let Some(code) = parse_named_key(text.trim()) {
        return Some(KeyPress {
            code,
            modifiers: normalized_modifiers(code, KeyModifiers::NONE),
        });
    }
    let mut modifiers = KeyModifiers::NONE;
    let normalized = text.trim().replace('+', "-");
    let mut parts: Vec<&str> = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect();
    let key_name = parts.pop()?;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" | "meta" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let code = if let Some(named) = parse_named_key(key_name) {
        named
    } else {
        let mut chars = key_name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCode::Char(c),
            _ => return None,
        }
    };
    let code = normalized_code(code, modifiers);
    Some(KeyPress {
        code,
        modifiers: normalized_modifiers(code, modifiers),
    })
}

fn normalized_code(code: KeyCode, modifiers: KeyModifiers) -> KeyCode {
    if modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(c) = code
    {
        KeyCode::Char(c.to_ascii_lowercase())
    } else {
        code
    }
}

fn parse_named_key(text: &str) -> Option<KeyCode> {
    let lower = text.to_ascii_lowercase();
    match lower.as_str() {
        "backspace" | "bs" => Some(KeyCode::Backspace),
        "enter" | "return" => Some(KeyCode::Enter),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "page-up" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "page-down" | "pgdn" => Some(KeyCode::PageDown),
        "tab" => Some(KeyCode::Tab),
        "backtab" | "shift-tab" => Some(KeyCode::BackTab),
        "delete" | "del" => Some(KeyCode::Delete),
        "insert" | "ins" => Some(KeyCode::Insert),
        "esc" | "escape" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        _ if lower.len() > 1 && lower.starts_with('f') => {
            lower[1..].parse::<u8>().ok().and_then(|n| {
                if (1..=24).contains(&n) {
                    Some(KeyCode::F(n))
                } else {
                    None
                }
            })
        }
        _ => None,
    }
}

fn normalized_modifiers(code: KeyCode, mut modifiers: KeyModifiers) -> KeyModifiers {
    if matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase()) {
        modifiers.remove(KeyModifiers::SHIFT);
    }
    modifiers
}

fn table_name(line: &str) -> Option<String> {
    line.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn parse_value_strings(value: &str) -> Vec<String> {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return parse_array_strings(inner);
    }
    unquote(value)
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

fn parse_array_strings(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = inner.char_indices().peekable();
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() || ch == ',' {
            let _ = chars.next();
            continue;
        }
        if ch != '"' {
            break;
        }
        let start = chars.next().map(|(idx, _)| idx + 1);
        let Some(start) = start else {
            break;
        };
        let mut escaped = false;
        let mut end = None;
        for (idx, c) in chars.by_ref() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '"' {
                end = Some(idx);
                break;
            }
        }
        let Some(end) = end else {
            break;
        };
        out.push(inner[start..end].replace("\\\"", "\""));
    }
    out
}

fn unquote(value: &str) -> Option<&str> {
    if value.is_empty() {
        None
    } else if let Some(inner) = value.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        Some(inner)
    } else {
        Some(value)
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..idx],
            _ => {}
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{CalendarAction, DialogAction, PickAction, RecAction};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn named(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Built-in stand-in: `q` quits, nothing else is claimed.
    fn builtin_quit(key: KeyEvent) -> Option<Action> {
        (key.code == KeyCode::Char('q')).then_some(Action::Quit)
    }

    fn none<A>(_: KeyEvent) -> Option<A> {
        None
    }

    #[test]
    fn parses_single_keys_arrays_and_chords() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            open_help = ["F1", "Ctrl-h"]
            quit = "ZZ"
            begin_add = "N"
            open_command_palette = "Ctrl-P"
            half_page_down = "Page-Down"
            "#,
        );
        let mut chord = Chord::default();
        let mut hit = |key| bindings.resolve_chorded::<Action>(key, &mut chord, |_, _| None);

        assert_eq!(hit(named(KeyCode::F(1))), Outcome::Run(Action::OpenHelp));
        assert_eq!(hit(ctrl('h')), Outcome::Run(Action::OpenHelp));
        // `ZZ` is a chord: the first press arms, the second fires.
        assert_eq!(hit(key('Z')), Outcome::Swallow);
        assert_eq!(hit(key('Z')), Outcome::Run(Action::Quit));
        assert_eq!(hit(key('N')), Outcome::Run(Action::BeginAdd));
        assert_eq!(hit(ctrl('p')), Outcome::Run(Action::OpenCommandPalette));
        assert_eq!(
            hit(named(KeyCode::PageDown)),
            Outcome::Run(Action::HalfPageDown)
        );
    }

    #[test]
    fn unknown_tables_and_action_names_are_ignored() {
        let bindings = KeyBindings::parse(
            r#"
            [nonsense]
            quit = "z"
            [normal]
            not_an_action = "x"
            open_settings = ","
            "#,
        );
        // An unknown table binds nothing anywhere.
        assert_eq!(
            bindings.resolve(key('z'), none::<Action>),
            Outcome::Fallthrough
        );
        // An unknown action name leaves the key to the built-ins.
        assert_eq!(
            bindings.resolve(key('x'), none::<Action>),
            Outcome::Fallthrough
        );
        assert_eq!(
            bindings.resolve(key(','), none::<Action>),
            Outcome::Run(Action::OpenSettings)
        );
    }

    #[test]
    fn custom_binding_wins_over_the_builtin() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = \"q\"\n");
        assert_eq!(
            bindings.resolve(key('q'), builtin_quit),
            Outcome::Run(Action::OpenHelp)
        );
    }

    #[test]
    fn builtin_survives_when_the_config_is_silent() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = \"F1\"\n");
        assert_eq!(
            bindings.resolve(key('q'), builtin_quit),
            Outcome::Run(Action::Quit)
        );
    }

    #[test]
    fn unbind_switches_a_builtin_off() {
        let bindings = KeyBindings::parse("[normal]\nunbind = [\"q\"]\n");
        // Swallowed, not passed through: the built-in must not see it.
        assert_eq!(bindings.resolve(key('q'), builtin_quit), Outcome::Swallow);
    }

    #[test]
    fn unbind_is_scoped_to_its_own_table() {
        let bindings = KeyBindings::parse("[pick]\nunbind = [\"q\"]\n");
        // `q` is dead in [pick]...
        assert_eq!(
            bindings.resolve(key('q'), |_| Some(PickAction::Next)),
            Outcome::Swallow
        );
        // ...and untouched in [normal].
        assert_eq!(
            bindings.resolve(key('q'), builtin_quit),
            Outcome::Run(Action::Quit)
        );
    }

    #[test]
    fn unbinding_a_chord_blocks_its_leader() {
        let bindings = KeyBindings::parse("[normal]\nunbind = [\"dd\"]\n");
        let mut chord = Chord::default();
        assert_eq!(
            bindings.resolve_chorded::<Action>(key('d'), &mut chord, |_, _| None),
            Outcome::Swallow
        );
        assert!(chord.active().is_none(), "leader must not stay armed");
    }

    #[test]
    fn replace_defaults_drops_every_builtin_in_that_table() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            replace_defaults = true
            open_help = "h"
            "#,
        );
        // The one configured key still works...
        assert_eq!(
            bindings.resolve(key('h'), builtin_quit),
            Outcome::Run(Action::OpenHelp)
        );
        // ...and every built-in is gone. Fallthrough, not Swallow: a table
        // with a text field must still be able to type the key.
        assert_eq!(
            bindings.resolve(key('q'), builtin_quit),
            Outcome::Fallthrough
        );
    }

    #[test]
    fn replace_defaults_is_scoped_to_its_own_table() {
        let bindings = KeyBindings::parse("[calendar]\nreplace_defaults = true\n");
        assert_eq!(
            bindings.resolve(key('t'), |_| Some(CalendarAction::Today)),
            Outcome::Fallthrough
        );
        assert_eq!(
            bindings.resolve(key('q'), builtin_quit),
            Outcome::Run(Action::Quit)
        );
    }

    #[test]
    fn tables_are_independent_namespaces() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            quit = "x"

            [recurrence]
            cancel = "x"

            [pick]
            rename = "x"
            "#,
        );
        assert_eq!(
            bindings.resolve(key('x'), none::<Action>),
            Outcome::Run(Action::Quit)
        );
        assert_eq!(
            bindings.resolve(key('x'), none::<RecAction>),
            Outcome::Run(RecAction::Cancel)
        );
        assert_eq!(
            bindings.resolve(key('x'), none::<PickAction>),
            Outcome::Run(PickAction::Rename)
        );
    }

    #[test]
    fn chords_bind_in_the_dialog_table_too() {
        let bindings = KeyBindings::parse("[dialog]\ndelete_word = \"qw\"\n");
        let mut chord = Chord::default();
        let mut hit = |key| bindings.resolve_chorded::<DialogAction>(key, &mut chord, |_, _| None);
        assert_eq!(hit(key('q')), Outcome::Swallow);
        assert_eq!(hit(key('w')), Outcome::Run(DialogAction::DeleteWord));
    }

    #[test]
    fn chords_are_dropped_in_a_table_without_leader_state() {
        // `resolve` (no chord) is what the overlay handlers call. A chord
        // configured there can never complete, so its leader must not
        // swallow the key.
        let bindings = KeyBindings::parse("[pick]\naccept = \"gg\"\n");
        assert_eq!(
            bindings.resolve(key('g'), none::<PickAction>),
            Outcome::Fallthrough
        );
    }

    #[test]
    fn later_binding_on_the_same_key_replaces_the_earlier_one() {
        let bindings = KeyBindings::parse(
            r#"
            [pick]
            next = "z"
            rename = "z"
            "#,
        );
        assert_eq!(
            bindings.resolve(key('z'), none::<PickAction>),
            Outcome::Run(PickAction::Rename)
        );
    }

    #[test]
    fn header_less_lines_still_mean_normal() {
        let bindings = KeyBindings::parse("open_help = \"F1\"\n");
        assert_eq!(
            bindings.resolve(named(KeyCode::F(1)), none::<Action>),
            Outcome::Run(Action::OpenHelp)
        );
    }

    #[test]
    fn path_uses_tuxedo_keybinds_toml() {
        let path = KeyBindings::path_in(Path::new("/tmp/config"));
        assert!(path.ends_with("tuxedo/keybinds.toml"));
    }
}
