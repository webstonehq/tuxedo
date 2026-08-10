//! App-level mutation wrappers. Each resolves the cursor to an absolute task
//! index, delegates to the headless [`Store`](crate::core::Store), then maps the
//! returned outcome to a flash message and refreshes the visible cache/cursor.
//! All task logic (recurrence, persistence, reconciliation) lives in the store.

use super::App;
use super::types::{AddOutcome, View};
use crate::app::WeekStart;
use crate::core::AddOutcome as CoreAdd;
use crate::core::{
    ArchiveDeleteOutcome, ArchiveOutcome, CompleteOutcome, DeleteOutcome, EditOutcome,
    PriorityOutcome, TagOutcome, UnarchiveOutcome, UndoOutcome,
};
use crate::nl;
use crate::note;

impl App {
    pub fn toggle_complete(&mut self, abs: usize) {
        match self.store.toggle_complete(abs) {
            CompleteOutcome::Completed { abs } => {
                self.flash("completed");
                self.after_mutation(abs);
            }
            CompleteOutcome::CompletedSpawned { next, .. } => {
                self.flash("completed +next");
                self.after_mutation(next);
            }
            CompleteOutcome::Uncompleted { abs } => {
                self.flash("uncompleted");
                self.after_mutation(abs);
            }
            CompleteOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            CompleteOutcome::OutOfRange => {}
            CompleteOutcome::Error(e) => self.flash(format!("complete failed: {e}")),
        }
    }

    pub fn cycle_priority(&mut self, abs: usize) {
        match self.store.cycle_priority(abs) {
            PriorityOutcome::Changed { abs, .. } => self.after_mutation(abs),
            PriorityOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            PriorityOutcome::OutOfRange => {}
            PriorityOutcome::Error(e) => self.flash(format!("priority failed: {e}")),
        }
    }

    pub fn delete(&mut self, abs: usize) {
        match self.store.delete(abs) {
            DeleteOutcome::Deleted { .. } => {
                self.flash("deleted");
                self.recompute_visible();
                self.clamp_cursor();
            }
            DeleteOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            DeleteOutcome::OutOfRange => {}
            DeleteOutcome::Error(e) => self.flash(format!("write failed: {e}")),
        }
    }

