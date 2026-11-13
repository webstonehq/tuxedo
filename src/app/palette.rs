//! Command palette catalog and filter. Maps every `Action` variant to a
//! human-readable label and its current keybinding, then filters by fuzzy
//! subsequence match against the label. Reuses `search::subseq_match_ci` so
//! the matching semantics stay identical to the `/` task search.
use super::types::Mode;
use crate::action::Action;
use crate::search::subseq_match_ci;

#[derive(Debug, Clone, Copy)]
pub struct PaletteEntry {
    pub label: &'static str,
    pub action: Action,
}

/// Every action that's meaningful to invoke from the palette. `ArmF` is
/// omitted: it only exists as the leader of `fp` / `fc`, both of which appear
/// here under their full names.
pub const ENTRIES: &[PaletteEntry] = &[
    PaletteEntry {
        label: "new task",
        action: Action::BeginAdd,
    },
    PaletteEntry {
        label: "edit current task (normal mode)",
        action: Action::BeginEdit,
    },
    PaletteEntry {
        label: "edit current task (insert mode)",
        action: Action::BeginEditInsert,
    },
    PaletteEntry {
        label: "toggle complete",
        action: Action::ToggleComplete,
    },
    PaletteEntry {
        label: "delete task",
        action: Action::Delete,
    },
    PaletteEntry {
        label: "cycle priority",
        action: Action::CyclePriority,
    },
    PaletteEntry {
        label: "move task down",
        action: Action::MoveTaskDown,
    },
    PaletteEntry {
        label: "move task up",
        action: Action::MoveTaskUp,
    },
    PaletteEntry {
        label: "add project to current task",
        action: Action::BeginPromptProject,
    },
    PaletteEntry {
        label: "add or remove context on current task",
        action: Action::BeginPromptContext,
    },
    PaletteEntry {
        label: "copy line to clipboard",
        action: Action::CopyLine,
    },
    PaletteEntry {
        label: "copy body to clipboard",
        action: Action::CopyBody,
    },
    PaletteEntry {
        label: "open task note",
        action: Action::OpenNote,
    },
    PaletteEntry {
        label: "create or open task note",
        action: Action::CreateOrOpenNote,
    },
    PaletteEntry {
        label: "undo",
        action: Action::Undo,
    },
    PaletteEntry {
        label: "cursor down",
        action: Action::CursorDown,
    },
    PaletteEntry {
        label: "cursor up",
        action: Action::CursorUp,
    },
    PaletteEntry {
        label: "jump to first task",
        action: Action::CursorTop,
    },
    PaletteEntry {
        label: "jump to last task",
        action: Action::CursorBottom,
    },
    PaletteEntry {
        label: "page down",
        action: Action::HalfPageDown,
    },
    PaletteEntry {
        label: "page up",
        action: Action::HalfPageUp,
    },
    PaletteEntry {
        label: "fuzzy search",
        action: Action::BeginSearch,
    },
    PaletteEntry {
        label: "filter by project",
        action: Action::PickProject,
    },
    PaletteEntry {
        label: "filter by context",
        action: Action::PickContext,
    },
    PaletteEntry {
        label: "pick saved filter",
        action: Action::PickSavedFilter,
    },
    PaletteEntry {
        label: "save search as filter",
        action: Action::SaveCurrentFilter,
    },
    PaletteEntry {
        label: "cycle sort",
        action: Action::CycleSort,
    },
    PaletteEntry {
        label: "toggle visual / multi-select",
        action: Action::ToggleVisual,
    },
    PaletteEntry {
        label: "toggle selected row",
        action: Action::ToggleSelected,
    },
    PaletteEntry {
        label: "list view",
        action: Action::GoList,
    },
    PaletteEntry {
        label: "toggle archive view",
        action: Action::ToggleArchiveView,
    },
    PaletteEntry {
        label: "archive completed tasks",
        action: Action::ArchiveCompleted,
    },
    PaletteEntry {
        label: "show done in list",
        action: Action::ToggleShowDone,
    },
    PaletteEntry {
        label: "show future in list",
        action: Action::ToggleShowFuture,
    },
    PaletteEntry {
        label: "toggle filter pane",
        action: Action::ToggleLeftPane,
    },
    PaletteEntry {
        label: "toggle detail pane",
        action: Action::ToggleRightPane,
    },
    PaletteEntry {
        label: "pick theme",
        action: Action::OpenThemePicker,
    },
    PaletteEntry {
        label: "cycle theme",
        action: Action::CycleTheme,
    },
    PaletteEntry {
        label: "cycle density",
        action: Action::CycleDensity,
    },
    PaletteEntry {
        label: "toggle line numbers",
        action: Action::ToggleLineNum,
    },
    PaletteEntry {
        label: "open help",
        action: Action::OpenHelp,
    },
    PaletteEntry {
        label: "open settings",
        action: Action::OpenSettings,
    },
    PaletteEntry {
        label: "open command palette",
        action: Action::OpenCommandPalette,
    },
    PaletteEntry {
        label: "show capture QR",
        action: Action::OpenShare,
    },
    PaletteEntry {
        label: "escape / clear",
        action: Action::EscapeStack,
    },
    PaletteEntry {
        label: "quit",
        action: Action::Quit,
    },
    PaletteEntry {
        label: "reschedule",
        action: Action::Reschedule,
    },
    PaletteEntry {
        label: "Change week start",
        action: Action::ChangeWeekStart,
    },
];

