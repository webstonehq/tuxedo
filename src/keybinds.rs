//! User-configurable keybindings.
//!
//! Path: `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/keybinds.toml`
//!
//! Format: a `[normal]` table whose keys are `Action` names in snake_case and
//! whose values are a string or array of strings, for example:
//! `open_help = "F1"` or `begin_add = ["N", "Ctrl-n"]`.
//!
//! A `[recurrence]` table binds motions inside the `↻ REPEAT` overlay to
//! [`RecAction`] names, e.g. `focus_next = ["j", "Tab"]`. Overlay keys are a
//! separate namespace: the overlay owns the keyboard while open, so letters
//! may be reused without colliding with `[normal]`. Two-key chords are
//! normal-mode only — a chord in `[recurrence]` is ignored.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{Action, RecAction};
use crate::app::Chord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedKey {
    Action(Action),
    Pending,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    normal: Vec<Binding<Action>>,
    recurrence: Vec<Binding<RecAction>>,
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
        let mut section: Option<String> = None;
        for raw_line in s.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = table_name(line) {
                section = Some(name.to_ascii_lowercase());
                continue;
            }
            // No table header yet means `[normal]`, preserving the original
            // header-less format.
            let table = section.as_deref().unwrap_or("normal");
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            match table {
                "normal" => {
                    let Some(action) = Action::from_keybind_name(name) else {
                        continue;
                    };
                    for key_text in parse_value_strings(value) {
                        if let Some(binding) = Binding::parse(action, &key_text) {
                            bindings.push_normal(binding);
                        }
                    }
                }
                "recurrence" => {
                    let Some(action) = RecAction::from_keybind_name(name) else {
                        continue;
                    };
                    for key_text in parse_value_strings(value) {
                        // Overlay keys are single presses — a chord here would
                        // have no leader state to arm, so drop it rather than
                        // bind something that could never fire.
                        if let Some(binding) = Binding::parse(action, &key_text)
                            && binding.second.is_none()
                        {
                            bindings.push_recurrence(binding);
                        }
                    }
                }
                _ => continue,
            }
        }
        bindings
    }

    /// Resolve a custom normal-mode binding. Custom bindings are checked
    /// before built-ins by the caller; `Pending` means this key was the first
    /// key of a configured two-key chord and should be consumed.
    pub fn resolve_normal(&self, key: KeyEvent, chord: &mut Chord) -> Option<ResolvedKey> {
        for binding in &self.normal {
            let Some(second) = binding.second.as_ref() else {
                continue;
            };
            let Some(leader) = binding.first.leader_char() else {
                continue;
            };
            if chord.active() == Some(leader) && second.matches(key) {
                chord.clear();
                return Some(ResolvedKey::Action(binding.action));
            }
        }
        for binding in &self.normal {
            if binding.second.is_none() && binding.first.matches(key) {
                chord.clear();
                return Some(ResolvedKey::Action(binding.action));
            }
        }
        for binding in &self.normal {
            if binding.second.is_some()
                && binding.first.matches(key)
                && let Some(leader) = binding.first.leader_char()
            {
                chord.arm(leader);
                return Some(ResolvedKey::Pending);
            }
        }
        None
    }

    /// Resolve the task-list namespace, preserving custom binding precedence
    /// and the built-in chord state machine. Dispatch and displayed-key queries
    /// both use this resolver.
    pub fn resolve_action(&self, key: KeyEvent, chord: &mut Chord) -> Option<Action> {
        match self.resolve_normal(key, chord) {
            Some(ResolvedKey::Action(action)) => return Some(action),
            Some(ResolvedKey::Pending) => return None,
            None => {}
        }

        if key.code == KeyCode::Char('q')
            || (key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL)
        {
            return Some(Action::Quit);
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            return match key.code {
                KeyCode::Char('d') => Some(Action::HalfPageDown),
                KeyCode::Char('u') => Some(Action::HalfPageUp),
                KeyCode::Char('p') => Some(Action::OpenCommandPalette),
                _ => None,
            };
        }
        Some(match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::CursorDown,
            KeyCode::Char('k') | KeyCode::Up => Action::CursorUp,
            KeyCode::Char('J') => Action::MoveTaskDown,
            KeyCode::Char('K') => Action::MoveTaskUp,
            KeyCode::Char('G') => Action::CursorBottom,
            // First 'g' arms the chord; second 'g' fires CursorTop.
            KeyCode::Char('g') if chord.consume('f') => Action::PickGrouping,
            KeyCode::Char('g') if chord.toggle('g') => Action::CursorTop,
            KeyCode::Enter => Action::ToggleGroup,
            KeyCode::Char('n') => Action::BeginAdd,
            KeyCode::Char('r') => Action::Reschedule,
            KeyCode::Char('a') => Action::ToggleArchiveView,
            KeyCode::Char('l') => Action::GoList,
            KeyCode::Char('e') => Action::BeginEdit,
            KeyCode::Char('i') => Action::BeginEditInsert,
            KeyCode::Char('o') => Action::OpenNote,
            KeyCode::Char('O') => Action::CreateOrOpenNote,
            KeyCode::Char('x') => Action::ToggleComplete,
            // 'dd' chord. First press arms; second fires.
            KeyCode::Char('d') if chord.toggle('d') => Action::Delete,
            // 'yy' chord copies the whole line; 'yb' (after 'y' is armed) copies
            // the body only. Plain 'y' just arms the leader.
            KeyCode::Char('y') if chord.toggle('y') => Action::CopyLine,
            KeyCode::Char('b') if chord.consume('y') => Action::CopyBody,
            KeyCode::Char('p') => {
                // After 'f' arms, 'fp' opens the project picker. Otherwise plain
                // 'p' cycles priority.
                if chord.consume('f') {
                    Action::PickProject
                } else {
                    Action::CyclePriority
                }
            }
            KeyCode::Char('c') => {
                if chord.consume('f') {
                    Action::PickContext
                } else {
                    Action::BeginPromptContext
                }
            }
            KeyCode::Char('/') => Action::BeginSearch,
            KeyCode::Char('?') => Action::OpenHelp,
            KeyCode::Char(',') => Action::OpenSettings,
            KeyCode::Char(':') => Action::OpenCommandPalette,
            KeyCode::Char('u') => Action::Undo,
            KeyCode::Char('v') => Action::ToggleVisual,
            KeyCode::Char(' ') => Action::ToggleSelected,
            KeyCode::Char('A') => Action::ArchiveCompleted,
            // First 'f' arms the leader; a second 'f' (`ff`) opens the saved-
            // search picker. Mirrors the `fp`/`fc` pattern below.
            KeyCode::Char('f') => {
                if chord.consume('f') {
                    Action::PickSavedFilter
                } else {
                    Action::ArmF
                }
            }
            KeyCode::Char('s') => {
                // `fs` saves the active search; plain 's' opens the share QR.
                if chord.consume('f') {
                    Action::SaveCurrentFilter
                } else {
                    Action::OpenShare
                }
            }
            KeyCode::Char('S') => Action::CycleSort,
            KeyCode::Char('+') => Action::BeginPromptProject,
            KeyCode::Char('[') => Action::ToggleLeftPane,
            KeyCode::Char(']') => Action::ToggleRightPane,
            KeyCode::Char('T') => Action::OpenThemePicker,
            KeyCode::Char('D') => Action::CycleDensity,
            KeyCode::Char('L') => Action::ToggleLineNum,
            KeyCode::Char('H') => Action::ToggleShowDone,
            KeyCode::Char('F') => Action::ToggleShowFuture,
            KeyCode::Esc => Action::EscapeStack,
            KeyCode::Char('W') => Action::ChangeWeekStart,
            _ => return None,
        })
    }

    /// Resolve the recurrence-overlay namespace, including its built-ins.
    pub fn recurrence_action(&self, key: KeyEvent) -> Option<RecAction> {
        if let Some(action) = self.resolve_recurrence(key) {
            return Some(action);
        }
        Some(match key.code {
            // Horizontal keys change the focused field's *value*: both the unit
            // and the mode render as horizontal segmented controls, so Left/Right
            // moving along them is the affordance the layout already suggests.
            KeyCode::Char('h') | KeyCode::Left => RecAction::ValuePrev,
            KeyCode::Char('l') | KeyCode::Right => RecAction::ValueNext,
            // Vertical keys (and Tab) move *between* fields.
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => RecAction::FocusNext,
            KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => RecAction::FocusPrev,
            // `=` is the unshifted `+` on US keyboards — accept both so users
            // don't have to chord Shift to bump the interval.
            KeyCode::Char('+') | KeyCode::Char('=') => RecAction::ValueNext,
            KeyCode::Char('-') | KeyCode::Char('_') => RecAction::ValuePrev,
            KeyCode::Enter => RecAction::Accept,
            KeyCode::Esc => RecAction::Cancel,
            _ => return None,
        })
    }

    /// Reachable task-list bindings, checked by replaying each candidate
    /// through the same resolver used for input. Aliases retain any defaults
    /// they do not actually shadow. Unreachable custom chords are omitted too.
    pub fn effective_keys(&self, action: Action) -> Vec<String> {
        let defaults = NORMAL_DEFAULTS
            .iter()
            .filter_map(|(text, action)| default_binding(*action, text));
        let candidates = defaults.chain(self.normal.iter().cloned());
        let mut labels = Vec::new();
        for binding in candidates.filter(|binding| binding.action == action) {
            let mut chord = Chord::default();
            let first = self.resolve_action(binding.first.event(), &mut chord);
            let resolved = if let Some(second) = &binding.second {
                match first {
                    // ArmF is applied by dispatch rather than the resolver.
                    Some(Action::ArmF) => chord.arm('f'),
                    None => {}
                    Some(_) => continue,
                }
                self.resolve_action(second.event(), &mut chord)
            } else {
                first
            };
            if resolved == Some(action) {
                let label = binding.label();
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
        }
        labels
    }

    /// Display all reachable task-list keys, or an empty string for an action
    /// available only through the command palette.
    pub fn keys_label(&self, action: Action) -> String {
        self.effective_keys(action).join(" / ")
    }

    /// Reachable keys inside the recurrence overlay's separate namespace.
    pub fn effective_recurrence_keys(&self, action: RecAction) -> Vec<String> {
        let defaults = RECURRENCE_DEFAULTS
            .iter()
            .filter_map(|(text, action)| default_binding(*action, text));
        let candidates = defaults.chain(self.recurrence.iter().cloned());
        let mut labels = Vec::new();
        for binding in candidates.filter(|binding| binding.action == action) {
            if self.recurrence_action(binding.first.event()) == Some(action) {
                let label = binding.label();
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
        }
        labels
    }

    pub fn recurrence_label(&self, action: RecAction) -> String {
        self.effective_recurrence_keys(action).join(" / ")
    }

    pub fn path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(Self::path_in(&base))
    }

    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxedo").join("keybinds.toml")
    }

    /// Resolve a key pressed while the recurrence-builder overlay is open.
    /// `None` means no custom binding matched and the caller should fall back
    /// to the built-in motions.
    pub fn resolve_recurrence(&self, key: KeyEvent) -> Option<RecAction> {
        self.recurrence
            .iter()
            .find(|binding| binding.first.matches(key))
            .map(|binding| binding.action)
    }

    fn push_normal(&mut self, binding: Binding<Action>) {
        self.normal.retain(|existing| {
            existing.first != binding.first || existing.second != binding.second
        });
        self.normal.push(binding);
    }

    fn push_recurrence(&mut self, binding: Binding<RecAction>) {
        self.recurrence
            .retain(|existing| existing.first != binding.first);
        self.recurrence.push(binding);
    }
}

