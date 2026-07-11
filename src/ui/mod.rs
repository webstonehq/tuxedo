use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear};

use crate::app::{App, Mode, View};

pub mod archive;
pub mod command_palette;
pub mod detail;
pub mod dialog;
pub mod empty;
pub mod filters;
pub mod header;
pub mod help;
pub mod hyperlinks;
pub mod list;
pub mod logo;
pub mod settings;
pub mod share;
pub mod status;
pub mod task_row;
pub mod theme_picker;
pub mod title;
pub mod welcome;

// Pane and overlay sizing. Promoted out of inline literals so the three
// `MIN_BODY_W` references below stay in sync, and so tweaking a sidebar
// width is a one-line change.
const LEFT_PANE_W: u16 = 26;
const RIGHT_PANE_W: u16 = 34;
const MIN_BODY_W: u16 = 40;

const DIALOG_H: u16 = 8;
const DIALOG_MIN_W: u16 = 40;
const DIALOG_MAX_W: u16 = 100;

const HELP_MIN_W: u16 = 76;
const HELP_MAX_W: u16 = 120;

const PROMPT_H: u16 = 5;
const PROMPT_MAX_W: u16 = 50;

const PALETTE_MAX_H: u16 = 20;
const PALETTE_MIN_W: u16 = 50;
const PALETTE_MAX_W: u16 = 80;

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = app.theme();
    let area = frame.area();

    // Paint full background.
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let bottom = if app.prefs.layout.status_bar { 1 } else { 0 };
    let [body_area, bottom_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(bottom)]).areas(area);

    // Determine pane widths. Sidebars apply to every view; navigation +
    // detail pane track the cursor regardless of which view is active.
    let show_left = app.prefs.layout.left;
    let show_right = app.prefs.layout.right;
    let left_w = if show_left { LEFT_PANE_W } else { 0 };
    let right_w = if show_right { RIGHT_PANE_W } else { 0 };

    let constraints = match (show_left, show_right) {
        (true, true) => vec![
            Constraint::Length(left_w),
            Constraint::Min(MIN_BODY_W),
            Constraint::Length(right_w),
        ],
        (true, false) => vec![Constraint::Length(left_w), Constraint::Min(MIN_BODY_W)],
        (false, true) => vec![Constraint::Min(MIN_BODY_W), Constraint::Length(right_w)],
        (false, false) => vec![Constraint::Min(1)],
    };
    let chunks = Layout::horizontal(constraints).split(body_area);

    let (left_area, center_area, right_area) = match (show_left, show_right) {
        (true, true) => (Some(chunks[0]), chunks[1], Some(chunks[2])),
        (true, false) => (Some(chunks[0]), chunks[1], None),
        (false, true) => (None, chunks[0], Some(chunks[1])),
        (false, false) => (None, chunks[0], None),
    };

    if let Some(la) = left_area {
        filters::render(frame, la, app);
    }
    match app.view() {
        View::List => list::render(frame, center_area, app),
        View::Archive => archive::render(frame, center_area, app),
    }
    if let Some(ra) = right_area {
        detail::render(frame, ra, app);
    }

    if app.prefs.layout.status_bar {
        if app.mode == Mode::Search {
            status::render_command_line(frame, bottom_area, app);
        } else {
            status::render(frame, bottom_area, app);
        }
    }

    // Overlays
    match app.mode {
        Mode::Insert => {
            let dlg_w: u16 = (u32::from(center_area.width) * 4 / 5)
                .clamp(u32::from(DIALOG_MIN_W), u32::from(DIALOG_MAX_W))
                as u16;
            let dlg = centered_in(area, dlg_w, DIALOG_H);
            frame.render_widget(Clear, dlg);
            dialog::render(frame, dlg, app);
            // At most one overlay shows at a time. The autocomplete popup is
            // suppressed while a metadata picker is open so we don't stack
            // two floating panels in the same spot.
            if !dialog::render_overlay(frame, dlg, area, app) {
                dialog::render_autocomplete(frame, dlg, area, app);
            }
        }
        Mode::Help => {
            // Height tracks the help content (see `help::required_height`)
            // so new keybinding rows can't clip the format section, with a
            // 2-row margin kept on short terminals.
            let h: u16 = area.height.saturating_sub(2).min(help::required_height());
            let w: u16 = (u32::from(area.width) * 9 / 10)
                .clamp(u32::from(HELP_MIN_W), u32::from(HELP_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            help::render(frame, r, app);
        }
        Mode::Settings => {
            frame.render_widget(Clear, body_area);
            settings::render(frame, body_area, app);
        }
        Mode::PromptProject | Mode::PromptContext | Mode::PromptSaveFilter => {
            let w: u16 = PROMPT_MAX_W.min(area.width.saturating_sub(4));
            let r = centered_in(area, w, PROMPT_H);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
            if matches!(app.mode, Mode::PromptProject | Mode::PromptContext) {
                dialog::render_autocomplete(frame, r, area, app);
            }
        }
        Mode::CommandPalette => {
            let h: u16 = area.height.saturating_sub(4).min(PALETTE_MAX_H);
            let w: u16 = (u32::from(area.width) * 3 / 5)
                .clamp(u32::from(PALETTE_MIN_W), u32::from(PALETTE_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            command_palette::render(frame, r, app);
        }
        Mode::Share => {
            let (w, h) = share::size_for(app);
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            share::render(frame, r, app);
        }
        Mode::PickTheme => {
            let h: u16 = area.height.saturating_sub(4).min(PALETTE_MAX_H);
            let w: u16 = (u32::from(area.width) * 3 / 5)
                .clamp(u32::from(PALETTE_MIN_W), u32::from(PALETTE_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            theme_picker::render(frame, r, app);
        }
        Mode::Welcome => {
            let r = centered_in(area, welcome::WIDTH, welcome::HEIGHT);
            frame.render_widget(Clear, r);
            welcome::render(frame, r, app);
        }
        _ => {}
    }
    // OSC 8 hyperlinks are applied post-draw by the caller (see
    // `hyperlinks::collect` + `emit_overlay`). Doing it inside the buffer
    // breaks ratatui's diff width calculation — keep cell symbols pristine.
}

pub(crate) fn centered_in(parent: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(parent.width);
    let h = h.min(parent.height);
    let x = parent.x + (parent.width - w) / 2;
    let y = parent.y + (parent.height - h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

pub(crate) fn fill_bg(frame: &mut Frame, area: Rect, style: Style) {
    frame.render_widget(Block::default().style(style), area);
}

pub(crate) fn density_blank_lines(d: crate::app::Density) -> usize {
    match d {
        crate::app::Density::Compact => 0,
        crate::app::Density::Comfortable => 1,
        crate::app::Density::Cozy => 2,
    }
}

/// Compute the new vertical scroll offset for a paragraph-backed list so the
/// cursor row stays inside the viewport. `prev` is the previous frame's offset,
/// `cursor_span` is the cursor row's `(first_line, line_count)` (or `None` if
/// there's no cursor row in the current build, e.g. when the list is empty) —
/// `line_count` is 1 today unless row wrapping is on. `height` is the viewport
/// height in rows; `total` is the total line count. A cursor row taller than
/// the viewport pins its first line to the top.
pub(crate) fn keep_cursor_visible(
    prev: u16,
    cursor_span: Option<(usize, usize)>,
    height: u16,
    total: usize,
) -> u16 {
    let h = usize::from(height);
    if h == 0 || total == 0 {
        return 0;
    }
    let max_offset = total.saturating_sub(h);
    let prev = usize::from(prev).min(max_offset);
    let new = match cursor_span {
        Some((start, len)) => {
            let end = start + len.max(1) - 1;
            let mut off = prev;
            if end >= off + h {
                off = end + 1 - h;
            }
            if start < off {
                off = start;
            }
            off
        }
        None => prev,
    };
    new.min(max_offset).min(usize::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::keep_cursor_visible;

    #[test]
    fn no_scroll_when_content_fits() {
        assert_eq!(keep_cursor_visible(0, Some((5, 1)), 10, 8), 0);
        assert_eq!(keep_cursor_visible(0, Some((7, 1)), 10, 8), 0);
    }

    #[test]
    fn scrolls_down_when_cursor_below_viewport() {
        // viewport rows 0..5, cursor at line 7 -> offset = 7 - 5 + 1 = 3
        assert_eq!(keep_cursor_visible(0, Some((7, 1)), 5, 20), 3);
    }

    #[test]
    fn scrolls_up_when_cursor_above_viewport() {
        // prev offset 10, cursor at line 3 -> offset = 3
        assert_eq!(keep_cursor_visible(10, Some((3, 1)), 5, 20), 3);
    }

    #[test]
    fn keeps_previous_offset_when_cursor_in_viewport() {
        // prev 5, cursor at line 7, height 5 -> 7 in [5, 10), stays 5
        assert_eq!(keep_cursor_visible(5, Some((7, 1)), 5, 20), 5);
    }

    #[test]
    fn clamps_to_max_offset_when_previous_exceeds_it() {
        // total shrank since last frame; previous offset 50 is now too large.
        assert_eq!(keep_cursor_visible(50, None, 5, 8), 3);
    }

    #[test]
    fn handles_degenerate_inputs() {
        assert_eq!(keep_cursor_visible(0, None, 0, 100), 0);
        assert_eq!(keep_cursor_visible(0, Some((0, 1)), 5, 0), 0);
    }

    #[test]
    fn scrolls_down_until_wrapped_cursor_row_fully_visible() {
        // Cursor row spans lines 7..=9 (3 wrapped lines), viewport height 5.
        // The whole span must land inside: offset = 9 + 1 - 5 = 5.
        assert_eq!(keep_cursor_visible(0, Some((7, 3)), 5, 20), 5);
    }

    #[test]
    fn keeps_offset_when_wrapped_cursor_row_already_visible() {
        // Span 5..=7 inside viewport [4, 9) -> unchanged.
        assert_eq!(keep_cursor_visible(4, Some((5, 3)), 5, 20), 4);
    }

    #[test]
    fn wrapped_row_taller_than_viewport_pins_first_line() {
        // Span 10..=17 can't fit in height 5; show its first line.
        assert_eq!(keep_cursor_visible(0, Some((10, 8)), 5, 40), 10);
    }
}
