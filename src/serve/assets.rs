//! Embedded static assets served by `tuxedo serve`. Inlined via
//! `include_str!` so the binary stays self-contained — no runtime
//! filesystem lookup, no separate asset directory to ship.

pub const INDEX_HTML: &str = include_str!("assets/index.html");
pub const MANIFEST: &str = include_str!("assets/manifest.webmanifest");
pub const ICON_SVG: &str = include_str!("assets/icon.svg");
