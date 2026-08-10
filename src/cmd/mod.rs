//! One-shot CLI commands (a todo.txt-cli-style surface), driving the headless
//! [`Store`](crate::core::Store). Invoked by `main` when the first argument is a
//! recognized subcommand; otherwise the binary launches the TUI.

mod json;

use anyhow::{Context, Result};

use crate::app::Filter;
use crate::core::filter as corefilter;
use crate::core::{
    AddOutcome, ArchiveOutcome, CompleteOutcome, DeleteOutcome, EditOutcome, PriorityOutcome, Store,
};
use crate::todo::Task;

/// Parsed global options shared by every subcommand.
struct Args {
    json: bool,
    /// `-f`/`--force`: skip confirmation prompts (matches todo.sh's `-f`).
    force: bool,
    free: Vec<String>,
}

fn parse_args(rest: &[String]) -> Result<Args, String> {
    let mut json = false;
    let mut force = false;
    let mut free = Vec::new();
    for a in rest {
        match a.as_str() {
            "--json" => json = true,
            "-f" | "--force" => force = true,
            s if s.starts_with('-') && s != "-" => {
                return Err(format!("unknown option: {s}"));
            }
            _ => free.push(a.clone()),
        }
    }
    Ok(Args { json, force, free })
}

/// Every recognized subcommand and alias.
const SUBCOMMANDS: &[&str] = &[
    "add", "a", "append", "app", "prepend", "prep", "replace", "edit", "e", "pri", "p", "depri",
    "dp", "done", "do", "complete", "del", "rm", "archive", "list", "ls", "listall", "lsa",
    "listpri", "lsp", "listproj", "lsprj", "listcon", "lsc",
];

/// Locate the subcommand: the first non-global token, if it is a known
/// subcommand. Returns its index in `argv`. Global flags (`--json`, `-f`/
/// `--force`) may precede it, todo.sh-style.
fn find_subcommand(argv: &[String]) -> Option<usize> {
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "--json" || a == "-f" || a == "--force" {
            i += 1;
        } else {
            return SUBCOMMANDS.contains(&a).then_some(i);
        }
    }
    None
}

/// Try to run a one-shot CLI command from the full argument list (everything
/// after the binary name). Returns `Ok(None)` when the args are not a CLI
/// invocation, so the caller falls through to the TUI. Otherwise returns the
/// process exit code (0 ok, 1 user error, 2 usage error).
pub fn run(argv: &[String]) -> Result<Option<i32>> {
    let Some(cmd_pos) = find_subcommand(argv) else {
        return Ok(None);
    };
    let cmd = argv[cmd_pos].clone();
    // Everything except the subcommand token is options + the command's args.
    let mut rest: Vec<String> = Vec::with_capacity(argv.len().saturating_sub(1));
    rest.extend_from_slice(&argv[..cmd_pos]);
    rest.extend_from_slice(&argv[cmd_pos + 1..]);

    let args = match parse_args(&rest) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("tuxedo: {e}");
            return Ok(Some(2));
        }
    };

    // todo.sh-style path resolution via $TODO_FILE / $TODO_DIR / $DONE_FILE.
    let path = crate::cli::resolve_path(None).context("resolving todo file")?;
    let done = crate::cli::done_path(&path);
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut store = Store::open_sync_with_done(path, done, body, today);

    let json = args.json;
    let force = args.force;
    let pos = &args.free;
    let code = match cmd.as_str() {
        "add" | "a" => cmd_add(&mut store, pos, json),
        "append" | "app" => cmd_text_op(&mut store, pos, json, TextOp::Append),
        "prepend" | "prep" => cmd_text_op(&mut store, pos, json, TextOp::Prepend),
        "replace" => cmd_text_op(&mut store, pos, json, TextOp::Replace),
        "pri" | "p" => cmd_pri(&mut store, pos, json),
        "depri" | "dp" => cmd_depri(&mut store, pos, json),
        "done" | "do" | "complete" => cmd_done(&mut store, pos, json),
        "del" | "rm" => cmd_del(&mut store, pos, json, force),
        "archive" => cmd_archive(&mut store, json),
        "list" | "ls" => cmd_list(&store, pos, json),
        "listall" | "lsa" => cmd_listall(&store, pos, json),
        "listpri" | "lsp" => cmd_listpri(&store, pos, json),
        "listproj" | "lsprj" => cmd_listtags(&store, json, TagKind::Project),
        "listcon" | "lsc" => cmd_listtags(&store, json, TagKind::Context),
        "edit" | "e" => cmd_edit(&mut store, pos, json),
        other => {
            eprintln!("tuxedo: unknown command: {other}");
            2
        }
    };
    Ok(Some(code))
}

