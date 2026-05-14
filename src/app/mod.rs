use std::cell::Cell;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::config::Config;
use crate::serve::{self, ShareInfo};
use crate::theme::Theme;
use crate::todo::{self, Task};

mod archive;
mod autocomplete;
mod bulk;
mod chord;
mod draft;
mod draft_overlay;
mod external;
mod flash;
mod history;
mod mutations;
pub mod palette;
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
pub use draft_overlay::{
    BuilderField, CalendarState, CalendarTarget, DraftOverlay, OverlayKind, PriorityChooserState,
    REC_UNIT_ORDER, RecurrenceBuilderState, SLASH_ENTRIES, SlashEntry, SlashKind, SlashMenuState,
    format_rec_value, recurrence_next_preview,
};
pub use flash::Flash;
pub use history::History;
pub use palette::CommandPaletteState;
pub use prefs::{Layout, Prefs};
pub use selection::Selection;
pub use types::{
    AUTOCOMPLETE_CAP, AddOutcome, Density, FLASH_TTL, Filter, LEADER_WINDOW, Mode, Sort,
    UNDO_LIMIT, View,
};
pub use visibility::{GroupKey, ListDueBucket, ordered_unique};

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
    pub(crate) view_cursor: [usize; 2],
    /// Crate-private: same reason as `view` — `visible_cache` would drift.
    /// Read via `filter()`; mutate via `set_search`/`set_project`/etc.
    pub(crate) filter: Filter,
    pub draft: DraftState,
    pub selection: Selection,
    history: History,
    flash_state: Flash,
    pub chord: Chord,
    pub file_path: PathBuf,
    /// Resolved path of the on-disk config file. Set by the binary after
    /// construction so the settings overlay can render a stable, real path
    /// without the renderer having to reach into the environment itself.
    /// `None` in tests/examples that don't care about the value.
    pub config_path: Option<PathBuf>,
    pub today: String,
    pub should_quit: bool,
    visible_cache: Vec<usize>,
    /// Parallel to `visible_cache`: `visible_groups[i]` is the group key for
    /// the row at `visible_cache[i]`. `GroupKey::None` for List under
    /// `Sort::File`; priority/due bucket keys under other List sorts; date
    /// keys for Archive. Renderers read this to draw section headers.
    visible_groups: Vec<crate::app::visibility::GroupKey>,
    /// Snapshot of the file body the last time we read or wrote it.
    /// Used by `check_external_changes` to detect edits made outside the TUI.
    last_disk: String,
    /// Sibling `done.txt`. Holds tasks the user has archived; populated
    /// off-thread at startup so the first frame doesn't wait on this I/O.
    pub archive: Archive,
    /// Latest known release tag, populated asynchronously by the update
    /// checker. `None` while we haven't heard back (or the check is disabled,
    /// e.g. in tests). The UI compares this against `CARGO_PKG_VERSION` to
    /// decide whether to surface an "update available" hint.
    pub(crate) latest_version: Option<String>,
    /// Receiver for the background update check. Drained each tick; cleared
    /// once a result has been received or the sender hung up.
    update_check: Option<Receiver<Option<String>>>,
    pub command_palette: CommandPaletteState,
    /// Vertical scroll offset (rows from the top of the line list) for each
    /// view, keyed by `View::idx()`. Updated at render time via `Cell` so the
    /// renderer can keep the cursor row visible without taking `&mut self`.
    pub(crate) view_scroll: [Cell<u16>; 2],
    /// Handle to the in-TUI capture server. `None` until the first time
    /// the user presses `s` (or invokes "show capture QR" from the
    /// palette). Once bound, the entry stays for the rest of the
    /// session and the overlay just re-displays the saved QR.
    share: Option<ShareInfo>,
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
            view_cursor: [0; 2],
            filter: Filter::default(),
            draft: DraftState::default(),
            selection: Selection::default(),
            history: History::default(),
            flash_state: Flash::default(),
            chord: Chord::default(),
            file_path,
            config_path: None,
            today,
            should_quit: false,
            visible_cache: Vec::new(),
            visible_groups: Vec::new(),
            last_disk: body,
            archive,
            latest_version: None,
            update_check: None,
            command_palette: CommandPaletteState::default(),
            view_scroll: [Cell::new(0), Cell::new(0)],
            share: None,
        };
        app.recompute_visible();
        app
    }

    /// Idempotent: bind the capture server on first call, then store
    /// the [`ShareInfo`] so subsequent calls just re-show the overlay.
    ///
    /// First-call behavior: if the user has a persisted token + port in
    /// config, reuse them so phone bookmarks survive across sessions.
    /// Otherwise, generate a fresh token, let the OS pick a port, and
    /// write both back to the config. If the persisted port is taken
    /// (another tuxedo instance on the same machine, say), fall back to
    /// an OS-assigned port and rewrite the config so the next session
    /// starts fresh.
    pub fn ensure_share_started(&mut self) -> Result<&ShareInfo, String> {
        // Two-step to dodge stable's lack of Polonius: do the bind work
        // first (which needs `&mut self`), then the unconditional final
        // borrow returns the now-present `ShareInfo`.
        if self.share.is_none() {
            let info = self.bind_share()?;
            self.share = Some(info);
        }
        Ok(self
            .share
            .as_ref()
            .expect("share is Some after the bind branch"))
    }

    fn bind_share(&mut self) -> Result<ShareInfo, String> {
        let cfg = Config::load();
        let token = match cfg.share_token {
            Some(t) => t,
            None => serve::net::generate_token().map_err(|e| format!("token: {e}"))?,
        };
        let requested_port = cfg.share_port.unwrap_or(0);
        let info = match serve::start(self.file_path.clone(), token.clone(), requested_port) {
            Ok(info) => info,
            Err(_) if requested_port != 0 => {
                // Configured port is taken — fall back to OS-assigned.
                serve::start(self.file_path.clone(), token.clone(), 0)
                    .map_err(|e| format!("bind: {e}"))?
            }
            Err(e) => return Err(format!("bind: {e}")),
        };
        // Persist token + port back to config so phone bookmarks survive.
        // Load fresh first so we don't clobber any prefs the user has
        // toggled since this App was constructed.
        let mut to_save = Config::load();
        to_save.share_token = Some(info.token.clone());
        to_save.share_port = Some(info.port);
        if let Err(e) = to_save.save() {
            self.flash(format!("share config save failed: {e}"));
        }
        Ok(info)
    }

    pub fn share_info(&self) -> Option<&ShareInfo> {
        self.share.as_ref()
    }

    /// Install the receiver from [`update::spawn_check`](crate::update::spawn_check).
    /// `main` calls this; tests leave it unset so the App stays inert and
    /// doesn't spawn network work as a side effect of construction.
    pub fn set_update_check(&mut self, rx: Receiver<Option<String>>) {
        self.update_check = Some(rx);
    }

    /// Drain the update-check receiver. Returns `true` if a new latest
    /// version was just received — callers use this to trigger a redraw so
    /// the status bar can paint the indicator on the next frame.
    pub fn poll_update_check(&mut self) -> bool {
        let Some(rx) = &self.update_check else {
            return false;
        };
        match rx.try_recv() {
            Ok(maybe_tag) => {
                self.latest_version = maybe_tag;
                self.update_check = None;
                self.latest_version.is_some()
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.update_check = None;
                false
            }
        }
    }

    /// Returns the latest known release tag *if* it is strictly newer than
    /// the running binary. The status bar uses this to decide whether to
    /// draw an "update available" hint.
    pub fn update_available(&self) -> Option<&str> {
        let latest = self.latest_version.as_deref()?;
        if crate::update::is_newer(latest, env!("CARGO_PKG_VERSION")) {
            Some(latest)
        } else {
            None
        }
    }

    pub fn theme(&self) -> &'static Theme {
        self.prefs.theme()
    }

    /// Mode the rest of the UI should react to. While the command palette is
    /// open, the underlying list/sidebars should keep rendering as if the
    /// user were still in the mode they came from — otherwise opening the
    /// palette mid-Visual hides the multi-select checkboxes and similar
    /// mode-driven affordances.
    pub fn effective_mode(&self) -> Mode {
        match self.mode {
            Mode::CommandPalette => self.command_palette.prior().unwrap_or(self.mode),
            m => m,
        }
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

    /// Active top-level view (List/Archive).
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

    /// Update the cached "today" string. When it changes, the visible cache
    /// is recomputed so threshold-hidden tasks become visible the moment the
    /// wall clock crosses midnight (without requiring an app restart).
    /// Returns `true` iff the date actually advanced — callers use this to
    /// trigger a redraw.
    pub fn refresh_today(&mut self, now: String) -> bool {
        if self.today == now {
            return false;
        }
        self.today = now;
        self.recompute_visible();
        true
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
