//! The which-key menu: after a chord leader has been armed for a beat, the
//! TUI floats the list of continuations that leader accepts — LazyVim's
//! discoverability, applied to tuxedo's `gg` / `dd` / `yy` / `f…` chords.
//!
//! The menu is *derived*, never hand-written. Built-in rows come from
//! [`crate::app::palette::ENTRIES`] — the same catalog the command palette
//! renders — by picking out every entry whose keys are a two-character
//! chord. Custom chords from `keybinds.toml` are layered on top, so a
//! rebound chord shows up in the menu under the key the user actually
//! pressed. Adding a chord to the palette catalog is therefore enough to
//! make it appear here.

use std::collections::BTreeMap;

use crate::action::Action;
use crate::app::palette;
use crate::keybinds::KeyBindings;

/// One continuation row: the key that completes the chord and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// Display form of the second key (`"g"`, `"p"`, `"Enter"`).
    pub keys: String,
    /// Human-readable description, borrowed from the palette catalog.
    pub label: String,
}

/// Every menu, keyed by leader. Built once at startup: `keybinds.toml` is
/// read a single time, so the derived menus can't drift from the bindings.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    groups: BTreeMap<char, Vec<MenuEntry>>,
}

impl Registry {
    /// Built-in chords only. Used by tests, examples, and any caller that
    /// hasn't loaded a keybinds file.
    pub fn builtin() -> Self {
        let mut groups: BTreeMap<char, Vec<MenuEntry>> = BTreeMap::new();
        for entry in palette::ENTRIES {
            if let Some((leader, second)) = builtin_chord(entry.keys) {
                push(&mut groups, leader, second.to_string(), entry.label.into());
            }
        }
        Self { groups }
    }

    /// Built-ins overlaid with the chords configured in `keybinds.toml`.
    ///
    /// A rebound action loses its built-in row: the user moved it, so
    /// listing it under the old key would be a lie. Its new home is added
    /// back if the new binding is itself a chord (a rebind to a single key
    /// simply leaves the menus).
    pub fn from_keybinds(keybinds: &KeyBindings) -> Self {
        let mut registry = Self::builtin();
        let rebound = keybinds.bound_actions();
        for entries in registry.groups.values_mut() {
            entries.retain(|entry| !rebound.iter().any(|a| Some(*a) == action_for(entry)));
        }
        for (leader, second, action) in keybinds.chords() {
            let Some(label) = palette::label_for(action) else {
                continue;
            };
            push(&mut registry.groups, leader, second, label.to_string());
        }
        registry.groups.retain(|_, entries| !entries.is_empty());
        registry
    }

    /// Rows for `leader`, or `None` when that leader has no continuations to
    /// advertise (in which case the caller draws no menu at all).
    pub fn menu(&self, leader: char) -> Option<&[MenuEntry]> {
        self.groups.get(&leader).map(Vec::as_slice)
    }

    /// Title shown in the menu's border, e.g. `f` → `"f · find"`.
    pub fn title(leader: char) -> String {
        match group_name(leader) {
            Some(name) => format!("{leader} · {name}"),
            None => leader.to_string(),
        }
    }
}

/// Well-known leaders get a name, mirroring how vim groups these verbs.
/// Unknown leaders (a user-defined chord) fall back to the bare key.
fn group_name(leader: char) -> Option<&'static str> {
    match leader {
        'g' => Some("goto"),
        'd' => Some("delete"),
        'y' => Some("yank"),
        'f' => Some("find"),
        'c' => Some("change"),
        _ => None,
    }
}

/// Split a palette `keys` string into `(leader, second)` when it denotes a
/// plain two-key chord. Rejects everything else the catalog holds: single
/// keys, modifier combos (`Ctrl-r`), named keys (`Esc`), and the `a / b`
/// alternation form.
fn builtin_chord(keys: &str) -> Option<(char, char)> {
    let mut chars = keys.chars();
    let (first, second, rest) = (chars.next()?, chars.next()?, chars.next());
    if rest.is_some() || !first.is_ascii_graphic() || !second.is_ascii_graphic() {
        return None;
    }
    Some((first, second))
}

