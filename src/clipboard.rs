use std::io::{self, Write};

/// Build an OSC 52 escape sequence that asks the controlling terminal to
/// place `content` on the system clipboard. Most modern terminals (kitty,
/// alacritty, wezterm, iTerm2, foot, modern xterm) honor this directly;
/// tmux forwards it when `set-clipboard on` is configured. Older terminals
/// silently ignore the sequence.
pub fn format_osc52(content: &str) -> String {
    let encoded = base64_encode(content.as_bytes());
    format!("\x1b]52;c;{encoded}\x1b\\")
}

/// Write `content` to the system clipboard via OSC 52. Errors only surface
/// I/O problems on stdout; whether the terminal actually honored the
/// sequence is not observable from here.
pub fn copy(content: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(format_osc52(content).as_bytes())?;
    stdout.flush()
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((n >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_wraps_base64_payload_in_escape_sequence() {
        // "hi" → "aGk=" in base64. The full sequence is the OSC introducer
        // (ESC ]), the 52;c; selector for the system clipboard, the payload,
        // and the ST terminator (ESC \).
        assert_eq!(format_osc52("hi"), "\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn osc52_handles_empty_input() {
        assert_eq!(format_osc52(""), "\x1b]52;c;\x1b\\");
    }

    #[test]
    fn base64_known_vectors() {
        // RFC 4648 test vectors plus a UTF-8 case to confirm we encode bytes,
        // not chars.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode("café".as_bytes()), "Y2Fmw6k=");
    }
}
