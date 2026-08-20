use core::fmt;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError};

use ratatui::layout::Rect;

use crate::config::Config;
use crate::core::Store;
use crate::core::outcome::{DrainReport, Reconcile};
use crate::note;
use crate::serve::{self, ShareInfo};
use crate::theme::{self, Theme};
use crate::todo::Task;

mod autocomplete;
mod bulk;
mod chord;
mod draft;
mod draft_overlay;
mod flash;
mod mutations;
pub mod palette;
mod picker;
mod prefs;
mod saved;
mod selection;
mod types;
mod visibility;

#[cfg(test)]
pub(crate) mod test_support;

pub use crate::core::Archive;
pub use crate::core::History;
pub use crate::core::filter::{ListDueBucket, ordered_unique};
pub use autocomplete::{ActiveToken, AutocompleteTarget, TokenKind, active_token};
pub use chord::Chord;
pub use draft::{DialogInputMode, DraftCursor, DraftState};
pub use draft_overlay::{
    BuilderField, CalendarState, CalendarTarget, DraftOverlay, OverlayKind, PriorityChooserState,
    REC_UNIT_ORDER, RecurrenceBuilderState, SLASH_ENTRIES, SlashEntry, SlashKind, SlashMenuState,
    format_rec_value, recurrence_next_preview,
};
pub use flash::Flash;
pub use palette::CommandPaletteState;
pub use prefs::{Layout, Prefs};
pub use selection::Selection;
pub use types::{
    AUTOCOMPLETE_CAP, AddOutcome, Density, FLASH_TTL, Filter, FilterTarget, LEADER_WINDOW, Mode,
    SavedFilter, Sort, UNDO_LIMIT, View,
};
pub use visibility::GroupKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekStart {
    Sunday,
    Monday,
}

impl WeekStart {
    pub fn as_str(self) -> &'static str {
        match self {
            WeekStart::Sunday => "sunday",
            WeekStart::Monday => "monday",
        }
    }
}

impl fmt::Display for WeekStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WeekStart {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sunday" => Ok(WeekStart::Sunday),
            "monday" => Ok(WeekStart::Monday),
            _ => Err(()),
        }
    }
}

