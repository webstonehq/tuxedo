//! Pure filtering, sorting, and grouping helpers over `[Task]`.
//!
//! These take plain data (`&Task` / `&[Task]` + `Filter`/`Sort` + flags) and
//! return decisions or orderings. They hold no view state, so both the TUI's
//! `recompute_visible` (which owns the visible-index cache) and the CLI's
//! `list`/`listpri`/`listproj`/`listcon` commands reuse them.

use std::cmp::Ordering;

use chrono::{Datelike, Days, NaiveDate};

use crate::app::{Filter, Sort, WeekStart};
use crate::due_filter;
use crate::search::subseq_match_ci;
use crate::threshold;
use crate::todo::{self, Task};

/// Which canonical bucket a List-view row belongs to when the active sort is
/// `Sort::Due`. `NoDue` covers tasks with no `due:` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListDueBucket {
    Overdue,
    Today,
    ThisWeek,
    NextWeek,
    Later,
    NoDue,
}

impl ListDueBucket {
    pub fn label(self) -> &'static str {
        match self {
            ListDueBucket::Overdue => "OVERDUE",
            ListDueBucket::Today => "TODAY",
            ListDueBucket::ThisWeek => "THIS WEEK",
            ListDueBucket::NextWeek => "NEXT WEEK",
            ListDueBucket::Later => "LATER",
            ListDueBucket::NoDue => "NO DUE DATE",
        }
    }
}

pub fn get_week_cutoff(today: &str, week_start: &WeekStart) -> Option<(String, String)> {
    let today = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    let weekday = today.weekday();

    let days_from_start_week = match week_start {
        WeekStart::Sunday => weekday.num_days_from_sunday(),
        WeekStart::Monday => weekday.num_days_from_monday(),
    };

    let days_til_week_end = 6 - days_from_start_week;

    let end_this_week = today.checked_add_days(Days::new(days_til_week_end as u64))?;
    let end_next_week = today.checked_add_days(Days::new((days_til_week_end + 7) as u64))?;

    Some((end_this_week.to_string(), end_next_week.to_string()))
}

/// If the date cannot be parsed we assign to Later
pub fn due_bucket(task: &Task, today: &str, week_start: &WeekStart) -> ListDueBucket {
    match task.due.as_deref() {
        None => ListDueBucket::NoDue,
        Some(d) => {
            let Some((this_week, next_week)) = get_week_cutoff(today, week_start) else {
                return ListDueBucket::Later;
            };

            match d.cmp(today) {
                Ordering::Less => ListDueBucket::Overdue,
                Ordering::Equal => ListDueBucket::Today,
                Ordering::Greater if d <= this_week.as_str() => ListDueBucket::ThisWeek,
                Ordering::Greater if d <= next_week.as_str() => ListDueBucket::NextWeek,
                Ordering::Greater => ListDueBucket::Later,
            }
        }
    }
}

pub fn sort_by_prefs(idxs: &mut [usize], tasks: &[Task], sort: Sort) {
    match sort {
        Sort::Priority => idxs.sort_by(cmp_priority(tasks)),
        Sort::Due => idxs.sort_by(cmp_due(tasks)),
        Sort::File => { /* preserve order */ }
    }
}

/// A search string resolved once per query: the literal text to
/// subsequence-match, plus an inclusive `due:` range if the query had a
/// parseable `due:` term. Building this is not free — a `due:` value can
/// drive a business-day walk in [`crate::threshold`] — so callers resolve it
/// once per search change via [`resolve_needle`] and reuse it across every
/// task, rather than re-parsing per task inside [`passes_user_filter`].
pub struct ResolvedNeedle {
    /// Remaining free text to subsequence-match against the task body — the
    /// original search with any parsed `due:` term removed. Equal to the
    /// original search string when no `due:` term parsed.
    pub text: String,
    due_range: Option<(String, String)>,
}

