use std::collections::VecDeque;

use super::App;
use super::types::UNDO_LIMIT;
use crate::todo::Task;

#[derive(Debug, Default, Clone)]
pub struct History {
    stack: VecDeque<Vec<Task>>,
}

impl History {
    pub fn push(&mut self, snapshot: Vec<Task>) {
        if self.stack.len() >= UNDO_LIMIT {
            self.stack.pop_front();
        }
        self.stack.push_back(snapshot);
    }

    pub fn pop(&mut self) -> Option<Vec<Task>> {
        self.stack.pop_back()
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }
}

impl App {
    pub(super) fn push_history(&mut self) {
        self.history.push(self.tasks.clone());
    }

    pub fn undo(&mut self) {
        if !self.check_external_changes() {
            return;
        }
        if let Some(prev) = self.history.pop() {
            self.tasks = prev;
            self.flash("undo");
            self.persist();
            self.recompute_visible();
            self.clamp_cursor();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn history_evicts_fifo_at_undo_limit() {
        let mut app = build_app("a\n");
        for _ in 0..(UNDO_LIMIT + 5) {
            app.push_history();
        }
        assert_eq!(app.history.len(), UNDO_LIMIT);
    }
}