// These are display candidates, not a second resolver: each is verified by
// replay through resolve_action / recurrence_action before it is advertised.
const NORMAL_DEFAULTS: &[(&str, Action)] = &[
    ("q", Action::Quit),
    ("Ctrl-c", Action::Quit),
    ("j", Action::CursorDown),
    ("Down", Action::CursorDown),
    ("k", Action::CursorUp),
    ("Up", Action::CursorUp),
    ("gg", Action::CursorTop),
    ("G", Action::CursorBottom),
    ("Ctrl-d", Action::HalfPageDown),
    ("Ctrl-u", Action::HalfPageUp),
    ("n", Action::BeginAdd),
    ("e", Action::BeginEdit),
    ("i", Action::BeginEditInsert),
    ("x", Action::ToggleComplete),
    ("dd", Action::Delete),
    ("r", Action::Reschedule),
    ("p", Action::CyclePriority),
    ("J", Action::MoveTaskDown),
    ("K", Action::MoveTaskUp),
    ("/", Action::BeginSearch),
    ("?", Action::OpenHelp),
    (",", Action::OpenSettings),
    (":", Action::OpenCommandPalette),
    ("Ctrl-p", Action::OpenCommandPalette),
    ("u", Action::Undo),
    ("v", Action::ToggleVisual),
    ("Space", Action::ToggleSelected),
    ("l", Action::GoList),
    ("a", Action::ToggleArchiveView),
    ("A", Action::ArchiveCompleted),
    ("f", Action::ArmF),
    ("fp", Action::PickProject),
    ("fc", Action::PickContext),
    ("ff", Action::PickSavedFilter),
    ("fs", Action::SaveCurrentFilter),
    ("S", Action::CycleSort),
    ("fg", Action::PickGrouping),
    ("Enter", Action::ToggleGroup),
    ("+", Action::BeginPromptProject),
    ("c", Action::BeginPromptContext),
    ("[", Action::ToggleLeftPane),
    ("]", Action::ToggleRightPane),
    ("T", Action::OpenThemePicker),
    ("D", Action::CycleDensity),
    ("L", Action::ToggleLineNum),
    ("H", Action::ToggleShowDone),
    ("F", Action::ToggleShowFuture),
    ("yy", Action::CopyLine),
    ("yb", Action::CopyBody),
    ("o", Action::OpenNote),
    ("O", Action::CreateOrOpenNote),
    ("Esc", Action::EscapeStack),
    ("s", Action::OpenShare),
    ("W", Action::ChangeWeekStart),
];

