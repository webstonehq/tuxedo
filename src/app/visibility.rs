use std::cmp::Ordering;

use super::App;
use super::types::{Filter, Sort, View};
use crate::search::subseq_match_ci;
use crate::threshold;
use crate::todo::{self, Task};

/// Which canonical bucket a List-view row belongs to when the active sort is
/// `Sort::Due`. `NoDue` covers tasks with no `due:` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListDueBucket {
    Overdue,
    Today,
    Upcoming,
    NoDue,
}

impl ListDueBucket {
    pub fn label(self) -> &'static str {
        match self {
            ListDueBucket::Overdue => "OVERDUE",
            ListDueBucket::Today => "TODAY",
            ListDueBucket::Upcoming => "UPCOMING",
            ListDueBucket::NoDue => "NO DUE DATE",
        }
    }
}

/// One entry per visible row, parallel to `visible_cache`. Renderers detect
/// group transitions by comparing successive entries; under `Sort::File` every
/// row is `None` so the renderer skips headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    None,
    ArchiveDate(String),
    /// `Some('A'..='Z')` for a graded priority, `None` for unprioritized.
    ListPriority(Option<char>),
    ListDue(ListDueBucket),
}

impl App {
    /// Indices into the active view's task source after filter + sort, in
    /// display order. The source is `self.archive.tasks()` in Archive view,
    /// `self.tasks` otherwise. Reads the cache populated by `recompute_visible`.
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_cache
    }

    /// Group key per row, parallel to `visible_indices()`. `GroupKey::None`
    /// when List is sorted by file order; priority/due bucket keys under
    /// other List sorts; date keys under Archive.
    pub fn visible_groups(&self) -> &[GroupKey] {
        &self.visible_groups
    }

    /// Recompute the cached visible-index list and parallel group keys. Call
    /// after any mutation that affects filter/sort/view/tasks/archive.
    pub fn recompute_visible(&mut self) {
        match self.view {
            View::List => self.rebuild_list_cache(),
            View::Archive => self.rebuild_archive_cache(),
        }
    }

    fn rebuild_list_cache(&mut self) {
        let needle = (!self.filter.search.is_empty()).then_some(self.filter.search.as_str());

        let mut idxs: Vec<usize> = (0..self.tasks.len())
            .filter(|&i| {
                list_predicate(
                    &self.tasks[i],
                    self.prefs.show_done,
                    self.prefs.show_future,
                    self.today.as_str(),
                    &self.filter,
                    needle,
                )
            })
            .collect();

        sort_by_prefs(&mut idxs, &self.tasks, self.prefs.sort);

        let groups: Vec<GroupKey> = match self.prefs.sort {
            Sort::File => vec![GroupKey::None; idxs.len()],
            Sort::Priority => idxs
                .iter()
                .map(|&i| GroupKey::ListPriority(self.tasks[i].priority))
                .collect(),
            Sort::Due => {
                let today = self.today.as_str();
                idxs.iter()
                    .map(|&i| GroupKey::ListDue(due_bucket(&self.tasks[i], today)))
                    .collect()
            }
        };
        self.visible_groups = groups;
        self.visible_cache = idxs;
    }

    fn rebuild_archive_cache(&mut self) {
        let archive = self.archive.tasks();
        let mut idxs: Vec<usize> = (0..archive.len()).collect();
        idxs.sort_by(|&a, &b| {
            archive[b]
                .done_date
                .as_deref()
                .unwrap_or("")
                .cmp(archive[a].done_date.as_deref().unwrap_or(""))
        });
        let groups: Vec<GroupKey> = idxs
            .iter()
            .map(|&i| {
                let date = archive[i]
                    .done_date
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                GroupKey::ArchiveDate(date)
            })
            .collect();
        self.visible_cache = idxs;
        self.visible_groups = groups;
    }

    pub fn cur_abs(&self) -> Option<usize> {
        self.visible_cache.get(self.cursor).copied()
    }

    pub fn clamp_cursor(&mut self) {
        let len = self.visible_cache.len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    /// Move the cursor to wherever `abs` lives in the current visible list.
    /// Falls back to clamping if `abs` was filtered out.
    pub(super) fn follow_cursor(&mut self, abs: usize) {
        if let Some(pos) = self.visible_cache.iter().position(|&i| i == abs) {
            self.cursor = pos;
        } else {
            self.clamp_cursor();
        }
    }
}

fn due_bucket(task: &Task, today: &str) -> ListDueBucket {
    match task.due.as_deref() {
        None => ListDueBucket::NoDue,
        Some(d) => match d.cmp(today) {
            Ordering::Less => ListDueBucket::Overdue,
            Ordering::Equal => ListDueBucket::Today,
            Ordering::Greater => ListDueBucket::Upcoming,
        },
    }
}

fn sort_by_prefs(idxs: &mut [usize], tasks: &[Task], sort: Sort) {
    match sort {
        Sort::Priority => idxs.sort_by(cmp_priority(tasks)),
        Sort::Due => idxs.sort_by(cmp_due(tasks)),
        Sort::File => { /* preserve order */ }
    }
}

