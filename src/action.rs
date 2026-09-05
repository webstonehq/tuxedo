//! Every discrete behavior the user can trigger, decoupled from the keystroke
//! that fires it. Lives at the crate root (not under `app`) so both the binary
//! (which dispatches actions in `apply_action`) and the command palette
//! (which lists them) can name them without a cyclic dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    CursorDown,
    CursorUp,
    CursorTop,
    CursorBottom,
    HalfPageDown,
    HalfPageUp,
    BeginAdd,
    /// Edit the current task starting in Normal (vim) mode (`e`).
    BeginEdit,
    /// Edit the current task starting in Insert mode (`i`).
    BeginEditInsert,
    ToggleComplete,
    Delete,
    Reschedule,
    CyclePriority,
    MoveTaskDown,
    MoveTaskUp,
    BeginSearch,
    OpenHelp,
    OpenSettings,
    OpenCommandPalette,
    Undo,
    ToggleVisual,
    ToggleSelected,
    GoList,
    ToggleArchiveView,
    ArchiveCompleted,
    ArmF,
    PickProject,
    PickContext,
    /// `ff` — open the saved-search cycle picker.
    PickSavedFilter,
    /// `fs` — name the active `/`-search and persist it.
    SaveCurrentFilter,
    CycleSort,
    BeginPromptProject,
    BeginPromptContext,
    ToggleLeftPane,
    ToggleRightPane,
    CycleTheme,
    CycleDensity,
    ToggleLineNum,
    ToggleShowDone,
    ToggleShowFuture,
    ToggleAutocompleteArchive,
    CopyLine,
    CopyBody,
    OpenNote,
    CreateOrOpenNote,
    EscapeStack,
    /// Open the phone-capture overlay (QR + URL). First invocation lazily
    /// binds the HTTP server; subsequent invocations just re-show the
    /// overlay.
    OpenShare,
    /// Open the theme picker dialog (j/k to preview, Enter to accept).
    OpenThemePicker,
    ChangeWeekStart,
}

impl Action {
    pub fn from_keybind_name(s: &str) -> Option<Self> {
        let normalized = s.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "quit" => Some(Self::Quit),
            "cursor_down" => Some(Self::CursorDown),
            "cursor_up" => Some(Self::CursorUp),
            "cursor_top" => Some(Self::CursorTop),
            "cursor_bottom" => Some(Self::CursorBottom),
            "half_page_down" => Some(Self::HalfPageDown),
            "half_page_up" => Some(Self::HalfPageUp),
            "begin_add" | "add" => Some(Self::BeginAdd),
            "begin_edit" | "edit" => Some(Self::BeginEdit),
            "begin_edit_insert" | "edit_insert" => Some(Self::BeginEditInsert),
            "toggle_complete" => Some(Self::ToggleComplete),
            "delete" => Some(Self::Delete),
            "reschedule" => Some(Self::Reschedule),
            "cycle_priority" => Some(Self::CyclePriority),
            "move_task_down" => Some(Self::MoveTaskDown),
            "move_task_up" => Some(Self::MoveTaskUp),
            "begin_search" | "search" => Some(Self::BeginSearch),
            "open_help" | "help" => Some(Self::OpenHelp),
            "open_settings" | "settings" => Some(Self::OpenSettings),
            "open_command_palette" | "command_palette" => Some(Self::OpenCommandPalette),
            "undo" => Some(Self::Undo),
            "toggle_visual" => Some(Self::ToggleVisual),
            "toggle_selected" => Some(Self::ToggleSelected),
            "go_list" | "list" => Some(Self::GoList),
            "toggle_archive_view" | "archive_view" => Some(Self::ToggleArchiveView),
            "archive_completed" => Some(Self::ArchiveCompleted),
            "arm_f" => Some(Self::ArmF),
            "pick_project" => Some(Self::PickProject),
            "pick_context" => Some(Self::PickContext),
            "pick_saved_filter" => Some(Self::PickSavedFilter),
            "save_current_filter" => Some(Self::SaveCurrentFilter),
            "cycle_sort" => Some(Self::CycleSort),
            "begin_prompt_project" | "prompt_project" => Some(Self::BeginPromptProject),
            "begin_prompt_context" | "prompt_context" => Some(Self::BeginPromptContext),
            "toggle_left_pane" => Some(Self::ToggleLeftPane),
            "toggle_right_pane" => Some(Self::ToggleRightPane),
            "cycle_theme" => Some(Self::CycleTheme),
            "cycle_density" => Some(Self::CycleDensity),
            "toggle_line_num" | "toggle_line_numbers" => Some(Self::ToggleLineNum),
            "toggle_show_done" => Some(Self::ToggleShowDone),
            "toggle_show_future" => Some(Self::ToggleShowFuture),
            "toggle_autocomplete_archive" => Some(Self::ToggleAutocompleteArchive),
            "copy_line" => Some(Self::CopyLine),
            "copy_body" => Some(Self::CopyBody),
            "open_note" | "note" => Some(Self::OpenNote),
            "create_or_open_note" | "create_note" => Some(Self::CreateOrOpenNote),
            "escape_stack" | "escape" => Some(Self::EscapeStack),
            "open_share" | "share" => Some(Self::OpenShare),
            "open_theme_picker" | "theme_picker" => Some(Self::OpenThemePicker),
            "change_week_start" => Some(Self::ChangeWeekStart),
            _ => None,
        }
    }
}

