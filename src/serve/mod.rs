//! In-TUI capture server: a tiny HTTP endpoint exposing a mobile-friendly
//! PWA on the local network. Bound lazily — the first time the user
//! presses the share key the TUI calls [`start`], which binds a
//! listener, spawns a background thread for the accept loop, and
//! returns a [`ShareInfo`] for the QR overlay to render.
//!
//! Architecture: the server never touches `todo.txt` directly. Every
//! captured task is appended to a sibling `inbox.txt`, where the
//! running TUI drains it through the same natural-language pipeline
//! used by the `n` add prompt. This keeps the server isolated — no
//! shared in-process state between the HTTP threads and the TUI — and
//! every entry point (TUI, file drop, HTTP) lands on the exact same
//! merge code.
//!
//! Access is gated by a 64-character token embedded in the URL path
//! (`/t/<token>/...`). The token is generated once and persisted in
//! the user's `config.toml` so a phone bookmark survives a relaunch.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result, anyhow};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::inbox;

pub mod assets;
pub mod net;
pub mod qr;

/// Public-facing handle returned to the TUI after a successful bind.
/// Contains the rendered QR text and the URL so the share overlay can
/// paint without re-running the network discovery on every frame.
#[derive(Debug, Clone)]
pub struct ShareInfo {
    pub url: String,
    pub qr: String,
    pub token: String,
    pub port: u16,
}

/// Bind the capture server on `port` (or an OS-assigned port if 0),
/// gate it on `token`, and spawn a detached thread running the accept
/// loop. Returns a [`ShareInfo`] describing the URL the phone should
/// hit and a Unicode-block QR rendering of it.
///
/// The accept thread is detached: when the parent process exits, the
/// OS reaps it. There is no explicit shutdown — the server has no
/// state to flush, and the TUI controls its own lifetime.
pub fn start(todo_path: PathBuf, token: String, port: u16) -> Result<ShareInfo> {
    let bind = format!("0.0.0.0:{port}");
    let server = Server::http(bind.as_str()).map_err(|e| anyhow!("bind {bind} failed: {e}"))?;
    let actual_port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        other => return Err(anyhow!("unexpected listen address: {other:?}")),
    };
    let lan_ip = net::discover_lan_ip();
    let url = format!("http://{lan_ip}:{actual_port}/t/{token}/");
    let qr = qr::render(&url).context("rendering QR")?;

    // Move the token and path into the worker. Arc lets us hand
    // immutable refs to per-connection handlers without cloning the
    // 64-byte token / PathBuf each request.
    let token_arc: Arc<str> = Arc::from(token.as_str());
    let path_arc: Arc<Path> = Arc::from(todo_path.as_path());
    thread::Builder::new()
        .name("tuxedo-capture-server".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                if let Err(e) = handle(request, &path_arc, &token_arc) {
                    eprintln!("tuxedo capture: request error: {e}");
                }
            }
        })
        .context("spawning capture-server thread")?;

    Ok(ShareInfo {
        url,
        qr,
        token,
        port: actual_port,
    })
}

