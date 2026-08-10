# External Editor Integration — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `$EDITOR` launching to tuxedo — a new CLI `edit` command and a TUI `E` keybinding that opens the current task in an external editor, saves the result back through the existing `store.edit_line()` path.

**Architecture:** A new self-contained `editor.rs` module handles editor resolution and temp-file-based editing. The CLI adds an `edit` subcommand that calls it. The TUI adds a `LaunchEditor` action in `apply_action` that suspends the terminal, calls `editor::edit_in_editor()`, restores the terminal, and saves the result.

**Tech Stack:** Rust stdlib (`std::process::Command`, `std::env`, `std::fs`, tempfile via `std::env::temp_dir`), `crossterm` for terminal suspend/restore, `anyhow` for errors.

**Files:**
- Create: `src/editor.rs`
- Modify: `src/lib.rs` (register module)
- Modify: `src/action.rs` (add `LaunchEditor` variant + keybind name)
- Modify: `src/cmd/mod.rs` (add `edit` subcommand)
- Modify: `src/main.rs` (add keybinding dispatch + action handler)
- Modify: `src/app/mod.rs` (add `launch_editor` method on `App`)

---

### Task 1: Add `LaunchEditor` to the `Action` enum

**Files:**
- Modify: `src/action.rs:1-59`

- [ ] **Step 1: Add the variant**

Add `LaunchEditor` after `OpenThemePicker` (line 58):

```rust
    /// Edit the current task with $EDITOR (or a fallback editor).
    LaunchEditor,
```

- [ ] **Step 2: Add `from_keybind_name` mapping**

Add the mapping inside the match block (after line 108, before `_ => None`):

```rust
            "launch_editor" => Some(Self::LaunchEditor),
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1`
Expected: compiles cleanly (unused variant warning is OK for now)

- [ ] **Step 4: Commit**

```bash
git add src/action.rs
git commit -m "feat(action): add LaunchEditor variant for external editor integration"
```

---

### Task 2: Create `editor.rs` module

**Files:**
- Create: `src/editor.rs`

- [ ] **Step 1: Write the module**

```rust
use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(content: &str) -> Result<Self> {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "tuxedo-edit-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, content)
            .with_context(|| format!("writing temp file {}", path.display()))?;
        Ok(Self { path })
    }

    fn read(&self) -> Result<String> {
        std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading temp file {}", self.path.display()))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn resolve_editor() -> Result<String> {
    if let Ok(editor) = std::env::var("EDITOR") {
        let editor = editor.trim();
        if !editor.is_empty() {
            return Ok(editor.to_string());
        }
    }
    let fallbacks = ["nvim", "vim", "vi", "nano", "emacs", "helix"];
    for name in fallbacks {
        if which::which(name).is_ok() {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!("no editor found (set $EDITOR or install vim/nano)")
}

fn which(name: &str) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        let path = PathBuf::from(name);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!("editor '{name}' not found")
    }
    #[cfg(unix)]
    {
        let path_env = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path_env) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    #[cfg(not(unix))]
    {
        std::env::var_os("PATH");
        anyhow::bail!("PATH-based editor lookup not implemented on this platform")
    }
    anyhow::bail!("editor '{name}' not found in PATH")
}

pub fn edit_in_editor(content: &str) -> Result<Option<String>> {
    let tf = TempFile::create(content)?;
    let editor = resolve_editor()?;
    let status = Command::new(&editor)
        .arg(&tf.path)
        .status()
        .with_context(|| format!("spawning editor: {editor}"))?;
    if !status.success() {
        anyhow::bail!("editor exited with {}", status);
    }
    let new_content = tf.read()?;
    if new_content.trim() == content.trim() {
        return Ok(None);
    }
    Ok(Some(new_content))
}
```

- [ ] **Step 2: Check stdlib-only approach**

The `which` helper above is hand-rolled. Check that `std::env::split_paths` exists in Rust.

Run: `cargo check 2>&1`
Expected: compiles cleanly

- [ ] **Step 3: Commit**

```bash
git add src/editor.rs
git commit -m "feat(editor): add external editor launching with $EDITOR fallback chain"
```

---

### Task 3: Register `editor` module in `lib.rs`

**Files:**
- Modify: `src/lib.rs:1-22`

