#![warn(clippy::unwrap_used)]

use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use tuxedo::action::Action;
use tuxedo::app::{App, Mode, OverlayKind, View};
use tuxedo::config::Config;
use tuxedo::{clipboard, sample, todo, ui, update};

const EVENT_POLL: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    let path = match arg.as_deref() {
        Some("--help") | Some("-h") => {
            print_usage();
            return Ok(());
        }
        Some("--version") | Some("-V") => {
            println!("tuxedo {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("update") => {
            update::run()?;
            return Ok(());
        }
        Some("--sample") => sample_path()?,
        Some(s) if s.starts_with('-') => {
            eprintln!("tuxedo: unknown option: {s}");
            eprintln!("try `tuxedo --help`");
            std::process::exit(2);
        }
        _ => resolve_path(arg)?,
    };
    // A freshly-created file is empty; otherwise read it. We accept NotFound
    // (race with deletion between resolve_path and now) as "empty file" but
    // refuse to silently swallow other IO errors — an unreadable or non-UTF-8
    // file would otherwise present as an empty editor that, on first save,
    // overwrites the user's data.
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", path.display()));
        }
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cfg = Config::load();
    let mut app_state = App::new(path.clone(), body, today, cfg);
    app_state.config_path = Config::path();
    if std::env::var_os("TUXEDO_NO_UPDATE_CHECK").is_none() {
        app_state.set_update_check(update::spawn_check());
    }

    let terminal = ratatui::init();
    let result = run(terminal, &mut app_state);
    ratatui::restore();
    // Print the file path *after* restoring the terminal so the message
    // survives in the user's scrollback rather than being eaten by the
    // alt-screen.
    eprintln!("tuxedo: {}", path.display());
    result
}

fn print_usage() {
    println!("usage: tuxedo [FILE]");
    println!("       tuxedo update");
    println!();
    println!("Without FILE, opens ./todo.txt if present, otherwise a sample");
    println!("todo.txt in the system temp dir.");
    println!();
    println!("Commands:");
    println!("  update         print instructions for upgrading tuxedo");
    println!();
    println!("Options:");
    println!("  -h, --help     show this message and exit");
    println!("  -V, --version  print version and exit");
    println!("      --sample   open the sample todo.txt in the system temp dir");
}

fn resolve_path(arg: Option<String>) -> io::Result<PathBuf> {
    if let Some(p) = arg {
        let pb = PathBuf::from(p);
        // Atomically create-if-missing. `create_new` fails with AlreadyExists
        // when the file is there — that's the success path: just keep the
        // existing contents. Avoids the TOCTOU window between exists() and
        // write() that would otherwise truncate a concurrently-created file.
        match OpenOptions::new().write(true).create_new(true).open(&pb) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        return Ok(pb);
    }
    let cwd_todo = PathBuf::from("todo.txt");
    if cwd_todo.is_file() {
        return Ok(cwd_todo);
    }
    sample_path()
}

fn sample_path() -> io::Result<PathBuf> {
    let dir = std::env::temp_dir();
    let pb = dir.join("tuxedo-sample.txt");
    std::fs::write(&pb, sample::TODO_RAW)?;
    // Also reset the sibling done.txt — otherwise archived rows from a
    // previous session leak back as duplicates the next time the sample is
    // opened.
    match std::fs::remove_file(dir.join("done.txt")) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(pb)
}

fn run(mut terminal: DefaultTerminal, app: &mut App) -> Result<()> {
    let mut dirty = true;
    while !app.should_quit {
        // Pick up midnight rollover so threshold-hidden tasks reveal
        // themselves without requiring an app restart.
        if app.refresh_today(chrono::Local::now().format("%Y-%m-%d").to_string()) {
            dirty = true;
        }
        // Drain the startup archive loader (and pick up external edits to
        // done.txt). Non-blocking: the first frame can render todo.txt
        // before the archive read completes.
        if app.poll_archive() {
            dirty = true;
        }
        // Pick up the update-check result so the status-bar indicator can
        // appear without waiting for a keystroke.
        if app.poll_update_check() {
            dirty = true;
        }
        if dirty {
            terminal.draw(|f| ui::draw(f, app))?;
            dirty = false;
        }
        let timeout = next_timeout(app);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                handle_key(app, key);
                dirty = true;
            }
        } else if !app.check_external_changes() {
            // Idle tick — file changed under us; reload was performed.
            dirty = true;
        }
        if app.flash_should_clear() {
            app.clear_flash();
            dirty = true;
        }
        if app.chord.should_clear() {
            app.chord.clear();
            dirty = true;
        }
    }
    Ok(())
}