pub struct App {
    /// The headless durable store: tasks, archive, history, persistence, and
    /// `today`. Mutate via the methods on `App` (which map store outcomes to
    /// flash messages and refresh the visible cache); read via `tasks()`,
    /// `archive()`, `task_raw()`, etc.
    pub(crate) store: Store,
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
    flash_state: Flash,
    pub chord: Chord,
    pub file_path: PathBuf,
    /// Resolved path of the on-disk config file. Set by the binary after
    /// construction so the settings overlay can render a stable, real path
    /// without the renderer having to reach into the environment itself.
    /// `None` in tests/examples that don't care about the value.
    pub config_path: Option<PathBuf>,
    pub should_quit: bool,
    visible_cache: Vec<usize>,
    /// Parallel to `visible_cache`: `visible_groups[i]` is the group key for
    /// the row at `visible_cache[i]`. `GroupKey::None` for List under
    /// `Sort::File`; priority/due bucket keys under other List sorts; date
    /// keys for Archive. Renderers read this to draw section headers.
    visible_groups: Vec<crate::app::visibility::GroupKey>,
    /// Latest known release tag, populated asynchronously by the update
    /// checker. `None` while we haven't heard back (or the check is disabled,
    /// e.g. in tests). The UI compares this against `CARGO_PKG_VERSION` to
    /// decide whether to surface an "update available" hint.
    pub(crate) latest_version: Option<String>,
    /// Receiver for the background update check. Drained each tick; cleared
    /// once a result has been received or the sender hung up.
    update_check: Option<Receiver<Option<String>>>,
    /// User-named saved searches, loaded from config at startup and
    /// upserted via `fs`. Recalled with the `ff` picker.
    pub saved_filters: Vec<SavedFilter>,
    /// The search string that was active when the `ff` picker opened, so
    /// cancelling (`Esc`) restores it instead of leaving the previewed
    /// filter applied. `None` outside `Mode::PickSavedFilter`.
    saved_pick_restore: Option<String>,
    /// Index into `saved_filters` of the row the `ff` picker currently
    /// previews. Tracked explicitly rather than re-derived from
    /// `filter.search` so duplicate queries don't strand j/k. Only
    /// meaningful while `Mode::PickSavedFilter`; re-seeded on each open.
    saved_pick_idx: usize,
    pub command_palette: CommandPaletteState,
    /// Vertical scroll offset (rows from the top of the line list) for each
    /// view, keyed by `View::idx()`. Updated at render time via `Cell` so the
    /// renderer can keep the cursor row visible without taking `&mut self`.
    pub(crate) view_scroll: [Cell<u16>; 2],
    /// Screen rect of the last-rendered task-list body for the view at
    /// `View::idx()`. Set at render time via `Cell` (same pattern as
    /// `view_scroll`) so a later mouse click can be tested against where
    /// rows actually landed on screen.
    pub(crate) view_body_rect: [Cell<Rect>; 2],
    /// Sparse `(rendered line, cursor index)` pairs recorded for the last
    /// render of each view, keyed by `View::idx()`. Only lines that are
    /// task rows appear — blank/group-header lines are absent. Mouse click
    /// handling looks up the clicked line here to find which task it hit.
    pub(crate) view_row_index: [RefCell<Vec<(usize, usize)>>; 2],
    /// Screen rect of the last-rendered left sidebar (filters list), for
    /// mapping a click back to the project/context row it landed on.
    pub(crate) filters_rect: Cell<Rect>,
    /// Sparse `(rendered line, target)` pairs for the sidebar's filter rows
    /// — only project/context row lines appear. The sidebar doesn't scroll,
    /// so a line index is directly a row offset from `filters_rect.y`.
    pub(crate) filters_row_index: RefCell<Vec<(usize, FilterTarget)>>,
    /// Screen rect of the last-rendered detail pane (right sidebar).
    pub(crate) detail_rect: Cell<Rect>,
    /// `(line, col_start, col_end, target)` for each `+project`/`@context`
    /// token in the detail pane's last render, so a click on the token text
    /// (not just its row) can be resolved back to a filter target.
    pub(crate) detail_token_index: RefCell<Vec<(usize, u16, u16, FilterTarget)>>,
    /// Handle to the in-TUI capture server. `None` until the first time
    /// the user presses `s` (or invokes "show capture QR" from the
    /// palette). Once bound, the entry stays for the rest of the
    /// session and the overlay just re-displays the saved QR.
    share: Option<ShareInfo>,
    /// Base directory used by note actions. Relative `note:<path>` tokens are
    /// resolved under this directory, and generated notes are created below it.
    pub(crate) notes_dir: PathBuf,
    /// Path queued for opening in the user's editor after the TUI temporarily
    /// restores the terminal. Set by OpenNote and drained by the run loop.
    pending_editor_path: Option<PathBuf>,
    /// Theme index captured when the theme picker opened, so cancel
    /// can restore it.
    theme_pick_orig: usize,
    pub week_start: WeekStart,
}

impl App {
    /// Construct an App whose archive is the sibling `done.txt` of `file_path`.
    pub fn new(file_path: PathBuf, body: String, today: String, cfg: Config) -> Self {
        let store = Store::new(file_path.clone(), body, today);
        Self::from_store(store, file_path, cfg)
    }