const RECURRENCE_DEFAULTS: &[(&str, RecAction)] = &[
    ("j", RecAction::FocusNext),
    ("Down", RecAction::FocusNext),
    ("Tab", RecAction::FocusNext),
    ("k", RecAction::FocusPrev),
    ("Up", RecAction::FocusPrev),
    ("Shift-Tab", RecAction::FocusPrev),
    ("l", RecAction::ValueNext),
    ("Right", RecAction::ValueNext),
    ("+", RecAction::ValueNext),
    ("=", RecAction::ValueNext),
    ("h", RecAction::ValuePrev),
    ("Left", RecAction::ValuePrev),
    ("-", RecAction::ValuePrev),
    ("_", RecAction::ValuePrev),
    ("Enter", RecAction::Accept),
    ("Esc", RecAction::Cancel),
];

fn default_binding<A>(action: A, text: &str) -> Option<Binding<A>> {
    Binding::parse(action, text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding<A> {
    action: A,
    first: KeyPress,
    second: Option<KeyPress>,
}

impl<A> Binding<A> {
    fn label(&self) -> String {
        let first = self.first.label();
        match &self.second {
            Some(second)
                if matches!(self.first.code, KeyCode::Char(c) if c != ' ')
                    && self.first.modifiers.is_empty()
                    && matches!(second.code, KeyCode::Char(c) if c != ' ')
                    && second.modifiers.is_empty() =>
            {
                format!("{first}{}", second.label())
            }
            Some(second) => format!("{first} {}", second.label()),
            None => first,
        }
    }

    fn parse(action: A, text: &str) -> Option<Self> {
        let keys = parse_key_sequence(text)?;
        match keys.as_slice() {
            [first] => Some(Self {
                action,
                first: first.clone(),
                second: None,
            }),
            [first, second] if first.leader_char().is_some() => Some(Self {
                action,
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
    fn event(&self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }

    fn label(&self) -> String {
        let key = match self.code {
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::BackTab => "Shift-Tab".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            KeyCode::Insert => "Insert".to_string(),
            KeyCode::F(n) => format!("F{n}"),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Esc => "Esc".to_string(),
            _ => format!("{:?}", self.code),
        };
        let mut prefix = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            prefix.push_str("Ctrl-");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            prefix.push_str("Alt-");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            prefix.push_str("Shift-");
        }
        format!("{prefix}{key}")
    }

    fn matches(&self, key: KeyEvent) -> bool {
        let code = normalized_code(key.code, key.modifiers);
        self.code == code && self.modifiers == normalized_modifiers(code, key.modifiers)
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
    // A bare symbol is a key, not an empty modifier combination.
    if text.trim() == "+" || text.trim() == "-" {
        return Some(KeyPress {
            code: KeyCode::Char(text.trim().chars().next()?),
            modifiers: KeyModifiers::NONE,
        });
    }

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

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn dispatch_sequence(bindings: &KeyBindings, text: &str) -> Option<Action> {
        let binding = default_binding(Action::Quit, text).expect("displayed sequence parses");
        let mut chord = Chord::default();
        let first = bindings.resolve_action(binding.first.event(), &mut chord);
        if let Some(second) = binding.second {
            match first {
                Some(Action::ArmF) => chord.arm('f'),
                None => {}
                Some(action) => panic!("{text} dispatches {action:?} before its final key"),
            }
            bindings.resolve_action(second.event(), &mut chord)
        } else {
            first
        }
    }

    #[test]
    fn every_advertised_default_dispatches_its_action() {
        let bindings = KeyBindings::default();
        for (_, action) in NORMAL_DEFAULTS {
            let labels = bindings.effective_keys(*action);
            assert!(!labels.is_empty(), "missing {action:?}");
            for label in labels {
                assert_eq!(
                    dispatch_sequence(&bindings, &label),
                    Some(*action),
                    "{label}"
                );
            }
        }
        assert_eq!(bindings.keys_label(Action::CycleTheme), "");
        assert_eq!(bindings.keys_label(Action::BeginPromptProject), "+");
    }

    #[test]
    fn grouping_bindings_respect_custom_leaders_suffixes_and_aliases() {
        let defaults = KeyBindings::default();
        assert_eq!(defaults.keys_label(Action::PickGrouping), "fg");
        assert_eq!(defaults.keys_label(Action::ToggleGroup), "Enter");
        assert_eq!(
            dispatch_sequence(&defaults, "fg"),
            Some(Action::PickGrouping)
        );
        for source in [
            "open_help = \"f\"",
            "open_help = \"g\"",
            "open_help = \"fg\"",
        ] {
            let bindings = KeyBindings::parse(&format!(
                "[normal]\n{source}\npick_grouping = \"z\"\ntoggle_group = \"Tab\""
            ));
            assert_eq!(bindings.keys_label(Action::PickGrouping), "z");
            assert_eq!(
                dispatch_sequence(&bindings, "z"),
                Some(Action::PickGrouping)
            );
            assert_eq!(
                dispatch_sequence(&bindings, "Tab"),
                Some(Action::ToggleGroup)
            );
        }
    }

    #[test]
    fn custom_aliases_keep_reachable_defaults() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = [\"F1\", \"Ctrl-h\"]");
        assert_eq!(
            bindings.effective_keys(Action::OpenHelp),
            ["?", "F1", "Ctrl-h"]
        );
        for label in bindings.effective_keys(Action::OpenHelp) {
            assert_eq!(dispatch_sequence(&bindings, &label), Some(Action::OpenHelp));
        }
    }

    #[test]
    fn stolen_single_keys_are_not_advertised_for_old_actions() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = [\"q\", \"p\", \"Ctrl-c\"]");
        assert!(bindings.effective_keys(Action::Quit).is_empty());
        assert!(bindings.effective_keys(Action::CyclePriority).is_empty());
        assert!(bindings.effective_keys(Action::PickProject).is_empty());
        assert_eq!(
            bindings.effective_keys(Action::OpenHelp),
            ["?", "q", "p", "Ctrl-c"]
        );
        // A stolen second key affects that chord, not the entire f namespace.
        assert_eq!(bindings.keys_label(Action::PickContext), "fc");
    }

    #[test]
    fn custom_prefixes_shadow_singles_but_can_reach_builtin_suffixes() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = [\"nx\", \"gp\", \"fq\"]");
        assert!(bindings.effective_keys(Action::BeginAdd).is_empty());
        assert!(bindings.effective_keys(Action::CursorTop).is_empty());
        assert_eq!(bindings.keys_label(Action::PickProject), "fp");
        assert!(bindings.effective_keys(Action::PickSavedFilter).is_empty());
        for label in ["nx", "gp", "fq"] {
            assert_eq!(dispatch_sequence(&bindings, label), Some(Action::OpenHelp));
        }
        assert_eq!(
            bindings.effective_keys(Action::OpenHelp),
            ["?", "nx", "gp", "fq"]
        );
    }

    #[test]
    fn single_prefix_conflict_hides_unreachable_custom_chord() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = \"nx\"\nbegin_edit = \"n\"");
        assert_eq!(bindings.keys_label(Action::OpenHelp), "?");
        assert_eq!(bindings.effective_keys(Action::BeginEdit), ["e", "n"]);
        assert!(bindings.effective_keys(Action::BeginAdd).is_empty());
    }

    #[test]
    fn custom_chord_overrides_builtin_and_later_duplicate_wins() {
        let bindings = KeyBindings::parse("[normal]\nopen_help = \"dd\"\nquit = \"dd\"");
        assert!(bindings.effective_keys(Action::Delete).is_empty());
        assert_eq!(bindings.keys_label(Action::OpenHelp), "?");
        assert_eq!(bindings.effective_keys(Action::Quit), ["q", "Ctrl-c", "dd"]);
        assert_eq!(dispatch_sequence(&bindings, "dd"), Some(Action::Quit));
    }

    #[test]
    fn every_advertised_custom_binding_replays_against_dispatch() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            open_help = ["q", "nx", "g Enter", "Ctrl-d"]
            begin_edit = "f"
            begin_add = ["z", "j Down"]
            quit = "gg"
        "#,
        );
        for (_, action) in NORMAL_DEFAULTS {
            for label in bindings.effective_keys(*action) {
                assert_eq!(
                    dispatch_sequence(&bindings, &label),
                    Some(*action),
                    "{label}"
                );
            }
        }
        assert!(bindings.effective_keys(Action::PickProject).is_empty());
        assert!(bindings.effective_keys(Action::HalfPageDown).is_empty());
        assert_eq!(bindings.keys_label(Action::BeginAdd), "z / j Down");
    }

    #[test]
    fn bare_plus_and_minus_are_configurable_keys() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            open_help = "+"
            [recurrence]
            focus_next = ["+", "-"]
        "#,
        );
        assert_eq!(dispatch_sequence(&bindings, "+"), Some(Action::OpenHelp));
        assert!(
            bindings
                .effective_keys(Action::BeginPromptProject)
                .is_empty()
        );
        assert!(
            bindings
                .effective_recurrence_keys(RecAction::ValueNext)
                .iter()
                .all(|label| label != "+")
        );
        assert!(
            bindings
                .effective_recurrence_keys(RecAction::ValuePrev)
                .iter()
                .all(|label| label != "-")
        );
        assert_eq!(
            bindings.recurrence_action(key('+')),
            Some(RecAction::FocusNext)
        );
        assert_eq!(
            bindings.recurrence_action(key('-')),
            Some(RecAction::FocusNext)
        );
    }

    #[test]
    fn recurrence_aliases_and_stolen_keys_use_overlay_dispatch() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            quit = "l"
            [recurrence]
            focus_next = ["l", "Ctrl-n"]
            cancel = "Enter"
            accept = "q"
        "#,
        );
        assert_eq!(bindings.recurrence_label(RecAction::Accept), "q");
        assert_eq!(
            bindings.effective_recurrence_keys(RecAction::Cancel),
            ["Esc", "Enter"]
        );
        assert_eq!(
            bindings.effective_recurrence_keys(RecAction::ValueNext),
            ["Right", "+", "="]
        );
        assert_eq!(
            bindings.effective_recurrence_keys(RecAction::FocusNext),
            ["j", "Down", "Tab", "l", "Ctrl-n"]
        );
        for (_, action) in RECURRENCE_DEFAULTS {
            for label in bindings.effective_recurrence_keys(*action) {
                let binding =
                    default_binding(*action, &label).expect("displayed recurrence key parses");
                assert_eq!(
                    bindings.recurrence_action(binding.first.event()),
                    Some(*action),
                    "{label}"
                );
            }
        }
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
        assert_eq!(
            bindings.resolve_normal(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), &mut chord),
            Some(ResolvedKey::Action(Action::OpenHelp))
        );
        assert_eq!(
            bindings.resolve_normal(ctrl('h'), &mut chord),
            Some(ResolvedKey::Action(Action::OpenHelp))
        );
        assert_eq!(
            bindings.resolve_normal(key('Z'), &mut chord),
            Some(ResolvedKey::Pending)
        );
        assert_eq!(
            bindings.resolve_normal(key('Z'), &mut chord),
            Some(ResolvedKey::Action(Action::Quit))
        );
        assert_eq!(
            bindings.resolve_normal(key('N'), &mut chord),
            Some(ResolvedKey::Action(Action::BeginAdd))
        );
        assert_eq!(
            bindings.resolve_normal(ctrl('p'), &mut chord),
            Some(ResolvedKey::Action(Action::OpenCommandPalette))
        );
        assert_eq!(
            bindings.resolve_normal(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                &mut chord,
            ),
            Some(ResolvedKey::Action(Action::HalfPageDown))
        );
    }

    #[test]
    fn ignores_other_tables_and_unknown_actions() {
        let bindings = KeyBindings::parse(
            r#"
            [insert]
            quit = "q"
            [normal]
            not_an_action = "x"
            open_settings = ","
            "#,
        );
        let mut chord = Chord::default();
        assert_eq!(bindings.resolve_normal(key('q'), &mut chord), None);
        assert_eq!(
            bindings.resolve_normal(key(','), &mut chord),
            Some(ResolvedKey::Action(Action::OpenSettings))
        );
    }

    #[test]
    fn path_uses_tuxedo_keybinds_toml() {
        let path = KeyBindings::path_in(Path::new("/tmp/config"));
        assert!(path.ends_with("tuxedo/keybinds.toml"));
    }

    #[test]
    fn parses_recurrence_table() {
        let bindings = KeyBindings::parse(
            r#"
            [recurrence]
            focus_next = ["Tab", "Ctrl-n"]
            value_next = "L"
            cancel = "q"
            "#,
        );
        assert_eq!(
            bindings.resolve_recurrence(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(RecAction::FocusNext)
        );
        assert_eq!(
            bindings.resolve_recurrence(ctrl('n')),
            Some(RecAction::FocusNext)
        );
        assert_eq!(
            bindings.resolve_recurrence(key('L')),
            Some(RecAction::ValueNext)
        );
        assert_eq!(
            bindings.resolve_recurrence(key('q')),
            Some(RecAction::Cancel)
        );
        // Unbound keys fall through so the caller can apply the built-ins.
        assert_eq!(bindings.resolve_recurrence(key('z')), None);
    }

    #[test]
    fn recurrence_ignores_chords_and_unknown_actions() {
        let bindings = KeyBindings::parse(
            r#"
            [recurrence]
            accept = "gg"
            not_an_action = "z"
            "#,
        );
        // A chord has no leader state to arm inside the overlay, so it is
        // dropped rather than bound to a key that could never fire.
        assert_eq!(bindings.resolve_recurrence(key('g')), None);
        assert_eq!(bindings.resolve_recurrence(key('z')), None);
    }

    #[test]
    fn recurrence_and_normal_namespaces_are_independent() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            quit = "x"

            [recurrence]
            cancel = "x"
            "#,
        );
        let mut chord = Chord::default();
        assert_eq!(
            bindings.resolve_normal(key('x'), &mut chord),
            Some(ResolvedKey::Action(Action::Quit))
        );
        assert_eq!(
            bindings.resolve_recurrence(key('x')),
            Some(RecAction::Cancel)
        );
    }

    #[test]
    fn later_recurrence_binding_replaces_earlier_one() {
        let bindings = KeyBindings::parse(
            r#"
            [recurrence]
            value_next = "n"
            focus_next = "n"
            "#,
        );
        assert_eq!(
            bindings.resolve_recurrence(key('n')),
            Some(RecAction::FocusNext)
        );
    }
}