fn next_timeout(app: &App) -> Duration {
    let earliest = match (app.flash_deadline(), app.chord.deadline()) {
        (Some(f), Some(c)) => Some(f.min(c)),
        (a, b) => a.or(b),
    };
    match earliest {
        Some(deadline) => deadline
            .saturating_duration_since(Instant::now())
            .min(EVENT_POLL),
        None => EVENT_POLL,
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    // Detect external edits before processing the key. On detection the
    // file is reloaded, the keystroke is consumed (re-press to act on the
    // new state), and the per-mutator checks become no-ops downstream.
    if !app.check_external_changes() {
        return;
    }
    match app.mode {
        Mode::Insert => handle_insert(app, key),
        Mode::Search => handle_search(app, key),
        Mode::Help => handle_help(app, key),
        Mode::Settings => handle_settings(app, key),
        Mode::PromptProject | Mode::PromptContext => handle_prompt(app, key),
        Mode::PickProject | Mode::PickContext => handle_pick(app, key),
        Mode::CommandPalette => handle_command_palette(app, key),
        Mode::Normal | Mode::Visual => handle_normal(app, key),
    }
}

/// What the draft buffer changed (or didn't) in response to a key. Lets
/// callers like search distinguish a text edit (which must re-run the filter)
/// from a cursor move (which must not, otherwise navigating within the search
/// box would reset the visible-list cursor on every arrow press).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftEffect {
    Unhandled,
    CursorMoved,
    TextChanged,
}

/// Apply a standard text-editing key (Backspace/Delete/arrows/Home/End/Char)
/// to the draft. Centralizes the canonical key list so insert/search/prompt
/// modes stay in sync as bindings evolve.
fn apply_to_draft(app: &mut App, key: KeyEvent) -> DraftEffect {
    match key.code {
        KeyCode::Backspace => {
            app.draft_backspace();
            DraftEffect::TextChanged
        }
        KeyCode::Delete => {
            app.draft_delete_forward();
            DraftEffect::TextChanged
        }
        KeyCode::Char(c) => {
            app.draft_insert_char(c);
            DraftEffect::TextChanged
        }
        KeyCode::Left => {
            app.draft_left();
            DraftEffect::CursorMoved
        }
        KeyCode::Right => {
            app.draft_right();
            DraftEffect::CursorMoved
        }
        KeyCode::Home => {
            app.draft_home();
            DraftEffect::CursorMoved
        }
        KeyCode::End => {
            app.draft_end();
            DraftEffect::CursorMoved
        }
        _ => DraftEffect::Unhandled,
    }
}