#[derive(Debug, Default, Clone)]
pub struct CommandPaletteState {
    /// Highlighted row in the *filtered* list. Reset to 0 whenever the user
    /// edits the search text so the highlight doesn't get stranded past the
    /// new result count.
    pub cursor: usize,
    /// Mode the user was in when they opened the palette. Restored on close
    /// so that opening the palette from Visual mode (with a selection) and
    /// cancelling — or running a visual-aware action like ToggleComplete —
    /// keeps the selection meaningful instead of silently dropping into
    /// Normal.
    prior_mode: Option<Mode>,
    /// Cached filter inputs and outputs. `refresh` recomputes only when the
    /// needle changes, so multiple call sites (Enter, Up/Down, render) can
    /// read `hits()` per frame without re-running the match each time.
    cached_needle: String,
    cached_hits: Vec<PaletteHit>,
}

impl CommandPaletteState {
    /// Snapshot the mode the palette is being opened from, reset the
    /// highlight, and seed the cache with the unfiltered list so the first
    /// frame doesn't have to fall through `refresh`.
    pub fn open(&mut self, prior: Mode) {
        self.cursor = 0;
        self.prior_mode = Some(prior);
        self.cached_needle.clear();
        self.cached_hits = filtered("");
    }

    /// Consume the snapshot taken in `open`. Defaults to `Normal` if the
    /// palette was somehow closed without a matching open — keeps the close
    /// path total instead of panicking.
    pub fn take_prior(&mut self) -> Mode {
        self.prior_mode.take().unwrap_or(Mode::Normal)
    }

    /// Read the snapshot without consuming it. Renderers use this to keep
    /// the underlying UI looking the same while the palette overlay is open.
    pub fn prior(&self) -> Option<Mode> {
        self.prior_mode
    }

    /// Currently visible hits, in rank order. Computed at most once per
    /// `refresh` call.
    pub fn hits(&self) -> &[PaletteHit] {
        &self.cached_hits
    }

    /// Recompute `hits` if `needle` differs from the cached needle and snap
    /// the highlight back to the top. Cheap no-op when the needle is the
    /// same (e.g., a cursor-move keystroke that doesn't change the draft).
    pub fn refresh(&mut self, needle: &str) {
        if self.cached_needle == needle {
            return;
        }
        self.cached_needle.clear();
        self.cached_needle.push_str(needle);
        self.cached_hits = filtered(needle);
        self.cursor = 0;
    }