// ----- helpers -----------------------------------------------------------

fn err(msg: impl std::fmt::Display) -> i32 {
    eprintln!("tuxedo: {msg}");
    1
}

fn usage(msg: impl std::fmt::Display) -> i32 {
    eprintln!("usage: tuxedo {msg}");
    2
}

/// Parse a 1-based task number against the live list, returning a 0-based index.
fn parse_index(s: &str, len: usize) -> Result<usize, String> {
    let n: usize = s.parse().map_err(|_| format!("not a task number: {s}"))?;
    if n == 0 || n > len {
        return Err(format!("no task {n}"));
    }
    Ok(n - 1)
}

/// The message prefix todo.sh derives from the file name: the basename up to
/// the first `.`, uppercased (e.g. `todo.txt` → `TODO`).
fn file_prefix(store: &Store) -> String {
    store
        .file_path()
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.split('.').next())
        .unwrap_or("TODO")
        .to_uppercase()
}

/// Emit a single-task JSON success object (`{"ok":true,"action":..,"task":..}`).
fn json_task(action: &str, n: usize, t: &Task) {
    let mut s = String::from("{\"ok\":true,\"action\":\"");
    s.push_str(action);
    s.push_str("\",\"task\":");
    json::task_object(n, t, &mut s);
    s.push('}');
    println!("{s}");
}

fn store_error(json: bool, action: &str, e: impl std::fmt::Display) -> i32 {
    if json {
        let mut s = String::from("{\"ok\":false,\"action\":\"");
        s.push_str(action);
        s.push_str("\",\"error\":");
        json::esc(&e.to_string(), &mut s);
        s.push('}');
        eprintln!("{s}");
    } else {
        eprintln!("tuxedo: {e}");
    }
    1
}

// ----- mutating commands -------------------------------------------------

fn cmd_add(store: &mut Store, pos: &[String], json: bool) -> i32 {
    let text = pos.join(" ");
    if text.trim().is_empty() {
        return usage("add TEXT...");
    }
    match store.add_line(&text) {
        AddOutcome::Added { abs } => {
            let n = abs + 1;
            if json {
                json_task("add", n, &store.tasks()[abs]);
            } else {
                println!("{n} {}", store.tasks()[abs].raw);
                println!("{}: {n} added.", file_prefix(store));
            }
            0
        }
        AddOutcome::Empty => usage("add TEXT..."),
        AddOutcome::Aborted(_) => err("file changed on disk; nothing added"),
        AddOutcome::Error(e) => store_error(json, "add", e),
    }
}

enum TextOp {
    Append,
    Prepend,
    Replace,
}

fn cmd_text_op(store: &mut Store, pos: &[String], json: bool, op: TextOp) -> i32 {
    let name = match op {
        TextOp::Append => "append",
        TextOp::Prepend => "prepend",
        TextOp::Replace => "replace",
    };
    if pos.len() < 2 {
        return usage(format!("{name} N TEXT..."));
    }
    let len = store.tasks().len();
    let abs = match parse_index(&pos[0], len) {
        Ok(i) => i,
        Err(e) => return err(e),
    };
    let text = pos[1..].join(" ");
    // Capture the pre-edit line so `replace` can echo it like todo.sh.
    let old_raw = store.tasks()[abs].raw.clone();
    let outcome = match op {
        TextOp::Append => store.append_at(abs, &text),
        TextOp::Prepend => store.prepend_at(abs, &text),
        TextOp::Replace => store.edit_line(abs, &text),
    };
    match outcome {
        EditOutcome::Saved { abs } => {
            let n = abs + 1;
            let t = &store.tasks()[abs];
            if json {
                json_task(name, n, t);
            } else if matches!(op, TextOp::Replace) {
                // todo.sh: old line, "Replaced task with:", new line.
                println!("{n} {old_raw}");
                println!("TODO: Replaced task with:");
                println!("{n} {}", t.raw);
            } else {
                // append/prepend: just the updated, renumbered line.
                println!("{n} {}", t.raw);
            }
            0
        }
        EditOutcome::Empty => usage(format!("{name} N TEXT...")),
        EditOutcome::OutOfRange => err(format!("no task {}", abs + 1)),
        EditOutcome::TermNotFound => err("term not found"),
        EditOutcome::Aborted(_) => err("file changed on disk; nothing changed"),
        EditOutcome::Error(e) => store_error(json, name, e),
    }
}

