use std::fmt;
use std::str::FromStr;
use std::time::Duration;

pub const LEADER_WINDOW: Duration = Duration::from_millis(600);
pub const FLASH_TTL: Duration = Duration::from_millis(1400);
pub const UNDO_LIMIT: usize = 50;
pub const AUTOCOMPLETE_CAP: usize = 8;

/// Outcome of `add_from_draft`. The Enter handler in `main.rs` uses this to
/// decide whether to exit Insert mode: `Parsed` means the NL pre-pass
/// rewrote the buffer but did not save, so the user should stay in Insert
/// to review/edit before pressing Enter a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Saved,
    Parsed,
    Empty,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Visual,
    Help,
    Settings,
    PromptProject,       // text input → add project on current task
    PromptContext,       // text input → add/remove context on current task
    PromptRenameProject, // text input → rename all project occurrences on current project in project list
    PromptRenameContext, // text input → rename all context occurrences on current context in context list
    PickProject,         // j/k cycles through projects to filter by
    PickContext,         // j/k cycles through contexts to filter by
    PickSavedFilter,     // j/k cycles through saved searches to apply
    PromptSaveFilter,    // text input → name the current search and save it
    CommandPalette,
    /// QR + URL overlay for the in-TUI capture server. Any key
    /// dismisses; press `s` again to re-open without rebinding (the
    /// server stays running once started).
    Share,
    /// Theme picker dialog — j/k to preview themes, Enter to accept,
    /// Esc to revert.
    PickTheme,
    /// First-run welcome prompt, shown when `tuxedo` is launched with no
    /// target and no `./todo.txt` exists. `c` creates `./todo.txt`, `s`
    /// opens the bundled sample, `q`/`Esc` quits without creating anything.
    Welcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Archive,
}

impl View {
    /// Stable slot index for keying per-view state arrays. Don't reorder the
    /// `View` variants without updating this together.
    pub fn idx(self) -> usize {
        match self {
            View::List => 0,
            View::Archive => 1,
        }
    }
}

/// A project or context a mouse click landed on, in the sidebar's filter
/// list or the detail pane's `+project`/`@context` tokens. Both surfaces
/// resolve a click to one of these and hand it to the same toggle logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterTarget {
    Project(String),
    Context(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Priority,
    Due,
    File,
}

impl Sort {
    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Priority => "priority",
            Sort::Due => "due",
            Sort::File => "file",
        }
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Sort {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "priority" => Ok(Sort::Priority),
            "due" => Ok(Sort::Due),
            "file" => Ok(Sort::File),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Compact,
    Comfortable,
    Cozy,
}

impl Density {
    pub fn as_str(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Cozy => "cozy",
        }
    }
}

impl fmt::Display for Density {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Density {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "compact" => Ok(Density::Compact),
            "comfortable" => Ok(Density::Comfortable),
            "cozy" => Ok(Density::Cozy),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub project: Option<String>,
    pub context: Option<String>,
    pub search: String,
}

impl Filter {
    /// True when at least one of project / context / search is non-empty.
    pub fn has_any(&self) -> bool {
        self.project.is_some() || self.context.is_some() || !self.search.is_empty()
    }

    /// The active `+project` / `@context` tags as an add-prompt prefix, with
    /// a trailing space; empty when neither is set. A task added under a
    /// filter that doesn't carry its tags drops out of the view the moment it
    /// saves. `search` contributes nothing — it is a needle, not a tag.
    pub fn tag_seed(&self) -> String {
        let project = self.project.as_deref().map(|p| format!("+{p} "));
        let context = self.context.as_deref().map(|c| format!("@{c} "));
        project.unwrap_or_default() + &context.unwrap_or_default()
    }

    /// Drop every filter component back to its empty state.
    pub fn clear(&mut self) {
        self.project = None;
        self.context = None;
        self.search.clear();
    }
}

/// A user-named saved search. `query` is a `/`-search needle (case-insensitive
/// subsequence match on the task body), recalled via the `ff` picker and
/// persisted as a `filter.<name> = <query>` line in the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFilter {
    pub name: String,
    pub query: String,
}

#[cfg(test)]
mod tests {
    use super::Filter;

    #[test]
    fn tag_seed_is_empty_without_project_or_context() {
        let filter = Filter {
            search: "milk".to_string(),
            ..Filter::default()
        };
        assert_eq!(filter.tag_seed(), "", "a search needle is not a tag");
    }

    #[test]
    fn tag_seed_leads_with_project_then_context() {
        let filter = Filter {
            project: Some("work".to_string()),
            context: Some("home".to_string()),
            search: String::new(),
        };
        // Trailing space: the seed is a prefix the body gets typed after.
        assert_eq!(filter.tag_seed(), "+work @home ");
    }

    #[test]
    fn tag_seed_covers_a_single_active_filter() {
        let filter = Filter {
            context: Some("home".to_string()),
            ..Filter::default()
        };
        assert_eq!(filter.tag_seed(), "@home ");
    }
}
