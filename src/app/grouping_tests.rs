//! Grouping is a projection over matching tasks; it must never change storage.
use super::test_support::build_app;
use super::*;

#[test]
fn grouping_sort_matrix_preserves_members_order_and_storage() {
    let raw = "(Z) z due:2026-05-06\n(A) a due:2026-05-25\nplain\n(A) tie due:2026-05-25\nx 2026-05-01 (A) done due:2026-05-04\n";
    for group_by in GroupBy::ALL {
        for sort in [Sort::Priority, Sort::Due, Sort::File] {
            let mut app = build_app(raw);
            app.prefs.group_by = group_by;
            app.prefs.sort = sort;
            app.prefs.show_done = true;
            app.recompute_visible();
            let effective = group_by.effective(sort);
            assert_eq!(app.visible_indices().len(), 5);
            let unique: std::collections::HashSet<_> = app.visible_indices().iter().collect();
            assert_eq!(unique.len(), 5);
            let mut expected = vec![0, 1, 2, 3, 4];
            crate::core::filter::sort_by_prefs(&mut expected, app.tasks(), sort);
            if effective != GroupBy::None {
                assert_eq!(app.visible_indices().last(), Some(&4));
                assert_eq!(app.visible_groups().last(), Some(&GroupKey::Done));
                for row in app.display_rows() {
                    if let DisplayRow::Header { key, count } = row {
                        let members: Vec<_> = app
                            .visible_indices()
                            .iter()
                            .zip(app.visible_groups())
                            .filter(|(_, g)| *g == key)
                            .map(|(i, _)| *i)
                            .collect();
                        let sorted: Vec<_> = expected
                            .iter()
                            .filter(|i| members.contains(i))
                            .copied()
                            .collect();
                        assert_eq!(members, sorted);
                        assert_eq!(*count, members.len());
                    }
                }
                if effective == GroupBy::Priority {
                    assert_eq!(app.visible_indices(), &[1, 3, 0, 2, 4]);
                } else {
                    assert_eq!(app.visible_indices(), &[0, 1, 3, 2, 4]);
                }
            } else {
                assert_eq!(app.visible_indices(), expected);
                assert!(
                    app.display_rows()
                        .iter()
                        .all(|r| matches!(r, DisplayRow::Task(_)))
                );
            }
            assert_eq!(
                std::fs::read_to_string(&app.file_path).expect("fixture IO"),
                raw
            );
            assert!(app.store.history.is_empty());
        }
    }
}

#[test]
fn grouped_filters_counts_and_header_identity() {
    let mut app = build_app(
        "(A) match +work @desk\n(Z) match +work @desk\nmatch +work @desk\nx 2026-05-01 (A) match +work @desk\n(A) match +work @desk t:2030-01-01\n(A) other +home\n",
    );
    app.prefs.group_by = GroupBy::Priority;
    app.set_project_filter(Some("work".into()));
    app.set_context_filter(Some("desk".into()));
    app.set_search("match".into());
    assert_eq!(app.visible_indices(), &[0, 1, 2]);
    app.prefs.show_done = true;
    app.prefs.show_future = true;
    app.recompute_visible();
    assert_eq!(app.visible_indices(), &[0, 4, 1, 2, 3]);
    let header = app
        .display_rows()
        .iter()
        .position(|r| matches!(r, DisplayRow::Header { .. }))
        .expect("group header");
    app.cursor = header;
    assert_eq!(app.cur_abs(), None);
    assert!(app.cur_task().is_none());
    let before = std::fs::read(&app.file_path).expect("fixture IO");
    app.add_project_to_current("accident");
    app.toggle_context_on_current("accident");
    app.move_tasks(true);
    app.create_or_open_note_for_current();
    assert_eq!(std::fs::read(&app.file_path).expect("fixture IO"), before);
    assert!(app.store.history.is_empty());
    assert_eq!(app.flash_active(), Some("group header — choose a task"));
}