fn cmd_edit(store: &mut Store, pos: &[String], json: bool) -> i32 {
    if pos.is_empty() {
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
            0
        }
        Err(e) => {
            eprintln!("tuxedo edit: {e}");
            1
        }
    }
}

fn cmd_pri(store: &mut Store, pos: &[String], json: bool) -> i32 {
    if pos.len() != 2 {
        return usage("pri N PRIORITY");
    }
    let len = store.tasks().len();
    let abs = match parse_index(&pos[0], len) {
        Ok(i) => i,
        Err(e) => return err(e),
    };
    let p = pos[1].to_uppercase();
    let mut chars = p.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return usage("pri N PRIORITY (A-Z)");
    };
    if !c.is_ascii_uppercase() {
        return usage("pri N PRIORITY (A-Z)");
    }
    let prefix = file_prefix(store);
    let old = store.tasks()[abs].priority;
    match store.set_priority_at(abs, Some(c)) {
        PriorityOutcome::Changed { abs, priority } => {
            let n = abs + 1;
            let t = &store.tasks()[abs];
            if json {
                json_task("pri", n, t);
            } else {
                println!("{n} {}", t.raw);
                let new = priority.unwrap_or(c);
                match old {
                    Some(o) if o != new => {
                        println!("{prefix}: {n} re-prioritized from ({o}) to ({new}).")
                    }
                    _ => println!("{prefix}: {n} prioritized ({new})."),
                }
            }
            0
        }
        PriorityOutcome::OutOfRange => err(format!("no task {}", abs + 1)),
        PriorityOutcome::Aborted(_) => err("file changed on disk; nothing changed"),
        PriorityOutcome::Error(e) => store_error(json, "pri", e),
    }
}

fn cmd_depri(store: &mut Store, pos: &[String], json: bool) -> i32 {
    if pos.is_empty() {
        return usage("depri N...");
    }
    let prefix = file_prefix(store);
    let len = store.tasks().len();
    let mut indices = Vec::new();
    for s in pos {
        match parse_index(s, len) {
            Ok(i) => indices.push(i),
            Err(e) => return err(e),
        }
    }
    let mut code = 0;
    for abs in indices {
        match store.set_priority_at(abs, None) {
            PriorityOutcome::Changed { abs, .. } => {
                let n = abs + 1;
                let t = &store.tasks()[abs];
                if json {
                    json_task("depri", n, t);
                } else {
                    println!("{n} {}", t.raw);
                    println!("{prefix}: {n} deprioritized.");
                }
            }
            PriorityOutcome::OutOfRange => code = err(format!("no task {}", abs + 1)),
            PriorityOutcome::Aborted(_) => code = err("file changed on disk; nothing changed"),
            PriorityOutcome::Error(e) => code = store_error(json, "depri", e),
        }
    }
    code
}

fn cmd_done(store: &mut Store, pos: &[String], json: bool) -> i32 {
    if pos.is_empty() {
        return usage("done N...");
    }
    let len = store.tasks().len();
    let mut indices = Vec::new();
    for s in pos {
        match parse_index(s, len) {
            Ok(i) => indices.push(i),
            Err(e) => return err(e),
        }
    }
    // Process highest-first so a recurrence successor inserted after a row
    // doesn't shift the indices of rows we haven't completed yet.
    indices.sort_unstable();
    indices.dedup();
    indices.reverse();

    let prefix = file_prefix(store);
    // (number, completed task, optional (next number, spawned task))
    type Completed = (usize, Task, Option<(usize, Task)>);
    let mut completed: Vec<Completed> = Vec::new();
    let mut code = 0;
    for abs in indices {
        if store.tasks()[abs].done {
            code = err(format!("task {} already done", abs + 1));
            continue;
        }
        match store.toggle_complete(abs) {
            CompleteOutcome::Completed { abs } => {
                completed.push((abs + 1, store.tasks()[abs].clone(), None));
            }
            CompleteOutcome::CompletedSpawned { abs, next } => {
                completed.push((
                    abs + 1,
                    store.tasks()[abs].clone(),
                    Some((next + 1, store.tasks()[next].clone())),
                ));
            }
            CompleteOutcome::Uncompleted { .. } | CompleteOutcome::OutOfRange => {}
            CompleteOutcome::Aborted(_) => code = err("file changed on disk; nothing done"),
            CompleteOutcome::Error(e) => code = store_error(json, "done", e),
        }
    }
    completed.reverse(); // back to ascending for display
    if json {
        let refs: Vec<(usize, &Task)> = completed.iter().map(|(n, t, _)| (*n, t)).collect();
        println!(
            "{{\"ok\":true,\"action\":\"done\",\"tasks\":{}}}",
            json::task_array(&refs)
        );
    } else {
        for (n, t, next) in &completed {
            // todo.sh format for the completion itself.
            println!("{n} {}", t.raw);
            println!("{prefix}: {n} marked as done.");
            // Recurrence is a tuxedo feature todo.sh lacks; surface the spawned
            // next instance as a freshly-added task in the same idiom.
            if let Some((nn, nt)) = next {
                println!("{nn} {}", nt.raw);
                println!("{prefix}: {nn} added.");
            }
        }
    }
    code
}

