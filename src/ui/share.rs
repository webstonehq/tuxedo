//! Share overlay: renders the QR code and URL for the in-TUI capture
//! server. Painted on top of the normal list/sidebars when the user
//! presses `s`. Dismissal is any key (see `handle_share` in main.rs).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::App;

/// Width of the inner content padding, on each side of the QR.
const INNER_PAD: u16 = 2;

/// Render the share overlay inside `area`. The caller is responsible
/// for clipping/centering — see [`size_for`] for the natural size.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let Some(share) = app.share_info() else {
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "tuxedo",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · capture ", Style::default().fg(theme.dim)),
        ]))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let qr_lines: Vec<Line> = share
        .qr
        .lines()
        .map(|s| Line::from(Span::raw(s.to_string())))
        .collect();
    let qr_height = u16::try_from(qr_lines.len()).unwrap_or(u16::MAX);

    // Vertical layout: QR, spacer, URL, spacer, hint. Constraints pick
    // a Min row so a too-small terminal still shows what it can.
    let [qr_area, _spacer, url_area, hint_area] = Layout::vertical([
        Constraint::Length(qr_height),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(qr_lines)
            .style(Style::default().bg(theme.panel).fg(theme.fg))
            .centered(),
        qr_area,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            share.url.clone(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel))
        .centered(),
        url_area,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "scan with phone · any key dismisses",
            Style::default().fg(theme.dim),
        )))
        .style(Style::default().bg(theme.panel))
        .centered(),
        hint_area,
    );
}

/// Natural (width, height) for the overlay given the rendered QR.
/// `render` will clip if the caller hands it less space; the centering
/// in `ui::mod` is expected to ask for at least this.
pub fn size_for(app: &App) -> (u16, u16) {
    let Some(share) = app.share_info() else {
        return (0, 0);
    };
    let qr_width = share
        .qr
        .lines()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0);
    let qr_height = u16::try_from(share.qr.lines().count()).unwrap_or(u16::MAX);
    let url_width = share.url.chars().count() as u16;
    let hint_width = "scan with phone · any key dismisses".chars().count() as u16;
    // 2 cols of border + INNER_PAD per side, computed widths are inner.
    let inner_w = qr_width.max(url_width).max(hint_width);
    let w = inner_w + 2 + INNER_PAD * 2;
    // Border (2) + qr + spacer (1) + url (1) + hint (1).
    let h = qr_height + 2 + 1 + 1 + 1;
    (w, h)
}