/// Project / context / search predicate, shared by every view that honors
/// user filters. `needle` matches as a case-insensitive subsequence of the
/// task body — chars must appear in order, gaps allowed.
fn passes_user_filter(t: &Task, filter: &Filter, needle: Option<&str>) -> bool {
    if let Some(p) = &filter.project
        && !t.projects.iter().any(|x| x == p)
    {
        return false;
    }
    if let Some(c) = &filter.context
        && !t.contexts.iter().any(|x| x == c)
    {
        return false;
    }
    if let Some(needle) = needle {
        let body = todo::body_after_priority(&t.raw);
        if subseq_match_ci(body, needle).is_none() {
            return false;
        }
    }
    true
}

fn list_predicate(
    t: &Task,
    show_done: bool,
    show_future: bool,
    today: &str,
    filter: &Filter,
    needle: Option<&str>,
) -> bool {
    if t.done && !show_done {
        return false;
    }
    if !show_future && is_future_threshold(t, today) {
        return false;
    }
    passes_user_filter(t, filter, needle)
}

/// True when the task carries a `t:` value that resolves to a date strictly
/// after `today`. Malformed values, missing anchors for relative offsets,
/// and arithmetic overflow all leave the task visible — better to surface a
/// task the user might miss than to hide it because of a bad threshold.
fn is_future_threshold(t: &Task, today: &str) -> bool {
    let Some(raw) = t.threshold.as_deref() else {
        return false;
    };
    let Some(spec) = threshold::parse_threshold(raw) else {
        return false;
    };
    let Some(date) = threshold::resolve(&spec, t.due.as_deref(), t.created_date.as_deref()) else {
        return false;
    };
    date.format("%Y-%m-%d").to_string().as_str() > today
}

/// Sort by priority asc (None last), tie-broken by due-date asc.
fn cmp_priority(tasks: &[Task]) -> impl Fn(&usize, &usize) -> Ordering + '_ {
    |&a, &b| {
        let ta = &tasks[a];
        let tb = &tasks[b];
        let pa = ta.priority.unwrap_or('Z');
        let pb = tb.priority.unwrap_or('Z');
        pa.cmp(&pb).then_with(|| {
            ta.due
                .as_deref()
                .unwrap_or("z")
                .cmp(tb.due.as_deref().unwrap_or("z"))
        })
    }
}

/// Sort by due-date asc (None last).
fn cmp_due(tasks: &[Task]) -> impl Fn(&usize, &usize) -> Ordering + '_ {
    |&a, &b| {
        tasks[a]
            .due
            .as_deref()
            .unwrap_or("z")
            .cmp(tasks[b].due.as_deref().unwrap_or("z"))
    }
}

