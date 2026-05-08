use std::cmp::Ordering;

use super::App;
use super::types::{Filter, Sort, View};
use crate::todo::{self, Task};

impl App {
    /// Indices into `self.tasks` after filter + sort, for the active view.
    /// Reads the cache populated by `recompute_visible`.
    pub fn visible_indices(&self) -> &[usize] {
        &self.visible_cache
    }

    /// Recompute the cached visible-index list. Call after any mutation that
    /// affects filter/sort/view/tasks.
    pub fn recompute_visible(&mut self) {
        let needle_owned =
            (!self.filter.search.is_empty()).then(|| self.filter.search.to_lowercase());
        let needle = needle_owned.as_deref();

        let mut idxs: Vec<usize> = (0..self.tasks.len())
            .filter(|&i| match self.view {
                View::List => {
                    list_predicate(&self.tasks[i], self.prefs.show_done, &self.filter, needle)
                }
                View::Today => today_predicate(&self.tasks[i], &self.filter, needle),
                View::Archive => archive_predicate(&self.tasks[i]),
            })
            .collect();

        match self.prefs.sort {
            Sort::Priority => idxs.sort_by(cmp_priority(&self.tasks)),
            Sort::Due => idxs.sort_by(cmp_due(&self.tasks)),
            Sort::File => { /* preserve order */ }
        }

        if matches!(self.view, View::Archive) {
            idxs.sort_by(cmp_archive_done_date(&self.tasks));
        }

        self.visible_cache = idxs;
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

fn archive_predicate(t: &Task) -> bool {
    t.done
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

/// Archive view: most-recently-completed first.
fn cmp_archive_done_date(tasks: &[Task]) -> impl Fn(&usize, &usize) -> Ordering + '_ {
    |&a, &b| {
        tasks[b]
            .done_date
            .as_deref()
            .unwrap_or("")
            .cmp(tasks[a].done_date.as_deref().unwrap_or(""))
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
}
