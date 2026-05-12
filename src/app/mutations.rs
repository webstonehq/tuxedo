use super::{AddOutcome, App};
use crate::recurrence::{self, RecSpec};
use crate::todo::{self, TagError};

impl App {
    pub fn toggle_complete(&mut self, abs: usize) {
        if !self.check_external_changes() {
            return;
        }
        let Some(t) = self.tasks.get(abs) else {
            return;
        };
        let was_done = t.done;
        // Capture rec/due/raw of the *pre-completion* task — `mark_done`
        // rewrites `raw` (and strips priority), so the next-instance build
        // must read these fields before the mutation lands.
        let rec_spec = if was_done {
            None
        } else {
            t.rec.as_deref().and_then(recurrence::parse_rec_spec)
        };
        let raw_before = t.raw.clone();
        let due_before = t.due.clone();

        self.push_history();
        let result = if was_done {
            self.tasks[abs].unmark_done()
        } else {
            self.tasks[abs].mark_done(&self.today)
        };
        match result {
            Ok(()) => {
                let spawned = rec_spec.and_then(|spec| {
                    let next_raw = build_next_instance(
                        &raw_before,
                        due_before.as_deref(),
                        &spec,
                        &self.today,
                    )?;
                    let parsed = todo::parse_line(&next_raw).ok()?;
                    // Vec only grew between check_external_changes and here
                    // (mark_done replaced one entry in place), so abs+1 is
                    // always valid for insert.
                    self.tasks.insert(abs + 1, parsed);
                    Some(abs + 1)
                });
                self.flash(if was_done {
                    "uncompleted"
                } else if spawned.is_some() {
                    "completed +next"
                } else {
                    "completed"
                });
                self.persist();
                self.recompute_visible();
                self.follow_cursor(spawned.unwrap_or(abs));
            }
            Err(e) => {
                let verb = if was_done { "uncomplete" } else { "complete" };
                self.flash(format!("{verb} failed: {e}"));
            }
        }
    }

    pub fn cycle_priority(&mut self, abs: usize) {
        if !self.check_external_changes() {
            return;
        }
        if abs >= self.tasks.len() {
            return;
        }
        self.push_history();
        if let Err(e) = self.tasks[abs].cycle_priority() {
            self.flash(format!("priority failed: {e}"));
            return;
        }
        self.persist();
        self.recompute_visible();
        self.follow_cursor(abs);
    }

    pub fn delete(&mut self, abs: usize) {
        if !self.check_external_changes() {
            return;
        }
        if abs >= self.tasks.len() {
            return;
        }
        self.push_history();
        self.tasks.remove(abs);
        self.flash("deleted");
        self.persist();
        self.recompute_visible();
        self.clamp_cursor();
    }

    pub fn add_from_draft(&mut self) -> AddOutcome {
        let mut text = self.draft.text().trim().to_string();
        if text.is_empty() {
            return AddOutcome::Empty;
        }
        if !self.check_external_changes() {
            return AddOutcome::Invalid;
        }

        // Natural-language pre-pass. If the buffer reads like prose and the
        // parser extracted anything structured, rewrite the draft to canonical
        // todo.txt and bail before push_history — the actual save happens on
        // the user's *next* Enter through this same function, which by then
        // sees the canonical form and falls through to the parse path below.
        if crate::nl::looks_like_natural_language(&text)
            && let Ok(today) = chrono::NaiveDate::parse_from_str(&self.today, "%Y-%m-%d")
            && let Some(parsed) = crate::nl::try_parse(&text, today)
        {
            let canonical = crate::nl::format_as_todo_txt(&parsed);
            if canonical != text {
                let body_was_filled = !parsed.body.trim().is_empty();
                self.draft_set(canonical);
                if body_was_filled {
                    self.flash("parsed natural language; press Enter to save");
                } else {
                    self.flash("parsed; please edit the body, then Enter to save");
                }
                return AddOutcome::Parsed;
            }
        }

        self.push_history();
        if !todo::starts_with_priority(&text)
            && !todo::starts_with_iso_date(&text)
            && !text.starts_with("x ")
        {
            text = format!("{} {}", self.today, text);
        }
        match todo::parse_line(&text) {
            Ok(parsed) => {
                self.tasks.push(parsed);
                self.flash("added");
                self.persist();
                self.recompute_visible();
                AddOutcome::Saved
            }
            Err(e) => {
                self.flash(format!("invalid: {e}"));
                AddOutcome::Invalid
            }
        }
    }