/// Resolve `needle` against `today`: pull the first parseable `due:` term out
/// (if any) and compute its inclusive range, leaving the rest as literal
/// search text. An unparseable `due:` term, or a second `due:` term, is left
/// in place as literal text — only the first parseable one is honored.
pub fn resolve_needle(needle: &str, today: &str) -> ResolvedNeedle {
    match extract_due_range(needle, today) {
        Some((rest, range)) => ResolvedNeedle {
            text: rest,
            due_range: Some(range),
        },
        None => ResolvedNeedle {
            text: needle.to_string(),
            due_range: None,
        },
    }
}

/// Project / context / search predicate, shared by every view that honors
/// user filters. `needle` matches as a case-insensitive subsequence of the
/// task body — chars must appear in order, gaps allowed. When it carries a
/// resolved `due:` range, that's matched against `t.due` instead; see
/// [`resolve_needle`].
pub fn passes_user_filter(t: &Task, filter: &Filter, needle: Option<&ResolvedNeedle>) -> bool {
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
        if let Some((from, to)) = &needle.due_range {
            match t.due.as_deref() {
                Some(d) if d >= from.as_str() && d <= to.as_str() => {}
                _ => return false,
            }
        }
        if !needle.text.is_empty() {
            let body = todo::body_after_priority(&t.raw);
            if subseq_match_ci(body, &needle.text).is_none() {
                return false;
            }
        }
    }
    true
}

/// Pull the first parseable `due:` term out of `needle`, if any, returning
/// the remaining text plus the resolved `(from, to)` range. An unparseable
/// `due:` term is left in place as literal text. `None` when nothing parses,
/// so callers with no `due:` usage see unchanged matching behavior.
fn extract_due_range(needle: &str, today: &str) -> Option<(String, (String, String))> {
    let today_date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    let mut found = None;
    let mut rest: Vec<&str> = Vec::new();
    for term in needle.split_whitespace() {
        if found.is_none()
            && let Some(value) = term.strip_prefix("due:")
            && let Some(range) = due_filter::parse_due_range(value, today_date)
        {
            found = Some(range);
            continue;
        }
        rest.push(term);
    }
    found.map(|range| (rest.join(" "), range))
}

