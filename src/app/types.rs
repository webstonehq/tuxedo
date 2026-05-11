use std::fmt;
use std::str::FromStr;
use std::time::Duration;

pub const LEADER_WINDOW: Duration = Duration::from_millis(600);
pub const FLASH_TTL: Duration = Duration::from_millis(1400);
pub const UNDO_LIMIT: usize = 50;
pub const AUTOCOMPLETE_CAP: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Visual,
    Help,
    Settings,
    PromptProject, // text input → add project on current task
    PromptContext, // text input → add/remove context on current task
    PickProject,   // j/k cycles through projects to filter by
    PickContext,   // j/k cycles through contexts to filter by
    CommandPalette,
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

    /// Drop every filter component back to its empty state.
    pub fn clear(&mut self) {
        self.project = None;
        self.context = None;
        self.search.clear();
    }
}