    /// Like [`App::new`] but with an explicit `done.txt` path (e.g. `DONE_FILE`).
    pub fn new_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
        cfg: Config,
    ) -> Self {
        let store = Store::new_with_done(file_path.clone(), done_path, body, today);
        Self::from_store(store, file_path, cfg)
    }

    fn from_store(store: Store, file_path: PathBuf, cfg: Config) -> Self {
        // Read saved filters before `cfg` is moved into `Prefs::from_config`.
        let note_dir = note::notes_dir_from_config(cfg.notes_dir.as_deref());
        let saved_filters = cfg
            .filters
            .iter()
            .map(|(name, query)| SavedFilter {
                name: name.clone(),
                query: query.clone(),
            })
            .collect();
        let mut app = Self {
            store,
            view: View::List,
            mode: Mode::Normal,
            prefs: Prefs::from_config(cfg),
            cursor: 0,
            view_cursor: [0; 2],
            filter: Filter::default(),
            draft: DraftState::default(),
            selection: Selection::default(),
            flash_state: Flash::default(),
            chord: Chord::default(),
            file_path,
            config_path: None,
            should_quit: false,
            visible_cache: Vec::new(),
            visible_groups: Vec::new(),
            latest_version: None,
            update_check: None,
            saved_filters,
            saved_pick_restore: None,
            saved_pick_idx: 0,
            command_palette: CommandPaletteState::default(),
            view_scroll: [Cell::new(0), Cell::new(0)],
            view_body_rect: [Cell::new(Rect::default()), Cell::new(Rect::default())],
            view_row_index: [RefCell::new(Vec::new()), RefCell::new(Vec::new())],
            filters_rect: Cell::new(Rect::default()),
            filters_row_index: RefCell::new(Vec::new()),
            detail_rect: Cell::new(Rect::default()),
            detail_token_index: RefCell::new(Vec::new()),
            share: None,
            notes_dir: note_dir,
            pending_editor_path: None,
            theme_pick_orig: 0,
            week_start: WeekStart::Sunday,
        };
        app.recompute_visible();
        app
    }

    /// Rebind the App to a different on-disk file at runtime, replacing the
    /// store (tasks, archive, history, external-change baseline) with a fresh
    /// one for `file_path`/`done_path` loaded from `body`. Prefs, saved
    /// filters, theme, and config live on `App` and are left intact. Used by
    /// the first-run welcome prompt to swap from the placeholder file to the
    /// chosen one. Resets the cursor and recomputes the visible cache.
    pub fn open_file(&mut self, file_path: PathBuf, done_path: PathBuf, body: String) {
        let today = self.store.today().to_string();
        self.store = Store::new_with_done(file_path.clone(), done_path, body, today);
        self.file_path = file_path;
        self.cursor = 0;
        self.recompute_visible();
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

    /// Enter theme picker mode. Snapshot the current theme index so
    /// cancel can restore it. j/k/T live-previews; Enter accepts; Esc
    /// reverts.
    pub fn enter_pick_theme(&mut self) {
        self.theme_pick_orig = self.prefs.theme_idx();
        self.mode = Mode::PickTheme;
    }

    /// Step through themes in `forward` (true = next) direction with
    /// wrap-around. The theme is applied immediately for live preview.
    pub fn pick_theme_step(&mut self, forward: bool) {
        let all = theme::all();
        let len = all.len();
        if len <= 1 {
            return;
        }
        let cur = self.prefs.theme_idx();
        let next = if forward {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        self.prefs.set_theme_idx(next);
    }

    /// Accept the previewed theme and persist to config.
    pub fn pick_theme_accept(&mut self) {
        self.mode = Mode::Normal;
        self.save_prefs();
        self.flash(format!("theme: {}", self.theme().name));
    }

    /// Cancel the picker and restore the theme that was active when
    /// the picker opened.
    pub fn pick_theme_cancel(&mut self) {
        self.prefs.set_theme_idx(self.theme_pick_orig);
        self.mode = Mode::Normal;
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
        self.store.tasks()
    }

    /// Read-only view of the archived (`done.txt`) tasks.
    pub fn archive(&self) -> &Archive {
        self.store.archive()
    }

    /// The cached "today" (ISO `YYYY-MM-DD`) the store resolves dates against.
    pub fn today(&self) -> &str {
        self.store.today()
    }

    pub fn queue_editor_path(&mut self, path: PathBuf) {
        self.pending_editor_path = Some(path);
    }

    pub fn notes_dir(&self) -> &PathBuf {
        &self.notes_dir
    }

    pub fn take_pending_editor_path(&mut self) -> Option<PathBuf> {
        self.pending_editor_path.take()
    }

    /// True when at least one task is marked done. Used by the binary to
    /// decide whether `A` archives or just toggles the archive view.
    pub fn has_completed_tasks(&self) -> bool {
        self.store.has_completed()
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    /// Returning an owned `String` so the caller can hold it across `&mut self`
    /// calls (the common shape for "load draft from current task").
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.store.task_raw(abs)
    }

    /// Task under the cursor, resolved against the active view's source:
    /// `archive.tasks()` in Archive view, `tasks` otherwise.
    pub fn cur_task(&self) -> Option<&Task> {
        let i = self.cur_abs()?;
        match self.view {
            View::Archive => self.store.archive().tasks().get(i),
            _ => self.store.tasks().get(i),
        }
    }

    /// Pump archive state (startup loader + external `done.txt` edits). Returns
    /// true when the visible archive changed, so the caller redraws. Refreshes
    /// the visible cache when the Archive view is active.
    pub fn poll_archive(&mut self) -> bool {
        let changed = self.store.poll_archive();
        if changed && matches!(self.view, View::Archive) {
            self.recompute_visible();
            self.clamp_cursor();
        }
        changed
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

    /// Map a terminal cell to the visible-list cursor index it landed on,
    /// using the body rect and line map captured during the current view's
    /// last render. `None` when the cell falls outside the list body or on a
    /// blank/group-header line.
    pub fn row_at(&self, col: u16, row: u16) -> Option<usize> {
        let idx = self.view.idx();
        let rect = self.view_body_rect[idx].get();
        if col < rect.x || col >= rect.x + rect.width || row < rect.y || row >= rect.y + rect.height
        {
            return None;
        }
        let scroll = self.view_scroll[idx].get() as usize;
        let line = scroll + (row - rect.y) as usize;
        self.view_row_index[idx]
            .borrow()
            .iter()
            .find(|&&(l, _)| l == line)
            .map(|&(_, i)| i)
    }

    /// Map a terminal cell to the project/context row it landed on in the
    /// left sidebar's filter list, using the rect and line map captured
    /// during the sidebar's last render. `None` outside the sidebar body or
    /// on a non-row line (section header, blank, hint).
    pub fn filter_target_at(&self, col: u16, row: u16) -> Option<FilterTarget> {
        let rect = self.filters_rect.get();
        if col < rect.x || col >= rect.x + rect.width || row < rect.y || row >= rect.y + rect.height
        {
            return None;
        }
        // The sidebar renders as one unscrolled Paragraph, so the line index
        // is just the row offset from the rect's top.
        let line = (row - rect.y) as usize;
        self.filters_row_index
            .borrow()
            .iter()
            .find(|(l, _)| *l == line)
            .map(|(_, target)| target.clone())
    }

    /// Map a terminal cell to the `+project`/`@context` token it landed on
    /// in the detail pane, using the token map captured during the pane's
    /// last render. `None` outside the pane or between/outside token spans.
    pub fn detail_target_at(&self, col: u16, row: u16) -> Option<FilterTarget> {
        let rect = self.detail_rect.get();
        if col < rect.x || col >= rect.x + rect.width || row < rect.y || row >= rect.y + rect.height
        {
            return None;
        }
        let line = (row - rect.y) as usize;
        let rel_col = col - rect.x;
        self.detail_token_index
            .borrow()
            .iter()
            .find(|(l, start, end, _)| *l == line && rel_col >= *start && rel_col < *end)
            .map(|(_, _, _, target)| target.clone())
    }

    /// Screen rect of the last-rendered detail pane, if it was on screen.
    pub fn detail_rect(&self) -> Rect {
        self.detail_rect.get()
    }

    /// Apply `target`'s filter if it isn't already active, otherwise clear
    /// it.The toggle behind both sidebar and detail-pane filter clicks.
    pub fn toggle_filter_target(&mut self, target: FilterTarget) {
        match target {
            FilterTarget::Project(name) => {
                if self.filter.project.as_deref() == Some(name.as_str()) {
                    self.set_project_filter(None);
                } else {
                    self.set_project_filter(Some(name));
                }
            }
            FilterTarget::Context(name) => {
                if self.filter.context.as_deref() == Some(name.as_str()) {
                    self.set_context_filter(None);
                } else {
                    self.set_context_filter(Some(name));
                }
            }
        }
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
        if self.store.set_today(now) {
            self.recompute_visible();
            true
        } else {
            false
        }
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

    // ---- shared helpers for the mutation wrappers -----------------------

    /// After a successful mutation that returned an absolute task index,
    /// rebuild the visible cache and move the cursor to follow that task.
    pub(crate) fn after_mutation(&mut self, follow_abs: usize) {
        self.recompute_visible();
        self.follow_cursor(follow_abs);
    }

    /// Handle a store reconcile that reloaded the file from disk: reset
    /// transient input state and refresh the view, matching the old
    /// `apply_external_state` behavior.
    pub(crate) fn on_reload(&mut self) {
        self.selection.clear();
        self.selection.exit_edit();
        self.recompute_visible();
        self.clamp_cursor();
        self.flash("file changed on disk — reloaded");
    }

    /// Map a store reconcile result to the matching flash + view refresh for a
    /// mutation that produced no change of its own (used on the abort paths).
    pub(crate) fn handle_reconcile_abort(&mut self, r: Reconcile) {
        match r {
            Reconcile::Reloaded => self.on_reload(),
            Reconcile::ReadError => self.flash("read failed"),
            Reconcile::Unchanged => {}
        }
    }

    /// Surface a [`DrainReport`] from `Store::drain_inbox` as a flash, matching
    /// the wording the inline drain used to emit, and refresh the view when
    /// tasks were merged.
    pub(crate) fn apply_drain(&mut self, report: DrainReport) {
        if report.merged > 0 {
            self.recompute_visible();
            self.clamp_cursor();
        }
        if let Some(err) = report.error {
            self.flash(err);
        } else if report.merged > 0 {
            if report.skipped > 0 {
                self.flash(format!(
                    "merged {} from inbox ({} skipped)",
                    report.merged, report.skipped
                ));
            } else {
                self.flash(format!("merged {} from inbox", report.merged));
            }
        } else if report.skipped > 0 {
            self.flash(format!(
                "inbox: {} unparseable, nothing merged",
                report.skipped
            ));
        }
    }

    /// Apply a freshly loaded [`Config`] at runtime — used by the hot-reload
    /// watcher. Rebuilds `prefs` and `saved_filters` from the new config
    /// values, then refreshes the visible task cache so theme/density/sort/
    /// layout changes take effect immediately.
    pub fn reload_config(&mut self, new_cfg: Config) {
        self.prefs = Prefs::from_config(new_cfg.clone());
        self.saved_filters = new_cfg
            .filters
            .iter()
            .map(|(name, query)| SavedFilter {
                name: name.clone(),
                query: query.clone(),
            })
            .collect();
        self.week_start = new_cfg.week_start.unwrap_or(WeekStart::Sunday);
        self.recompute_visible();
    }

    /// Reconcile against disk and drain the inbox. Returns `true` when it is
    /// safe to proceed (disk unchanged); `false` when the file was reloaded or
    /// unreadable. The TUI run loop and `handle_key` call this each tick.
    pub fn check_external_changes(&mut self) -> bool {
        let reconcile = self.store.reconcile();
        if matches!(reconcile, Reconcile::Reloaded) {
            self.on_reload();
        }
        let report = self.store.drain_inbox();
        self.apply_drain(report);
        matches!(reconcile, Reconcile::Unchanged)
    }
}
