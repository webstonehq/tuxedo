#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn fixture(script_body: &str) -> (PathBuf, PathBuf) {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tuxedo-cli-hook-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture directory");
    let todo = dir.join("todo.txt");
    std::fs::write(&todo, "").expect("seed todo file");
    let script = dir.join("hook");
    std::fs::write(&script, format!("#!/bin/sh\n{script_body}\n")).expect("write hook");
    let mut permissions = std::fs::metadata(&script)
        .expect("hook metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("make hook executable");
    let config = dir.join("config/tuxedo/config.toml");
    std::fs::create_dir_all(config.parent().expect("config parent")).expect("create config dir");
    std::fs::write(
        &config,
        format!("hook.after_mutation = \"{}\"\n", script.display()),
    )
    .expect("write config");
    (dir, todo)
}

fn run_add(dir: &Path, todo: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tuxedo"))
        .args(["--json", "add", "hooked task"])
        .env("TODO_FILE", todo)
        .env("DONE_FILE", dir.join("done.txt"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .output()
        .expect("run tuxedo")
}

#[test]
fn successful_hook_preserves_json_success() {
    let (dir, todo) = fixture("exit 0");
    let output = run_add(&dir, &todo);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"ok\":true"));
    assert!(output.stderr.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn failed_hook_preserves_json_and_returns_one() {
    let (dir, todo) =
        fixture("printf 'child output with hooked task'\nprintf 'child error' >&2\nexit 7");
    let output = run_add(&dir, &todo);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"ok\":true"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("post-commit hook create failed"));
    assert!(stderr.contains("exited with status 7"));
    assert!(!stderr.contains("hooked task"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("child output"));
    let _ = std::fs::remove_dir_all(&dir);
}