    /// Move the highlight by `dir` rows with wrap-around. No-op when the
    /// filtered list is empty.
    pub fn step(&mut self, dir: i32) {
        let len = self.cached_hits.len();
        if len == 0 {
            return;
        }
        let cur = self.cursor.min(len - 1) as i32;
        let next = (cur + dir).rem_euclid(len as i32) as usize;
        self.cursor = next;
    }

    /// Action under the highlight, if any. Returns `None` when the filter
    /// produced no matches.
    pub fn current_action(&self) -> Option<Action> {
        let hit = self.cached_hits.get(self.cursor)?;
        ENTRIES.get(hit.entry_idx).map(|e| e.action)
    }
}

/// One filtered hit: the index into `ENTRIES`, plus the matched byte offsets
/// inside that entry's label (for highlighting).
#[derive(Debug, Clone)]
pub struct PaletteHit {
    pub entry_idx: usize,
    pub match_positions: Vec<usize>,
}

/// Filter `ENTRIES` against `needle`. Empty needle returns every entry in
/// declaration order. Non-empty needle returns only entries whose label
/// contains the needle as a case-insensitive subsequence, ranked by:
///   1. where the first match sits — byte 0 beats a word-boundary match beats
///      a mid-word match. ("arch" → "archive…" before "toggle archive…"
///      before "fuzzy search".)
///   2. tightness of the run (smaller span first).
///   3. declaration order, via the stable sort.
pub fn filtered(needle: &str) -> Vec<PaletteHit> {
    if needle.is_empty() {
        return ENTRIES
            .iter()
            .enumerate()
            .map(|(i, _)| PaletteHit {
                entry_idx: i,
                match_positions: Vec::new(),
            })
            .collect();
    }
    let mut hits: Vec<((u8, usize), PaletteHit)> = ENTRIES
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            subseq_match_ci(e.label, needle).map(|positions| {
                let rank = rank_hit(e.label, &positions);
                (
                    rank,
                    PaletteHit {
                        entry_idx: i,
                        match_positions: positions,
                    },
                )
            })
        })
        .collect();
    // Stable sort preserves declaration order within a rank tier.
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, h)| h).collect()
}

fn rank_hit(label: &str, positions: &[usize]) -> (u8, usize) {
    // `positions` is non-empty here: subseq_match_ci returns Some only for
    // a fully-matched needle, and the empty-needle case returns earlier.
    let first = positions[0];
    let last = positions[positions.len() - 1];
    let span = last - first;
    let start_tier = if first == 0 {
        0
    } else if is_word_boundary(label, first) {
        1
    } else {
        2
    };
    (start_tier, span)
}

