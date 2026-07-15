---
name: tuxedo-cli
description: Manage todo.txt tasks with the installed Tuxedo CLI. Use when the user names Tuxedo or Tuxedo is the established task backend.
---

# Tuxedo CLI

Manage tasks through a **resolve-mutate-verify** loop using Tuxedo's structured, one-shot CLI.

## Core operations

Use these stable operations directly:

- List or find tasks: `tuxedo list [TERM...] --json`
- Add a task: `tuxedo add TEXT... --json`
- Replace a task's text: `tuxedo replace N TEXT... --json`
- Append or prepend text: `tuxedo append N TEXT... --json` or `tuxedo prepend N TEXT... --json`
- Set or remove priority: `tuxedo pri N PRIORITY --json` or `tuxedo depri N --json`
- Complete tasks: `tuxedo done N... --json`
- Delete one task non-interactively: `tuxedo del N --force --json`
- Archive completed tasks: `tuxedo archive --json`

Task numbers are 1-based live file-line positions. Consult `tuxedo --help` for operations outside this core map and for syntax added by newer versions.

## Workflow

1. Run `tuxedo --version`. For an operation outside the core map, run `tuxedo --help` and continue only when it advertises the requested capability. Otherwise, explain the missing capability and offer an upgrade as a possible next step.
2. Resolve the target todo file before invoking a task command:
   - Prefer an existing `TODO_FILE`.
   - Otherwise, resolve to `$TODO_DIR/todo.txt` when `TODO_DIR` exists.
   - Otherwise, resolve to an existing `./todo.txt` only when the working directory clearly identifies it as the intended task store.
   - When none of these identifies one file, ask the user for the path and stop before invoking a task command. This keeps the automatic sample fallback untouched.
   - Set `TODO_FILE` explicitly on every subsequent command.
3. Read the relevant tasks with `tuxedo list [TERM...] --json`. Keep each candidate's returned task number and complete task data. This step is complete when the intended task set and current identities are known.
4. Immediately before any numbered mutation, take a fresh JSON snapshot and resolve the intended task from that snapshot.
   - Proceed only for one unambiguous match.
   - Ask the user to choose when multiple plausible tasks match.
   - Report no match instead of guessing.
5. Apply the mutation gate below. Once it passes, run exactly the requested mutation with `--json` and the advertised non-interactive flag when required.
6. Take a new JSON snapshot. Finish only when it proves the requested resulting state. For deletion, prove the exact task is absent. For recurring completion, account for both the completed task and any successor Tuxedo created.

Keep the resolved `TODO_FILE` attached to every read and mutation, for example:

```sh
TODO_FILE='/resolved/path/todo.txt' tuxedo list --json
```

Pass paths and user-provided task text as safely quoted shell arguments, without command-string concatenation.

## Mutation gate

- A clear request authorizes an ordinary addition, one unambiguous edit or completion, or a priority change.
- Bulk operations and archive require explicit approval immediately before mutation.
- An explicit request to delete one unambiguously identified task authorizes that deletion. Use the advertised force/non-interactive option with JSON output.
- Broad, inferred, or multi-task deletion requires explicit approval of the proposed scope immediately before mutation.
- Preserve the user's meaning when adding a task. Pass their wording to Tuxedo so it can canonicalize supported dates and recurrence. Do not invent priorities, projects, contexts, due dates, or recurrence rules.
- Treat edits that substantially reinterpret a task as ambiguous and ask first.

## Errors and concurrency

Report structured errors faithfully and leave the requested state unchanged rather than improvising another mutation.

Retry exactly once only when Tuxedo explicitly says the file changed on disk. Before retrying:

1. Read the target again with JSON output.
2. Resolve the intended task again from the new state.
3. Retry only if the match remains unambiguous.

Do not retry other errors automatically. If the one allowed retry also fails, stop and surface the error.
