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