/// Order projects/contexts the same way the filter sidebar does:
/// count descending, then name ascending. Used by both the picker and
/// the sidebar so j/k advances visibly down the list.
pub fn ordered_unique<F>(tasks: &[Task], pick: F) -> Vec<(String, usize)>
where
    F: Fn(&Task) -> &Vec<String>,
{
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in tasks.iter().filter(|t| !t.done) {
        for v in pick(t) {
            *counts.entry(v.clone()).or_insert(0) += 1;
        }
    }
    let mut out: Vec<(String, usize)> = counts.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

pub(super) fn unique_values<F>(tasks: &[Task], pick: F) -> Vec<String>
where
    F: Fn(&Task) -> &Vec<String>,
{
    ordered_unique(tasks, pick)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn unique_values_dedups_and_sorts() {
        let raw = "(A) 2026-05-01 a +work +health\n2026-05-01 b +work\n2026-05-01 c +health\n";
        let tasks = crate::todo::parse_file(raw);
        let projects = unique_values(&tasks, |t| &t.projects);
        assert_eq!(projects, vec!["health".to_string(), "work".to_string()]);
    }

    #[test]
    fn search_matches_subsequence() {
        // Subsequence match: "cade" finds C, a, D, e in "Call dentist".
        let mut app = build_app("2026-05-01 Call dentist\n2026-05-01 buy milk\n");
        app.filter.search = "cade".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn search_matches_body_not_dates() {
        // A task whose only "2026" sits in the created-date prefix should
        // NOT match a search for "2026" — search runs against the body.
        let mut app = build_app("2026-05-01 buy milk\n2026-04-01 something else\n");
        app.filter.search = "2026".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 0);
    }

    #[test]
    fn visible_cache_updates_after_mutation() {
        let mut app = build_app("a\nb\nc\n");
        assert_eq!(app.visible_indices().len(), 3);
        app.draft_set("d".into());
        app.add_from_draft();
        assert_eq!(app.visible_indices().len(), 4);
    }

    #[test]
    fn list_cursor_survives_archive_roundtrip() {
        let mut app = build_app("a\nb\nc\nd\ne\n");
        app.cursor = 3;
        app.set_view(View::Archive);
        // Archive's visible_cache may have a different length; the per-view
        // cursor cache restores cursor on the way back regardless.
        app.set_view(View::List);
        assert_eq!(app.cursor, 3, "cursor lost on List → Archive → List");
    }

    #[test]
    fn archive_indices_point_into_archive_tasks() {
        let mut app = build_app("a\n");
        let path = app.archive.path().to_path_buf();
        app.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(
                "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-02 second\n",
            ),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let idxs = app.visible_indices();
        assert_eq!(idxs.len(), 2);
        for &i in idxs {
            assert!(app.archive.tasks().get(i).is_some());
        }
    }

    #[test]
    fn list_groups_are_none_under_sort_file() {
        let mut app = build_app("(A) a\n(B) b\nc\n");
        app.prefs.sort = Sort::File;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 3);
        for g in groups {
            assert!(matches!(g, GroupKey::None));
        }
    }

    #[test]
    fn list_groups_track_priority_under_sort_priority() {
        let mut app = build_app("(A) a\n(B) b\nc\n(A) a2\n");
        app.prefs.sort = Sort::Priority;
        app.recompute_visible();
        // After priority sort: (A) a, (A) a2, (B) b, c (no priority).
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[1], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[2], GroupKey::ListPriority(Some('B')));
        assert_eq!(groups[3], GroupKey::ListPriority(None));
    }

    #[test]
    fn list_groups_bucket_due_dates_under_sort_due() {
        // build_app uses today = 2026-05-06.
        let raw = "a due:2026-05-04\n\
                   b due:2026-05-06\n\
                   c due:2026-05-08\n\
                   d\n";
        let mut app = build_app(raw);
        app.prefs.sort = Sort::Due;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0], GroupKey::ListDue(ListDueBucket::Overdue));
        assert_eq!(groups[1], GroupKey::ListDue(ListDueBucket::Today));
        assert_eq!(groups[2], GroupKey::ListDue(ListDueBucket::Upcoming));
        assert_eq!(groups[3], GroupKey::ListDue(ListDueBucket::NoDue));
    }

    #[test]
    fn future_absolute_threshold_hidden_by_default() {
        // build_app uses today = 2026-05-06.
        let mut app = build_app("future task t:2030-01-01\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);
        assert_eq!(app.tasks[app.visible_indices()[0]].raw, "visible task");
        // Toggling show_future reveals it.
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn past_absolute_threshold_always_visible() {
        let app = build_app("past task t:2020-01-01\n");
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn relative_threshold_anchors_on_due() {
        // today = 2026-05-06; due = 2026-05-15; t:-3d → threshold 2026-05-12,
        // which is after today → hidden.
        let mut app = build_app("Pay rent due:2026-05-15 t:-3d\n");
        assert_eq!(app.visible_indices().len(), 0);
        // Bumping due into the past brings it back even with the same offset.
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn relative_threshold_falls_back_to_created_date() {
        // today = 2026-05-06; created = 2026-04-01; t:7d → 2026-04-08, past → visible.
        let app = build_app("2026-04-01 deferred task t:7d\n");
        assert_eq!(app.visible_indices().len(), 1);
        // Same task with t:60d → 2026-05-31, future → hidden.
        let app = build_app("2026-04-01 deferred task t:60d\n");
        assert_eq!(app.visible_indices().len(), 0);
    }

    #[test]
    fn relative_threshold_without_anchor_is_ignored() {
        // No due, no created_date, relative threshold → can't resolve →
        // task stays visible (permissive fallback).
        let app = build_app("orphan t:-3d\n");
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn malformed_threshold_is_ignored() {
        let app = build_app("buggy t:not-a-date\n");
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn refresh_today_unhides_tasks_when_date_advances() {
        // build_app uses today = 2026-05-06; threshold 2026-05-07 hides the
        // task. Crossing midnight should reveal it without an app restart.
        let mut app = build_app("future task t:2026-05-07\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);

        let changed = app.refresh_today("2026-05-07".into());
        assert!(
            changed,
            "refresh_today must report a change on date advance"
        );
        assert_eq!(app.today, "2026-05-07");
        assert_eq!(
            app.visible_indices().len(),
            2,
            "task whose threshold is now today must become visible"
        );
    }

    #[test]
    fn refresh_today_is_noop_when_date_unchanged() {
        let mut app = build_app("a\n");
        let changed = app.refresh_today("2026-05-06".into());
        assert!(!changed, "same date must report no change");
        assert_eq!(app.today, "2026-05-06");
    }

    #[test]
    fn archive_visible_groups_are_done_date_desc() {
        let mut app = build_app("a\n");
        let path = app.archive.path().to_path_buf();
        app.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(
                "x 2026-04-01 2026-03-01 older\nx 2026-05-02 2026-04-02 newer\n",
            ),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 2);
        // First is most-recent done_date.
        let first = match &groups[0] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        let second = match &groups[1] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        assert_eq!(first, "2026-05-02");
        assert_eq!(second, "2026-04-01");
    }
}