fn handle_insert(app: &mut App, key: KeyEvent) {
    // Metadata-picker overlays take precedence. Non-slash overlays fully
    // consume keys until accepted or cancelled; the slash menu intercepts
    // only its navigation keys and lets text editing flow through so the
    // filter text in the buffer keeps growing as the user types.
    let overlay = app.draft.overlay().map(|o| o.kind());
    match overlay {
        Some(OverlayKind::Calendar) => {
            handle_insert_calendar(app, key);
            return;
        }
        Some(OverlayKind::RecurrenceBuilder) => {
            handle_insert_rec_builder(app, key);
            return;
        }
        Some(OverlayKind::PriorityChooser) => {
            handle_insert_priority(app, key);
            return;
        }
        Some(OverlayKind::SlashMenu) => {
            if handle_insert_slash_menu(app, key) {
                return;
            }
            // Fall through — let the key flow into the editor so filter chars
            // can be typed/erased. We re-check the overlay invariants after.
            apply_to_draft(app, key);
            // Backspacing past the `/` closes the menu; typing more chars
            // just narrows the filter.
            app.slash_menu_revalidate();
            return;
        }
        None => {}
    }

    // Autocomplete bindings take precedence — only when the popup is visible.
    // Tab accepts; Enter falls through to save so the popup never swallows the
    // submit keystroke (e.g. when the typed token already matches an existing
    // project/context). Esc with the popup open dismisses the popup but leaves
    // Insert mode intact; a second Esc cancels the add (handled below).
    if app.autocomplete_visible() {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => {
                app.autocomplete_accept();
                return;
            }
            KeyCode::Up => {
                app.autocomplete_step(false);
                return;
            }
            KeyCode::Down => {
                app.autocomplete_step(true);
                return;
            }
            KeyCode::Char('n') if ctrl => {
                app.autocomplete_step(true);
                return;
            }
            KeyCode::Char('p') if ctrl => {
                app.autocomplete_step(false);
                return;
            }
            KeyCode::Esc => {
                app.draft.suppress_autocomplete();
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.draft_clear();
            app.selection.exit_edit();
        }
        KeyCode::Enter => {
            if app.selection.editing().is_some() {
                app.save_edit();
            } else {
                app.add_from_draft();
            }
            app.mode = Mode::Normal;
            app.draft_clear();
            app.selection.exit_edit();
        }
        _ => {
            let before = app.draft.text().len();
            let effect = apply_to_draft(app, key);
            // `/` opens the slash menu; `:` after a recognised key
            // (`due` / `t` / `rec`) opens the matching picker directly. Both
            // detections run post-insert so they inspect what actually
            // landed in the buffer.
            if effect == DraftEffect::TextChanged && app.draft.text().len() > before {
                match key.code {
                    KeyCode::Char('/') => app.maybe_open_slash_menu(),
                    KeyCode::Char(':') => app.maybe_open_kv_overlay(),
                    _ => {}
                }
            }
        }
    }
}

/// Slash-menu key handler. Returns `true` when the key was consumed by the
/// menu (navigation, accept, dismiss); `false` when the key should fall
/// through to text editing so filter chars are typed into the buffer.
fn handle_insert_slash_menu(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => {
            app.slash_step(false);
            true
        }
        KeyCode::Down => {
            app.slash_step(true);
            true
        }
        KeyCode::Char('n') if ctrl => {
            app.slash_step(true);
            true
        }
        KeyCode::Char('p') if ctrl => {
            app.slash_step(false);
            true
        }
        KeyCode::Tab | KeyCode::Enter => {
            app.slash_accept();
            true
        }
        KeyCode::Esc => {
            app.slash_cancel();
            true
        }
        _ => false,
    }
}

fn handle_insert_calendar(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => app.calendar_move(-1, 0),
        KeyCode::Char('l') | KeyCode::Right => app.calendar_move(1, 0),
        KeyCode::Char('k') | KeyCode::Up => app.calendar_move(0, -1),
        KeyCode::Char('j') | KeyCode::Down => app.calendar_move(0, 1),
        KeyCode::Char('t') => app.calendar_set_relative(0),
        KeyCode::Char('T') => app.calendar_set_relative(1),
        KeyCode::Char('w') => app.calendar_set_relative(7),
        KeyCode::Char('m') => app.calendar_add_months(1),
        KeyCode::Char('M') => app.calendar_add_months(-1),
        KeyCode::Char('x') => app.calendar_clear(),
        KeyCode::Enter => app.calendar_accept(),
        KeyCode::Esc => app.calendar_cancel(),
        _ => {}
    }
}

fn handle_insert_rec_builder(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => app.recurrence_focus(-1),
        KeyCode::Char('l') | KeyCode::Right => app.recurrence_focus(1),
        KeyCode::Char('j') | KeyCode::Down => app.recurrence_focus(1),
        KeyCode::Char('k') | KeyCode::Up => app.recurrence_focus(-1),
        // `=` is the unshifted `+` on US keyboards — accept both so users
        // don't have to chord Shift to bump the interval.
        KeyCode::Char('+') | KeyCode::Char('=') => app.recurrence_adjust(1),
        KeyCode::Char('-') | KeyCode::Char('_') => app.recurrence_adjust(-1),
        KeyCode::Enter => app.recurrence_accept(),
        KeyCode::Esc => app.recurrence_cancel(),
        _ => {}
    }
}

