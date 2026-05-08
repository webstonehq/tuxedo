#![warn(clippy::unwrap_used)]

use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use tuxedo::app::{App, Mode, View};
use tuxedo::config::Config;
use tuxedo::{sample, ui};

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
    println!();
    println!("Without FILE, opens ./todo.txt if present, otherwise a sample");
    println!("todo.txt in the system temp dir.");
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
    let pb = std::env::temp_dir().join("tuxedo-sample.txt");
    match OpenOptions::new().write(true).create_new(true).open(&pb) {
        Ok(mut f) => {
            use std::io::Write;
            f.write_all(sample::TODO_RAW.as_bytes())?;
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(pb)
}

fn run(mut terminal: DefaultTerminal, app: &mut App) -> Result<()> {
    let mut dirty = true;
    while !app.should_quit {
        // Drain the startup archive loader (and pick up external edits to
        // done.txt). Non-blocking: the first frame can render todo.txt
        // before the archive read completes.
        if app.poll_archive() {
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
            apply_to_draft(app, key);
        }
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

/// One discrete behavior triggered from Normal/Visual mode. Keeping the key
/// table and the side-effects in separate functions lets us unit-test each
/// half: `resolve_normal_key` proves the binding is correct, `apply_action`
/// proves the effect is correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Quit,
    CursorDown,
    CursorUp,
    CursorTop,
    CursorBottom,
    HalfPageDown,
    HalfPageUp,
    BeginAdd,
    BeginEdit,
    ToggleComplete,
    Delete,
    CyclePriority,
    BeginSearch,
    OpenHelp,
    OpenSettings,
    Undo,
    ToggleVisual,
    ToggleSelected,
    ToggleToday,
    ArchiveOrToggleView,
    ArmF,
    PickProject,
    PickContext,
    CycleSort,
    BeginPromptProject,
    BeginPromptContext,
    ToggleLeftPane,
    ToggleRightPane,
    CycleTheme,
    CycleDensity,
    ToggleLineNum,
    ToggleShowDone,
    EscapeStack,
}

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
            _ => None,
        };
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::CursorDown,
        KeyCode::Char('k') | KeyCode::Up => Action::CursorUp,
        KeyCode::Char('G') => Action::CursorBottom,
        KeyCode::Char('g') => {
            // First 'g' arms the chord; second 'g' fires CursorTop.
            if app.chord.toggle('g') {
                Action::CursorTop
            } else {
                return None;
            }
        }
        KeyCode::Char('a') => Action::BeginAdd,
        KeyCode::Char('e' | 'i') => Action::BeginEdit,
        KeyCode::Char('x') => Action::ToggleComplete,
        KeyCode::Char('d') => {
            // 'dd' chord. First press arms; second fires.
            if app.chord.toggle('d') {
                Action::Delete
            } else {
                return None;
            }
        }
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
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('v') => Action::ToggleVisual,
        KeyCode::Char(' ') => Action::ToggleSelected,
        KeyCode::Char('t') => Action::ToggleToday,
        KeyCode::Char('A') => Action::ArchiveOrToggleView,
        KeyCode::Char('f') => Action::ArmF,
        KeyCode::Char('s') => Action::CycleSort,
        KeyCode::Char('+') => Action::BeginPromptProject,
        KeyCode::Char('[') => Action::ToggleLeftPane,
        KeyCode::Char(']') => Action::ToggleRightPane,
        KeyCode::Char('T') => Action::CycleTheme,
        KeyCode::Char('D') => Action::CycleDensity,
        KeyCode::Char('L') => Action::ToggleLineNum,
        KeyCode::Char('H') => Action::ToggleShowDone,
        KeyCode::Esc => Action::EscapeStack,
        _ => return None,
    })
}

fn apply_action(app: &mut App, action: Action) {
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
                app.complete(abs);
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
        Action::ToggleToday => {
            let next = if app.view() == View::Today {
                View::List
            } else {
                View::Today
            };
            app.set_view(next);
        }
        Action::ArchiveOrToggleView => {
            if app.has_completed_tasks() && app.view() != View::Archive {
                app.archive_completed();
            } else {
                let next = if app.view() == View::Archive {
                    View::List
                } else {
                    View::Archive
                };
                app.set_view(next);
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
}
