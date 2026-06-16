//! User-configurable normal-mode keybindings.
//!
//! Path: `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/keybinds.toml`
//!
//! Format: a `[normal]` table whose keys are `Action` names in snake_case and
//! whose values are a string or array of strings, for example:
//! `open_help = "F1"` or `begin_add = ["N", "Ctrl-n"]`.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::app::Chord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedKey {
    Action(Action),
    Pending,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    normal: Vec<Binding>,
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
            if section.as_deref().is_some_and(|name| name != "normal") {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let Some(action) = Action::from_keybind_name(name) else {
                continue;
            };
            for key_text in parse_value_strings(value) {
                if let Some(binding) = Binding::parse(action, &key_text) {
                    bindings.push_normal(binding);
                }
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

    pub fn path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(Self::path_in(&base))
    }

    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxedo").join("keybinds.toml")
    }

    fn push_normal(&mut self, binding: Binding) {
        self.normal.retain(|existing| {
            existing.first != binding.first || existing.second != binding.second
        });
        self.normal.push(binding);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    action: Action,
    first: KeyPress,
    second: Option<KeyPress>,
}

impl Binding {
    fn parse(action: Action, text: &str) -> Option<Self> {
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
}
