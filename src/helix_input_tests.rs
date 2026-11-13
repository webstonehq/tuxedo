//! Full input sequences use disposable task/config paths, never user files.
#![allow(clippy::unwrap_used)]

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    app: App,
    dir: std::path::PathBuf,
}
impl Fixture {
    fn new(profile: EditingProfile) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-helix-input-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("todo.txt");
        let body = "one two\nother +work @home\n";
        std::fs::write(&path, body).unwrap();
        let mut app = App::new(
            path,
            body.into(),
            "2026-11-13".into(),
            tuxedo::config::Config {
                editing_profile: Some(profile),
                ..Default::default()
            },
        );
        app.config_path = Some(dir.join("config.toml"));
        Self { app, dir }
    }
    fn press(&mut self, code: KeyCode) {
        handle_key(
            &mut self.app,
            KeyEvent::new(code, KeyModifiers::NONE),
            &KeyBindings::default(),
        );
    }
    fn keys(&mut self, text: &str) {
        for c in text.chars() {
            self.press(KeyCode::Char(c));
        }
    }
    fn disk(&self) -> String {
        std::fs::read_to_string(self.dir.join("todo.txt")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn helix_sequence_edits_only_on_submit_and_keeps_session_register() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("e");
    assert_eq!(f.app.draft.profile(), EditingProfile::Helix);
    f.keys("wy");
    assert_eq!(f.app.draft.register(), "one ");
    f.keys("cX");
    assert_eq!(f.app.draft.text(), "Xtwo");
    assert!(f.disk().starts_with("one two"));
    f.press(KeyCode::Esc);
    f.keys("uU");
    f.press(KeyCode::Enter);
    assert!(f.disk().starts_with("Xtwo\n"));
    f.keys("n");
    f.press(KeyCode::Esc);
    f.keys("p");
    assert_eq!(f.app.draft.text(), "one ");
    f.press(KeyCode::Esc);
    assert!(f.disk().starts_with("Xtwo\n"));
}

#[test]
fn profile_is_captured_and_non_task_inputs_stay_plain() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("e");
    f.app.reload_config(tuxedo::config::Config::default());
    assert_eq!(f.app.draft.profile(), EditingProfile::Helix);
    f.press(KeyCode::Esc);
    f.keys("e");
    assert_eq!(f.app.draft.profile(), EditingProfile::Vim);
    f.press(KeyCode::Esc);
    f.app.reload_config(tuxedo::config::Config {
        editing_profile: Some(EditingProfile::Helix),
        ..Default::default()
    });
    f.keys("/dx");
    assert_eq!(f.app.mode, Mode::Search);
    assert_eq!(f.app.draft.text(), "dx");
    assert_eq!(f.app.draft.profile(), EditingProfile::Vim);
    f.press(KeyCode::Esc);
    f.keys(":dx");
    assert_eq!(f.app.draft.profile(), EditingProfile::Vim);
    assert_eq!(f.app.draft.text(), "dx");
}

#[test]
fn settings_profile_persists_explicit_path_and_applies_to_next_dialog() {
    let mut f = Fixture::new(EditingProfile::Vim);
    f.keys(",E");
    assert_eq!(f.app.mode, Mode::Settings);
    let cfg = tuxedo::config::Config::load_from(f.app.config_path.as_ref().unwrap());
    assert_eq!(cfg.editing_profile, Some(EditingProfile::Helix));
    f.press(KeyCode::Esc);
    f.keys("e");
    assert_eq!(f.app.draft.profile(), EditingProfile::Helix);
}

#[test]
fn nlp_confirmation_preserves_profile_and_is_undoable() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("nBuy milk tomorrow");
    let before = f.app.draft.text().to_string();
    f.press(KeyCode::Enter);
    assert_eq!(f.app.mode, Mode::Insert);
    assert_eq!(f.app.draft.profile(), EditingProfile::Helix);
    assert!(f.app.draft.text().contains("due:2026-11-14"));
    let parsed = f.app.draft.text().to_string();
    f.press(KeyCode::Esc);
    f.keys("u");
    assert_eq!(f.app.draft.text(), before);
    f.keys("U");
    assert_eq!(f.app.draft.text(), parsed);
    f.press(KeyCode::Enter);
    assert_eq!(f.app.mode, Mode::Normal);
    assert!(f.disk().contains("due:2026-11-14"));
}