/// Reverse-lookup an entry's action through the palette catalog, matching on
/// the label it was built from. Used to drop rows for rebound actions.
fn action_for(entry: &MenuEntry) -> Option<Action> {
    palette::ENTRIES
        .iter()
        .find(|e| e.label == entry.label.as_str())
        .map(|e| e.action)
}

/// Insert a row, replacing any existing row for the same key so a custom
/// binding wins over the built-in it shadows. Rows stay sorted by key.
fn push(groups: &mut BTreeMap<char, Vec<MenuEntry>>, leader: char, keys: String, label: String) {
    let entries = groups.entry(leader).or_default();
    entries.retain(|e| e.keys != keys);
    entries.push(MenuEntry { keys, label });
    entries.sort_by(|a, b| a.keys.cmp(&b.keys));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_menus_come_from_the_palette_catalog() {
        let reg = Registry::builtin();
        let f = reg.menu('f').expect("f menu");
        let keys: Vec<&str> = f.iter().map(|e| e.keys.as_str()).collect();
        assert_eq!(keys, vec!["c", "f", "p", "s"]);
        let y = reg.menu('y').expect("y menu");
        assert_eq!(y.len(), 2);
        assert!(y.iter().any(|e| e.keys == "b" && e.label.contains("body")));
        assert_eq!(reg.menu('g').map(<[MenuEntry]>::len), Some(1));
        assert_eq!(reg.menu('d').map(<[MenuEntry]>::len), Some(1));
        // A leader with no chords advertises nothing.
        assert!(reg.menu('z').is_none());
    }

    #[test]
    fn multi_char_and_modifier_keys_are_not_chords() {
        assert_eq!(builtin_chord("dd"), Some(('d', 'd')));
        assert!(builtin_chord("d").is_none());
        assert!(builtin_chord("Ctrl-r").is_none());
        assert!(builtin_chord("j / ↓").is_none());
        assert!(builtin_chord("").is_none());
    }

    #[test]
    fn custom_chord_is_added_under_its_own_leader() {
        let kb = KeyBindings::parse("[normal]\ntoggle_show_done = \"gd\"\n");
        let reg = Registry::from_keybinds(&kb);
        let g = reg.menu('g').expect("g menu");
        assert!(g.iter().any(|e| e.keys == "d" && e.label.contains("done")));
        // The built-in `gg` row survives alongside it.
        assert!(g.iter().any(|e| e.keys == "g"));
    }

    #[test]
    fn rebound_action_leaves_its_builtin_row() {
        // `dd` moved to `xd`: the `d` menu loses its only row and disappears,
        // and `x` gains one.
        let kb = KeyBindings::parse("[normal]\ndelete = \"xd\"\n");
        let reg = Registry::from_keybinds(&kb);
        assert!(reg.menu('d').is_none());
        let x = reg.menu('x').expect("x menu");
        assert_eq!(x.len(), 1);
        assert!(x[0].label.contains("delete"));
    }

    #[test]
    fn custom_binding_replaces_the_builtin_on_the_same_key() {
        let kb = KeyBindings::parse("[normal]\ncycle_sort = \"gg\"\n");
        let reg = Registry::from_keybinds(&kb);
        let g = reg.menu('g').expect("g menu");
        let gg: Vec<&MenuEntry> = g.iter().filter(|e| e.keys == "g").collect();
        assert_eq!(gg.len(), 1, "one row per key");
        assert!(gg[0].label.contains("sort"));
    }

    #[test]
    fn titles_name_known_groups_and_fall_back_otherwise() {
        assert_eq!(Registry::title('f'), "f · find");
        assert_eq!(Registry::title('y'), "y · yank");
        assert_eq!(Registry::title('z'), "z");
    }
}