fn handle_insert_priority(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.priority_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.priority_step(false),
        KeyCode::Enter => app.priority_accept(),
        KeyCode::Esc => app.priority_cancel(),
        _ => {}
    }
}

fn handle_search(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.draft_clear();
            app.clear_search();
        }
        KeyCode::Enter => {
            app.mode = Mode::Normal;
            app.cursor = 0;
        }
        _ => {
            if apply_to_draft(app, key) == DraftEffect::TextChanged {
                app.set_search(app.draft.text().to_string());
            }
        }
    }
}

fn handle_help(app: &mut App, key: KeyEvent) {
    if matches!(
        key.code,
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
    ) {
        app.mode = Mode::Normal;
    }
}

fn handle_settings(app: &mut App, key: KeyEvent) {
    if matches!(
        key.code,
        KeyCode::Esc | KeyCode::Char(',') | KeyCode::Char('q')
    ) {
        app.mode = Mode::Normal;
    }
}

fn handle_pick(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.pick_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.pick_step(false),
        KeyCode::Enter => app.mode = Mode::Normal,
        KeyCode::Esc => app.pick_cancel(),
        _ => {}
    }
}

fn handle_command_palette(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    // List navigation. Plain j/k must type into the search box — the user
    // might be searching for "jump" — so navigation goes via arrows or
    // Ctrl-N/Ctrl-P (matches the autocomplete popup in handle_insert).
    match key.code {
        KeyCode::Esc => {
            app.mode = app.command_palette.take_prior();
            app.draft_clear();
            return;
        }
        KeyCode::Enter => {
            let chosen = app.command_palette.current_action();
            // Restore the prior mode (Normal or Visual) *before* dispatching
            // so visual-aware actions (ToggleComplete, Delete, ToggleSelected)
            // see the selection. The dispatched action may then set its own
            // mode (BeginAdd → Insert, etc.); we don't stomp it after.
            app.mode = app.command_palette.take_prior();
            app.draft_clear();
            if let Some(action) = chosen {
                apply_action(app, action);
            }
            return;
        }
        KeyCode::Down => {
            app.command_palette.step(1);
            return;
        }
        KeyCode::Up => {
            app.command_palette.step(-1);
            return;
        }
        KeyCode::Char('n') if ctrl => {
            app.command_palette.step(1);
            return;
        }
        KeyCode::Char('p') if ctrl => {
            app.command_palette.step(-1);
            return;
        }
        _ => {}
    }
    if apply_to_draft(app, key) == DraftEffect::TextChanged {
        // `refresh` resets the cursor when the needle actually changes; a
        // same-needle call (e.g. typed-and-deleted character) is a no-op.
        app.command_palette.refresh(app.draft.text());
    }
}

fn handle_prompt(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.draft_clear();
        }
        KeyCode::Enter => {
            let prev_mode = app.mode;
            let value = app.draft.text().to_string();
            app.draft_clear();
            app.mode = Mode::Normal;
            match prev_mode {
                Mode::PromptProject => app.add_project_to_current(&value),
                Mode::PromptContext => app.toggle_context_on_current(&value),
                _ => {}
            }
        }
        _ => {
            apply_to_draft(app, key);
        }
    }
}

// `Action` lives in `tuxedo::action` (see `src/action.rs`). Keeping it in the
// library lets the command palette enumerate every variant without pulling
// main.rs into the dependency graph.