#[test]
fn picker_and_completion_keys_do_not_delete_select_or_submit() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("n/");
    assert!(f.app.draft.overlay().is_some());
    f.keys("dx");
    assert_eq!(f.app.draft.text(), "/dx");
    f.press(KeyCode::Esc);
    assert!(f.app.draft.overlay().is_none());
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Insert);
    f.keys("due:");
    let before = f.app.draft.text().to_string();
    f.keys("d");
    assert_eq!(f.app.draft.text(), before);
    f.press(KeyCode::Enter);
    assert_eq!(f.app.mode, Mode::Insert);
    assert!(f.app.draft.overlay().is_none());
    let accepted = f.app.draft.text().to_string();
    f.press(KeyCode::Esc);
    f.keys("u");
    assert_eq!(f.app.draft.text(), before);
    f.keys("U");
    assert_eq!(f.app.draft.text(), accepted);
    f.keys("A +wo");
    assert!(f.app.autocomplete_visible());
    f.press(KeyCode::Enter);
    assert!(f.app.draft.text().contains("+work"));
    assert_eq!(f.app.mode, Mode::Insert);
    f.press(KeyCode::Esc);
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
}

#[test]
fn failed_add_and_edit_persistence_keep_editable_drafts() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" changed");
            let draft = f.app.draft.text().to_string();
            std::fs::create_dir(f.dir.join("todo.tmp")).unwrap();
            f.press(KeyCode::Enter);
            assert_eq!(f.app.mode, Mode::Insert);
            assert_eq!(f.app.draft.text(), draft);
            assert_eq!(f.app.draft.profile(), profile);
            assert_eq!(f.app.selection.editing().is_some(), edit);
            assert!(!f.disk().contains("changed"));
            assert!(f.app.flash_active().is_some());
        }
    }
}

#[test]
fn external_reload_keeps_edit_intent_and_blocks_stale_row_write_or_add() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("i edited");
    let draft = f.app.draft.text().to_string();
    std::fs::write(f.dir.join("todo.txt"), "replacement\none two\n").unwrap();
    f.press(KeyCode::Enter); // reload consumes this key
    assert_eq!(f.app.draft.text(), draft);
    assert!(f.app.selection.edit_stale());
    f.press(KeyCode::Enter); // explicit safe recovery notice, no write
    assert_eq!(f.app.mode, Mode::Insert);
    assert_eq!(f.disk(), "replacement\none two\n");
    assert!(f.app.flash_active().unwrap().contains("reopen"));
    f.keys("!");
    assert!(f.app.draft.text().ends_with('!'));
    f.press(KeyCode::Esc);
    f.press(KeyCode::Esc);
    assert_eq!(f.app.mode, Mode::Normal);
}

#[test]
fn reload_recovery_covers_profiles_reconcile_routes_and_changed_row_identity() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for through_key in [false, true] {
            for replacement in [
                "changed target\nother +work @home\n",
                "other +work @home\none two\n",
                "other +work @home\n",
            ] {
                let mut f = Fixture::new(profile);
                f.keys("i!");
                let text = f.app.draft.text().to_string();
                std::fs::write(f.dir.join("todo.txt"), replacement).unwrap();
                if through_key {
                    f.press(KeyCode::Enter);
                } else {
                    handle_insert(
                        &mut f.app,
                        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                        &KeyBindings::default(),
                    );
                }
                assert!(f.app.selection.edit_stale());
                assert_eq!(f.app.selection.editing(), Some(0));
                assert_eq!(f.app.draft.text(), text);
                assert_eq!(f.app.draft.profile(), profile);
                for _ in 0..2 {
                    f.press(KeyCode::Enter);
                }
                assert_eq!(f.disk(), replacement);
                assert_eq!(f.app.mode, Mode::Insert);
                assert_eq!(f.app.draft.text(), text);
                assert!(f.app.flash_active().unwrap().contains("reopen"));
                if profile == EditingProfile::Helix {
                    f.press(KeyCode::Esc);
                    f.keys("u");
                    assert_eq!(f.app.draft.text(), "one two");
                    f.keys("U");
                    assert_eq!(f.app.draft.text(), text);
                    f.keys("%y");
                    assert_eq!(f.app.draft.register(), text);
                } else {
                    assert!(f.app.flash_active().unwrap().contains("terminal"));
                    f.press(KeyCode::Esc);
                }
                f.press(KeyCode::Esc);
                assert_eq!(f.app.mode, Mode::Normal);
                f.keys("e");
                assert!(!f.app.selection.edit_stale());
                assert_eq!(
                    f.app.draft.text(),
                    f.app.task_raw(f.app.selection.editing().unwrap()).unwrap()
                );
                f.press(KeyCode::Enter);
                assert_eq!(f.app.mode, Mode::Normal);
                assert_eq!(f.disk(), replacement);
            }
        }
    }
}