/// True when `byte` is the start of a word: position 0, or the previous
/// char is non-alphanumeric (whitespace, punctuation, …). `byte` is assumed
/// to fall on a char boundary, which it does for every position returned by
/// `subseq_match_ci`.
fn is_word_boundary(s: &str, byte: usize) -> bool {
    if byte == 0 {
        return true;
    }
    s[..byte]
        .chars()
        .next_back()
        .is_none_or(|c| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_needle_returns_all_entries() {
        let hits = filtered("");
        assert_eq!(hits.len(), ENTRIES.len());
    }

    #[test]
    fn matches_subsequence_in_label() {
        let hits = filtered("arch");
        assert!(!hits.is_empty());
        // Every hit must contain 'a','r','c','h' (case-insensitive) in order.
        for h in &hits {
            let label = ENTRIES[h.entry_idx].label.to_lowercase();
            let mut needle = "arch".chars();
            let mut cur = needle.next();
            for ch in label.chars() {
                if Some(ch) == cur {
                    cur = needle.next();
                }
            }
            assert!(
                cur.is_none(),
                "label {:?} should match 'arch'",
                ENTRIES[h.entry_idx].label
            );
        }
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(filtered("zzzqqq").is_empty());
    }

    #[test]
    fn start_of_label_ranks_above_mid_label() {
        // "arch" appears at byte 0 of "archive completed tasks", after a
        // space in "toggle archive view", and mid-word in "fuzzy search".
        // Start-of-label must win.
        let hits = filtered("arch");
        let labels: Vec<&str> = hits.iter().map(|h| ENTRIES[h.entry_idx].label).collect();
        assert_eq!(labels.first().copied(), Some("archive completed tasks"));
    }

    #[test]
    fn word_boundary_ranks_above_mid_word() {
        let hits = filtered("arch");
        let labels: Vec<&str> = hits.iter().map(|h| ENTRIES[h.entry_idx].label).collect();
        let toggle = labels
            .iter()
            .position(|&l| l == "toggle archive view")
            .expect("toggle archive view in results");
        let fuzzy = labels
            .iter()
            .position(|&l| l == "fuzzy search")
            .expect("fuzzy search in results");
        assert!(
            toggle < fuzzy,
            "word-boundary match (toggle archive view) should rank above mid-word match (fuzzy search)"
        );
    }

    #[test]
    fn tighter_match_ranks_above_gappier_within_tier() {
        // Both labels start mid-word for needle "ye" (matches `y` then `e`).
        // "cycle theme":  y@2  e@5  → span 3
        // "cycle density": y@2 e@10 → span 8
        // Same start tier, so tightness decides.
        let hits = filtered("ye");
        let labels: Vec<&str> = hits.iter().map(|h| ENTRIES[h.entry_idx].label).collect();
        let theme = labels.iter().position(|&l| l == "cycle theme");
        let density = labels.iter().position(|&l| l == "cycle density");
        if let (Some(t), Some(d)) = (theme, density) {
            assert!(t < d, "tighter span should rank first within a tier");
        }
    }

    #[test]
    fn case_insensitive_match() {
        let upper = filtered("ARCH");
        let lower = filtered("arch");
        assert_eq!(upper.len(), lower.len());
        let upper_ids: Vec<usize> = upper.iter().map(|h| h.entry_idx).collect();
        let lower_ids: Vec<usize> = lower.iter().map(|h| h.entry_idx).collect();
        assert_eq!(upper_ids, lower_ids);
    }

    #[test]
    fn entries_cover_every_meaningful_action() {
        // `ArmF` is intentionally omitted (it's only a chord leader, not a
        // user-facing action). Every other Action variant must be reachable
        // from the palette.
        let actions: Vec<Action> = ENTRIES.iter().map(|e| e.action).collect();
        let required = [
            Action::Quit,
            Action::CursorDown,
            Action::CursorUp,
            Action::CursorTop,
            Action::CursorBottom,
            Action::HalfPageDown,
            Action::HalfPageUp,
            Action::BeginAdd,
            Action::BeginEdit,
            Action::BeginEditInsert,
            Action::ToggleComplete,
            Action::Delete,
            Action::CyclePriority,
            Action::MoveTaskDown,
            Action::MoveTaskUp,
            Action::BeginSearch,
            Action::OpenHelp,
            Action::OpenSettings,
            Action::OpenCommandPalette,
            Action::Undo,
            Action::ToggleVisual,
            Action::ToggleSelected,
            Action::GoList,
            Action::ToggleArchiveView,
            Action::ArchiveCompleted,
            Action::PickProject,
            Action::PickContext,
            Action::CycleSort,
            Action::BeginPromptProject,
            Action::BeginPromptContext,
            Action::ToggleLeftPane,
            Action::ToggleRightPane,
            Action::CycleTheme,
            Action::CycleDensity,
            Action::ToggleLineNum,
            Action::ToggleShowDone,
            Action::ToggleShowFuture,
            Action::CopyLine,
            Action::CopyBody,
            Action::EscapeStack,
            Action::OpenShare,
            Action::OpenThemePicker,
            Action::Reschedule,
        ];
        for a in required {
            assert!(actions.contains(&a), "missing palette entry for {a:?}");
        }
    }
}