- [ ] **Step 1: Add the module declaration**

After `pub mod config;` (line 9), add:

```rust
pub mod editor;
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check 2>&1`
Expected: compiles cleanly

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat(lib): register editor module"
```

---

### Task 4: Add CLI `edit` subcommand

**Files:**
- Modify: `src/cmd/mod.rs:42-46` (SUBCOMMANDS table)
- Modify: `src/cmd/mod.rs:100-119` (command dispatch)
- Modify: `src/cmd/mod.rs` (add `cmd_edit` function, after `cmd_text_op`)

- [ ] **Step 1: Add `edit` / `e` to SUBCOMMANDS**

Change line 42-46 to include `edit` and `e`:

```rust
const SUBCOMMANDS: &[&str] = &[
    "add", "a", "append", "app", "prepend", "prep", "replace", "edit", "e", "pri", "p", "depri",
    "dp", "done", "do", "complete", "del", "rm", "archive", "list", "ls", "listall", "lsa",
    "listpri", "lsp", "listproj", "lsprj", "listcon", "lsc",
];
```

- [ ] **Step 2: Add dispatch case**

In the `match cmd.as_str()` block (around line 100), add before the `other` arm:

```rust
        "edit" | "e" => cmd_edit(&mut store, pos, json),
```

- [ ] **Step 3: Implement `cmd_edit` function**

Add this function after `cmd_text_op` (after line 255):

```rust
fn cmd_edit(store: &mut Store, pos: &[String], json: bool) -> i32 {
    if pos.len() < 1 {
        return usage("edit N");
    }
    let len = store.tasks().len();
    let abs = match parse_index(&pos[0], len) {
        Ok(i) => i,
        Err(e) => return err(e),
    };
    let old_raw = store.tasks()[abs].raw.clone();
    match crate::editor::edit_in_editor(&old_raw) {
        Ok(Some(new_raw)) => {
            match store.edit_line(abs, &new_raw) {
                EditOutcome::Saved { abs } => {
                    let n = abs + 1;
                    let t = &store.tasks()[abs];
                    if json {
                        json_task("edit", n, t);
                    } else {
                        println!("{n} {old_raw}");
                        println!("TODO: Edited task to:");
                        println!("{n} {}", t.raw);
                    }
                    0
                }
                EditOutcome::Empty => err("task text cannot be empty"),
                EditOutcome::OutOfRange => err(format!("no task {}", abs + 1)),
                EditOutcome::TermNotFound => err("term not found"),
                EditOutcome::Aborted(_) => err("file changed on disk; nothing changed"),
                EditOutcome::Error(e) => store_error(json, "edit", e),
            }
        }
        Ok(None) => {
            // No changes made
            0
        }
        Err(e) => {
            eprintln!("tuxedo edit: {e}");
            1
        }
    }
}
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check 2>&1`
Expected: compiles cleanly

- [ ] **Step 5: Run existing tests to ensure no regressions**

Run: `cargo test --lib 2>&1`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add src/cmd/mod.rs
git commit -m "feat(cmd): add edit subcommand that launches $EDITOR"
```

---

### Task 5: Add `launch_editor` method to `App`

**Files:**
- Modify: `src/app/mod.rs` (add method after `save_edit` or near it)

- [ ] **Step 1: Add the method**

Add `launch_editor` to `impl App` in `src/app/mod.rs`. Place it after the `save_edit` method (which reads `selection.editing()`).

First, find `save_edit`:

```bash
rg "pub fn save_edit" src/app/mutations.rs
```

It's in `src/app/mutations.rs` at line 103. Add after it (after line 118):

```rust
    pub fn launch_editor(&mut self) {
        use std::io::Write as _;

        let idx = match self.selection.editing() {
            Some(idx) => idx,
            None => match self.cur_abs() {
                Some(idx) => idx,
                None => return,
            },
        };
        let raw = match self.task_raw(idx) {
            Some(r) => r,
            None => return,
        };

        crossterm::terminal::disable_raw_mode().ok();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen
        );

        let result = crate::editor::edit_in_editor(&raw);

        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen
        );
        crossterm::terminal::enable_raw_mode().ok();

        match result {
            Ok(Some(new_raw)) => {
                match self.store.edit_line(idx, &new_raw) {
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
```

