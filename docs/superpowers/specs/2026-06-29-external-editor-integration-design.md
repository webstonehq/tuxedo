# External Editor Integration — Design Spec

**Date**: 2026-06-29

## Summary

Add `$EDITOR` integration to tuxedo so users can edit todo items in a real text editor. Two integration points: a new CLI `edit` command and a TUI keybinding (`E`).

## Architecture

New module: `src/editor.rs` — self-contained, zero dependencies beyond stdlib.

```
editor.rs
  └── pub fn edit_in_editor(content: &str) -> anyhow::Result<Option<String>>
```

The function writes content to a temp file, resolves and spawns the editor, blocks until exit, reads the result, cleans up, and returns the new text (or `None` if unchanged).

Callers:
- `src/cmd/mod.rs` — for CLI `edit` command
- `src/main.rs` — for TUI `LaunchEditor` action

Both call the same `edit_in_editor()`, then pass results through the existing `store.edit_line()` path.

## Components

### 1. `editor.rs` module

Public API:

```rust
pub fn edit_in_editor(content: &str) -> anyhow::Result<Option<String>>
```

**Temp file**: Created in `std::env::temp_dir()` with prefix `tuxedo-edit-`. File is guaranteed cleaned up on return (both success and error paths) via a `scopeguard`-style drop guard.

**Editor resolution**:
1. Check `$EDITOR` env var. If set and it's a bare name (no `/` in it), search `$PATH`. If found, use it.
2. Fall back through a hardcoded search list: `nvim`, `vim`, `vi`, `nano`, `emacs`, `helix`
3. If nothing found, return `anyhow::Error("no editor found")`

**Process spawning**: `std::process::Command::new(editor).arg(&temp_path).status()`. Blocks. If exit code is non-zero, still returns the file content (user might have opened, made no changes, saved, exited). Only errors on spawn failure.

**Return value**:
- `Ok(Some(text))` if file content differs from input (whitespace-trimmed comparison)
- `Ok(None)` if unchanged — caller can skip the store mutation
- `Err(...)` if editor couldn't launch or temp file couldn't be read

### 2. CLI `edit` command

New entry in `src/cmd/mod.rs` `SUBCOMMANDS`:

```
"edit" => ["e"]
```

**Usage**: `tuxedo edit <N>`

**Behavior**:
1. Parse task number `N` (1-based)
2. Open `Store`, reconcile against disk (same pattern as `replace`)
3. Get task `N`'s raw text, pass to `edit_in_editor()`
4. On `Ok(Some(new_text))`: call `store.edit_line(abs, &new_text)`, print output
5. On `Ok(None)`: print "No changes." exit 0
6. On trimmed-empty text: error "task text cannot be empty"
7. On `Err`: print error to stderr, exit 1
8. Supports `--json` flag (as other commands do)

No changes to `replace`, `append`, or `prepend`.

### 3. TUI `LaunchEditor` action

**Keybinding**: `E` in Normal mode maps to `Action::LaunchEditor`.

Registered in `src/keybinds.rs` defaults and in `src/main.rs` action dispatch.

**Flow** in `handle_normal()`:

```
Action::LaunchEditor => {
    let Some(abs) = app.selection.editing()
        .or_else(|| app.visibility.cursor_abs()) else { return };

    let raw = app.store.tasks[abs].raw.clone();

    // Suspend terminal
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(stdout(), crossterm::terminal::LeaveAlternateScreen)?;

    let result = editor::edit_in_editor(&raw);

    // Restore terminal
    crossterm::execute!(stdout(), crossterm::terminal::EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;

    // crossterm resize events will be picked up by the
    // existing event loop on the next iteration

    match result {
        Ok(Some(new_raw)) => {
            app.store.edit_line(abs, &new_raw);
            app.after_mutation(abs);
            app.flash.show("saved");
        }
        Ok(None) => {
            app.flash.show("no changes");
        }
        Err(e) => {
            app.flash.show(format!("editor error: {e}"));
        }
    }

    // Full re-render
    app.needs_redraw = true;
}
```

**Terminal lifecycle**:
1. Before suspend: `disable_raw_mode()` + `LeaveAlternateScreen` — returns terminal to "normal" state so the editor can use it. The ratatui `Terminal` object stays alive; only the terminal mode changes.
2. After editor exits: `EnterAlternateScreen` + `enable_raw_mode()` — re-enters TUI mode. A full re-render is forced to repaint the screen.
3. No additional hook re-initialization needed: the project uses Rust's default panic handler, and crossterm `Resize` events continue to fire through the existing event loop.

**Edge cases**:
- Pressing `E` with no tasks visible: no-op
- Pressing `E` in Insert mode / any overlay: ignored (only Normal mode)
- Visual mode multi-selection: edits the primary selection, same as `e` does
- Editor crashes (non-zero exit): content still read, user may have saved before crash

## Error Handling

| Scenario | CLI behavior | TUI behavior |
|----------|-------------|--------------|
| No editor found | stderr error, exit 1 | flash "no editor found" |
| Temp file creation failure | stderr error, exit 1 | flash "temp file error" |
| Editor spawn failure | stderr error, exit 1 | flash "editor error: {msg}" |
| Task N out of range | stderr "task N not found" | N/A (always on valid task) |
| External file modified | "aborted: file modified" | Same as existing reconcile path |
| Editor returns empty text | stderr "task text cannot be empty", exit 1 | flash "cannot be empty", keep old text |

## Testing

- Unit tests in `src/editor.rs`: editor resolution logic (parsable in tests, mock process spawning not required)
- Unit test: `edit` command arg parsing in `src/cmd/mod.rs`
- Unit test: `LaunchEditor` action dispatch in `src/main.rs` test block

Manual verification:
- `tuxedo edit 1` with item 1
- `TUXEDO_TEST_FILE=/tmp/test-todo.txt tuxedo edit 1` with a test file
- TUI: open tuxedo, press `E` on a task, edit in vim, verify save
- TUI: ensure `e`/`i` inline editing still works unchanged
- TUI: ensure terminal state is clean after editor close (no leftover chars, correct sizing)

## Non-Goals

- Inline $EDITOR within the TUI without suspend (complex, fragile)
- Multiple-task batch editing in $EDITOR (future possibility, not v1)
- Editor configuration (syntax highlighting, file type detection for temp files)
- Windows terminal suspend support (out of scope for initial implementation)