/// Motions inside the `↻ REPEAT` recurrence-builder overlay. Separate from
/// [`Action`] because the overlay owns the keyboard while it is open, so its
/// keys occupy their own namespace and can safely reuse letters (`h`, `l`)
/// that mean something else in normal mode. Bound under `[recurrence]` in
/// `keybinds.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecAction {
    /// Move focus to the next field (interval → unit → mode, wrapping).
    FocusNext,
    /// Move focus to the previous field.
    FocusPrev,
    /// Increment the focused field: bump the interval, or cycle the unit /
    /// mode forward.
    ValueNext,
    /// Decrement the focused field.
    ValuePrev,
    /// Write the `rec:` token and close.
    Accept,
    /// Close without writing.
    Cancel,
}

impl RecAction {
    /// Map a `keybinds.toml` key name to an action. Aliases mirror the
    /// vocabulary used by `[normal]`: both a descriptive name and a shorter
    /// one where an obvious short form exists.
    pub fn from_keybind_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "focus_next" | "next_field" => Some(Self::FocusNext),
            "focus_prev" | "prev_field" => Some(Self::FocusPrev),
            "value_next" | "increase" => Some(Self::ValueNext),
            "value_prev" | "decrease" => Some(Self::ValuePrev),
            "accept" | "save" => Some(Self::Accept),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rec_actions_are_rebindable() {
        assert_eq!(
            RecAction::from_keybind_name("focus_next"),
            Some(RecAction::FocusNext)
        );
        assert_eq!(
            RecAction::from_keybind_name("next_field"),
            Some(RecAction::FocusNext)
        );
        assert_eq!(
            RecAction::from_keybind_name("increase"),
            Some(RecAction::ValueNext)
        );
        assert_eq!(
            RecAction::from_keybind_name("decrease"),
            Some(RecAction::ValuePrev)
        );
        assert_eq!(
            RecAction::from_keybind_name("cancel"),
            Some(RecAction::Cancel)
        );
        assert_eq!(RecAction::from_keybind_name("nope"), None);
    }

    #[test]
    fn reschedule_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("reschedule"),
            Some(Action::Reschedule)
        );
    }

    #[test]
    fn open_theme_picker_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("open_theme_picker"),
            Some(Action::OpenThemePicker)
        );
        assert_eq!(
            Action::from_keybind_name("theme_picker"),
            Some(Action::OpenThemePicker)
        );
    }

    #[test]
    fn open_note_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("open_note"),
            Some(Action::OpenNote)
        );
        assert_eq!(Action::from_keybind_name("note"), Some(Action::OpenNote));
        assert_eq!(
            Action::from_keybind_name("create_or_open_note"),
            Some(Action::CreateOrOpenNote)
        );
        assert_eq!(
            Action::from_keybind_name("create_note"),
            Some(Action::CreateOrOpenNote)
        );
    }

    #[test]
    fn task_movement_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("move_task_down"),
            Some(Action::MoveTaskDown)
        );
        assert_eq!(
            Action::from_keybind_name("move_task_up"),
            Some(Action::MoveTaskUp)
        );
    }
}