#[test]
fn recurrence_priority_calendar_and_slash_accepts_are_discrete_edits() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("nTask rec:");
    assert!(f.app.draft.overlay().is_some());
    let before = f.app.draft.text().to_string();
    f.keys("dx");
    assert_eq!(f.app.draft.text(), before);
    f.press(KeyCode::Enter);
    let accepted = f.app.draft.text().to_string();
    assert!(accepted.contains("rec:1"));
    f.press(KeyCode::Esc);
    f.keys("u");
    assert_eq!(f.app.draft.text(), before);
    f.keys("U");
    assert_eq!(f.app.draft.text(), accepted);
    f.keys("A /pri");
    f.press(KeyCode::Enter);
    assert!(f.app.draft.overlay().is_some());
    let before = f.app.draft.text().to_string();
    f.keys("dx");
    assert_eq!(f.app.draft.text(), before);
    f.press(KeyCode::Enter);
    assert!(f.app.draft.text().starts_with("(A) "));
    assert_eq!(f.app.mode, Mode::Insert);
    f.press(KeyCode::Esc);
    f.keys("u");
    assert_eq!(f.app.draft.text(), before);
    f.keys("A /project");
    let before = f.app.draft.text().to_string();
    f.press(KeyCode::Enter);
    assert!(f.app.draft.text().ends_with('+'));
    f.press(KeyCode::Esc); // autocomplete owns first Escape
    f.press(KeyCode::Esc); // leave Insert
    f.keys("u");
    assert_eq!(f.app.draft.text(), before);
}

#[test]
fn overlay_ownership_precedes_normal_and_extend_dispatch() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("r");
    assert!(f.app.draft.overlay().is_some());
    f.app.draft.set_input_mode(DialogInputMode::Extend);
    let text = f.app.draft.text().to_string();
    f.keys("d");
    assert_eq!(f.app.draft.text(), text);
    f.press(KeyCode::Esc);
    assert!(f.app.draft.overlay().is_none());
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Extend);
    f.press(KeyCode::Esc);
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
    assert_eq!(f.app.mode, Mode::Insert);
}

#[test]
fn failed_persistence_never_leaks_cancelled_edits_into_later_successful_writes() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            let original = f.disk();
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" cancelled");
            std::fs::create_dir(f.dir.join("todo.tmp")).unwrap();
            f.press(KeyCode::Enter);
            assert_eq!(f.app.tasks().len(), 2);
            assert_eq!(f.app.tasks()[0].raw, "one two");
            std::fs::remove_dir(f.dir.join("todo.tmp")).unwrap();
            f.press(KeyCode::Esc);
            f.press(KeyCode::Esc);
            assert_eq!(f.disk(), original);
            f.keys("nkept");
            f.press(KeyCode::Enter);
            assert_eq!(f.app.mode, Mode::Normal);
            assert!(!f.disk().contains("cancelled"));
            assert!(f.disk().contains("kept"));
            f.keys("u");
            assert_eq!(f.disk(), original);
        }
    }
}