    pub fn add_from_draft(&mut self) -> AddOutcome {
        let text = self.draft.text().trim().to_string();
        if text.is_empty() {
            return AddOutcome::Empty;
        }

        // Natural-language pre-pass. If the buffer reads like prose and the
        // parser extracted anything structured, rewrite the draft to canonical
        // todo.txt and bail before saving — the user's *next* Enter saves the
        // now-canonical form through the store.
        if nl::looks_like_natural_language(&text)
            && let Ok(today) = chrono::NaiveDate::parse_from_str(self.store.today(), "%Y-%m-%d")
            && let Some(parsed) = nl::try_parse(&text, today)
        {
            let canonical = nl::format_as_todo_txt(&parsed);
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

        match self.store.add_finalized(&text) {
            CoreAdd::Added { abs } => {
                self.flash("added");
                self.after_mutation(abs);
                AddOutcome::Saved
            }
            CoreAdd::Empty => AddOutcome::Empty,
            CoreAdd::Aborted(r) => {
                self.handle_reconcile_abort(r);
                AddOutcome::Invalid
            }
            CoreAdd::Error(e) => {
                self.flash(format!("invalid: {e}"));
                AddOutcome::Invalid
            }
        }
    }

    pub fn save_edit(&mut self) {
        let Some(idx) = self.selection.editing() else {
            return;
        };
        let text = self.draft.text().to_string();
        match self.store.edit_line(idx, &text) {
            EditOutcome::Saved { abs } => {
                self.flash("saved");
                self.after_mutation(abs);
            }
            // Empty draft / vanished index: quiet no-op, as before.
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn finish_editor_edit(&mut self, result: &anyhow::Result<Option<String>>) {
        let Some((idx, _old_raw)) = self.pending_editor_task.take() else {
            return;
        };
        match result {
            Ok(Some(new_raw)) => {
                match self.store.edit_line(idx, new_raw) {
                    EditOutcome::Saved { abs } => {
                        self.flash("saved");
                        self.after_mutation(abs);
                    }
                    EditOutcome::Empty => self.flash("task text cannot be empty"),
                    EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
                    EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
                    EditOutcome::Error(e) => self.flash(format!("invalid: {e}")),
                }
            }
            Ok(None) => {
                self.flash("no changes");
            }
            Err(e) => {
                self.flash(format!("editor error: {e}"));
            }
        }
    }

    pub fn add_project_to_current(&mut self, name: &str) {
        let Some(abs) = self.cur_abs() else {
            return;
        };
        match self.store.add_project(abs, name) {
            TagOutcome::Added { abs, name } => {
                self.flash(format!("+{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Removed { .. } | TagOutcome::Unchanged | TagOutcome::OutOfRange => {}
            TagOutcome::InvalidName => self.flash("invalid project name"),
            TagOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TagOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn toggle_context_on_current(&mut self, name: &str) {
        let Some(abs) = self.cur_abs() else {
            return;
        };
        match self.store.toggle_context(abs, name) {
            TagOutcome::Added { abs, name } => {
                self.flash(format!("@{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Removed { abs, name } => {
                self.flash(format!("removed @{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Unchanged | TagOutcome::OutOfRange => {}
            TagOutcome::InvalidName => self.flash("invalid context name"),
            TagOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TagOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn open_note_for_current(&mut self) {
        self.open_note_for_current_with_create(false);
    }

    pub fn create_or_open_note_for_current(&mut self) {
        self.open_note_for_current_with_create(true);
    }

    fn open_note_for_current_with_create(&mut self, create: bool) {
        let Some(task) = self.cur_task().cloned() else {
            return;
        };
        let target = note::target_for_task(&task, self.notes_dir());

        if !target.existed_in_task {
            if !create {
                self.flash("no note; press O to create");
                return;
            }
            if matches!(self.view(), View::Archive) {
                self.flash("archived task has no note");
                return;
            }
            let Some(abs) = self.cur_task_index_in_tasks() else {
                return;
            };
            match self.store.append_at(abs, &format!("note:{}", target.rel)) {
                EditOutcome::Saved { abs } => self.after_mutation(abs),
                EditOutcome::Aborted(r) => {
                    self.handle_reconcile_abort(r);
                    return;
                }
                EditOutcome::Error(e) => {
                    self.flash(format!("note link failed: {e}"));
                    return;
                }
                EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => return,
            }
        }

        if target.path.exists() {
            self.queue_editor_path(target.path);
            return;
        }
        if !create {
            self.flash("note missing; press O to create");
            return;
        }
        if let Some(parent) = target.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            self.flash(format!("note mkdir failed: {e}"));
            return;
        }
        if let Err(e) = std::fs::write(&target.path, note::note_template(&task)) {
            self.flash(format!("note create failed: {e}"));
            return;
        }
        self.queue_editor_path(target.path);
    }

    pub fn undo(&mut self) {
        match self.store.undo() {
            UndoOutcome::Undone => {
                self.flash("undo");
                self.recompute_visible();
                self.clamp_cursor();
            }
            UndoOutcome::Nothing => {}
            UndoOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            UndoOutcome::Error(e) => self.flash(format!("write failed: {e}")),
        }
    }

    pub fn archive_completed(&mut self) {
        match self.store.archive_completed() {
            ArchiveOutcome::Archived { count } => {
                self.flash(format!("archived {count}"));
                self.recompute_visible();
                self.clamp_cursor();
            }
            ArchiveOutcome::Nothing => self.flash("nothing to archive"),
            ArchiveOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            ArchiveOutcome::Error(e) => self.flash(format!("archive failed: {e}")),
        }
    }

    /// Move an archived task back into the live list. `archive_idx` indexes
    /// `archive().tasks()` (the cursor source in Archive view).
    pub fn unarchive(&mut self, archive_idx: usize) {
        match self.store.unarchive(archive_idx) {
            UnarchiveOutcome::Unarchived => {
                self.flash("unarchived");
                self.recompute_visible();
                self.clamp_cursor();
            }
            UnarchiveOutcome::OutOfRange => {}
            UnarchiveOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            UnarchiveOutcome::DoneReloaded => {
                self.flash("done.txt changed on disk — reloaded");
                self.recompute_visible();
                self.clamp_cursor();
            }
            UnarchiveOutcome::Error(e) => self.flash(format!("unarchive failed: {e}")),
        }
    }

    /// Permanently remove an archived task from `done.txt`.
    pub fn archive_delete(&mut self, archive_idx: usize) {
        match self.store.archive_delete(archive_idx) {
            ArchiveDeleteOutcome::Deleted => {
                self.flash("deleted from archive");
                self.recompute_visible();
                self.clamp_cursor();
            }
            ArchiveDeleteOutcome::OutOfRange => {}
            ArchiveDeleteOutcome::DoneReloaded => {
                self.flash("done.txt changed on disk — reloaded");
                self.recompute_visible();
                self.clamp_cursor();
            }
            ArchiveDeleteOutcome::Error(e) => self.flash(format!("delete failed: {e}")),
        }
    }

    pub fn toggle_week_start_date(&mut self) {
        let week_start = match self.week_start {
            WeekStart::Sunday => WeekStart::Monday,
            WeekStart::Monday => WeekStart::Sunday,
        };

        self.week_start = week_start
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{
        WeekStart,
        test_support::{build_app, build_app_with_config, test_path},
    };
    use crate::config::Config;

    #[test]
    fn open_file_rebinds_path_body_and_resets_cursor() {
        let mut app = build_app("old one\nold two\nold three\n");
        app.cursor = 2;
        let new_path = test_path();
        let done = new_path.parent().expect("temp parent").join("done.txt");

        app.open_file(new_path.clone(), done, "fresh task\n".into());

        assert_eq!(
            app.file_path, new_path,
            "file_path must point at the new file"
        );
        assert_eq!(app.tasks().len(), 1, "tasks must reflect the new body");
        assert_eq!(app.tasks()[0].raw, "fresh task");
        assert_eq!(
            app.visible_indices().len(),
            1,
            "visible cache must be recomputed"
        );
        assert_eq!(app.cursor, 0, "cursor must reset for the new file");
    }

    #[test]
    fn add_project_rejects_whitespace_in_name() {
        let mut app = build_app("a +health\n");
        app.add_project_to_current("two words");
        assert_eq!(app.tasks()[0].projects, vec!["health"]);
        assert_eq!(app.tasks()[0].raw, "a +health");
        assert_eq!(app.flash_active(), Some("invalid project name"));
    }

    #[test]
    fn add_project_accepts_dashes_underscores_unicode() {
        let mut app = build_app("a\n");
        app.add_project_to_current("life-admin_2026");
        assert_eq!(app.tasks()[0].projects, vec!["life-admin_2026"]);
        app.add_project_to_current("café");
        assert_eq!(app.tasks()[0].projects, vec!["life-admin_2026", "café"]);
    }

    #[test]
    fn toggle_complete_flashes_completed_then_spawned() {
        let mut app = build_app("a\n");
        app.toggle_complete(0);
        assert!(app.tasks()[0].done);
        assert_eq!(app.flash_active(), Some("completed"));

        let mut app = build_app("(A) 2026-04-15 Pay rent due:2026-04-15 rec:+1m\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks().len(), 2);
        assert_eq!(app.flash_active(), Some("completed +next"));
    }

    #[test]
    fn toggle_context_rejects_whitespace_in_name() {
        let mut app = build_app("a @home\n");
        app.toggle_context_on_current("two words");
        assert_eq!(app.tasks()[0].contexts, vec!["home"]);
        assert_eq!(app.tasks()[0].raw, "a @home");
        assert_eq!(app.flash_active(), Some("invalid context name"));
    }

    #[test]
    fn create_or_open_note_appends_link_creates_file_and_queues_editor() {
        let dir = test_path().with_extension("notes");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let mut app = build_app_with_config("Write PR summary +work @desk\n", cfg);

        app.create_or_open_note_for_current();

        let raw = &app.tasks()[0].raw;
        assert!(
            raw.contains("note:projects/tuxedo-tasks/write-pr-summary.md"),
            "task should get stable generated note token: {raw}"
        );
        let expected = dir.join("projects/tuxedo-tasks/write-pr-summary.md");
        assert_eq!(app.take_pending_editor_path(), Some(expected.clone()));
        let body = std::fs::read_to_string(expected).expect("created note exists");
        assert!(body.starts_with("# Write PR summary\n"));
        assert!(body.contains("## My notes\n\n"));
    }

    #[test]
    fn open_note_without_existing_token_does_not_create_or_mutate_task() {
        let dir = test_path().with_extension("notes");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let mut app = build_app_with_config("Write PR summary +work @desk\n", cfg);

        app.open_note_for_current();

        assert_eq!(app.tasks()[0].raw, "Write PR summary +work @desk");
        assert_eq!(app.flash_active(), Some("no note; press O to create"));
        assert!(app.take_pending_editor_path().is_none());
        assert!(!dir.exists());
    }

    #[test]
    fn open_note_with_existing_file_queues_editor_without_rewriting_task() {
        let dir = test_path().with_extension("notes");
        let note = dir.join("projects/example.md");
        std::fs::create_dir_all(note.parent().expect("note parent")).expect("create note parent");
        std::fs::write(&note, "# Existing\n").expect("write existing note");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let raw = "Write PR summary +work note:projects/example.md\n";
        let mut app = build_app_with_config(raw, cfg);

        app.open_note_for_current();

        assert_eq!(app.tasks()[0].raw, raw.trim());
        assert_eq!(app.take_pending_editor_path(), Some(note));
    }

    #[test]
    fn add_from_draft_rewrites_nl_prose_into_canonical_draft() {
        let mut app = build_app("");
        app.draft_set(
            "Pay rent monthly on the first of the month, show the todo 3 days before the due date. \
             It's part of project home and context bank"
                .into(),
        );
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks().len(), 0);
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
        let mut app = build_app("");
        app.draft_set("Buy milk tomorrow".into());
        assert_eq!(app.add_from_draft(), crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks().len(), 0);
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.contains("Buy milk"));
        assert_eq!(app.tasks()[0].due.as_deref(), Some("2026-05-07"));
    }

    #[test]
    fn add_from_draft_plain_words_save_on_first_enter() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.ends_with("Buy milk"));
        assert_eq!(app.flash_active(), Some("added"));
    }

    #[test]
    fn test_toggling_week_start() {
        let mut app = build_app("");
        app.toggle_week_start_date();
        assert_eq!(app.week_start, WeekStart::Monday);
        app.toggle_week_start_date();
        assert_eq!(app.week_start, WeekStart::Sunday);
    }
}