/// Ask the user to confirm an action on stdin (todo.sh-style). Returns false on
/// anything but an affirmative answer, including EOF (piped/non-interactive).
fn confirm(prompt: &str) -> bool {
    use std::io::Write;
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(n) if n > 0 => matches!(line.trim(), "y" | "Y" | "yes" | "YES"),
        _ => false,
    }
}

fn cmd_del(store: &mut Store, pos: &[String], json: bool, force: bool) -> i32 {
    if pos.is_empty() {
        return usage("del N [TERM]");
    }
    let prefix = file_prefix(store);
    let len = store.tasks().len();
    let all_numeric = pos.iter().all(|s| s.parse::<usize>().is_ok());

    if !all_numeric {
        // `del N TERM` — remove a single whitespace term from one task. todo.sh
        // does not prompt for term removal, only whole-task deletion.
        let abs = match parse_index(&pos[0], len) {
            Ok(i) => i,
            Err(e) => return err(e),
        };
        let term = pos[1..].join(" ");
        let old_raw = store.tasks()[abs].raw.clone();
        return match store.remove_term_at(abs, &term) {
            EditOutcome::Saved { abs } => {
                let n = abs + 1;
                if json {
                    json_task("del", n, &store.tasks()[abs]);
                } else {
                    // todo.sh: old line, "Removed '..' from task.", new line.
                    println!("{n} {old_raw}");
                    println!("{prefix}: Removed '{term}' from task.");
                    println!("{n} {}", store.tasks()[abs].raw);
                }
                0
            }
            EditOutcome::TermNotFound => {
                err(format!("term '{term}' not found on task {}", abs + 1))
            }
            EditOutcome::OutOfRange => err(format!("no task {}", abs + 1)),
            EditOutcome::Empty => usage("del N TERM"),
            EditOutcome::Aborted(_) => err("file changed on disk; nothing changed"),
            EditOutcome::Error(e) => store_error(json, "del", e),
        };
    }

    // `del N...` — delete whole tasks. Highest-first so removals don't shift.
    let mut indices = Vec::new();
    for s in pos {
        match parse_index(s, len) {
            Ok(i) => indices.push(i),
            Err(e) => return err(e),
        }
    }
    indices.sort_unstable();
    indices.dedup();
    indices.reverse();
    let mut removed: Vec<(usize, Task)> = Vec::new();
    let mut code = 0;
    for abs in indices {
        let task = store.tasks()[abs].clone();
        // todo.sh prompts before each deletion unless `-f`/`--force`.
        if !force && !confirm(&format!("Delete '{}'? (y/n) ", task.raw)) {
            println!("TODO: No tasks were deleted.");
            continue;
        }
        match store.delete(abs) {
            DeleteOutcome::Deleted { .. } => removed.push((abs + 1, task)),
            DeleteOutcome::OutOfRange => code = err(format!("no task {}", abs + 1)),
            DeleteOutcome::Aborted(_) => code = err("file changed on disk; nothing deleted"),
            DeleteOutcome::Error(e) => code = store_error(json, "del", e),
        }
    }
    removed.reverse();
    if json {
        let refs: Vec<(usize, &Task)> = removed.iter().map(|(n, t)| (*n, t)).collect();
        println!(
            "{{\"ok\":true,\"action\":\"del\",\"tasks\":{}}}",
            json::task_array(&refs)
        );
    } else {
        for (n, t) in &removed {
            println!("{n} {}", t.raw);
            println!("{prefix}: {n} deleted.");
        }
    }
    code
}