#[test]
fn unreadable_reconcile_reports_failure_and_preserves_draft_identity() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("i!");
    let text = f.app.draft.text().to_string();
    std::fs::remove_file(f.dir.join("todo.txt")).unwrap();
    std::fs::create_dir(f.dir.join("todo.txt")).unwrap();
    f.press(KeyCode::Enter);
    assert_eq!(f.app.draft.text(), text);
    assert_eq!(f.app.selection.editing(), Some(0));
    assert_eq!(f.app.mode, Mode::Insert);
    assert!(f.app.flash_active().unwrap().contains("read failed"));
}

#[test]
fn yank_leaves_extend_before_the_next_normal_motion() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("evly");
    assert_eq!(f.app.draft.register(), "on");
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
    f.keys("l");
    assert_eq!(&f.app.draft.text()[f.app.draft.selected_range()], "e");
}

#[test]
fn undo_redo_preserve_extend_mode_and_eoi_extend_selects_last_grapheme() {
    let mut f = Fixture::new(EditingProfile::Helix);
    f.keys("i!");
    f.press(KeyCode::Esc);
    assert_eq!(f.app.draft.cursor(), f.app.draft.text().len());
    f.keys("v");
    assert_eq!(&f.app.draft.text()[f.app.draft.selected_range()], "!");
    f.keys("u");
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Extend);
    assert_eq!(f.app.draft.text(), "one two");
    f.keys("U");
    assert_eq!(f.app.draft.input_mode(), DialogInputMode::Extend);
    assert_eq!(f.app.draft.text(), "one two!");
}

#[test]
fn failed_write_can_retry_the_same_editable_draft_once_storage_recovers() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            let original = f.disk();
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" retry");
            std::fs::create_dir(f.dir.join("todo.tmp")).unwrap();
            f.press(KeyCode::Enter);
            std::fs::remove_dir(f.dir.join("todo.tmp")).unwrap();
            f.keys("!");
            f.press(KeyCode::Enter);
            assert_eq!(f.app.mode, Mode::Normal);
            assert_eq!(f.disk().matches("retry!").count(), 1);
            f.keys("u");
            assert_eq!(f.disk(), original);
        }
    }
}

#[test]
fn persistent_read_error_allows_draft_edit_yank_history_and_cancel_without_mutation() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            let original = f.disk();
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" recover");
            let before = f.app.draft.text().to_string();
            std::fs::remove_file(f.dir.join("todo.txt")).unwrap();
            std::fs::create_dir(f.dir.join("todo.txt")).unwrap();
            for _ in 0..2 {
                f.press(KeyCode::Enter);
            }
            assert_eq!(f.app.draft.text(), before);
            assert_eq!(f.app.draft.profile(), profile);
            assert_eq!(f.app.selection.editing(), edit.then_some(0));
            assert!(!f.app.selection.edit_stale());
            assert_eq!(
                f.app
                    .tasks()
                    .iter()
                    .map(|t| t.raw.as_str())
                    .collect::<Vec<_>>(),
                ["one two", "other +work @home"]
            );
            assert!(f.dir.join("todo.txt").is_dir());
            assert!(!f.dir.join("todo.tmp").exists());
            f.keys("!"); // must work while the read error is still present
            assert_eq!(f.app.draft.text(), format!("{before}!"));
            f.press(KeyCode::Esc);
            assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
            if profile == EditingProfile::Helix {
                f.keys("u");
                assert_eq!(f.app.draft.text(), before);
                f.keys("U%y");
                assert_eq!(f.app.draft.register(), format!("{before}!"));
                f.keys("v");
                f.press(KeyCode::Esc);
                assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
            }
            f.press(KeyCode::Esc);
            assert_eq!(f.app.mode, Mode::Normal);
            assert_eq!(f.app.selection.editing(), None);
            assert_eq!(f.app.draft.text(), "");
            std::fs::remove_dir(f.dir.join("todo.txt")).unwrap();
            std::fs::write(f.dir.join("todo.txt"), &original).unwrap();
            f.app.clear_flash();
            f.keys("u");
            assert_eq!(
                f.app.flash_active(),
                None,
                "failed submits add no task undo entry"
            );
            assert_eq!(f.disk(), original);
            f.keys("nintended");
            f.press(KeyCode::Enter);
            assert_eq!(f.app.mode, Mode::Normal);
            assert!(!f.disk().contains("recover"));
            assert_eq!(f.disk().matches("intended").count(), 1);
            f.keys("u");
            assert_eq!(f.disk(), original);
        }
    }
}

