//! Every discrete behavior the user can trigger, decoupled from the keystroke
//! that fires it. Lives at the crate root (not under `app`) so both the binary
//! (which dispatches actions in `apply_action`) and the command palette
//! (which lists them) can name them without a cyclic dependency.
//!
//! Each keyboard context owns its own vocabulary: [`Action`] drives
//! `[normal]`, and one small enum per overlay drives that overlay's table in
//! `keybinds.toml`. They are separate types rather than one flat enum because
//! an overlay owns the keyboard while it is open, so its keys occupy their
//! own namespace and can safely reuse letters (`h`, `l`, `q`) that mean
//! something else elsewhere.

use crate::keybinds::KeyAction;

/// Define an action vocabulary and wire it to a `keybinds.toml` table.
///
/// Each variant lists the names a config may use for it; the first is the
/// canonical one that the documentation and `tuxedo keybinds` print.
macro_rules! key_actions {
    (
        $(#[$meta:meta])*
        pub enum $name:ident in $table:literal {
            $(
                $(#[$vmeta:meta])*
                $variant:ident = [$($alias:literal),+ $(,)?],
            )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $( $(#[$vmeta])* $variant, )*
        }

        impl KeyAction for $name {
            const TABLE: &'static str = $table;

            fn from_keybind_name(name: &str) -> Option<Self> {
                let normalized = name.trim().replace('-', "_").to_ascii_lowercase();
                match normalized.as_str() {
                    $( $($alias)|+ => Some(Self::$variant), )*
                    _ => None,
                }
            }
        }

        impl $name {
            /// Every action name a config may use for this table, canonical
            /// name first. Drives the generated reference and the docs.
            pub const NAMES: &'static [&'static [&'static str]] = &[
                $( &[$($alias),+], )*
            ];
        }
    };
}
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
    Redo,
    ToggleVisual,
    ToggleSelected,
    GoList,
    ToggleArchiveView,
    ToggleTrashView,
    /// `x` in the Trash view — put a deleted task back in the live list.
    TrashRestore,
    /// `E` in the Trash view — delete everything in the trash.
    EmptyTrash,
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

impl KeyAction for Action {
    const TABLE: &'static str = "normal";

    fn from_keybind_name(s: &str) -> Option<Self> {
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
            "redo" => Some(Self::Redo),
            "toggle_visual" => Some(Self::ToggleVisual),
            "toggle_selected" => Some(Self::ToggleSelected),
            "go_list" | "list" => Some(Self::GoList),
            "toggle_archive_view" | "archive_view" => Some(Self::ToggleArchiveView),
            "toggle_trash_view" | "trash_view" => Some(Self::ToggleTrashView),
            "trash_restore" | "restore" => Some(Self::TrashRestore),
            "empty_trash" => Some(Self::EmptyTrash),
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

impl KeyAction for RecAction {
    const TABLE: &'static str = "recurrence";

    /// Aliases mirror the vocabulary used by `[normal]`: both a descriptive
    /// name and a shorter one where an obvious short form exists.
    fn from_keybind_name(name: &str) -> Option<Self> {
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
    use crate::keybinds::KeyAction;

    /// Every alias a table advertises must resolve, and no alias may appear
    /// twice — a duplicate would silently bind one action's name to another.
    fn check_names<A: KeyAction + std::fmt::Debug>(names: &[&[&str]], table: &str) {
        let mut seen: Vec<&str> = Vec::new();
        for aliases in names {
            for alias in *aliases {
                assert!(
                    A::from_keybind_name(alias).is_some(),
                    "[{table}] advertises `{alias}` but does not resolve it"
                );
                assert!(!seen.contains(alias), "[{table}] lists `{alias}` twice");
                seen.push(alias);
            }
        }
        assert!(!seen.is_empty(), "[{table}] advertises no action names");
    }

    #[test]
    fn every_overlay_vocabulary_round_trips() {
        check_names::<GlobalAction>(GlobalAction::NAMES, GlobalAction::TABLE);
        check_names::<DialogAction>(DialogAction::NAMES, DialogAction::TABLE);
        check_names::<DialogInsertAction>(DialogInsertAction::NAMES, DialogInsertAction::TABLE);
        check_names::<CalendarAction>(CalendarAction::NAMES, CalendarAction::TABLE);
        check_names::<PriorityAction>(PriorityAction::NAMES, PriorityAction::TABLE);
        check_names::<SlashAction>(SlashAction::NAMES, SlashAction::TABLE);
        check_names::<AutocompleteAction>(AutocompleteAction::NAMES, AutocompleteAction::TABLE);
        check_names::<SearchAction>(SearchAction::NAMES, SearchAction::TABLE);
        check_names::<PromptAction>(PromptAction::NAMES, PromptAction::TABLE);
        check_names::<PickAction>(PickAction::NAMES, PickAction::TABLE);
        check_names::<ThemePickerAction>(ThemePickerAction::NAMES, ThemePickerAction::TABLE);
        check_names::<PaletteAction>(PaletteAction::NAMES, PaletteAction::TABLE);
        check_names::<HelpAction>(HelpAction::NAMES, HelpAction::TABLE);
        check_names::<SettingsAction>(SettingsAction::NAMES, SettingsAction::TABLE);
        check_names::<WelcomeAction>(WelcomeAction::NAMES, WelcomeAction::TABLE);
    }

    #[test]
    fn action_names_are_case_and_dash_insensitive() {
        assert_eq!(
            CalendarAction::from_keybind_name("MOVE-LEFT"),
            Some(CalendarAction::MoveLeft)
        );
        assert_eq!(
            PickAction::from_keybind_name("  rename  "),
            Some(PickAction::Rename)
        );
        assert_eq!(PickAction::from_keybind_name("nope"), None);
    }

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

key_actions! {
    /// Global keys, live in every mode and overlay. Checked before the
    /// context's own table so a quit key always works.
    pub enum GlobalAction in "global" {
        /// Leave the app.
        Quit = ["quit"],
    }
}

key_actions! {
    /// Vim-normal editing inside the create/edit dialog — the modal editor
    /// the `e` key opens. `[dialog]` supports two-key chords (`dw`, `cw`).
    pub enum DialogAction in "dialog" {
        /// Move the cursor one character left.
        CursorLeft = ["cursor_left", "left"],
        /// Move the cursor one character right.
        CursorRight = ["cursor_right", "right"],
        /// Jump to the start of the next word.
        WordForward = ["word_forward", "word_next"],
        /// Jump to the start of the previous word.
        WordBackward = ["word_backward", "word_prev"],
        /// Jump to the end of the current word.
        WordEnd = ["word_end"],
        /// Delete the character under the cursor.
        DeleteForward = ["delete_forward", "delete_char"],
        /// Delete to the end of the word (`dw`).
        DeleteWord = ["delete_word"],
        /// Delete to the end of the word and start typing (`cw`).
        ChangeWord = ["change_word"],
        /// Start typing at the cursor.
        Insert = ["insert"],
        /// Start typing one character right of the cursor.
        Append = ["append"],
        /// Start typing at the end of the line.
        AppendEnd = ["append_end", "append_line_end"],
        /// Write the task and close the dialog.
        Accept = ["accept", "save"],
        /// Close the dialog, discarding the draft.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// Typing inside the dialog's text field. Only the two keys that leave
    /// or commit are bindable; every other keystroke is text.
    pub enum DialogInsertAction in "dialog_insert" {
        /// Write the task and close the dialog.
        Accept = ["accept", "save"],
        /// Return to the dialog's normal mode.
        Normal = ["normal", "cancel"],
    }
}

key_actions! {
    /// The `📅` calendar overlay that `due:` / `t:` and the `r` key open.
    pub enum CalendarAction in "calendar" {
        /// Previous day.
        MoveLeft = ["move_left", "prev_day"],
        /// Next day.
        MoveRight = ["move_right", "next_day"],
        /// Same weekday, previous week.
        MoveUp = ["move_up", "prev_week"],
        /// Same weekday, next week.
        MoveDown = ["move_down", "next_week"],
        /// Jump to today.
        Today = ["today"],
        /// Jump to tomorrow.
        Tomorrow = ["tomorrow"],
        /// Jump a week out.
        WeekAhead = ["week_ahead", "in_a_week"],
        /// Same date, next month.
        MonthNext = ["month_next"],
        /// Same date, previous month.
        MonthPrev = ["month_prev"],
        /// Clear the date and close.
        Clear = ["clear"],
        /// Write the date and close.
        Accept = ["accept", "save"],
        /// Close without writing.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `(A)` priority chooser inside the dialog.
    pub enum PriorityAction in "priority" {
        /// Next priority down the list.
        Next = ["next"],
        /// Previous priority.
        Prev = ["prev"],
        /// Write the priority and close.
        Accept = ["accept", "save"],
        /// Close without writing.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `/`-triggered metadata menu inside the dialog. Keys not bound
    /// here type into the filter, so the list narrows as the user writes.
    pub enum SlashAction in "slash" {
        /// Highlight the next entry.
        Next = ["next"],
        /// Highlight the previous entry.
        Prev = ["prev"],
        /// Insert the highlighted entry.
        Accept = ["accept", "save"],
        /// Close the menu.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `+project` / `@context` completion popup. Shares its vocabulary
    /// with the dialog and the prompt, both of which can show it.
    pub enum AutocompleteAction in "autocomplete" {
        /// Highlight the next suggestion.
        Next = ["next"],
        /// Highlight the previous suggestion.
        Prev = ["prev"],
        /// Insert the highlighted suggestion.
        Accept = ["accept"],
        /// Hide the popup, leaving the typed text alone.
        Dismiss = ["dismiss", "cancel"],
    }
}

key_actions! {
    /// The `/` search line. Unbound keys are the search text.
    pub enum SearchAction in "search" {
        /// Keep the search and return to the list.
        Accept = ["accept", "save"],
        /// Clear the search and return to the list.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The single-field prompts: add project, add context, rename, and name
    /// a saved filter. Unbound keys are the prompt text.
    pub enum PromptAction in "prompt" {
        /// Submit the typed value.
        Accept = ["accept", "save"],
        /// Discard and return to the list.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `fp` / `fc` / `ff` cycle pickers.
    pub enum PickAction in "pick" {
        /// Next entry.
        Next = ["next"],
        /// Previous entry.
        Prev = ["prev"],
        /// Rename the highlighted project or context.
        Rename = ["rename"],
        /// Keep the highlighted entry as the active filter.
        Accept = ["accept", "save"],
        /// Revert to the filter that was active before.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `T` theme picker, which previews as it moves.
    pub enum ThemePickerAction in "theme_picker" {
        /// Preview the next theme.
        Next = ["next"],
        /// Preview the previous theme.
        Prev = ["prev"],
        /// Keep the previewed theme.
        Accept = ["accept", "save"],
        /// Revert to the theme that was active before.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `:` command palette. Unbound keys are the filter text — plain
    /// `j` / `k` must type, so navigation lives on arrows and `Ctrl-n` /
    /// `Ctrl-p` by default.
    pub enum PaletteAction in "palette" {
        /// Highlight the next command.
        Next = ["next"],
        /// Highlight the previous command.
        Prev = ["prev"],
        /// Run the highlighted command.
        Accept = ["accept", "run"],
        /// Close without running anything.
        Cancel = ["cancel"],
    }
}

key_actions! {
    /// The `?` help overlay.
    pub enum HelpAction in "help" {
        /// Close the overlay.
        Close = ["close", "cancel"],
    }
}

key_actions! {
    /// The `,` settings overlay. Its toggles mirror the `[normal]` ones so
    /// the same key works inside and outside the overlay.
    pub enum SettingsAction in "settings" {
        /// Close the overlay.
        Close = ["close", "cancel"],
        /// Next theme.
        CycleTheme = ["cycle_theme"],
        /// Next row density.
        CycleDensity = ["cycle_density"],
        /// Show or hide line numbers.
        ToggleLineNum = ["toggle_line_num", "toggle_line_numbers"],
        /// Show or hide the filter sidebar.
        ToggleLeftPane = ["toggle_left_pane"],
        /// Show or hide the detail pane.
        ToggleRightPane = ["toggle_right_pane"],
        /// Show or hide completed tasks.
        ToggleShowDone = ["toggle_show_done"],
        /// Show or hide future-threshold tasks.
        ToggleShowFuture = ["toggle_show_future"],
        /// Next sort order.
        CycleSort = ["cycle_sort"],
    }
}

key_actions! {
    /// The first-run overlay shown when there is no todo.txt to open.
    pub enum WelcomeAction in "welcome" {
        /// Create the todo.txt named on the command line.
        CreateFile = ["create_file", "create"],
        /// Open the bundled sample file instead.
        OpenSample = ["open_sample", "sample"],
        /// Leave without creating anything.
        Quit = ["quit", "cancel"],
    }
}