#[test]
fn grouping_persists_independently_and_hot_reloads() {
    let mut app = build_app("a\n");
    for group_by in GroupBy::ALL {
        app.prefs.group_by = group_by;
        app.prefs.sort = Sort::File;
        app.save_prefs();
        let cfg = Config::load_from(app.config_path.as_ref().expect("isolated config path"));
        assert_eq!(cfg.group_by, Some(group_by));
        assert_eq!(cfg.sort, Some(Sort::File));
        app.reload_config(cfg);
        assert_eq!(app.prefs.group_by, group_by);
        assert_eq!(app.prefs.sort, Sort::File);
    }
    for content in ["sort = due\n", "sort = due\ngroup_by = unknown\n"] {
        std::fs::write(
            app.config_path.as_ref().expect("isolated config path"),
            content,
        )
        .expect("fixture IO");
        app.reload_config(Config::load_from(
            app.config_path.as_ref().expect("isolated config path"),
        ));
        assert_eq!(app.prefs.group_by, GroupBy::Auto);
        assert!(matches!(
            app.display_rows()[0],
            DisplayRow::Header {
                key: GroupKey::ListDue(_),
                ..
            }
        ));
    }
}

#[test]
fn explicit_due_group_order_uses_effective_week_start() {
    let mut app = build_app(
        "no due\nlater due:2026-05-25\nnext due:2026-05-10\nthis due:2026-05-09\ntoday due:2026-05-06\nover due:2026-05-04\nx 2026-05-01 retained due:2026-05-04\n",
    );
    app.prefs.group_by = GroupBy::Due;
    app.prefs.sort = Sort::File;
    app.prefs.show_done = true;
    app.week_start = WeekStart::Sunday;
    app.recompute_visible();
    assert_eq!(app.visible_indices(), &[5, 4, 3, 2, 1, 0, 6]);
    let headers: Vec<_> = app
        .display_rows()
        .iter()
        .filter_map(|r| match r {
            DisplayRow::Header { key, .. } => Some(key.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        headers,
        [
            GroupKey::ListDue(ListDueBucket::Overdue),
            GroupKey::ListDue(ListDueBucket::Today),
            GroupKey::ListDue(ListDueBucket::ThisWeek),
            GroupKey::ListDue(ListDueBucket::NextWeek),
            GroupKey::ListDue(ListDueBucket::Later),
            GroupKey::ListDue(ListDueBucket::NoDue),
            GroupKey::Done
        ]
    );
    app.reload_config(Config {
        sort: Some(Sort::File),
        group_by: Some(GroupBy::Due),
        week_start: Some(WeekStart::Monday),
        show_done: Some(true),
        ..Config::default()
    });
    assert_eq!(
        app.visible_groups()[3],
        GroupKey::ListDue(ListDueBucket::ThisWeek)
    );
    assert_eq!(
        app.display_rows()
            .iter()
            .filter(|r| matches!(r, DisplayRow::Header { .. }))
            .count(),
        6
    );
}

#[test]
fn grouped_file_movement_respects_headers_groups_and_history() {
    let raw = "(A) first\n(B) other\n(A) second\n";
    let mut app = build_app(raw);
    app.prefs.group_by = GroupBy::Priority;
    app.prefs.sort = Sort::File;
    app.recompute_visible();
    app.follow_cursor(0);
    app.move_tasks(true);
    assert_eq!(app.tasks()[0].raw, "(A) second");
    assert_eq!(app.tasks()[1].raw, "(B) other");
    assert_eq!(app.cur_abs(), Some(2));
    let before = std::fs::read(&app.file_path).expect("fixture");
    app.move_tasks(true);
    assert_eq!(std::fs::read(&app.file_path).expect("fixture"), before);
    assert_eq!(app.store.history.len(), 1);
    app.undo();
    assert_eq!(
        std::fs::read_to_string(&app.file_path).expect("fixture"),
        raw
    );
    app.selection.toggle(0);
    app.selection.toggle(1);
    app.mode = Mode::Visual;
    app.move_tasks(true);
    assert_eq!(app.flash_active(), Some("selection spans sort groups"));
    assert!(app.store.history.is_empty());
}

fn focus_header(app: &mut App, key: GroupKey) {
    app.cursor = app
        .display_rows()
        .iter()
        .position(|r| matches!(r, DisplayRow::Header { key: k, .. } if *k == key))
        .expect("header");
}

#[test]
fn collapse_prunes_only_its_members_and_next_bulk_matches_count() {
    let raw = "(A) first\n(B) other\n(A) second\n";
    let mut app = build_app(raw);
    app.prefs.group_by = GroupBy::Priority;
    app.recompute_visible();
    for i in 0..3 {
        app.selection.toggle(i);
    }
    app.mode = Mode::Visual;
    focus_header(&mut app, GroupKey::ListPriority(Some('A')));
    app.toggle_group();
    assert!(app.focused_header().is_some());
    assert_eq!(app.cur_abs(), None);
    assert_eq!(app.navigable_indices().collect::<Vec<_>>(), [1]);
    assert_eq!(app.visible_indices().len(), 3);
    assert_eq!(app.hidden_by_collapse(), 2);
    assert_eq!(app.selection.len(), 1);
    assert!(app.selection.is_selected(1));
    assert_eq!(app.flash_active(), Some("cleared 2 hidden selected tasks"));
    assert_eq!(
        std::fs::read_to_string(&app.file_path).expect("fixture"),
        raw
    );
    assert!(app.store.history.is_empty());
    app.toggle_group();
    assert_eq!(app.selection.len(), 1, "expansion never restores selection");
    app.toggle_group();
    app.complete_selected();
    assert!(app.tasks()[1].done);
    assert!(!app.tasks()[0].done && !app.tasks()[2].done);
    app.undo();
    assert_eq!(
        std::fs::read_to_string(&app.file_path).expect("fixture"),
        raw
    );
    assert!(app.group_collapsed(&GroupKey::ListPriority(Some('A'))));
}

#[test]
fn all_collapsed_headers_remain_and_mode_changes_expand() {
    let mut app = build_app("(A) a\n(B) b\nplain\n");
    app.prefs.group_by = GroupBy::Priority;
    app.recompute_visible();
    for key in [
        GroupKey::ListPriority(Some('A')),
        GroupKey::ListPriority(Some('B')),
        GroupKey::ListPriority(None),
    ] {
        focus_header(&mut app, key);
        app.toggle_group();
    }
    assert_eq!(app.display_rows().len(), 3);
    assert_eq!(app.hidden_by_collapse(), 3);
    assert_eq!(app.navigable_indices().count(), 0);
    app.cycle_sort();
    assert_eq!(
        app.hidden_by_collapse(),
        3,
        "explicit grouping retains collapse across sort"
    );
    app.set_view(View::Archive);
    assert!(app.display_rows().is_empty());
    app.set_view(View::List);
    assert_eq!(app.hidden_by_collapse(), 3);
    app.toggle_group();
    assert_eq!(app.hidden_by_collapse(), 2);
    app.prefs.group_by = GroupBy::Auto;
    app.recompute_visible();
    assert_eq!(app.hidden_by_collapse(), 0);
    focus_header(&mut app, GroupKey::ListDue(ListDueBucket::NoDue));
    app.toggle_group();
    app.cycle_sort(); // due -> file changes effective grouping
    assert_eq!(app.hidden_by_collapse(), 0);
    assert!(
        app.display_rows()
            .iter()
            .all(|r| matches!(r, DisplayRow::Task(_)))
    );
}

#[test]
fn grouping_picker_cancel_preserves_every_view_state_and_commit_persists() {
    let mut app = build_app("(A) a\n(B) b\n");
    focus_header(&mut app, GroupKey::ListPriority(Some('A')));
    app.toggle_group();
    app.selection.toggle(1);
    app.mode = Mode::Visual;
    let rows = app.display_rows().to_vec();
    let cursor = app.cursor;
    app.enter_pick_grouping();
    app.pick_grouping_step(true);
    app.pick_grouping_cancel();
    assert_eq!(app.mode, Mode::Visual);
    assert_eq!(app.prefs.group_by, GroupBy::Auto);
    assert_eq!(app.display_rows(), rows);
    assert_eq!(app.cursor, cursor);
    assert!(app.selection.is_selected(1));
    assert!(app.group_collapsed(&GroupKey::ListPriority(Some('A'))));
    app.enter_pick_grouping();
    app.pick_grouping_step(true);
    app.pick_grouping_accept();
    assert_eq!(app.prefs.group_by, GroupBy::None);
    assert_eq!(app.hidden_by_collapse(), 0);
    assert!(app.selection.is_selected(1));
    let cfg = Config::load_from(app.config_path.as_ref().expect("config path"));
    assert_eq!(cfg.group_by, Some(GroupBy::None));
    assert_eq!(cfg.sort, Some(Sort::Priority));
    assert!(app.store.history.is_empty());
}

#[test]
fn selected_task_entering_collapsed_group_is_pruned_and_header_followed() {
    let mut app = build_app("(A) a\nplain\n");
    focus_header(&mut app, GroupKey::ListPriority(Some('A')));
    app.toggle_group();
    app.follow_cursor(1);
    app.selection.toggle(1);
    app.cycle_priority(1); // none -> A
    assert!(app.selection.is_empty());
    assert_eq!(app.flash_active(), Some("cleared 1 hidden selected task"));
    assert_eq!(
        app.focused_header(),
        Some(&GroupKey::ListPriority(Some('A')))
    );
    app.undo();
    assert_eq!(app.navigable_indices().collect::<Vec<_>>(), [1]);
    assert!(app.selection.is_empty());
    // Existing external-reload contract clears selections rather than trying
    // to infer identity from changed line positions; feedback includes count.
    app.selection.toggle(1);
    std::fs::write(&app.file_path, "(A) a\n(A) external\n").expect("fixture");
    assert!(!app.check_external_changes());
    assert_eq!(app.hidden_by_collapse(), 2);
    assert!(app.selection.is_empty());
    assert!(app.flash_active().expect("notice").contains("cleared 1"));
}

#[test]
fn collapsed_tasks_cannot_be_bulk_targets_even_with_invalid_explicit_selection() {
    for delete in [false, true] {
        let raw = "(A) hidden\n(B) visible\n";
        let mut app = build_app(raw);
        focus_header(&mut app, GroupKey::ListPriority(Some('A')));
        app.toggle_group();
        app.selection.toggle(0); // deliberately inject invalid stale selection
        if delete {
            app.delete_selected();
        } else {
            app.complete_selected();
        }
        assert_eq!(
            std::fs::read_to_string(&app.file_path).expect("fixture"),
            raw
        );
        assert_eq!(app.flash_active(), Some("selection includes hidden tasks"));
        app.mode = Mode::Visual;
        app.move_tasks(true);
        assert_eq!(
            std::fs::read_to_string(&app.file_path).expect("fixture"),
            raw
        );
        assert!(app.store.history.is_empty());
    }
}

#[test]
fn selection_follows_task_identity_after_delete_recurring_insert_and_archive() {
    // Selection may persist after leaving Visual mode; subsequent writes must
    // remap identities before recompute can prune collapsed memberships.
    let mut app = build_app("(A) first\n(B) selected\n(C) last\n");
    app.selection.toggle(1);
    app.delete(0);
    assert!(app.selection.is_selected(0));
    assert!(!app.selection.is_selected(1));
    app.mode = Mode::Visual;
    app.delete_selected();
    assert_eq!(app.tasks()[0].raw, "(C) last");

    let mut app = build_app("(A) recurring rec:1d\n(B) selected\n(C) last\n");
    app.selection.toggle(1);
    app.toggle_complete(0);
    assert!(app.selection.is_selected(2));
    assert_eq!(app.tasks()[2].raw, "(B) selected");
    app.mode = Mode::Visual;
    app.delete_selected();
    assert!(app.tasks().iter().any(|t| t.raw == "(C) last"));
    assert!(!app.tasks().iter().any(|t| t.raw == "(B) selected"));

    let mut app = build_app("x 2026-05-01 done\n(B) selected\n(C) last\n");
    app.selection.toggle(1);
    app.archive_completed();
    assert!(app.selection.is_selected(0));
    assert_eq!(app.tasks()[0].raw, "(B) selected");
    app.mode = Mode::Visual;
    app.delete_selected();
    assert_eq!(app.tasks()[0].raw, "(C) last");
}

#[test]
fn bulk_mutations_keep_unselected_surviving_focus_and_next_action() {
    for complete in [false, true] {
        let raw = if complete {
            "(A) Alpha rec:1d\n(A) Beta\n(A) Gamma\n(A) Delta\n"
        } else {
            "(A) Alpha\n(A) Beta\n(A) Gamma\n(A) Delta\n"
        };
        let mut app = build_app(raw);
        app.selection.toggle(0);
        app.follow_cursor(2);
        app.mode = Mode::Visual;
        if complete {
            app.complete_selected();
        } else {
            app.delete_selected();
        }
        assert_eq!(app.cur_task().expect("focused survivor").raw, "(A) Gamma");
        assert_eq!(app.cur_abs(), Some(if complete { 3 } else { 1 }));
        assert!(app.selection.is_empty());
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.store.history.len(), 1);
        let after_bulk = std::fs::read(&app.file_path).expect("fixture");
        app.delete(app.cur_abs().expect("focused task index"));
        assert!(!app.tasks().iter().any(|task| task.raw == "(A) Gamma"));
        assert!(app.tasks().iter().any(|task| task.raw == "(A) Beta"));
        assert!(app.tasks().iter().any(|task| task.raw == "(A) Delta"));
        assert_eq!(app.store.history.len(), 2);
        app.undo();
        assert_eq!(std::fs::read(&app.file_path).expect("fixture"), after_bulk);
        app.undo();
        assert_eq!(
            std::fs::read_to_string(&app.file_path).expect("fixture"),
            raw
        );
        assert!(app.store.history.is_empty());
    }
}

#[test]
fn bulk_focus_remapping_distinguishes_duplicates_and_actual_spawns() {
    // Only Alpha spawns: the other selected row has an invalid recurrence.
    // The two duplicate rows are byte-identical, so raw-text matching cannot
    // identify the focused second occurrence after positions shift.
    let raw = "(A) Alpha rec:1d\n(A) duplicate\n(A) no successor rec:invalid\n(A) duplicate\n(B) collapsed\n";
    for complete in [false, true] {
        let mut app = build_app(raw);
        focus_header(&mut app, GroupKey::ListPriority(Some('B')));
        app.toggle_group();
        app.selection.toggle(0);
        app.selection.toggle(2);
        app.follow_cursor(3);
        app.mode = Mode::Visual;
        if complete {
            app.complete_selected();
        } else {
            app.delete_selected();
        }
        let focused = if complete { 4 } else { 1 };
        let first_duplicate = if complete { 2 } else { 0 };
        assert_eq!(app.cur_abs(), Some(focused));
        assert_eq!(app.tasks()[focused].raw, app.tasks()[first_duplicate].raw);
        assert!(app.group_collapsed(&GroupKey::ListPriority(Some('B'))));
        assert_eq!(app.hidden_by_collapse(), 1);
        assert!(app.selection.is_empty());
        assert_eq!(app.store.history.len(), 1);
        app.cycle_priority(app.cur_abs().expect("second duplicate"));
        assert_eq!(app.tasks()[first_duplicate].priority, Some('A'));
        assert_eq!(app.tasks()[focused].priority, Some('B'));
        assert_eq!(
            app.focused_header(),
            Some(&GroupKey::ListPriority(Some('B')))
        );
        assert_eq!(app.hidden_by_collapse(), 2);
        assert_eq!(app.store.history.len(), 2);
        app.undo();
        app.undo();
        assert_eq!(
            std::fs::read_to_string(&app.file_path).expect("fixture"),
            raw
        );
        assert!(app.store.history.is_empty());
    }
}

#[test]
fn bulk_focus_preserves_collapsed_header_and_removed_task_falls_back_nearby() {
    for complete in [false, true] {
        let mut app = build_app("(A) Alpha rec:1d\n(A) Beta\n(B) hidden\n");
        focus_header(&mut app, GroupKey::ListPriority(Some('B')));
        app.toggle_group();
        app.selection.toggle(0);
        app.mode = Mode::Visual;
        if complete {
            app.complete_selected();
        } else {
            app.delete_selected();
        }
        assert_eq!(
            app.focused_header(),
            Some(&GroupKey::ListPriority(Some('B')))
        );
        assert_eq!(app.cur_abs(), None);
        assert!(app.group_collapsed(&GroupKey::ListPriority(Some('B'))));
    }
    let mut app = build_app("(A) Alpha\n(A) Beta\n(A) Gamma\n(A) Delta\n");
    app.follow_cursor(2);
    app.selection.toggle(2);
    app.delete_selected();
    assert_eq!(app.cur_task().expect("nearby fallback").raw, "(A) Delta");
    assert!(app.cursor < app.display_rows().len());
    // A completed focused task may be filtered out; use a valid nearby row.
    app.follow_cursor(0);
    app.selection.toggle(0);
    app.complete_selected();
    assert_eq!(
        app.cur_task().expect("filtered focus fallback").raw,
        "(A) Beta"
    );
}