fn cmd_archive(store: &mut Store, json: bool) -> i32 {
    let path = store.file_path().display().to_string();
    // Capture the completed lines before they move, so we can echo them like
    // todo.sh (which greps `^x ` from the file in verbose mode).
    let moved: Vec<String> = store
        .tasks()
        .iter()
        .filter(|t| t.done)
        .map(|t| t.raw.clone())
        .collect();
    match store.archive_completed() {
        ArchiveOutcome::Archived { count } => {
            if json {
                println!("{{\"ok\":true,\"action\":\"archive\",\"count\":{count}}}");
            } else {
                for line in &moved {
                    println!("{line}");
                }
                println!("TODO: {path} archived.");
            }
            0
        }
        ArchiveOutcome::Nothing => {
            if json {
                println!("{{\"ok\":true,\"action\":\"archive\",\"count\":0}}");
            } else {
                // todo.sh prints this unconditionally; nothing moved is just
                // an empty grep.
                println!("TODO: {path} archived.");
            }
            0
        }
        ArchiveOutcome::Aborted(_) => err("file changed on disk; nothing archived"),
        ArchiveOutcome::Error(e) => store_error(json, "archive", e),
    }
}

// ----- listing commands --------------------------------------------------

/// Build a [`Filter`] from `list` TERM tokens: `+proj`, `@ctx`, else text.
fn filter_from_terms(terms: &[String]) -> Filter {
    let mut f = Filter::default();
    let mut needle = String::new();
    for t in terms {
        if let Some(p) = t.strip_prefix('+').filter(|s| !s.is_empty()) {
            f.project = Some(p.to_string());
        } else if let Some(c) = t.strip_prefix('@').filter(|s| !s.is_empty()) {
            f.context = Some(c.to_string());
        } else {
            if !needle.is_empty() {
                needle.push(' ');
            }
            needle.push_str(t);
        }
    }
    f.search = needle;
    f
}

/// Indices into `tasks` matching `filter` (with an optional extra predicate),
/// ordered like todo.sh's default: a case-insensitive sort of the full task
/// line (`sort -f -k2 -b`), ties broken by file position.
fn matching_indices(tasks: &[Task], filter: &Filter, extra: impl Fn(&Task) -> bool) -> Vec<usize> {
    let needle = (!filter.search.is_empty()).then_some(filter.search.as_str());
    let mut idxs: Vec<usize> = (0..tasks.len())
        .filter(|&i| corefilter::passes_user_filter(&tasks[i], filter, needle) && extra(&tasks[i]))
        .collect();
    idxs.sort_by(|&a, &b| {
        tasks[a]
            .raw
            .to_lowercase()
            .cmp(&tasks[b].raw.to_lowercase())
            .then(a.cmp(&b))
    });
    idxs
}

/// Width to which todo.sh pads task numbers: the digit count of the file's
/// total task count.
fn num_width(total: usize) -> usize {
    total.max(1).to_string().len()
}

/// Print numbered task rows (todo.sh format: right-aligned number, single
/// space, raw line). `width` is shared so columns line up.
fn print_rows(tasks: &[Task], idxs: &[usize], width: usize) {
    for &i in idxs {
        println!("{:>width$} {}", i + 1, tasks[i].raw);
    }
}

fn cmd_list(store: &Store, pos: &[String], json: bool) -> i32 {
    let filter = filter_from_terms(pos);
    let tasks = store.tasks();
    let idxs = matching_indices(tasks, &filter, |_| true);
    if json {
        let refs: Vec<(usize, &Task)> = idxs.iter().map(|&i| (i + 1, &tasks[i])).collect();
        println!("{}", json::task_array(&refs));
    } else {
        print_rows(tasks, &idxs, num_width(tasks.len()));
        println!("--");
        println!(
            "{}: {} of {} tasks shown",
            file_prefix(store),
            idxs.len(),
            tasks.len()
        );
    }
    0
}

fn cmd_listall(store: &Store, pos: &[String], json: bool) -> i32 {
    let filter = filter_from_terms(pos);
    let tasks = store.tasks();
    let live = matching_indices(tasks, &filter, |_| true);
    let archived_tasks = store.archive().tasks();
    let done = matching_indices(archived_tasks, &filter, |_| true);
    if json {
        let live_refs: Vec<(usize, &Task)> = live.iter().map(|&i| (i + 1, &tasks[i])).collect();
        let done_refs: Vec<(usize, &Task)> =
            done.iter().map(|&i| (i + 1, &archived_tasks[i])).collect();
        println!(
            "{{\"ok\":true,\"action\":\"listall\",\"todo\":{},\"done\":{}}}",
            json::task_array(&live_refs),
            json::task_array(&done_refs)
        );
    } else {
        let width = num_width(tasks.len().max(archived_tasks.len()));
        print_rows(tasks, &live, width);
        print_rows(archived_tasks, &done, width);
        println!("--");
        println!(
            "{}: {} of {} tasks shown",
            file_prefix(store),
            live.len(),
            tasks.len()
        );
        println!(
            "DONE: {} of {} tasks shown",
            done.len(),
            archived_tasks.len()
        );
        println!(
            "total: {} of {} tasks shown",
            live.len() + done.len(),
            tasks.len() + archived_tasks.len()
        );
    }
    0
}

