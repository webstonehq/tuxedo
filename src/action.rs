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
