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
    BeginEdit,
    ToggleComplete,
    Delete,
    Reschedule,
    CyclePriority,
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
    CopyLine,
    CopyBody,
    EscapeStack,
    /// Open the phone-capture overlay (QR + URL). First invocation lazily
    /// binds the HTTP server; subsequent invocations just re-show the
    /// overlay.
    OpenShare,
    /// Open the theme picker dialog (j/k to preview, Enter to accept).
    OpenThemePicker,
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
            "toggle_complete" => Some(Self::ToggleComplete),
            "delete" => Some(Self::Delete),
            "reschedule" => Some(Self::Reschedule),
            "cycle_priority" => Some(Self::CyclePriority),
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
            "copy_line" => Some(Self::CopyLine),
            "copy_body" => Some(Self::CopyBody),
            "escape_stack" | "escape" => Some(Self::EscapeStack),
            "open_share" | "share" => Some(Self::OpenShare),
            "open_theme_picker" | "theme_picker" => Some(Self::OpenThemePicker),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
