//! Action guidance must reflect dispatch, and help must remain navigable.
use ratatui::{Terminal, backend::TestBackend};
use std::path::PathBuf;
use tuxedo::{
    app::{App, Mode},
    config::Config,
    keybinds::KeyBindings,
    ui,
};

fn app() -> App {
    App::new(
        PathBuf::from("/tmp/tuxedo-guidance-unused.txt"),
        "Alpha\nBeta\n".into(),
        "2026-05-06".into(),
        Config::default(),
    )
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn help_uses_effective_keys_and_scrolls_to_editor_recurrence_and_format() {
    let mut app = app();
    app.mode = Mode::Help;
    app.keybinds = KeyBindings::parse(
        "[normal]\nbegin_add = \"zz\"\nopen_help = \"n\"\n[recurrence]\naccept = \"j\"\n",
    );
    for (width, height) in [(110, 32), (80, 24), (30, 10)] {
        app.help_scroll.set(0);
        let first = screen(&app, width, height);
        if width >= 80 {
            assert!(first.contains("zz: new task"));
            assert!(!first.contains(" n: new task"));
            assert!(first.contains("scroll"));
        }
        let mut all = first;
        for _ in 0..150 {
            app.help_scroll.set(app.help_scroll.get().saturating_add(1));
            all.push_str(&screen(&app, width, height));
        }
        assert!(all.contains("INLINE EDITOR"));
        assert!(all.contains("RECURRENCE"));
        assert!(all.contains("FORMAT"));
        if width >= 80 {
            assert!(all.contains("Enter / j: save recurrence"));
        }
        let bottom = app.help_scroll.get();
        app.help_scroll.set(u16::MAX);
        screen(&app, width, height);
        assert_eq!(app.help_scroll.get(), bottom);
    }
}

#[test]
fn palette_displays_reachable_alias_for_overridden_default() {
    let mut app = app();
    app.keybinds = KeyBindings::parse("[normal]\nbegin_add = \"N\"\nopen_help = \"n\"\n");
    app.command_palette.open(Mode::Normal);
    app.mode = Mode::CommandPalette;
    app.draft_set_insert("new task".into());
    app.command_palette.refresh("new task");
    let text = screen(&app, 80, 24);
    assert!(text.contains("new task"));
    assert!(text.contains("N "));
    assert!(!text.contains("n / N"));
}

#[test]
fn help_draws_safely_in_tiny_panes() {
    let mut app = app();
    app.mode = Mode::Help;
    for width in 0..10 {
        for height in 0..8 {
            screen(&app, width, height);
        }
    }
}

#[test]
fn palette_keeps_complete_key_after_long_action_label() {
    let mut app = app();
    app.keybinds =
        KeyBindings::parse("[normal]\nbegin_prompt_context = \"Ctrl-c\"\nopen_help = \"c\"\n");
    app.command_palette.open(Mode::Normal);
    app.mode = Mode::CommandPalette;
    app.draft_set_insert("context on current task".into());
    app.command_palette.refresh("context on current task");
    let text = screen(&app, 80, 24);
    assert!(text.contains("Ctrl-c "));
    assert!(!text.contains("c / Ctrl"));
}