fn cmd_listpri(store: &Store, pos: &[String], json: bool) -> i32 {
    // Optional single PRIORITY filter (A-Z).
    let only: Option<char> = match pos.first() {
        None => None,
        Some(s) => {
            let up = s.to_uppercase();
            let mut chars = up.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_uppercase() => Some(c),
                _ => return usage("listpri [PRIORITY]"),
            }
        }
    };
    let tasks = store.tasks();
    let idxs = matching_indices(tasks, &Filter::default(), |t| match only {
        Some(c) => t.priority == Some(c),
        None => t.priority.is_some(),
    });
    if json {
        let refs: Vec<(usize, &Task)> = idxs.iter().map(|&i| (i + 1, &tasks[i])).collect();
        println!("{}", json::task_array(&refs));
    } else {
        print_rows(tasks, &idxs, num_width(tasks.len()));
        println!("--");
        println!(
            "{}: {} of {} tasks shown",
            file_prefix(store),
            idxs.len(),
            tasks.len()
        );
    }
    0
}

enum TagKind {
    Project,
    Context,
}

fn cmd_listtags(store: &Store, json: bool, kind: TagKind) -> i32 {
    // todo.sh `listproj`/`listcon` greps the whole todo.txt (completed lines
    // included) and `sort -u`s the results — alphabetical and unique. This
    // differs from the TUI sidebar, which orders by count and excludes done
    // tasks (`core::filter::unique_values`).
    use std::collections::BTreeSet;
    let (sigil, action) = match kind {
        TagKind::Project => ('+', "listproj"),
        TagKind::Context => ('@', "listcon"),
    };
    let mut set: BTreeSet<&str> = BTreeSet::new();
    for t in store.tasks() {
        let src = match kind {
            TagKind::Project => &t.projects,
            TagKind::Context => &t.contexts,
        };
        for v in src {
            set.insert(v.as_str());
        }
    }
    let names: Vec<String> = set.into_iter().map(|s| s.to_string()).collect();
    if json {
        println!(
            "{{\"ok\":true,\"action\":\"{action}\",\"tags\":{}}}",
            json::string_array(&names)
        );
    } else {
        for n in &names {
            println!("{sigil}{n}");
        }
    }
    0
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn find_subcommand_after_global_flags() {
        assert_eq!(find_subcommand(&argv(&["add", "buy"])), Some(0));
        assert_eq!(find_subcommand(&argv(&["--json", "ls"])), Some(1));
        // `-f`/`--force` take no value; the next token is the subcommand.
        assert_eq!(find_subcommand(&argv(&["-f", "del", "3"])), Some(1));
        assert_eq!(
            find_subcommand(&argv(&["--force", "--json", "done", "3"])),
            Some(2)
        );
    }

    #[test]
    fn find_subcommand_none_for_tui_invocations() {
        assert_eq!(find_subcommand(&argv(&[])), None);
        assert_eq!(find_subcommand(&argv(&["todo.txt"])), None);
        assert_eq!(find_subcommand(&argv(&["--help"])), None);
        assert_eq!(find_subcommand(&argv(&["--sample"])), None);
    }

    #[test]
    fn parse_args_extracts_globals_anywhere() {
        let a = parse_args(&argv(&["3", "--json", "-f"])).unwrap();
        assert!(a.json);
        assert!(a.force);
        assert_eq!(a.free, vec!["3".to_string()]);
    }

    #[test]
    fn parse_args_rejects_unknown_option() {
        assert!(parse_args(&argv(&["--bogus"])).is_err());
    }

    #[test]
    fn filter_from_terms_splits_sigils_and_text() {
        let f = filter_from_terms(&argv(&["+work", "@home", "pay", "rent"]));
        assert_eq!(f.project.as_deref(), Some("work"));
        assert_eq!(f.context.as_deref(), Some("home"));
        assert_eq!(f.search, "pay rent");
    }

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
}
