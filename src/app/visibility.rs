use std::cmp::Ordering;

use super::App;
use super::types::{Filter, Sort, View};
use crate::todo::{self, Task};

/// Which canonical group a Today row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodayBucket {
    Overdue,
    Today,
    Upcoming,
}

impl TodayBucket {
    pub fn idx(self) -> usize {
        match self {
            TodayBucket::Overdue => 0,
            TodayBucket::Today => 1,
            TodayBucket::Upcoming => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TodayBucket::Overdue => "OVERDUE",
            TodayBucket::Today => "TODAY",
            TodayBucket::Upcoming => "UPCOMING",
        }
    }
}

/// One entry per visible row, parallel to `visible_cache`. Renderers detect
/// group transitions by comparing successive entries; List has no groups so
/// every row is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    None,
    TodayBucket(TodayBucket),
    ArchiveDate(String),
}

impl App {
    /// Indices into the active view's task source after filter + sort, in
    /// display order. The source is `self.archive.tasks()` in Archive view,
    /// `self.tasks` otherwise. Reads the cache populated by `recompute_visible`.
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_cache
    }

    /// Group key per row, parallel to `visible_indices()`. `GroupKey::None`
    /// for List view; bucket/date keys for Today/Archive.
    pub fn visible_groups(&self) -> &[GroupKey] {
        &self.visible_groups
    }

    /// Recompute the cached visible-index list and parallel group keys. Call
    /// after any mutation that affects filter/sort/view/tasks/archive.
    pub fn recompute_visible(&mut self) {
        match self.view {
            View::List => self.rebuild_list_cache(),
            View::Today => self.rebuild_today_cache(),
            View::Archive => self.rebuild_archive_cache(),
        }
    }

    fn rebuild_list_cache(&mut self) {
        let needle_owned =
            (!self.filter.search.is_empty()).then(|| self.filter.search.to_lowercase());
        let needle = needle_owned.as_deref();

        let mut idxs: Vec<usize> = (0..self.tasks.len())
            .filter(|&i| list_predicate(&self.tasks[i], self.prefs.show_done, &self.filter, needle))
            .collect();

        sort_by_prefs(&mut idxs, &self.tasks, self.prefs.sort);

        self.visible_groups = vec![GroupKey::None; idxs.len()];
        self.visible_cache = idxs;
    }

    fn rebuild_today_cache(&mut self) {
        let needle_owned =
            (!self.filter.search.is_empty()).then(|| self.filter.search.to_lowercase());
        let needle = needle_owned.as_deref();
        let today_str = self.today.as_str();

        let mut overdue: Vec<usize> = Vec::new();
        let mut due_today: Vec<usize> = Vec::new();
        let mut upcoming: Vec<usize> = Vec::new();
        for i in 0..self.tasks.len() {
            if !today_predicate(&self.tasks[i], &self.filter, needle) {
                continue;
            }
            let Some(d) = self.tasks[i].due.as_deref() else {
                continue;
            };
            match d.cmp(today_str) {
                Ordering::Less => overdue.push(i),
                Ordering::Equal => due_today.push(i),
                Ordering::Greater => upcoming.push(i),
            }
        }
        sort_by_prefs(&mut overdue, &self.tasks, self.prefs.sort);
        sort_by_prefs(&mut due_today, &self.tasks, self.prefs.sort);
        // Upcoming: always due-asc within bucket so the soonest-due is at top
        // regardless of the user's global Sort preference.
        upcoming.sort_by(cmp_due(&self.tasks));

        let mut idxs: Vec<usize> = Vec::with_capacity(overdue.len() + due_today.len() + upcoming.len());
        let mut groups: Vec<GroupKey> = Vec::with_capacity(idxs.capacity());
        for i in &overdue {
            idxs.push(*i);
            groups.push(GroupKey::TodayBucket(TodayBucket::Overdue));
        }
        for i in &due_today {
            idxs.push(*i);
            groups.push(GroupKey::TodayBucket(TodayBucket::Today));
        }
        for i in &upcoming {
            idxs.push(*i);
            groups.push(GroupKey::TodayBucket(TodayBucket::Upcoming));
        }
        self.visible_cache = idxs;
        self.visible_groups = groups;
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

fn sort_by_prefs(idxs: &mut [usize], tasks: &[Task], sort: Sort) {
    match sort {
        Sort::Priority => idxs.sort_by(cmp_priority(tasks)),
        Sort::Due => idxs.sort_by(cmp_due(tasks)),
        Sort::File => { /* preserve order */ }
    }
}

/// Project / context / search predicate, shared by every view that honors
/// user filters. `needle` is pre-lowercased by the caller.
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
        let body = todo::body_after_priority(&t.raw).to_lowercase();
        if !body.contains(needle) {
            return false;
        }
    }
    true
}

fn list_predicate(t: &Task, show_done: bool, filter: &Filter, needle: Option<&str>) -> bool {
    if t.done && !show_done {
        return false;
    }
    passes_user_filter(t, filter, needle)
}

fn today_predicate(t: &Task, filter: &Filter, needle: Option<&str>) -> bool {
    !t.done && passes_user_filter(t, filter, needle)
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
    fn today_view_respects_user_filters() {
        let raw = "(A) 2026-05-01 a +work due:2026-05-06\n\
                   (A) 2026-05-01 b +home due:2026-05-06\n\
                   (A) 2026-05-01 c +work due:2026-05-10\n";
        let mut app = build_app(raw);
        app.view = View::Today;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 3);
        app.filter.project = Some("work".into());
        app.recompute_visible();
        // Only the two +work tasks should remain in the agenda.
        assert_eq!(app.visible_indices().len(), 2);
        for &i in app.visible_indices() {
            assert!(app.tasks[i].projects.iter().any(|p| p == "work"));
        }
    }

    #[test]
    fn today_indices_are_in_bucket_order() {
        // 2026-05-06 is "today" per build_app. Build one task in each bucket.
        let raw = "a due:2026-05-04\n\
                   b due:2026-05-06\n\
                   c due:2026-05-08\n";
        let mut app = build_app(raw);
        app.view = View::Today;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 3);
        assert!(matches!(
            groups[0],
            GroupKey::TodayBucket(TodayBucket::Overdue)
        ));
        assert!(matches!(
            groups[1],
            GroupKey::TodayBucket(TodayBucket::Today)
        ));
        assert!(matches!(
            groups[2],
            GroupKey::TodayBucket(TodayBucket::Upcoming)
        ));
    }

    #[test]
    fn today_groups_align_with_indices() {
        let raw = "x due:2026-05-04\n\
                   a due:2026-05-04\n\
                   b due:2026-05-08\n";
        let mut app = build_app(raw);
        // First task is done — must be excluded by today_predicate.
        app.view = View::Today;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), app.visible_groups().len());
        assert_eq!(app.visible_indices().len(), 2);
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
