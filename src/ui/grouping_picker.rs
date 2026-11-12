use crate::app::{App, GroupBy};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Group tasks ")
        .border_style(Style::default().fg(theme.accent))
        .style(Style::default().bg(theme.panel).fg(theme.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    let lines: Vec<_> = GroupBy::ALL
        .iter()
        .enumerate()
        .map(|(i, choice)| {
            let focused = i == app.grouping_pick_idx;
            Line::from(vec![
                Span::raw(if focused { " ▶ " } else { "   " }),
                Span::raw(if *choice == app.prefs.group_by {
                    "[x] "
                } else {
                    "[ ] "
                }),
                Span::raw(choice.label()),
            ])
            .style(
                Style::default()
                    .bg(if focused { theme.cursor } else { theme.panel })
                    .fg(theme.fg)
                    .add_modifier(if focused {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )
        })
        .collect();
    let scroll = app
        .grouping_pick_idx
        .saturating_sub(usize::from(list.height).saturating_sub(1));
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), list);
    frame.render_widget(
        Paragraph::new(if footer.width >= 35 {
            "j/k move · Enter select · Esc cancel"
        } else if footer.width >= 27 {
            "↑/↓ · Enter OK · Esc cancel"
        } else {
            "↵ OK · Esc cancel"
        })
        .style(Style::default().fg(theme.dim)),
        footer,
    );
}
