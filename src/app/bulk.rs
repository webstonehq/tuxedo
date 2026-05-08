use super::App;
use super::types::Mode;
use crate::todo;

impl App {
    /// Bulk-complete every task in the selection that isn't already done.
    /// Clears the selection and exits Visual mode on success.
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
        for abs in to_complete.iter().copied() {
            let t = &self.tasks[abs];
            let raw = t.raw.clone();
            let created = t.created_date.clone().unwrap_or_else(|| self.today.clone());
            let body = todo::body_after_priority(&raw).to_string();
            let new_raw = format!("x {} {} {}", self.today, created, body);
            if let Ok(parsed) = todo::parse_line(&new_raw) {
                self.tasks[abs] = parsed;
            }
        }
        let n = to_complete.len();
        self.selection.clear();
        self.mode = Mode::Normal;
        self.flash(format!("completed {}", n));
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
