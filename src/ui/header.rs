use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::Filter;
use crate::theme::Theme;

/// Build the human-readable filter chip shown in the header. `+project` /
/// `@context` / `/search` precedence matches what the sidebar shows. Returns
/// `None` when no filter is active.
pub fn filter_label(filter: &Filter) -> Option<String> {
    if let Some(p) = &filter.project {
        Some(format!("+{p}"))
    } else if let Some(c) = &filter.context {
        Some(format!("@{c}"))
    } else if !filter.search.is_empty() {
        Some(format!("/{}", filter.search))
    } else {
        None
    }
}

/// Inputs for the top-of-screen header bar. Grouped into a struct so call
/// sites pass labelled fields instead of positional `&str` args (which were
/// trivially swappable — `title` and `file` have the same type).
pub struct HeaderProps<'a> {
    pub title: Option<&'a str>,
    // pub file: &'a str,
    pub count: usize,
    pub sort: &'a str,
    pub filter: Option<&'a str>,
}

pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, props: HeaderProps<'_>) {
    // Mini cell-bowtie at one-row scale: two triangles flanking a block
    // cursor knot. Same body+knot+body structure and palette as the larger
    // mark used in the empty state and help overlay.
    let mut spans: Vec<Span> = vec![
        Span::raw(" "),
        Span::styled("▶", Style::default().fg(theme.accent)),
        Span::styled("▮", Style::default().fg(theme.pri_a)),
        Span::styled("◀", Style::default().fg(theme.accent)),
        Span::raw(" "),
    ];
    if let Some(t) = props.title {
        spans.push(Span::styled(
            t.to_string(),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ));
        // spans.push(Span::styled("  •  ", Style::default().fg(theme.dim)),);
    }
    spans.extend([
        // Span::styled(props.file.to_string(), Style::default().fg(theme.dim)),
        Span::styled("  •  ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("{} tasks", props.count),
            Style::default().fg(theme.dim),
        ),
        Span::styled("  •  ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("sort:{}", props.sort),
            Style::default().fg(theme.accent),
        ),
    ]);
    if let Some(f) = props.filter {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            format!("filter:{}", f),
            Style::default().fg(theme.context),
        ));
    }
    let line = Line::from(spans).style(Style::default().bg(theme.panel));
    let para = Paragraph::new(line).style(Style::default().bg(theme.panel));
    frame.render_widget(para, area);
}