#[test]
fn persistent_read_error_keeps_overlay_escape_ahead_of_mode_and_cancel() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" due:");
            assert!(f.app.draft.overlay().is_some());
            std::fs::remove_file(f.dir.join("todo.txt")).unwrap();
            std::fs::create_dir(f.dir.join("todo.txt")).unwrap();
            f.press(KeyCode::Esc);
            assert!(f.app.draft.overlay().is_none());
            assert_eq!(f.app.draft.input_mode(), DialogInputMode::Insert);
            assert_eq!(f.app.selection.editing(), edit.then_some(0));
            f.keys(" continued");
            assert!(f.app.draft.text().ends_with(" continued"));
            f.press(KeyCode::Esc);
            assert_eq!(f.app.draft.input_mode(), DialogInputMode::Normal);
            f.press(KeyCode::Esc);
            assert_eq!(f.app.mode, Mode::Normal);
            assert!(f.dir.join("todo.txt").is_dir());
        }
    }
}

#[test]
fn recovered_readable_storage_saves_only_the_intended_retained_draft_once() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            let original = f.disk();
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" recover");
            std::fs::remove_file(f.dir.join("todo.txt")).unwrap();
            std::fs::create_dir(f.dir.join("todo.txt")).unwrap();
            f.press(KeyCode::Enter);
            f.keys("!");
            let intended = f.app.draft.text().to_string();
            std::fs::remove_dir(f.dir.join("todo.txt")).unwrap();
            std::fs::write(f.dir.join("todo.txt"), &original).unwrap();
            f.press(KeyCode::Enter);
            assert_eq!(f.app.mode, Mode::Normal);
            assert!(f.disk().contains(intended.trim()));
            assert_eq!(f.disk().matches("recover!").count(), 1);
            assert_eq!(f.app.tasks().len(), if edit { 2 } else { 3 });
            f.keys("u");
            assert_eq!(f.disk(), original);
        }
    }
}

#[test]
fn unreadable_utf8_with_pending_inbox_never_overwrites_unverified_disk() {
    for profile in [EditingProfile::Vim, EditingProfile::Helix] {
        for edit in [false, true] {
            let mut f = Fixture::new(profile);
            let original = f.disk();
            if edit {
                f.keys("i");
            } else {
                f.keys("n");
            }
            f.keys(" cancelled");
            let unreadable = [0xff, 0xfe, b'\n'];
            std::fs::write(f.dir.join("todo.txt"), unreadable).unwrap();
            let inbox = "incoming due:2026-11-15\n";
            std::fs::write(f.dir.join("inbox.txt"), inbox).unwrap();
            f.keys("!");
            assert!(f.app.draft.text().ends_with("cancelled!"));
            for _ in 0..2 {
                f.press(KeyCode::Enter);
            }
            assert_eq!(std::fs::read(f.dir.join("todo.txt")).unwrap(), unreadable);
            assert_eq!(
                std::fs::read_to_string(f.dir.join("inbox.txt")).unwrap(),
                inbox
            );
            assert_eq!(f.app.tasks().len(), 2);
            assert_eq!(f.app.selection.editing(), edit.then_some(0));
            f.press(KeyCode::Esc);
            f.press(KeyCode::Esc);
            std::fs::write(f.dir.join("todo.txt"), &original).unwrap();
            f.press(KeyCode::Down); // only readable recovery may merge the pending capture
            assert_eq!(f.app.tasks().len(), 3);
            assert!(f.disk().contains("incoming"));
            assert!(!f.disk().contains("cancelled"));
            f.keys("u");
            assert_eq!(f.disk(), original);
        }
    }
}
