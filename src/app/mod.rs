use std::path::PathBuf;

use crate::config::Config;
use crate::theme::Theme;
use crate::todo::{self, Task};

mod archive;
mod autocomplete;
mod bulk;
mod chord;
mod draft;
mod external;
mod flash;
mod history;
mod mutations;
mod picker;
mod prefs;
mod selection;
mod types;
mod visibility;

#[cfg(test)]
pub(crate) mod test_support;

pub use archive::Archive;
pub use autocomplete::{ActiveToken, TokenKind, active_token};
pub use chord::Chord;
pub use draft::{DraftCursor, DraftState};
pub use flash::Flash;
pub use history::History;
pub use prefs::{Layout, Prefs};
pub use selection::Selection;
pub use types::{
    AUTOCOMPLETE_CAP, Density, FLASH_TTL, Filter, LEADER_WINDOW, Mode, Sort, UNDO_LIMIT, View,
};
pub use visibility::{GroupKey, TodayBucket, ordered_unique};

pub struct App {
    /// Crate-private: external mutation would bypass `push_history`,
    /// `persist`, and `recompute_visible`. Read via `tasks()` / `task_raw()`;
    /// mutate via the methods on `App` (add_from_draft, complete, delete, …).
    pub(crate) tasks: Vec<Task>,
    /// Crate-private: writing here would not invalidate `visible_cache`.
    /// Read via `view()`; mutate via `set_view()`.
    pub(crate) view: View,
    pub mode: Mode,
    pub prefs: Prefs,
    pub cursor: usize,
    /// Per-view saved cursor, indexed by `View::idx()`. `set_view` snapshots
    /// the outgoing view's cursor here and restores the incoming view's, so
    /// each view remembers where the user last was.
    pub(crate) view_cursor: [usize; 3],
    /// Crate-private: same reason as `view` — `visible_cache` would drift.
    /// Read via `filter()`; mutate via `set_search`/`set_project`/etc.
    pub(crate) filter: Filter,
    pub draft: DraftState,
    pub selection: Selection,
    history: History,
    flash_state: Flash,
    pub chord: Chord,
    pub file_path: PathBuf,
    pub today: String,
    pub should_quit: bool,
    visible_cache: Vec<usize>,
    /// Parallel to `visible_cache`: `visible_groups[i]` is the group key for
    /// the row at `visible_cache[i]`. `GroupKey::None` for List; bucket/date
    /// keys for Today/Archive. Renderers read this to draw section headers.
    visible_groups: Vec<crate::app::visibility::GroupKey>,
    /// Snapshot of the file body the last time we read or wrote it.
    /// Used by `check_external_changes` to detect edits made outside the TUI.
    last_disk: String,
    /// Sibling `done.txt`. Holds tasks the user has archived; populated
    /// off-thread at startup so the first frame doesn't wait on this I/O.
    pub archive: Archive,
}

impl App {
    pub fn new(file_path: PathBuf, body: String, today: String, cfg: Config) -> Self {
        let tasks = todo::parse_file(&body);
        let archive = Archive::spawn(&file_path);
        let mut app = Self {
            tasks,
            view: View::List,
            mode: Mode::Normal,
            prefs: Prefs::from_config(cfg),
            cursor: 0,
            view_cursor: [0; 3],
            filter: Filter::default(),
            draft: DraftState::default(),
            selection: Selection::default(),
            history: History::default(),
            flash_state: Flash::default(),
            chord: Chord::default(),
            file_path,
            today,
            should_quit: false,
            visible_cache: Vec::new(),
            visible_groups: Vec::new(),
            last_disk: body,
            archive,
        };
        app.recompute_visible();
        app
    }

    pub fn theme(&self) -> &'static Theme {
        self.prefs.theme()
    }

    pub fn sort_label(&self) -> &'static str {
        self.prefs.sort_label()
    }

    /// Persist preferences. On failure, flashes a short error so the user
    /// sees the problem inside the TUI (writing to stderr would smash the
    /// alt-screen).
    pub fn save_prefs(&mut self) {
        if let Err(e) = self.prefs.save() {
            self.flash(format!("config save failed: {e}"));
        }
    }

    pub fn cycle_theme(&mut self) {
        let msg = self.prefs.cycle_theme();
        self.flash(msg);
        self.save_prefs();
    }

    pub fn cycle_density(&mut self) {
        let msg = self.prefs.cycle_density();
        self.flash(msg);
        self.save_prefs();
    }

    pub fn cycle_sort(&mut self) {
        let msg = self.prefs.cycle_sort();
        self.flash(msg);
        self.recompute_visible();
        self.save_prefs();
    }

    /// Read-only view of the parsed task list. Mutations go through
    /// dedicated methods so history/persist/visible-cache stay coherent.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// True when at least one task is marked done. Used by the binary to
    /// decide whether `A` archives or just toggles the archive view.
    pub fn has_completed_tasks(&self) -> bool {
        self.tasks.iter().any(|t| t.done)
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    /// Returning an owned `String` so the caller can hold it across `&mut self`
    /// calls (the common shape for "load draft from current task").
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.tasks.get(abs).map(|t| t.raw.clone())
    }

    /// Task under the cursor, resolved against the active view's source:
    /// `archive.tasks()` in Archive view, `tasks` otherwise.
    pub fn cur_task(&self) -> Option<&Task> {
        let i = self.cur_abs()?;
        match self.view {
            View::Archive => self.archive.tasks().get(i),
            _ => self.tasks.get(i),
        }
    }

    /// Index of the task under the cursor *into `self.tasks`*. Returns `None`
    /// in Archive view because the cursor there points into `archive.tasks()`.
    /// Use this — not `cur_abs()` — for any write that mutates `self.tasks`.
    pub fn cur_task_index_in_tasks(&self) -> Option<usize> {
        if matches!(self.view, View::Archive) {
            return None;
        }
        self.cur_abs()
    }

    /// Read-only view of the active filter.
    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Active top-level view (List/Today/Archive).
    pub fn view(&self) -> View {
        self.view
    }

    /// Switch top-level view. Recomputes the cache so the next frame reflects
    /// the change, snapshots the outgoing view's cursor, and restores the
    /// incoming view's saved cursor (clamped to the new visible length).
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view_cursor[self.view.idx()] = self.cursor;
        self.view = view;
        self.recompute_visible();
        self.cursor = self.view_cursor[view.idx()];
        self.clamp_cursor();
    }

    /// Set the search-filter text. Cursor resets and the cache is recomputed.
    /// Typing into the search prompt calls this on every keystroke.
    pub fn set_search(&mut self, search: String) {
        self.filter.search = search;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Clear just the search component of the filter.
    pub fn clear_search(&mut self) {
        if self.filter.search.is_empty() {
            return;
        }
        self.filter.search.clear();
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Set or clear the active project filter. `None` removes it.
    pub fn set_project_filter(&mut self, project: Option<String>) {
        self.filter.project = project;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Set or clear the active context filter. `None` removes it.
    pub fn set_context_filter(&mut self, context: Option<String>) {
        self.filter.context = context;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Drop every filter component (project + context + search).
    pub fn clear_filter(&mut self) {
        if !self.filter.has_any() {
            return;
        }
        self.filter.clear();
        self.cursor = 0;
        self.recompute_visible();
    }
}
