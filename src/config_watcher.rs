use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(200);

/// Spawn a background filesystem watcher on the parent directory of
/// `config_path`. Returns a receiver that produces a `()` notification each
/// time `config.toml` has been modified, created, or renamed (atomic save).
/// Events are debounced so burst saves trigger only one reload signal.
///
/// When the config directory cannot be watched (e.g. missing parent, platform
/// limit) the function returns `None` and the caller should silently skip
/// hot-reload support rather than crashing or flashing an error.
pub fn spawn(config_path: PathBuf) -> Option<mpsc::Receiver<()>> {
    let target = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.to_string())?;
    let dir = config_path.parent()?.to_path_buf();

    let (tx, rx) = mpsc::channel();
    let (evt_tx, evt_rx) = mpsc::channel();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = evt_tx.send(event);
            }
        },
        notify::Config::default(),
    )
    .ok()?;

    watcher.watch(&dir, RecursiveMode::NonRecursive).ok()?;

    thread::spawn(move || {
        let _watcher = watcher;
        let mut pending: Option<Instant> = None;

        loop {
            match evt_rx.recv_timeout(DEBOUNCE) {
                Ok(event) => {
                    if is_relevant(&event, &target) {
                        pending = Some(Instant::now());
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if let Some(since) = pending
                && since.elapsed() >= DEBOUNCE
            {
                if tx.send(()).is_err() {
                    break;
                }
                pending = None;
            }
        }
    });

    Some(rx)
}

fn is_relevant(event: &Event, target: &str) -> bool {
    let matches_file = event
        .paths
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some(target));

    if !matches_file {
        return false;
    }

    matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_delivers_changes() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-config-watcher-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.toml");
        std::fs::write(&path, "theme = Muted Slate\n").expect("write config");
        let rx = spawn(path.clone()).expect("start watcher");
        std::fs::write(path, "theme = Terminal\n").expect("update config");
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok(()));
        drop(rx);
        let _ = std::fs::remove_dir_all(dir);
    }
}