    pub fn save_edit(&mut self) {
        let Some(idx) = self.selection.editing() else {
            return;
        };
        if self.draft.text().trim().is_empty() {
            return;
        }
        if !self.check_external_changes() {
            return;
        }
        self.push_history();
        match todo::parse_line(self.draft.text()) {
            Ok(parsed) if idx < self.tasks.len() => {
                self.tasks[idx] = parsed;
                self.flash("saved");
                self.persist();
                self.recompute_visible();
                self.follow_cursor(idx);
            }
            Ok(_) => {} // index disappeared under us; quiet no-op
            Err(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn add_project_to_current(&mut self, name: &str) {
        let name = name.trim();
        if !self.check_external_changes() {
            return;
        }
        let Some(abs) = self.cur_abs() else {
            return;
        };
        self.push_history();
        match self.tasks[abs].add_project(name) {
            Ok(true) => {
                self.flash(format!("+{name}"));
                self.persist();
                self.recompute_visible();
                self.follow_cursor(abs);
            }
            Ok(false) => {} // already present; quiet
            Err(TagError::Invalid) => self.flash("invalid project name"),
            Err(TagError::Parse(e)) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn toggle_context_on_current(&mut self, name: &str) {
        let name = name.trim();
        if !self.check_external_changes() {
            return;
        }
        let Some(abs) = self.cur_abs() else {
            return;
        };
        let has = self.tasks[abs].contexts.iter().any(|c| c == name);
        self.push_history();
        let result = if has {
            self.tasks[abs].remove_context(name).map(|_| ())
        } else {
            self.tasks[abs].add_context(name).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.flash(if has {
                    format!("removed @{name}")
                } else {
                    format!("@{name}")
                });
                self.persist();
                self.recompute_visible();
                self.follow_cursor(abs);
            }
            Err(TagError::Invalid) => self.flash("invalid context name"),
            Err(TagError::Parse(e)) => self.flash(format!("invalid: {e}")),
        }
    }
}

/// Build the raw line for the next occurrence of a recurring task.
///
/// Inputs are the *pre-completion* `raw` (before `mark_done` rewrites it),
/// the *pre-completion* `due:` value (used as the strict-mode anchor), the
/// parsed `RecSpec`, and `today` for the new creation date and the
/// normal-mode anchor.
///
/// Strict mode anchors on the previous due date when present and parseable;
/// otherwise it falls back to today + interval (matches sleek/dorecur). Date
/// overflow returns `None` so the caller skips spawning rather than panics.
///
/// Whitespace in the original line collapses to single spaces — the same
/// `split_whitespace().join(" ")` rewrite the tag mutators use at
/// `Task::add_tag` / `Task::remove_context`. Multiple `due:` tokens collapse
/// to one (the parser only reads the first anyway).
pub(super) fn build_next_instance(
    raw: &str,
    due: Option<&str>,
    spec: &RecSpec,
    today: &str,
) -> Option<String> {
    use chrono::NaiveDate;
    let today_date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    let anchor = if spec.strict {
        due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or(today_date)
    } else {
        today_date
    };
    let next_due = recurrence::advance(anchor, spec)?;
    let next_due_str = next_due.format("%Y-%m-%d").to_string();

    // Strip the leading `[x DONE] [PRIORITY] [CREATED]` prefix and rebuild
    // with today's date. `body_after_priority` already does the parse-side
    // trimming used by `mark_done`.
    let body = todo::body_after_priority(raw);

    // Rewrite the body tokens: substitute the first `due:` with the new value
    // and drop subsequent `due:` duplicates. Other tokens (projects, contexts,
    // rec, arbitrary key:value pairs, plain words) are preserved in order.
    let mut out_tokens: Vec<String> = Vec::new();
    let mut due_seen = false;
    for tok in body.split_whitespace() {
        if let Some(rest) = tok.strip_prefix("due:")
            && !rest.is_empty()
        {
            if !due_seen {
                out_tokens.push(format!("due:{next_due_str}"));
                due_seen = true;
            }
            continue;
        }
        out_tokens.push(tok.to_string());
    }
    if !due_seen {
        out_tokens.push(format!("due:{next_due_str}"));
    }

    let prefix = match todo::parse_line(raw).ok().and_then(|t| t.priority) {
        Some(p) => format!("({p}) {today} "),
        None => format!("{today} "),
    };
    Some(format!("{prefix}{}", out_tokens.join(" ")))
}

#[cfg(test)]
mod tests {
    use crate::app::test_support::build_app;

    #[test]
    fn add_project_rejects_whitespace_in_name() {
        let mut app = build_app("a +health\n");
        app.add_project_to_current("two words");
        // Task is unchanged; the bad input did not produce "+two words".
        assert_eq!(app.tasks[0].projects, vec!["health"]);
        assert_eq!(app.tasks[0].raw, "a +health");
        assert_eq!(app.flash_active(), Some("invalid project name"));
    }

    #[test]
    fn add_project_rejects_sigils_and_colons() {
        let mut app = build_app("a\n");
        for bad in ["+nested", "@context", "key:val", ""] {
            app.add_project_to_current(bad);
            assert_eq!(app.tasks[0].raw, "a", "input {:?} should be rejected", bad);
        }
    }

    #[test]
    fn add_project_accepts_dashes_underscores_unicode() {
        let mut app = build_app("a\n");
        app.add_project_to_current("life-admin_2026");
        assert_eq!(app.tasks[0].projects, vec!["life-admin_2026"]);
        app.add_project_to_current("café");
        assert_eq!(app.tasks[0].projects, vec!["life-admin_2026", "café"]);
    }

    #[test]
    fn toggle_complete_marks_pending_task_done() {
        let mut app = build_app("a\n");
        app.toggle_complete(0);
        assert!(app.tasks[0].done);
        assert_eq!(app.flash_active(), Some("completed"));
    }

    #[test]
    fn toggle_complete_undoes_done_task() {
        let mut app = build_app("x 2026-05-05 2026-05-01 finish report\n");
        assert!(app.tasks[0].done);
        app.toggle_complete(0);
        assert!(!app.tasks[0].done);
        assert_eq!(app.tasks[0].raw, "2026-05-01 finish report");
        assert_eq!(app.flash_active(), Some("uncompleted"));
    }

    #[test]
    fn toggle_complete_spawns_next_for_strict_monthly() {
        // Strict monthly: anchor on the previous due date, not the completion
        // date. Pay-the-rent semantics: due on the 15th every month even if
        // completed late.
        let mut app = build_app("(A) 2026-04-15 Pay rent due:2026-04-15 rec:+1m\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 2, "spawn must be inserted");
        assert!(app.tasks[0].done);
        assert!(!app.tasks[1].done);
        assert_eq!(app.tasks[1].due.as_deref(), Some("2026-05-15"));
        assert_eq!(app.tasks[1].rec.as_deref(), Some("+1m"));
        assert_eq!(app.tasks[1].priority, Some('A'));
        assert_eq!(app.flash_active(), Some("completed +next"));
    }

    #[test]
    fn toggle_complete_spawns_next_for_normal_weekly_no_due() {
        // Normal recurrence (no `+`): anchor on completion date. Original had
        // no due date — the spawn still gets one (today + 1 week).
        let app_today = "2026-05-09";
        let mut app = build_app("Water plants rec:1w\n");
        app.today = app_today.to_string();
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 2);
        // Today is 2026-05-09 + 1w = 2026-05-16.
        assert_eq!(app.tasks[1].due.as_deref(), Some("2026-05-16"));
        assert_eq!(app.tasks[1].rec.as_deref(), Some("1w"));
    }

    #[test]
    fn toggle_complete_clamps_month_end() {
        // Jan 31 + 1m clamps to Feb 28 (non-leap year via chrono::Months).
        let mut app = build_app("Pay bill due:2026-01-31 rec:+1m\n");
        app.today = "2026-01-31".to_string();
        app.toggle_complete(0);
        assert_eq!(app.tasks[1].due.as_deref(), Some("2026-02-28"));
    }

    #[test]
    fn toggle_complete_no_rec_does_not_spawn() {
        let mut app = build_app("a\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 1, "no rec means no spawn");
        assert_eq!(app.flash_active(), Some("completed"));
    }

    #[test]
    fn toggle_complete_invalid_rec_completes_without_spawn() {
        let mut app = build_app("a rec:bogus\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 1);
        assert!(app.tasks[0].done);
        assert_eq!(app.flash_active(), Some("completed"));
    }

    #[test]
    fn toggle_complete_strict_with_bad_due_falls_back_to_today() {
        // `due:tomorrow` is not ISO-parseable; strict mode should fall back
        // to today + interval rather than refusing to spawn.
        let mut app = build_app("Stretch due:tomorrow rec:+2d\n");
        app.today = "2026-05-09".to_string();
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 2);
        // Today (2026-05-09) + 2d = 2026-05-11. The bad due token in the
        // original is replaced in-place by the rewrite.
        assert_eq!(app.tasks[1].due.as_deref(), Some("2026-05-11"));
    }

    #[test]
    fn toggle_complete_undo_rolls_back_completion_and_spawn() {
        let mut app = build_app("Do thing due:2026-05-15 rec:+1w\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks.len(), 2);
        app.undo();
        assert_eq!(app.tasks.len(), 1, "undo must remove the spawn too");
        assert!(!app.tasks[0].done, "undo must un-complete the original");
    }

    #[test]
    fn toggle_complete_drops_duplicate_due_tokens_in_spawn() {
        // The parser only reads the first due:; the rewrite collapses
        // duplicates instead of carrying stale values forward.
        let mut app = build_app("Bug due:2026-05-15 due:2026-09-09 rec:+1d\n");
        app.toggle_complete(0);
        let next_raw = &app.tasks[1].raw;
        assert_eq!(next_raw.matches("due:").count(), 1);
        assert!(next_raw.contains("due:2026-05-16"));
    }

    #[test]
    fn toggle_context_rejects_whitespace_in_name() {
        let mut app = build_app("a @home\n");
        app.toggle_context_on_current("two words");
        assert_eq!(app.tasks[0].contexts, vec!["home"]);
        assert_eq!(app.tasks[0].raw, "a @home");
        assert_eq!(app.flash_active(), Some("invalid context name"));
    }

    #[test]
    fn add_from_draft_rewrites_nl_prose_into_canonical_draft() {
        // Prose buffer triggers the NL pre-pass: draft is rewritten to
        // canonical todo.txt, *no* task is appended yet, and the caller
        // receives `Parsed` so it can keep Insert mode open.
        let mut app = build_app("");
        app.draft_set(
            "Pay rent monthly on the first of the month, show the todo 3 days before the due date. \
             It's part of project home and context bank"
                .into(),
        );
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks.len(), 0);
        // today in test_support is 2026-05-06; "first of the month" rolls
        // forward to 2026-06-01 because 2026-05-01 < today.
        assert_eq!(
            app.draft.text(),
            "Pay rent +home @bank due:2026-06-01 rec:+1m t:-3d"
        );
        assert_eq!(
            app.flash_active(),
            Some("parsed natural language; press Enter to save")
        );
    }

    #[test]
    fn add_from_draft_second_call_saves_canonical_form() {
        // After the NL rewrite, the draft contains canonical todo.txt. The
        // second Enter falls through detection (due:/rec:/t: are present) and
        // saves through the normal path.
        let mut app = build_app("");
        app.draft_set("Buy milk tomorrow".into());
        assert_eq!(app.add_from_draft(), crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks.len(), 0);
        // Second call on the now-canonical draft.
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks.len(), 1);
        assert!(app.tasks[0].raw.contains("Buy milk"));
        assert_eq!(app.tasks[0].due.as_deref(), Some("2026-05-07"));
    }

    #[test]
    fn add_from_draft_already_structured_saves_on_first_enter() {
        // Input already contains a due: token, so detection refuses to
        // re-interpret it as prose. Single Enter saves directly.
        let mut app = build_app("");
        app.draft_set("Pay rent due:2026-06-01 rec:+1m +home".into());
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[0].due.as_deref(), Some("2026-06-01"));
        assert_eq!(app.tasks[0].rec.as_deref(), Some("+1m"));
    }

    #[test]
    fn add_from_draft_plain_words_save_on_first_enter() {
        // No NL trigger words → detection returns false → saves immediately.
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks.len(), 1);
        assert!(app.tasks[0].raw.ends_with("Buy milk"));
    }
}
