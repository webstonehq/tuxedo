//! Terminal QR code rendering for `tuxedo serve`. Uses the `qrcode`
//! crate's `unicode::Dense1x2` renderer, which encodes each pair of
//! rows into a single line of half-block glyphs — readable in any
//! modern terminal without an image library.

use qrcode::QrCode;
use qrcode::render::unicode::Dense1x2;

/// Render the given URL as a terminal-friendly QR code. Returns the
/// multi-line string ready to print. Errors propagate so the caller can
/// decide whether to flash the URL alone or abort.
pub fn render(url: &str) -> Result<String, qrcode::types::QrError> {
    let code = QrCode::new(url.as_bytes())?;
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_url_to_block_glyphs() {
        let out = render("http://192.168.1.42:8080/t/abc/").unwrap();
        // Dense1x2 uses half-block glyphs (▀ ▄ █ space).
        assert!(out.contains('\n'));
        assert!(
            out.chars().any(|c| matches!(c, '▀' | '▄' | '█' | ' ')),
            "expected block glyphs in QR rendering",
        );
    }
}