/// Map a single keystroke to an `Action`. Returns `None` when the keystroke
/// is the *first* press of a chord (e.g. `g` of `gg`) or unknown — in both
/// cases there is no immediate behavior to apply.
///
/// Mutates the chord state because chord progress is part of interpreting
/// the key, not a separate concern.
fn resolve_normal_key(app: &mut App, key: KeyEvent) -> Option<Action> {
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
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::CursorDown,
        KeyCode::Char('k') | KeyCode::Up => Action::CursorUp,
        KeyCode::Char('G') => Action::CursorBottom,
        // First 'g' arms the chord; second 'g' fires CursorTop.
        KeyCode::Char('g') if app.chord.toggle('g') => Action::CursorTop,
        KeyCode::Char('n') => Action::BeginAdd,
        KeyCode::Char('a') => Action::ToggleArchiveView,
        KeyCode::Char('l') => Action::GoList,
        KeyCode::Char('e' | 'i') => Action::BeginEdit,
        KeyCode::Char('x') => Action::ToggleComplete,
        // 'dd' chord. First press arms; second fires.
        KeyCode::Char('d') if app.chord.toggle('d') => Action::Delete,
        // 'yy' chord copies the whole line; 'yb' (after 'y' is armed) copies
        // the body only. Plain 'y' just arms the leader.
        KeyCode::Char('y') if app.chord.toggle('y') => Action::CopyLine,
        KeyCode::Char('b') if app.chord.consume('y') => Action::CopyBody,
        KeyCode::Char('p') => {
            // After 'f' arms, 'fp' opens the project picker. Otherwise plain
            // 'p' cycles priority.
            if app.chord.consume('f') {
                Action::PickProject
            } else {
                Action::CyclePriority
            }
        }
        KeyCode::Char('c') => {
            if app.chord.consume('f') {
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
        KeyCode::Char('f') => Action::ArmF,
        KeyCode::Char('S') => Action::CycleSort,
        KeyCode::Char('+') => Action::BeginPromptProject,
        KeyCode::Char('[') => Action::ToggleLeftPane,
        KeyCode::Char(']') => Action::ToggleRightPane,
        KeyCode::Char('T') => Action::CycleTheme,
        KeyCode::Char('D') => Action::CycleDensity,
        KeyCode::Char('L') => Action::ToggleLineNum,
        KeyCode::Char('H') => Action::ToggleShowDone,
        KeyCode::Char('F') => Action::ToggleShowFuture,
        KeyCode::Esc => Action::EscapeStack,
        _ => return None,
    })
}

