use super::App;
use crate::todo::{self, TagError};

impl App {
    pub fn complete(&mut self, abs: usize) {
        if !self.check_external_changes() {
            return;
        }
        let Some(t) = self.tasks.get(abs) else {
            return;
        };
        if t.done {
            return;
        }
        self.push_history();
        match self.tasks[abs].mark_done(&self.today) {
            Ok(()) => {
                self.flash("completed");
                self.persist();
                self.recompute_visible();
                self.follow_cursor(abs);
            }
            Err(e) => self.flash(format!("complete failed: {e}")),
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

    pub fn add_from_draft(&mut self) {
        let mut text = self.draft.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if !self.check_external_changes() {
            return;
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
            }
            Err(e) => {
                self.flash(format!("invalid: {e}"));
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
    fn toggle_context_rejects_whitespace_in_name() {
        let mut app = build_app("a @home\n");
        app.toggle_context_on_current("two words");
        assert_eq!(app.tasks[0].contexts, vec!["home"]);
        assert_eq!(app.tasks[0].raw, "a @home");
        assert_eq!(app.flash_active(), Some("invalid context name"));
    }
}