- [ ] **Step 2: Add the `use` imports needed in `mutations.rs`**

The file already imports from `crate::core` but needs `crossterm`. Check existing imports at the top of `src/app/mutations.rs`:

```bash
head -20 src/app/mutations.rs
```

Add these imports if not already present:

At the top, add:

```rust
use crossterm::Executor;
```

Actually, `crossterm::execute!` doesn't need a `use` import (it's a macro). But we need `use std::io::Write` for the macro. The file likely already imports this. Let me check.

- [ ] **Step 3: Verify compilation**

Run: `cargo check 2>&1`
Expected: compiles cleanly

- [ ] **Step 4: Commit**

```bash
git add src/app/mutations.rs
git commit -m "feat(app): add launch_editor method for external editor in TUI"
```

---

### Task 6: Wire `LaunchEditor` action to the TUI keybinding

**Files:**
- Modify: `src/main.rs:771-839` (add `E` in `resolve_normal_key`)
- Modify: `src/main.rs:1082-1093` (add `LaunchEditor` handler in `apply_action`)
- Modify: `src/main.rs:867-868` (add `LaunchEditor` to archive read-only guard)

- [ ] **Step 1: Add `E` keybinding in `resolve_normal_key`**

After line 783 (`KeyCode::Char('i') => Action::BeginEditInsert,`), add:

```rust
        KeyCode::Char('E') => Action::LaunchEditor,
```

- [ ] **Step 2: Add `LaunchEditor` to archive read-only guard**

In `apply_action`, line 867-868, add `LaunchEditor` to the set of disallowed actions in archive view:

```rust
            | Action::BeginEdit
            | Action::BeginEditInsert
            | Action::LaunchEditor
```

- [ ] **Step 3: Add handler in `apply_action`**

After the `Action::Reschedule` handler (around line 1091, before the closing `}` of the match block), add:

```rust
        Action::LaunchEditor => {
            app.launch_editor();
        }
```

- [ ] **Step 4: Verify compilation and run tests**

Run: `cargo check 2>&1 && cargo test --lib 2>&1`
Expected: compiles and all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(tui): wire E keybinding to LaunchEditor action with terminal suspend"
```

---

### Task 7: Write unit tests

**Files:**
- Modify: `src/editor.rs` (add `#[cfg(test)]` block)
- Modify: `src/cmd/mod.rs` (add test for `edit` subcommand)
- Modify: `src/main.rs` (add test for `E` keybinding resolves to `LaunchEditor`)

- [ ] **Step 1: Add editor resolution test**

At the bottom of `src/editor.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_editor_respects_env() {
        std::env::set_var("EDITOR", "my-editor");
        let result = resolve_editor();
        std::env::remove_var("EDITOR");
        // Should return the set value, even if my-editor doesn't exist
        assert_eq!(result.unwrap(), "my-editor");
    }

    #[test]
    fn resolve_editor_falls_back_when_unset() {
        // Save and clear EDITOR
        let saved = std::env::var("EDITOR").ok();
        std::env::remove_var("EDITOR");
        let result = resolve_editor();
        if let Some(v) = saved {
            std::env::set_var("EDITOR", v);
        }
        // Should find something or return an error; both are fine in test
        match result {
            Ok(_) => {} // found an editor in PATH
            Err(e) => assert!(
                e.to_string().contains("no editor found"),
                "unexpected error: {e}"
            ),
        }
    }

    #[test]
    fn resolve_editor_uses_env_var() {
        std::env::set_var("EDITOR", "/usr/bin/vim");
        let result = resolve_editor().unwrap();
        std::env::remove_var("EDITOR");
        assert_eq!(result, "/usr/bin/vim");
    }

    #[test]
    fn resolve_editor_trims_env_value() {
        std::env::set_var("EDITOR", "  nano  ");
        let result = resolve_editor().unwrap();
        std::env::remove_var("EDITOR");
        assert_eq!(result, "nano");
    }

    #[test]
    fn resolve_editor_finds_system_editor_when_env_unset() {
        let saved = std::env::var("EDITOR").ok();
        std::env::remove_var("EDITOR");
        let result = resolve_editor();
        if let Some(v) = saved {
            std::env::set_var("EDITOR", v);
        }
        assert!(
            result.is_ok(),
            "resolve_editor should find a system editor: {result:?}"
        );
    }
}
```