pub fn list_predicate(
    t: &Task,
    show_done: bool,
    show_future: bool,
    today: &str,
    filter: &Filter,
    needle: Option<&ResolvedNeedle>,
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
pub fn is_future_threshold(t: &Task, today: &str) -> bool {
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
        let ta = &tasks[a];
        let tb = &tasks[b];
        let da = ta.due.as_deref().unwrap_or("z");
        let db = tb.due.as_deref().unwrap_or("z");
        da.cmp(db)
            .then_with(|| ta.priority.unwrap_or('Z').cmp(&tb.priority.unwrap_or('Z')))
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

pub fn unique_values<F>(tasks: &[Task], pick: F) -> Vec<String>
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

    #[test]
    fn unique_values_dedups_and_sorts() {
        let raw = "(A) 2026-05-01 a +work +health\n2026-05-01 b +work\n2026-05-01 c +health\n";
        let tasks = crate::todo::parse_file(raw);
        let projects = unique_values(&tasks, |t| &t.projects);
        assert_eq!(projects, vec!["health".to_string(), "work".to_string()]);
    }

    fn resolved(needle: &str, today: &str) -> ResolvedNeedle {
        resolve_needle(needle, today)
    }

    #[test]
    fn due_exact_date_matches_structurally() {
        let raw = "buy milk due:2026-08-01\nbuy eggs due:2026-08-02\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:2026-08-01", "2026-07-28");
        assert!(passes_user_filter(&tasks[0], &filter, Some(&needle)));
        assert!(!passes_user_filter(&tasks[1], &filter, Some(&needle)));
    }

    #[test]
    fn due_plus_range_matches_within_window() {
        let raw = "today due:2026-07-28\nin range due:2026-08-04\nout of range due:2026-08-05\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:+1w", "2026-07-28");
        for t in &tasks[..2] {
            assert!(passes_user_filter(t, &filter, Some(&needle)));
        }
        assert!(!passes_user_filter(&tasks[2], &filter, Some(&needle)));
    }

    #[test]
    fn due_minus_range_matches_past_window() {
        let raw = "recent due:2026-07-25\ntoday due:2026-07-28\ntoo old due:2026-07-24\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:-3d", "2026-07-28");
        for t in &tasks[..2] {
            assert!(passes_user_filter(t, &filter, Some(&needle)));
        }
        assert!(!passes_user_filter(&tasks[2], &filter, Some(&needle)));
    }

    #[test]
    fn due_term_with_no_task_due_date_never_matches() {
        let raw = "no due date at all\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:+1w", "2026-07-28");
        assert!(!passes_user_filter(&tasks[0], &filter, Some(&needle)));
    }

    #[test]
    fn invalid_due_term_falls_back_to_literal_search() {
        let raw = "note due:xyz reminder\nunrelated task\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:xyz", "2026-07-28");
        assert!(passes_user_filter(&tasks[0], &filter, Some(&needle)));
        assert!(!passes_user_filter(&tasks[1], &filter, Some(&needle)));
    }

    #[test]
    fn second_due_term_falls_back_to_literal_text() {
        // Only the first `due:` term is honored; the second is literal text,
        // so a match here only happens to work because "2026-07-31 do"
        // spells out a `due:-3d` subsequence.
        let raw = "due:2026-07-31 do it\ndue:2026-08-01 other task\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:+1w due:-3d", "2026-07-28");
        assert!(passes_user_filter(&tasks[0], &filter, Some(&needle)));
        assert!(!passes_user_filter(&tasks[1], &filter, Some(&needle)));
    }

    #[test]
    fn malformed_task_due_value_compares_lexicographically() {
        // `due:` isn't validated as a real date — the range check is a plain
        // string compare, same as `due_bucket`/`cmp_due`.
        let raw = "garbage due:tbd\nin range due:2026-08-01\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:+1w", "2026-07-28");
        assert!(!passes_user_filter(&tasks[0], &filter, Some(&needle)));
        assert!(passes_user_filter(&tasks[1], &filter, Some(&needle)));
    }

    #[test]
    fn due_term_combined_with_free_text() {
        let raw = "buy groceries due:2026-08-01\nbuy stamps due:2026-08-01\n";
        let tasks = crate::todo::parse_file(raw);
        let filter = Filter::default();
        let needle = resolved("due:+1w groceries", "2026-07-28");
        assert!(passes_user_filter(&tasks[0], &filter, Some(&needle)));
        assert!(!passes_user_filter(&tasks[1], &filter, Some(&needle)));
    }

    #[test]
    fn priority_sort_keeps_due_fallback_and_file_order_for_ties() {
        let tasks = crate::todo::parse_file(
            "(A) later due:2026-06-20\n(B) middle\n(A) same first due:2026-06-01\n(A) sooner due:2026-05-01\n(A) same second due:2026-06-01\nnone\n",
        );
        let mut indices = vec![0, 1, 2, 3, 4, 5];
        sort_by_prefs(&mut indices, &tasks, Sort::Priority);
        assert_eq!(indices, [3, 2, 4, 0, 1, 5]);
    }

    #[test]
    fn get_week_cutoffs_for_all_configs() {
        let (end_this_week, end_next_week) = get_week_cutoff("2026-06-18", &WeekStart::Sunday)
            .expect("unable to get the week cutoff date");
        assert_eq!(end_this_week, "2026-06-20");
        assert_eq!(end_next_week, "2026-06-27");

        let (end_this_week, end_next_week) = get_week_cutoff("2026-06-18", &WeekStart::Monday)
            .expect("unable to get the week cutoff date");
        assert_eq!(end_this_week, "2026-06-21");
        assert_eq!(end_next_week, "2026-06-28");
    }
}
