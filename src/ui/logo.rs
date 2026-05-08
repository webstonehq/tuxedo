use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

// 7×5 cell-grid bowtie ("Concept A" from the design handoff). Each design
// cell renders as two block characters so the bowtie keeps its intended
// aspect ratio under typical 2:1 terminal cell metrics. Body cells take the
// theme accent (the muted-slate / equivalent in each palette); the central
// 1×3 knot takes priority-A red, mirroring the live UI.
pub const WIDTH: u16 = 14;
pub const HEIGHT: u16 = 5;

const BLOCK: &str = "██";
const SPACE: &str = "  ";

// 7 columns × 5 rows. 'b' = body, 'k' = knot, ' ' = empty.
const GRID: [&[u8; 7]; 5] = [
    b"b     b",
    b"bb k bb",
    b"bbbkbbb",
    b"bb k bb",
    b"b     b",
];

#[cfg(test)]
pub(crate) fn ascii_rows() -> Vec<String> {
    GRID.iter()
        .map(|row| {
            row.iter()
                .map(|c| match c {
                    b'b' | b'k' => BLOCK,
                    _ => SPACE,
                })
                .collect()
        })
        .collect()
}

pub fn centered_lines(theme: &Theme, inner_width: u16) -> Vec<Line<'static>> {
    let pad_w = inner_width.saturating_sub(WIDTH) / 2;
    let pad = " ".repeat(pad_w as usize);
    let body = Style::default().fg(theme.accent);
    let knot = Style::default().fg(theme.pri_a);
    GRID.iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(row.len() + 1);
            spans.push(Span::raw(pad.clone()));
            for &c in row.iter() {
                spans.push(match c {
                    b'b' => Span::styled(BLOCK, body),
                    b'k' => Span::styled(BLOCK, knot),
                    _ => Span::raw(SPACE),
                });
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bowtie_shape() {
        let rows = ascii_rows();
        assert_eq!(
            rows,
            vec![
                "██          ██",
                "████  ██  ████",
                "██████████████",
                "████  ██  ████",
                "██          ██",
            ]
        );
        for r in &rows {
            assert_eq!(r.chars().count(), WIDTH as usize);
        }
        assert_eq!(rows.len(), HEIGHT as usize);
    }
}
