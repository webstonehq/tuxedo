//! Network helpers for `tuxedo serve`: LAN-IP discovery, token
//! generation, form-encoded body decoding, and the atomic append to
//! `inbox.txt` that the POST handler uses.

use std::fs::OpenOptions;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::Path;

use crate::inbox;

/// Discover the LAN IP the user's machine is reachable on, using the
/// classic "connect a UDP socket somewhere routable and read back the
/// chosen local address" trick. No packet is actually sent — UDP
/// `connect` only consults the route table. Falls back to localhost
/// when nothing is routable (e.g. no network at all).
pub fn discover_lan_ip() -> IpAddr {
    let try_via = |peer: &str| -> Option<IpAddr> {
        let s = UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect(peer).ok()?;
        s.local_addr().ok().map(|a| a.ip())
    };
    try_via("8.8.8.8:80")
        .or_else(|| try_via("1.1.1.1:80"))
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST))
}

/// Generate a 64-character lowercase hex token from 32 bytes of system
/// entropy. Used as a path-prefix gate on the protected serve routes.
pub fn generate_token() -> io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(format!("getrandom: {e}")))?;
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push(hex_nibble(b >> 4));
        out.push(hex_nibble(b & 0xf));
    }
    Ok(out)
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'a' + (n - 10)) as char,
        _ => '?',
    }
}

/// Constant-time string equality. Used to compare path tokens so a
/// timing attacker on the LAN can't recover the token byte-by-byte.
pub fn ct_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.bytes().zip(b.bytes()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// Parse the first `text=…` field out of an `application/x-www-form-urlencoded`
/// body. Returns the URL-decoded value with `+` mapped to space. Any
/// malformed `%XX` sequence is preserved verbatim — the canonicalizer
/// downstream will reject it if it produces an empty task.
pub fn parse_form_text(body: &str) -> Option<String> {
    for pair in body.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        if k == "text" {
            return Some(url_decode(v));
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = hex_value(bytes[i + 1]);
                let lo = hex_value(bytes[i + 2]);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h << 4) | l);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + b - b'a'),
        b'A'..=b'F' => Some(10 + b - b'A'),
        _ => None,
    }
}

/// Append one line to the sibling `inbox.txt`. Uses `O_APPEND` under
/// [`inbox::acquire_lock`] so concurrent producers and the TUI drain
/// serialize: the drain's rename can't strand a pending writer's data
/// on an unlinked inode, and two writers can't tear each other's
/// lines. The line is trimmed and a trailing newline is added
/// regardless of what the caller passed.
pub fn append_to_inbox(todo_path: &Path, line: &str) -> io::Result<()> {
    let inbox_path = inbox::path_for(todo_path);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty"));
    }
    let _lock = inbox::acquire_lock(todo_path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&inbox_path)?;
    // If a previous external producer left the file without a trailing
    // newline (our own writes always end with one), prepend a newline so
    // we don't concatenate this line onto the prior one. The lock makes
    // the size + last-byte read consistent with the upcoming write.
    let needs_leading_nl = match file.metadata() {
        Ok(m) if m.len() > 0 => {
            file.seek(SeekFrom::End(-1))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)?;
            last[0] != b'\n'
        }
        _ => false,
    };
    let mut buf = Vec::with_capacity(trimmed.len() + 2);
    if needs_leading_nl {
        buf.push(b'\n');
    }
    buf.extend_from_slice(trimmed.as_bytes());
    buf.push(b'\n');
    file.write_all(&buf)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_lower_hex_chars() {
        let t = generate_token().unwrap();
        assert_eq!(t.len(), 64);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn tokens_are_unique() {
        // Probability of collision in 32 bytes is negligible; this just
        // confirms we're not returning a hardcoded value.
        assert_ne!(generate_token().unwrap(), generate_token().unwrap());
    }

    #[test]
    fn ct_eq_matches_exact() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "x"));
        assert!(ct_eq("", ""));
    }

    #[test]
    fn parse_form_text_handles_plus_and_percent() {
        assert_eq!(parse_form_text("text=hello"), Some("hello".into()));
        assert_eq!(
            parse_form_text("text=hello+world"),
            Some("hello world".into())
        );
        assert_eq!(
            parse_form_text("text=buy%20milk%21"),
            Some("buy milk!".into()),
        );
        assert_eq!(
            parse_form_text("other=x&text=tomorrow"),
            Some("tomorrow".into()),
        );
        assert_eq!(parse_form_text("nothing=here"), None);
        // A leading bare segment (no `=`) used to short-circuit the whole
        // parse via the `?` operator. Now we skip past it and still find
        // the real field.
        assert_eq!(
            parse_form_text("&text=after-leading-amp"),
            Some("after-leading-amp".into()),
        );
        assert_eq!(
            parse_form_text("garbage&text=after-garbage"),
            Some("after-garbage".into()),
        );
    }

    #[test]
    fn append_creates_inbox_when_missing() {
        let dir = std::env::temp_dir().join(format!(
            "tuxedo-serve-append-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "").unwrap();
        append_to_inbox(&todo_path, "  buy milk  ").unwrap();
        let body = std::fs::read_to_string(dir.join("inbox.txt")).unwrap();
        assert_eq!(body, "buy milk\n");
        append_to_inbox(&todo_path, "call mom").unwrap();
        let body = std::fs::read_to_string(dir.join("inbox.txt")).unwrap();
        assert_eq!(body, "buy milk\ncall mom\n");
    }

    #[test]
    fn append_rejects_empty_line() {
        let dir =
            std::env::temp_dir().join(format!("tuxedo-serve-append-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "").unwrap();
        assert!(append_to_inbox(&todo_path, "   ").is_err());
    }

    #[test]
    fn append_inserts_leading_newline_when_prior_content_lacks_one() {
        // Some external producer may have written `inbox.txt` without a
        // trailing newline. Our append must not concatenate onto that
        // dangling line — both lines have to land separately so the
        // drain parses them as distinct tasks.
        let dir =
            std::env::temp_dir().join(format!("tuxedo-serve-append-noeol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let todo_path = dir.join("todo.txt");
        std::fs::write(&todo_path, "").unwrap();
        let inbox = dir.join("inbox.txt");
        std::fs::write(&inbox, "dangling").unwrap();
        append_to_inbox(&todo_path, "next").unwrap();
        assert_eq!(std::fs::read_to_string(&inbox).unwrap(), "dangling\nnext\n");
    }

    #[test]
    fn lan_ip_returns_some_address() {
        // We can't assert a specific IP, but discover_lan_ip should
        // never panic and should always return a valid IpAddr (falling
        // back to loopback when no routes exist).
        let _ip = discover_lan_ip();
    }
}
