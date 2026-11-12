use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::Filter;
use crate::theme::Theme;

/// Build the human-readable filter chip shown in the header. Project,
/// context, and search combine as AND'd filters, so every active one must
/// appear in the label. `None` when no filter is active.
pub fn filter_label(filter: &Filter) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = &filter.project {
        parts.push(format!("+{p}"));
    }
    if let Some(c) = &filter.context {
        parts.push(format!("@{c}"));
    }
    if !filter.search.is_empty() {
        parts.push(format!("/{}", filter.search));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Inputs for the top-of-screen header bar. Grouped into a struct so call
/// sites pass labelled fields instead of positional `&str` args (which were
/// trivially swappable — `title` and `file` have the same type).
pub struct HeaderProps<'a> {
    pub title: Option<&'a str>,
    // pub file: &'a str,
    pub count: usize,
    pub hidden: usize,
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
            if props.hidden > 0 {
                format!("{} tasks ({} collapsed)", props.count, props.hidden)
            } else {
                format!("{} tasks", props.count)
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(project: Option<&str>, context: Option<&str>, search: &str) -> Filter {
        Filter {
            project: project.map(String::from),
            context: context.map(String::from),
            search: search.to_string(),
        }
    }

    #[test]
    fn no_active_filter_has_no_label() {
        assert_eq!(filter_label(&filter(None, None, "")), None);
    }

    #[test]
    fn single_project_shows_just_the_project() {
        assert_eq!(
            filter_label(&filter(Some("shop"), None, "")),
            Some("+shop".to_string())
        );
    }

    #[test]
    fn single_context_shows_just_the_context() {
        assert_eq!(
            filter_label(&filter(None, Some("home"), "")),
            Some("@home".to_string())
        );
    }

    #[test]
    fn project_and_context_together_show_both() {
        assert_eq!(
            filter_label(&filter(Some("shop"), Some("home"), "")),
            Some("+shop @home".to_string())
        );
    }

    #[test]
    fn project_context_and_search_all_show_together() {
        assert_eq!(
            filter_label(&filter(Some("shop"), Some("home"), "milk")),
            Some("+shop @home /milk".to_string())
        );
    }
}