fn apply_action(app: &mut App, action: Action) {
    // Archive view is read-only with two exceptions: `x` un-archives the
    // row at the cursor, `dd` permanently removes it from done.txt. Other
    // mutating actions flash a hint and abort. Navigation, view-switch,
    // theme/density/layout toggles, and overlays (help/settings) fall
    // through to the normal handler below.
    if app.view() == View::Archive {
        match action {
            Action::ToggleComplete => {
                if let Some(idx) = app.cur_abs() {
                    app.unarchive(idx);
                }
                return;
            }
            Action::Delete => {
                if let Some(idx) = app.cur_abs() {
                    app.archive_delete(idx);
                }
                return;
            }
            Action::BeginAdd
            | Action::BeginEdit
            | Action::CyclePriority
            | Action::ToggleVisual
            | Action::ToggleSelected
            | Action::BeginSearch
            | Action::BeginPromptProject
            | Action::BeginPromptContext
            | Action::PickProject
            | Action::PickContext
            | Action::CycleSort
            | Action::ToggleShowDone
            | Action::ToggleShowFuture
            | Action::Undo => {
                app.flash("read-only in archive");
                return;
            }
            _ => {}
        }
    }
    let len = app.visible_indices().len();
    match action {
        Action::Quit => app.should_quit = true,
        Action::CursorDown => {
            if len > 0 {
                app.cursor = (app.cursor + 1).min(len - 1);
            }
        }
        Action::CursorUp => app.cursor = app.cursor.saturating_sub(1),
        Action::CursorTop => app.cursor = 0,
        Action::CursorBottom => {
            if len > 0 {
                app.cursor = len - 1;
            }
        }
        Action::HalfPageDown => {
            app.cursor = (app.cursor + 10).min(len.saturating_sub(1));
        }
        Action::HalfPageUp => app.cursor = app.cursor.saturating_sub(10),
        Action::BeginAdd => {
            app.mode = Mode::Insert;
            app.draft_clear();
            app.selection.exit_edit();
        }
        Action::BeginEdit => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set(raw);
                app.mode = Mode::Insert;
            }
        }
        Action::ToggleComplete => {
            if app.mode == Mode::Visual && !app.selection.is_empty() {
                app.complete_selected();
            } else if let Some(abs) = app.cur_abs() {
                app.toggle_complete(abs);
            }
        }
        Action::Delete => {
            if app.mode == Mode::Visual && !app.selection.is_empty() {
                app.delete_selected();
            } else if let Some(abs) = app.cur_abs() {
                app.delete(abs);
            }
        }
        Action::CyclePriority => {
            if let Some(abs) = app.cur_abs() {
                app.cycle_priority(abs);
            }
        }
        Action::BeginSearch => {
            app.mode = Mode::Search;
            app.draft_clear();
            app.clear_search();
        }
        Action::OpenHelp => app.mode = Mode::Help,
        Action::OpenSettings => app.mode = Mode::Settings,
        Action::OpenCommandPalette => {
            // Snapshot the current mode (Normal or Visual) so cancel/run
            // can restore it — otherwise opening the palette from Visual
            // and cancelling silently exits Visual.
            let prior = app.mode;
            app.command_palette.open(prior);
            app.mode = Mode::CommandPalette;
            app.draft_clear();
        }
        Action::Undo => app.undo(),
        Action::ToggleVisual => {
            app.mode = if app.mode == Mode::Visual {
                Mode::Normal
            } else {
                Mode::Visual
            };
        }
        Action::ToggleSelected => {
            if app.mode == Mode::Visual
                && let Some(abs) = app.cur_abs()
            {
                app.selection.toggle(abs);
            }
        }
        Action::GoList => app.set_view(View::List),
        Action::ToggleArchiveView => {
            let next = if app.view() == View::Archive {
                View::List
            } else {
                View::Archive
            };
            app.set_view(next);
        }
        Action::ArchiveCompleted => {
            if app.view() == View::Archive {
                app.flash("already in archive");
            } else if app.has_completed_tasks() {
                app.archive_completed();
            } else {
                app.flash("no completed tasks to archive");
            }
        }
        Action::ArmF => app.chord.arm('f'),
        Action::PickProject => app.enter_pick_project(),
        Action::PickContext => app.enter_pick_context(),
        Action::CycleSort => app.cycle_sort(),
        Action::BeginPromptProject => {
            app.mode = Mode::PromptProject;
            app.draft_clear();
        }
        Action::BeginPromptContext => {
            app.mode = Mode::PromptContext;
            app.draft_clear();
        }
        Action::ToggleLeftPane => {
            app.prefs.toggle_left();
            app.save_prefs();
        }
        Action::ToggleRightPane => {
            app.prefs.toggle_right();
            app.save_prefs();
        }
        Action::CycleTheme => app.cycle_theme(),
        Action::CycleDensity => app.cycle_density(),
        Action::ToggleLineNum => {
            app.prefs.toggle_line_num();
            app.save_prefs();
        }
        Action::ToggleShowDone => {
            app.prefs.toggle_show_done();
            app.cursor = 0;
            app.recompute_visible();
            app.save_prefs();
        }
        Action::ToggleShowFuture => {
            app.prefs.toggle_show_future();
            app.cursor = 0;
            app.recompute_visible();
            app.save_prefs();
        }
        Action::CopyLine => copy_current_task(app, false),
        Action::CopyBody => copy_current_task(app, true),
        Action::EscapeStack => {
            let has_pc = app.filter().project.is_some() || app.filter().context.is_some();
            let has_search = !app.filter().search.is_empty();
            if has_pc {
                app.set_project_filter(None);
                app.set_context_filter(None);
            } else if has_search {
                app.draft_clear();
                app.clear_search();
            } else if !app.selection.is_empty() {
                app.selection.clear();
            } else if app.mode == Mode::Visual {
                app.mode = Mode::Normal;
            } else if app.view() != View::List {
                app.set_view(View::List);
            }
        }
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    if let Some(action) = resolve_normal_key(app, key) {
        apply_action(app, action);
    }
    app.clamp_cursor();
}

