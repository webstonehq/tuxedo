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
            assert!(first.contains("zz") && first.contains("new task"));
            assert!(!first.contains("n: new task"));
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
            assert!(all.contains("Enter, j") && all.contains("save recurrence"));
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
fn default_help_keeps_the_task_overview_together_and_fits_a_large_pane() {
    let mut app = app();
    app.mode = Mode::Help;
    let text = screen(&app, 110, 32);
    for heading in [
        "NAVIGATION",
        "TASK EDITING",
        "TASK SELECTION",
        "FILTER & VIEW",
        "LAYOUT & APPEARANCE",
        "SYSTEM",
    ] {
        assert!(text.contains(heading), "missing {heading}");
    }
    assert!(text.contains("quit"));
    let text = screen(&app, 164, 52);
    for heading in ["INLINE EDITOR", "RECURRENCE", "FORMAT"] {
        assert!(text.contains(heading), "missing {heading}");
    }
    app.help_scroll.set(u16::MAX);
    screen(&app, 164, 52);
    assert_eq!(
        app.help_scroll.get(),
        0,
        "default help should fit this pane"
    );
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

#[test]
fn hidden_bar_search_keeps_long_query_cursor_count_controls_and_notices() {
    let mut app = app();
    app.prefs.layout.status_bar = false;
    app.mode = Mode::Search;
    let query = format!("{}QUERY_END", "long-query-".repeat(20));
    app.draft_set_insert(query.clone());
    app.set_search(query);
    app.flash("External change detected; tasks reloaded. Review the new list before continuing.");
    for (width, height) in [(110, 32), (80, 24), (40, 12)] {
        let text = screen(&app, width, height);
        assert!(text.contains("QUERY_END█"));
        assert!(text.contains("0 matches") || text.contains("0 hits"));
        assert!(text.contains("Enter"));
        assert!(text.contains("Esc cancel"));
        assert!(text.contains("Notice:"));
        assert!(text.contains("continuing."));
    }
}

#[test]
fn selected_task_count_survives_hidden_bar_and_narrow_layout() {
    let mut app = app();
    app.prefs.layout.status_bar = false;
    app.mode = Mode::Visual;
    app.selection.toggle(0);
    app.selection.toggle(1);
    app.chord.arm('d');
    for width in [110, 80, 40, 30, 20, 12] {
        let text = screen(&app, width, 24);
        assert!(
            text.contains("2 tasks selected")
                || text.contains("2 selected")
                || text.contains("2 sel"),
            "width {width}: {text}"
        );
        assert!(text.contains("TASK"));
        if width >= 80 {
            assert!(text.contains("[x]"));
        }
    }
}

#[test]
fn help_over_archive_names_active_interaction_and_keeps_archive() {
    let mut app = app();
    app.set_view(tuxedo::app::View::Archive);
    app.mode = Mode::Help;
    let before = app.archive().len();
    let text = screen(&app, 80, 24);
    assert!(text.contains("HELP"));
    assert!(!text.contains("ARCHIVE ·"));
    assert_eq!(app.archive().len(), before);
    assert_eq!(app.view(), tuxedo::app::View::Archive);
}

#[test]
fn hidden_bar_is_contextual_and_notices_do_not_restore_general_footer() {
    let mut app = app();
    app.prefs.layout.status_bar = false;
    let text = screen(&app, 80, 24);
    assert!(!text.contains("NORMAL"));
    app.flash("Notice stays readable");
    let text = screen(&app, 80, 24);
    assert!(text.contains("Notice stays readable"));
    assert!(!text.contains("NORMAL"));
    assert!(!text.contains("tuxedo 2026"));
}

#[test]
fn narrow_state_layouts_do_not_panic_or_drop_search_cursor() {
    let mut app = app();
    app.prefs.layout.status_bar = false;
    app.flash("A notice");
    for mode in [
        Mode::Normal,
        Mode::Search,
        Mode::Visual,
        Mode::Help,
        Mode::Insert,
    ] {
        app.mode = mode;
        app.draft_set_insert("a long draft with a visible tail".into());
        for width in [0, 1, 5, 12, 20, 30] {
            for height in [0, 1, 2, 5, 10] {
                screen(&app, width, height);
            }
        }
    }
    app.mode = Mode::Search;
    assert!(screen(&app, 20, 10).contains("█"));
}

#[test]
fn long_words_in_notices_are_fully_visible_and_tiny_search_keeps_cursor() {
    let mut app = app();
    app.prefs.layout.status_bar = false;
    app.mode = Mode::Search;
    app.draft_set_insert("query".into());
    app.selection.toggle(0);
    app.selection.toggle(1);
    app.flash(format!("{} FINISHED", vec!["z".repeat(41); 6].join(" ")));
    let text = screen(&app, 80, 24);
    assert!(text.contains("FINISHED"));
    assert!(text.contains("query█"));
    assert!(text.contains("Enter accept"));
    let text = screen(&app, 20, 10);
    assert!(text.contains("2sel"));
    assert!(text.contains("↵OK Esc×"));
    app.clear_flash();
    for width in [1, 2] {
        assert!(screen(&app, width, 4).contains("█"));
    }
}

#[test]
fn calendar_controls_survive_full_draw_for_both_targets_and_status_settings() {
    use tuxedo::app::{CalendarTarget, DialogInputMode};
    for target in [CalendarTarget::Due, CalendarTarget::Threshold] {
        for status in [false, true] {
            let mut app = app();
            app.prefs.layout.status_bar = status;
            app.mode = Mode::Insert;
            let draft = match target {
                CalendarTarget::Due => "Example due:",
                CalendarTarget::Threshold => "Example t:",
            };
            app.draft_set_insert(draft.into());
            app.open_calendar(target);
            for (width, height) in [(110, 32), (80, 24), (40, 12)] {
                let text = screen(&app, width, height);
                assert!(
                    text.contains("Enter select · Esc cancel"),
                    "{width}x{height} status={status} target={target:?}: {text}"
                );
                assert!(text.contains("Vim · Insert"));
                assert!(text.contains(&format!("{draft}█")));
                assert!(app.calendar_state().is_some());
                assert_eq!(app.draft.input_mode(), DialogInputMode::Insert);
            }
            for (width, height) in [(20, 8), (12, 6), (2, 2), (0, 0)] {
                screen(&app, width, height);
            }
            assert!(screen(&app, 20, 8).contains("↵OK Esc×"));
        }
    }
}

#[test]
fn settings_hints_follow_shared_dispatch_keys() {
    let mut app = app();
    app.mode = Mode::Settings;
    app.keybinds = KeyBindings::parse(
        "[normal]\ncycle_density = \"z\"\nopen_help = \"D\"\npick_grouping = \"Tg\"\n",
    );
    let text = screen(&app, 110, 32);
    assert!(text.contains("z to cycle"));
    assert!(!text.contains("D to cycle"));
    assert!(text.contains("Tg to choose"));
    assert!(
        text.contains("palette to cycle"),
        "stolen T must not advertise theme cycle"
    );
}
