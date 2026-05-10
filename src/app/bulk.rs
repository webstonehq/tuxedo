use super::App;
use super::mutations::build_next_instance;
use super::types::Mode;
use crate::recurrence;
use crate::todo;

impl App {
    /// Bulk-complete every task in the selection that isn't already done.
    /// Clears the selection and exits Visual mode on success.
    ///
    /// Recurring tasks (with a parseable `rec:`) also spawn their next
    /// instance. Spawns are inserted *after* every in-place completion has
    /// landed, in descending original-index order — mirrors `delete_selected`
    /// at this file's `delete_selected` so earlier inserts don't shift the
    /// indices of later ones.
    pub fn complete_selected(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        if !self.check_external_changes() {
            return;
        }
        let to_complete: Vec<usize> = self
            .selection
            .iter()
            .filter(|&i| i < self.tasks.len() && !self.tasks[i].done)
            .collect();
        if to_complete.is_empty() {
            self.flash("nothing to complete");
            self.selection.clear();
            self.mode = Mode::Normal;
            return;
        }
        self.push_history();
        // Pass 1: complete in place, collecting spawn lines by original index.
        // Reading `rec` and `due` happens *before* the in-place rewrite, since
        // the post-completion task has its priority stripped and dates re-laid.
        let mut spawns: Vec<(usize, todo::Task)> = Vec::new();
        for abs in to_complete.iter().copied() {
            let t = &self.tasks[abs];
            let raw = t.raw.clone();
            let due = t.due.clone();
            let rec_spec = t.rec.as_deref().and_then(recurrence::parse_rec_spec);
            let created = t.created_date.clone().unwrap_or_else(|| self.today.clone());
            let body = todo::body_after_priority(&raw).to_string();
            let new_raw = format!("x {} {} {}", self.today, created, body);
            if let Ok(parsed) = todo::parse_line(&new_raw) {
                self.tasks[abs] = parsed;
            }
            if let Some(spec) = rec_spec
                && let Some(next_raw) =
                    build_next_instance(&raw, due.as_deref(), &spec, &self.today)
                && let Ok(next) = todo::parse_line(&next_raw)
            {
                spawns.push((abs, next));
            }
        }
        // Pass 2: insert spawns at original_abs+1, descending. Sorting
        // descending means later inserts can't shift earlier indices.
        spawns.sort_by_key(|s| std::cmp::Reverse(s.0));
        let spawned = spawns.len();
        for (abs, parsed) in spawns {
            self.tasks.insert(abs + 1, parsed);
        }
        let n = to_complete.len();
        self.selection.clear();
        self.mode = Mode::Normal;
        self.flash(if spawned > 0 {
            format!("completed {n} (+{spawned} recurring)")
        } else {
            format!("completed {n}")
        });
        self.persist();
        self.recompute_visible();
        self.clamp_cursor();
    }

    /// Bulk-delete every task in the selection. Indices are removed in
    /// descending order so earlier removals don't shift later ones.
    pub fn delete_selected(&mut self) {
        if self.selection.is_empty() {
            return;
        }
        if !self.check_external_changes() {
            return;
        }
        let mut indices: Vec<usize> = self
            .selection
            .iter()
            .filter(|&i| i < self.tasks.len())
            .collect();
        if indices.is_empty() {
            self.selection.clear();
            self.mode = Mode::Normal;
            return;
        }
        indices.sort_by(|a, b| b.cmp(a));
        self.push_history();
        let n = indices.len();
        for abs in indices {
            self.tasks.remove(abs);
        }
        self.selection.clear();
        self.mode = Mode::Normal;
        self.flash(format!("deleted {}", n));
        self.persist();
        self.recompute_visible();
        self.clamp_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn complete_selected_marks_all_in_selection() {
        let mut app = build_app("a\nb\nc\n");
        app.selection.toggle(0);
        app.selection.toggle(2);
        app.mode = Mode::Visual;
        app.complete_selected();
        assert!(app.tasks[0].done);
        assert!(!app.tasks[1].done);
        assert!(app.tasks[2].done);
        assert!(app.selection.is_empty());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn complete_selected_spawns_recurring_next_instances() {
        let mut app = build_app("a\nPay rent due:2026-04-15 rec:+1m\nb\nWater plants rec:1w\n");
        app.today = "2026-05-09".to_string();
        // Select the two recurring tasks (indices 1 and 3).
        app.selection.toggle(1);
        app.selection.toggle(3);
        app.mode = Mode::Visual;
        app.complete_selected();
        // 4 originals + 2 spawns = 6 lines. Spawn for `Pay rent` lands at
        // original_index+1 = 2; spawn for `Water plants` at 3+1+1 = 5
        // (the rent spawn shifted it by one).
        assert_eq!(app.tasks.len(), 6, "two spawns must be inserted");
        // Original `Pay rent` at index 1 is now done.
        assert!(app.tasks[1].done);
        // Spawn for rent: due advanced by 1m strict (Apr 15 + 1m = May 15).
        assert_eq!(app.tasks[2].due.as_deref(), Some("2026-05-15"));
        assert!(!app.tasks[2].done);
        // `b` (originally idx 2) shifted to idx 3.
        assert_eq!(app.tasks[3].raw, "b");
        // Original `Water plants` (originally idx 3) is at idx 4 and done.
        assert!(app.tasks[4].done);
        // Its spawn is at idx 5 with today + 1w = 2026-05-16.
        assert_eq!(app.tasks[5].due.as_deref(), Some("2026-05-16"));
        assert_eq!(app.flash_active(), Some("completed 2 (+2 recurring)"));
    }

    #[test]
    fn complete_selected_skips_already_done() {
        let mut app = build_app("a\nx 2026-05-05 2026-05-01 b\nc\n");
        app.selection.toggle(0);
        app.selection.toggle(1);
        app.selection.toggle(2);
        app.mode = Mode::Visual;
        app.complete_selected();
        assert!(app.tasks[0].done);
        // Already done; left alone (done_date should still be 2026-05-05).
        assert_eq!(app.tasks[1].done_date.as_deref(), Some("2026-05-05"));
        assert!(app.tasks[2].done);
    }

    #[test]
    fn delete_selected_removes_all_in_selection() {
        let mut app = build_app("a\nb\nc\nd\n");
        app.selection.toggle(1);
        app.selection.toggle(3);
        app.mode = Mode::Visual;
        app.delete_selected();
        assert_eq!(app.tasks.len(), 2);
        assert_eq!(app.tasks[0].raw, "a");
        assert_eq!(app.tasks[1].raw, "c");
        assert!(app.selection.is_empty());
        assert_eq!(app.mode, Mode::Normal);
    }
}