fn copy_current_task(app: &mut App, body_only: bool) {
    let Some(raw) = app.cur_task().map(|t| t.raw.clone()) else {
        return;
    };
    let payload = if body_only {
        todo::body_only(&raw)
    } else {
        raw
    };
    match clipboard::copy(&payload) {
        Ok(()) => app.flash(if body_only { "copied (body)" } else { "copied" }),
        Err(e) => app.flash(format!("copy failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use tuxedo::config::Config;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn build_app() -> App {
        let path = std::env::temp_dir().join(format!(
            "tuxedo-bindings-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::write(&path, "a\nb\nc\n");
        App::new(
            path,
            "a\nb\nc\n".into(),
            "2026-05-07".into(),
            Config::default(),
        )
    }

    #[test]
    fn plain_keys_resolve_to_their_actions() {
        let mut app = build_app();
        assert_eq!(resolve_normal_key(&mut app, key('q')), Some(Action::Quit));
        assert_eq!(
            resolve_normal_key(&mut app, key('j')),
            Some(Action::CursorDown),
        );
        assert_eq!(
            resolve_normal_key(&mut app, key('?')),
            Some(Action::OpenHelp)
        );
        assert_eq!(
            resolve_normal_key(&mut app, ctrl('d')),
            Some(Action::HalfPageDown),
        );
        assert_eq!(
            resolve_normal_key(&mut app, key('n')),
            Some(Action::BeginAdd),
        );
        assert_eq!(
            resolve_normal_key(&mut app, key('a')),
            Some(Action::ToggleArchiveView),
        );
        assert_eq!(
            resolve_normal_key(&mut app, key('A')),
            Some(Action::ArchiveCompleted),
        );
        assert_eq!(
            resolve_normal_key(&mut app, key('S')),
            Some(Action::CycleSort),
        );
    }

    #[test]
    fn capital_a_archives_only_when_completed_tasks_exist() {
        // No completed tasks → flash, no archive write.
        let mut app = build_app_with_archive("a\nb\nc\n", None);
        apply_action(&mut app, Action::ArchiveCompleted);
        assert_eq!(app.flash_active(), Some("no completed tasks to archive"));
        assert_eq!(app.tasks().len(), 3);

        // One completed task → archive_completed runs.
        let mut app = build_app_with_archive("x 2026-05-08 done one\nb\n", None);
        apply_action(&mut app, Action::ArchiveCompleted);
        assert_eq!(app.tasks().len(), 1, "completed task must be archived");
    }

    #[test]
    fn lowercase_l_returns_to_list_from_any_view() {
        let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done\n"));
        app.set_view(View::Archive);
        apply_action(&mut app, Action::GoList);
        assert_eq!(app.view(), View::List);
    }

    #[test]
    fn lowercase_a_toggles_archive_view() {
        let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done\n"));
        assert_eq!(app.view(), View::List);
        apply_action(&mut app, Action::ToggleArchiveView);
        assert_eq!(app.view(), View::Archive);
        apply_action(&mut app, Action::ToggleArchiveView);
        assert_eq!(app.view(), View::List);
    }

    #[test]
    fn gg_chord_only_fires_on_second_press() {
        let mut app = build_app();
        // First 'g' arms the chord but produces no action.
        assert_eq!(resolve_normal_key(&mut app, key('g')), None);
        // Second 'g' fires.
        assert_eq!(
            resolve_normal_key(&mut app, key('g')),
            Some(Action::CursorTop)
        );
    }

    #[test]
    fn fp_chord_routes_to_pick_project() {
        let mut app = build_app();
        // 'f' arms the leader.
        assert_eq!(resolve_normal_key(&mut app, key('f')), Some(Action::ArmF));
        apply_action(&mut app, Action::ArmF);
        // 'p' after armed 'f' picks project, not cycles priority.
        assert_eq!(
            resolve_normal_key(&mut app, key('p')),
            Some(Action::PickProject)
        );
    }

    #[test]
    fn p_without_chord_cycles_priority() {
        let mut app = build_app();
        assert_eq!(
            resolve_normal_key(&mut app, key('p')),
            Some(Action::CyclePriority),
        );
    }

    #[test]
    fn unknown_key_returns_none() {
        let mut app = build_app();
        let k = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(resolve_normal_key(&mut app, k), None);
    }

    #[test]
    fn yy_chord_only_fires_on_second_press() {
        let mut app = build_app();
        // First 'y' arms the chord but produces no action.
        assert_eq!(resolve_normal_key(&mut app, key('y')), None);
        // Second 'y' fires the line copy.
        assert_eq!(
            resolve_normal_key(&mut app, key('y')),
            Some(Action::CopyLine)
        );
    }

    #[test]
    fn yb_chord_routes_to_copy_body() {
        let mut app = build_app();
        // 'y' arms the leader without firing.
        assert_eq!(resolve_normal_key(&mut app, key('y')), None);
        // 'b' after armed 'y' copies the body.
        assert_eq!(
            resolve_normal_key(&mut app, key('b')),
            Some(Action::CopyBody)
        );
    }

    #[test]
    fn plain_b_without_y_armed_is_unhandled() {
        let mut app = build_app();
        // No leader → 'b' is not bound to anything else, so nothing fires.
        assert_eq!(resolve_normal_key(&mut app, key('b')), None);
    }

    #[test]
    fn cursor_actions_clamp_to_visible_range() {
        let mut app = build_app();
        // 3 visible tasks, cursor starts at 0.
        apply_action(&mut app, Action::CursorBottom);
        assert_eq!(app.cursor, 2);
        apply_action(&mut app, Action::CursorDown);
        assert_eq!(app.cursor, 2);
        apply_action(&mut app, Action::CursorTop);
        assert_eq!(app.cursor, 0);
        apply_action(&mut app, Action::CursorUp);
        assert_eq!(app.cursor, 0);
    }

    /// Build an isolated App rooted in a fresh temp dir, optionally seeding
    /// done.txt and waiting for the startup loader to land.
    fn build_app_with_archive(todo_raw: &str, done_raw: Option<&str>) -> App {
        use std::time::{Duration, Instant};
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("tuxedo-bindings-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test dir");
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, todo_raw).expect("write todo.txt");
        if let Some(body) = done_raw {
            std::fs::write(dir.join("done.txt"), body).expect("write done.txt");
        }
        let mut app = App::new(
            todo_path,
            todo_raw.into(),
            "2026-05-06".into(),
            Config::default(),
        );
        if done_raw.is_some() {
            // Drain the startup archive loader so app.archive is populated.
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                let _ = app.poll_archive();
                if !app.archive.is_empty() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(!app.archive.is_empty(), "archive failed to load in time");
        }
        app
    }

    #[test]
    fn cursor_navigation_works_in_archive() {
        let mut app = build_app_with_archive(
            "a due:2026-05-04\nb due:2026-05-06\nc due:2026-05-08\n",
            Some("x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-02 second\n"),
        );
        app.set_view(View::Archive);
        assert_eq!(app.cursor, 0);
        apply_action(&mut app, Action::CursorDown);
        assert_eq!(app.cursor, 1, "Archive view must allow CursorDown");
        apply_action(&mut app, Action::CursorTop);
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn archive_x_unarchives_task_under_cursor() {
        let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
        app.set_view(View::Archive);
        apply_action(&mut app, Action::ToggleComplete);
        assert_eq!(app.archive.len(), 0, "task must leave the archive");
        assert!(
            app.tasks()
                .iter()
                .any(|t| t.raw.contains("done one") && !t.done),
            "un-completed entry must rejoin live tasks"
        );
    }

    #[test]
    fn archive_dd_permanently_deletes_task_under_cursor() {
        let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
        app.set_view(View::Archive);
        apply_action(&mut app, Action::Delete);
        assert_eq!(app.archive.len(), 0);
        assert_eq!(app.tasks().len(), 1, "todo.txt must be untouched");
    }

    #[test]
    fn archive_e_and_p_flash_readonly() {
        let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
        app.set_view(View::Archive);
        apply_action(&mut app, Action::BeginEdit);
        assert_eq!(app.flash_active(), Some("read-only in archive"));
        apply_action(&mut app, Action::CyclePriority);
        assert_eq!(app.flash_active(), Some("read-only in archive"));
        assert!(app.archive.tasks()[0].done);
    }
}
