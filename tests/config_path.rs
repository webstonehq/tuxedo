//! Exercise config destinations in a subprocess so XDG overrides cannot race
//! with other tests or reach the developer's configuration.

use std::path::Path;
use std::process::Command;

use tuxedo::app::{App, Density, Sort};
use tuxedo::config::Config;

#[test]
fn preference_save_destinations() {
    const CHILD_ROOT: &str = "TUXEDO_TEST_CONFIG_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        check_destinations(Path::new(&root));
        return;
    }

    let root = std::env::temp_dir().join(format!("tuxedo-config-path-{}", std::process::id()));
    std::fs::create_dir(&root).expect("create isolated test directory");
    let result = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", "preference_save_destinations", "--nocapture"])
        .env(CHILD_ROOT, &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("XDG_STATE_HOME", root.join("state"))
        .output()
        .expect("run isolated preference check");
    std::fs::remove_dir_all(&root).expect("remove isolated test directory");
    assert!(
        result.status.success(),
        "isolated check failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
}

fn check_destinations(root: &Path) {
    let fallback = Config::path().expect("isolated XDG config path");
    assert_eq!(fallback, root.join("config/tuxedo/config.toml"));
    let baseline = Config {
        notes_dir: Some("fallback-notes".into()),
        sort: Some(Sort::File),
        ..Config::default()
    };
    baseline.save_to(&fallback).expect("seed fallback config");
    let fallback_bytes = std::fs::read(&fallback).expect("read fallback config");

    let explicit = root.join("explicit/config.toml");
    let seed = Config {
        notes_dir: Some("explicit-notes".into()),
        filters: vec![("work".into(), "+work".into())],
        recurrence_builder: Some(false),
        ..Config::default()
    };
    seed.save_to(&explicit).expect("seed explicit config");
    let mut app = App::new(
        root.join("todo.txt"),
        String::new(),
        "2026-05-06".into(),
        Config::load_from(&explicit),
    );
    app.config_path = Some(explicit.clone());
    app.cycle_density();
    app.cycle_sort();
    app.cycle_theme();
    let saved = Config::load_from(&explicit);
    assert_eq!(saved.density, Some(app.prefs.density));
    assert_eq!(saved.sort, Some(app.prefs.sort));
    assert_eq!(saved.theme.as_deref(), Some(app.theme().name));
    assert_eq!(saved.notes_dir, seed.notes_dir);
    assert_eq!(saved.filters, seed.filters);
    assert_eq!(saved.recurrence_builder, seed.recurrence_builder);
    assert_eq!(
        std::fs::read(&fallback).expect("read fallback"),
        fallback_bytes
    );

    // An unusable explicit path must report failure without falling back.
    let blocked_parent = root.join("regular-file");
    std::fs::write(&blocked_parent, "not a directory").expect("create blocked parent");
    app.config_path = Some(blocked_parent.join("config.toml"));
    app.save_prefs();
    assert!(
        app.flash_active()
            .is_some_and(|msg| msg.starts_with("config save failed:"))
    );
    assert_eq!(
        std::fs::read(&fallback).expect("read fallback"),
        fallback_bytes
    );

    // Without an explicit path, the existing XDG default remains supported.
    app.config_path = None;
    app.prefs.density = Density::Cozy;
    app.save_prefs();
    let saved = Config::load_from(&fallback);
    assert_eq!(saved.density, Some(Density::Cozy));
    assert_eq!(saved.notes_dir, baseline.notes_dir);
}
