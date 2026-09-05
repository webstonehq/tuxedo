//! End-to-end proof that undo/redo survives a restart, driven through the same
//! public API `main.rs` uses.
//!
//! This lives in its own test binary because it sets `XDG_STATE_HOME`, and a
//! process-global env var must not race other tests. Keep it to one test.

use std::path::PathBuf;

use tuxedo::app::App;
use tuxedo::config::Config;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tuxedo-persist-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp dir");
    d
}

fn open(todo: &PathBuf) -> App {
    let body = std::fs::read_to_string(todo).unwrap_or_default();
    let mut app = App::new(todo.clone(), body, "2026-05-06".into(), Config::default());
    app.enable_history_persistence();
    // No async archive loader has landed yet in a fresh App; pump until the
    // deferred history load resolves, exactly as the event loop does.
    for _ in 0..500 {
        let _ = app.poll_archive();
        if app.history_len() > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    app
}

#[test]
fn undo_survives_a_restart() {
    let d = dir("restart");
    let state = d.join("state");
    // SAFETY: single-threaded, and this is the only test in this binary.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state) };

    let todo = d.join("todo.txt");
    std::fs::write(&todo, "(A) buy milk\n(B) call dentist\n").expect("seed todo.txt");

    // --- session one: delete a task, then quit cleanly ---
    let mut app = open(&todo);
    app.cursor = 1;
    app.delete(1);
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(
        std::fs::read_to_string(d.join("trash.txt")).expect("trash.txt"),
        "(B) call dentist\n"
    );
    let hist_path = app.history_path().expect("history path").to_path_buf();
    assert!(app.flush_history(), "clean quit must write the history");
    assert!(
        hist_path.starts_with(&state),
        "history must live under XDG_STATE_HOME"
    );
    drop(app);

    // --- session two: the delete is still undoable ---
    let mut next = open(&todo);
    assert_eq!(
        next.tasks().len(),
        1,
        "reopened with the task still deleted"
    );
    next.undo();
    assert_eq!(next.tasks().len(), 2, "undo must survive the restart");
    assert_eq!(
        std::fs::read_to_string(&todo).expect("todo.txt"),
        "(A) buy milk\n(B) call dentist\n"
    );
    assert_eq!(
        std::fs::read_to_string(d.join("trash.txt")).expect("trash.txt"),
        "",
        "undo must take it back out of the trash, not duplicate it"
    );

    let _ = std::fs::remove_dir_all(&d);
}