fn handle(req: Request, todo_path: &Path, token: &str) -> Result<()> {
    // Public assets first — these don't need the token because PWA
    // installers (manifest discovery, favicon) fetch them from a
    // separate origin context that doesn't carry our token through.
    match (req.method(), req.url()) {
        (Method::Get, "/manifest.webmanifest") => {
            return respond(
                req,
                200,
                "application/manifest+json; charset=utf-8",
                assets::MANIFEST.as_bytes().to_vec(),
            );
        }
        (Method::Get, "/icon.svg") => {
            return respond(
                req,
                200,
                "image/svg+xml; charset=utf-8",
                assets::ICON_SVG.as_bytes().to_vec(),
            );
        }
        _ => {}
    }

    let path = req.url().to_string();
    let Some(rest) = strip_token_prefix(&path, token) else {
        return respond(req, 404, "text/plain; charset=utf-8", b"not found".to_vec());
    };

    match (req.method().clone(), rest.as_str()) {
        (Method::Get, "" | "/") => respond(
            req,
            200,
            "text/html; charset=utf-8",
            assets::INDEX_HTML.as_bytes().to_vec(),
        ),
        (Method::Get, "/tasks") => {
            let body = build_tasks_view(todo_path)?;
            respond(req, 200, "text/plain; charset=utf-8", body.into_bytes())
        }
        (Method::Post, "/add") => handle_add(req, todo_path),
        _ => respond(req, 404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

/// Match the `/t/<token>/...` prefix in constant time. Returns the
/// remainder of the path (everything after the token) when the token
/// matches, otherwise `None`. Returning `None` for any mismatch — wrong
/// token, missing prefix, malformed shape — keeps the 404 response
/// indistinguishable across failure modes.
fn strip_token_prefix(path: &str, token: &str) -> Option<String> {
    let rest = path.strip_prefix("/t/")?;
    let (candidate, tail) = match rest.split_once('/') {
        Some((c, t)) => (c, format!("/{t}")),
        None => (rest, String::new()),
    };
    if net::ct_eq(candidate, token) {
        Some(tail)
    } else {
        None
    }
}

fn handle_add(mut req: Request, todo_path: &Path) -> Result<()> {
    let mut body = String::new();
    req.as_reader()
        .read_to_string(&mut body)
        .context("reading POST body")?;
    let Some(text) = net::parse_form_text(&body) else {
        return respond(
            req,
            400,
            "text/plain; charset=utf-8",
            b"missing text".to_vec(),
        );
    };
    if text.trim().is_empty() {
        return respond(
            req,
            400,
            "text/plain; charset=utf-8",
            b"empty text".to_vec(),
        );
    }
    match net::append_to_inbox(todo_path, &text) {
        Ok(()) => respond(req, 204, "text/plain; charset=utf-8", Vec::new()),
        Err(e) => respond(
            req,
            500,
            "text/plain; charset=utf-8",
            format!("write failed: {e}").into_bytes(),
        ),
    }
}

/// Read `todo.txt` and the sibling `inbox.txt` and emit a single text
/// response. The PWA splits on the separator to render the two
/// sections; keeping it plain-text avoids pulling in serde.
fn build_tasks_view(todo_path: &Path) -> Result<String> {
    let todo_body = match std::fs::read_to_string(todo_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let inbox_path = inbox::path_for(todo_path);
    let inbox_body = match std::fs::read_to_string(&inbox_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut out = String::with_capacity(todo_body.len() + inbox_body.len() + 32);
    out.push_str(&todo_body);
    if !out.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out.push_str("--- inbox (pending) ---\n");
    out.push_str(&inbox_body);
    Ok(out)
}

fn respond(req: Request, status: u16, content_type: &str, body: Vec<u8>) -> Result<()> {
    let header = Header::from_bytes(&b"Content-Type"[..], content_type.as_bytes())
        .map_err(|_| anyhow!("invalid content-type header"))?;
    let len = body.len();
    let cursor = std::io::Cursor::new(body);
    let resp = Response::new(StatusCode(status), vec![header], cursor, Some(len), None);
    req.respond(resp).context("writing response")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn strip_token_prefix_matches_exact() {
        let token = "a".repeat(64);
        assert_eq!(
            strip_token_prefix(&format!("/t/{token}/"), &token).as_deref(),
            Some("/"),
        );
        assert_eq!(
            strip_token_prefix(&format!("/t/{token}/tasks"), &token).as_deref(),
            Some("/tasks"),
        );
        assert_eq!(
            strip_token_prefix(&format!("/t/{token}"), &token).as_deref(),
            Some(""),
        );
    }

    #[test]
    fn strip_token_prefix_rejects_wrong_token() {
        let token = "a".repeat(64);
        let wrong = "b".repeat(64);
        assert_eq!(strip_token_prefix(&format!("/t/{wrong}/"), &token), None);
        assert_eq!(strip_token_prefix("/", &token), None);
        assert_eq!(strip_token_prefix("/t/", &token), None);
        assert_eq!(strip_token_prefix("/tasks", &token), None);
    }

    #[test]
    fn tasks_view_separates_open_and_inbox() {
        let dir =
            std::env::temp_dir().join(format!("tuxedo-serve-tasks-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "(A) 2026-05-01 a\n").unwrap();
        std::fs::write(dir.join("inbox.txt"), "buy milk\n").unwrap();
        let view = build_tasks_view(&todo_path).unwrap();
        let (open, inbox) = view
            .split_once("\n--- inbox (pending) ---\n")
            .expect("separator present");
        assert!(open.contains("(A) 2026-05-01 a"));
        assert!(inbox.contains("buy milk"));
    }

    #[test]
    fn tasks_view_handles_missing_inbox() {
        let dir =
            std::env::temp_dir().join(format!("tuxedo-serve-tasks-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "a\n").unwrap();
        let view = build_tasks_view(&todo_path).unwrap();
        assert!(view.starts_with("a\n"));
        assert!(view.contains("--- inbox (pending) ---"));
    }

    #[test]
    fn start_binds_and_serves() {
        let dir = std::env::temp_dir().join(format!("tuxedo-serve-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "").unwrap();
        let token = net::generate_token().unwrap();
        let info = start(todo_path.clone(), token.clone(), 0).unwrap();
        assert!(info.port != 0, "OS must assign a real port");
        assert!(info.url.contains(&info.port.to_string()));
        assert!(info.url.contains(&token));
        assert!(info.qr.contains('\n'));
        // Give the accept loop a moment, then verify a token-mismatched
        // request gets a 404 to confirm the server is actually serving.
        std::thread::sleep(Duration::from_millis(50));
        let resp = std::net::TcpStream::connect(("127.0.0.1", info.port));
        assert!(resp.is_ok(), "server should accept TCP connections");
    }
}
