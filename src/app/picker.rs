use super::App;
use super::types::Mode;
use super::visibility::unique_values;

impl App {
    /// Enter project-picker mode. Seeds the filter from the cursor task's
    /// first project (falling back to the current filter, then alphabetical
    /// first). Inside the picker, j/k cycle through projects.
    pub fn enter_pick_project(&mut self) {
        let all = unique_values(&self.tasks, |t| &t.projects);
        if all.is_empty() {
            self.flash("no projects");
            return;
        }
        let seed = self
            .cur_abs()
            .and_then(|i| self.tasks[i].projects.first().cloned())
            .filter(|p| all.contains(p))
            .or_else(|| self.filter.project.clone())
            .filter(|p| all.contains(p))
            .unwrap_or_else(|| all[0].clone());
        self.filter.project = Some(seed);
        self.cursor = 0;
        self.mode = Mode::PickProject;
        self.recompute_visible();
        self.flash_pick_project();
    }

    pub fn enter_pick_context(&mut self) {
        let all = unique_values(&self.tasks, |t| &t.contexts);
        if all.is_empty() {
            self.flash("no contexts");
            return;
        }
        let seed = self
            .cur_abs()
            .and_then(|i| self.tasks[i].contexts.first().cloned())
            .filter(|c| all.contains(c))
            .or_else(|| self.filter.context.clone())
            .filter(|c| all.contains(c))
            .unwrap_or_else(|| all[0].clone());
        self.filter.context = Some(seed);
        self.cursor = 0;
        self.mode = Mode::PickContext;
        self.recompute_visible();
        self.flash_pick_context();
    }

    /// Cancel an open picker. Clears only the filter that was being picked
    /// (so escaping the context picker doesn't drop a project filter that
    /// the user set independently).
    pub fn pick_cancel(&mut self) {
        match self.mode {
            Mode::PickProject => self.filter.project = None,
            Mode::PickContext => self.filter.context = None,
            _ => {}
        }
        self.cursor = 0;
        self.mode = Mode::Normal;
        self.recompute_visible();
    }

    /// Step through projects/contexts within picker mode.
    pub fn pick_step(&mut self, forward: bool) {
        match self.mode {
            Mode::PickProject => {
                let all = unique_values(&self.tasks, |t| &t.projects);
                if all.is_empty() {
                    return;
                }
                self.filter.project = Some(step(&all, self.filter.project.as_deref(), forward));
                self.cursor = 0;
                self.recompute_visible();
                self.flash_pick_project();
            }
            Mode::PickContext => {
                let all = unique_values(&self.tasks, |t| &t.contexts);
                if all.is_empty() {
                    return;
                }
                self.filter.context = Some(step(&all, self.filter.context.as_deref(), forward));
                self.cursor = 0;
                self.recompute_visible();
                self.flash_pick_context();
            }
            _ => {}
        }
    }

    fn flash_pick_project(&mut self) {
        let all = unique_values(&self.tasks, |t| &t.projects);
        if let Some(cur) = self.filter.project.clone() {
            let pos = position_of(&all, &cur);
            self.flash(format!("+{}  ({}/{})", cur, pos + 1, all.len()));
        }
    }

    fn flash_pick_context(&mut self) {
        let all = unique_values(&self.tasks, |t| &t.contexts);
        if let Some(cur) = self.filter.context.clone() {
            let pos = position_of(&all, &cur);
            self.flash(format!("@{}  ({}/{})", cur, pos + 1, all.len()));
        }
    }
}

/// Wrap-around step through `all` in the requested direction.
fn step(all: &[String], current: Option<&str>, forward: bool) -> String {
    debug_assert!(!all.is_empty());
    let len = all.len();
    let cur_idx = current.and_then(|c| all.iter().position(|x| x == c));
    let next = match cur_idx {
        None => 0,
        Some(i) if forward => (i + 1) % len,
        Some(i) => (i + len - 1) % len,
    };
    all[next].clone()
}

fn position_of(all: &[String], needle: &str) -> usize {
    all.iter().position(|x| x == needle).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn step_forward_wraps() {
        let all = vec![
            "finance".to_string(),
            "health".to_string(),
            "work".to_string(),
        ];
        assert_eq!(step(&all, None, true), "finance".to_string());
        assert_eq!(step(&all, Some("finance"), true), "health".to_string());
        assert_eq!(step(&all, Some("health"), true), "work".to_string());
        assert_eq!(step(&all, Some("work"), true), "finance".to_string());
    }

    #[test]
    fn step_backward_wraps() {
        let all = vec![
            "finance".to_string(),
            "health".to_string(),
            "work".to_string(),
        ];
        assert_eq!(step(&all, Some("finance"), false), "work".to_string());
        assert_eq!(step(&all, Some("work"), false), "health".to_string());
        assert_eq!(step(&all, Some("health"), false), "finance".to_string());
    }

    #[test]
    fn pick_cancel_clears_only_relevant_filter() {
        let mut app = build_app(crate::sample::TODO_RAW);
        // Pretend the user already had a project filter set, then opened
        // the context picker. Cancelling the context picker must keep the
        // project filter intact.
        app.filter.project = Some("work".into());
        app.enter_pick_context();
        assert!(app.filter.context.is_some());
        app.pick_cancel();
        assert_eq!(app.filter.project.as_deref(), Some("work"));
        assert!(app.filter.context.is_none());
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn picker_seeds_from_cursor_task_and_steps() {
        let mut app = build_app(crate::sample::TODO_RAW);
        // Cursor task's first project is "work". Sidebar order is by count
        // desc, name asc: [work(4), health(3), finance(1), home(1),
        // learning(1), personal(1), travel(1)].
        app.enter_pick_project();
        assert!(matches!(app.mode, Mode::PickProject));
        assert_eq!(app.filter.project.as_deref(), Some("work"));
        // Forward: work → health
        app.pick_step(true);
        assert_eq!(app.filter.project.as_deref(), Some("health"));
        // Forward: health → finance
        app.pick_step(true);
        assert_eq!(app.filter.project.as_deref(), Some("finance"));
        // Backward from finance → health
        app.pick_step(false);
        assert_eq!(app.filter.project.as_deref(), Some("health"));
    }
}