- [ ] **Step 2: Add keybinding resolution test**

In `src/main.rs` tests block (after line 1116), add:

```rust
    #[test]
    fn e_resolves_to_launch_editor() {
        let mut app = mini_app();
        let action = resolve(&mut app, key('E'));
        assert_eq!(action, Some(Action::LaunchEditor));
    }
```

- [ ] **Step 3: Add CLI `edit` subcommand test**

In `src/cmd/mod.rs` tests block, add a test that verifies `edit` is recognized as a subcommand:

```rust
    #[test]
    fn edit_subcommand_recognized() {
        let args: Vec<String> = ["edit".into(), "1".into()].into();
        let idx = find_subcommand(&args);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn e_alias_recognized() {
        let args: Vec<String> = ["e".into(), "1".into()].into();
        let idx = find_subcommand(&args);
        assert_eq!(idx, Some(0));
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib 2>&1`
Expected: all tests pass (the `edit_in_editor` test may be skipped if no editor; that's OK)

- [ ] **Step 5: Commit**

```bash
git add src/editor.rs src/main.rs src/cmd/mod.rs
git commit -m "test: add tests for editor integration"
```

---

### Task 8: End-to-end verification

- [ ] **Step 1: Test CLI `edit` command with a test file**

```bash
export TODO_FILE=/tmp/test-todo-edit.txt
echo "foo +bar @home" > $TODO_FILE
tuxedo edit 1
# Should open editor, make changes, save, exit
# Verify the file was updated
cat $TODO_FILE
rm $TODO_FILE
unset TODO_FILE
```

- [ ] **Step 2: Test TUI `E` keybinding**

```bash
export TODO_FILE=/tmp/test-todo-tui.txt
echo "test task one" > $TODO_FILE
echo "test task two" >> $TODO_FILE
tuxedo
# Press E on first task → editor opens → change text → save/exit
# Verify TUI shows updated text
# Press E on second task → change text → save/exit
cat $TODO_FILE
rm $TODO_FILE
unset TODO_FILE
```

- [ ] **Step 3: Test `$EDITOR` unset fallback**

```bash
unset EDITOR
export TODO_FILE=/tmp/test-fallback.txt
echo "fallback test" > $TODO_FILE
tuxedo edit 1
# Should find and open vim/nvim/nano
rm $TODO_FILE
unset TODO_FILE
```

- [ ] **Step 4: Test no-changes flow**

```bash
export TODO_FILE=/tmp/test-nochange.txt
export EDITOR=true  # true exits 0 without changing file
echo "unchanged task" > $TODO_FILE
tuxedo edit 1
# Should print nothing or "No changes"
cat $TODO_FILE  # Should still say "unchanged task"
rm $TODO_FILE
unset EDITOR
unset TODO_FILE
```

- [ ] **Step 5: Test error when no editor exists**

```bash
unset EDITOR
# In an env with no vim/nano (like CI):
tuxedo edit 1 2>&1
# Expected: "tuxedo edit: no editor found" on stderr
```

---

### Task 9: Run full test suite and lint

- [ ] **Step 1: Run all tests**

```bash
cargo test 2>&1
```
Expected: all tests pass (lib, integration, snapshots)

- [ ] **Step 2: Run clippy**

```bash
cargo clippy -- -D warnings 2>&1
```
Expected: no warnings

- [ ] **Step 3: Final commit (if any fixes needed)**

```bash
git add -A && git diff --cached --stat
# Commit if there are changes from lints/test fixes
```

---

## Verification Checklist

- [ ] `cargo build` succeeds
- [ ] `cargo test` all pass
- [ ] `cargo clippy` clean
- [ ] `tuxedo edit 1` opens $EDITOR with task 1's raw text
- [ ] `tuxedo edit 1` with no changes exits cleanly
- [ ] `tuxedo edit 1` with empty result shows error
- [ ] TUI `E` key suspends terminal, launches editor, restores
- [ ] TUI `e` and `i` inline editing remain unchanged
- [ ] TUI `E` in archive view flashes "read-only in archive"
- [ ] `$EDITOR` unset falls back to vim/nvim/nano/emacs/helix
- [ ] `$EDITOR` set to nonexistent editor shows error
