use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::theme::Theme;
use crate::todo::Task;
use crate::ui::task_row::{due_label, due_token_style, is_url_token, url_token_style};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.panel));

    let task = app.cur_task();
    // Leave one cell each for left padding and the right margin.
    let wrap_w = usize::from(area.width).saturating_sub(2);
    let lines = build_lines(theme, task, app.today(), wrap_w);
    let para = Paragraph::new(lines).style(Style::default().bg(theme.panel).fg(theme.fg));
    frame.render_widget(para, area);
}

fn build_lines<'a>(
    theme: &Theme,
    task: Option<&'a Task>,
    today: &'a str,
    wrap_w: usize,
) -> Vec<Line<'a>> {
    let mut rows: Vec<Line> = Vec::new();
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " DETAIL",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let Some(t) = task else {
        rows.push(line_panel(
            theme,
            vec![Span::styled(" (no task)", Style::default().fg(theme.dim))],
        ));
        return rows;
    };

    let priority_value = if let Some(p) = t.priority {
        Span::styled(
            format!("({p})"),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" priority  ", Style::default().fg(theme.dim)),
            priority_value,
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" created   ", Style::default().fg(theme.dim)),
            Span::styled(
                t.created_date.as_deref().unwrap_or("—"),
                Style::default().fg(theme.fg),
            ),
        ],
    ));
    if let Some(due) = &t.due {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" due       ", Style::default().fg(theme.dim)),
                Span::styled(due.as_str(), Style::default().fg(theme.fg)),
                Span::raw("  "),
                Span::styled(due_label(due, today), Style::default().fg(theme.overdue)),
            ],
        ));
    }
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" projects  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.projects
                    .iter()
                    .map(|p| format!("+{p}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.project),
            ),
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" contexts  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.contexts
                    .iter()
                    .map(|c| format!("@{c}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.context),
            ),
        ],
    ));

    // Rendering notes line by line
    if !t.notes.is_empty() {
        rows.push(line_panel(
            theme,
            vec![Span::styled(" notes", Style::default().fg(theme.dim))],
        ));
        for note in &t.notes {
            let style = Style::default().fg(theme.fg);
            let chunks = wrap_words(note, wrap_w.saturating_sub(4), |_| style);
            for (i, mut spans) in chunks.into_iter().enumerate() {
                let prefix = if i == 0 { "   - " } else { "     " };
                spans.insert(0, Span::styled(prefix, style));
                rows.push(line_panel(theme, spans));
            }
        }
    }

    if t.done {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" done      ", Style::default().fg(theme.dim)),
                Span::styled(
                    t.done_date.as_deref().unwrap_or(""),
                    Style::default().fg(theme.done),
                ),
            ],
        ));
    }
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " RAW",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let mut state = RawWalk::default();
    let chunks = wrap_words(&t.raw, wrap_w, |word| {
        raw_token_style(word, t, today, theme, &mut state)
    });
    for mut spans in chunks {
        spans.insert(0, Span::raw(" "));
        rows.push(line_panel(theme, spans));
    }
    rows
}

#[derive(Default)]
struct RawWalk {
    done_marker_consumed: bool,
    priority_consumed: bool,
}

fn raw_token_style(
    token: &str,
    task: &Task,
    today: &str,
    theme: &Theme,
    state: &mut RawWalk,
) -> Style {
    if task.done && !state.done_marker_consumed {
        state.done_marker_consumed = true;
        if token == "x" {
            return Style::default().fg(theme.done);
        }
    }
    if !state.priority_consumed
        && let Some(p) = task.priority
        && token.len() == 3
        && token.as_bytes()[0] == b'('
        && token.as_bytes()[1] == p as u8
        && token.as_bytes()[2] == b')'
    {
        state.priority_consumed = true;
        return Style::default()
            .fg(theme.priority_color(p))
            .add_modifier(Modifier::BOLD);
    }
    if let Some(rest) = token.strip_prefix("due:") {
        return due_token_style(task.done, rest, today, theme);
    }
    if is_url_token(token) {
        return url_token_style(task.done, theme);
    }
    if token.len() > 1 && token.starts_with('+') {
        return Style::default().fg(theme.project);
    }
    if token.len() > 1 && token.starts_with('@') {
        return Style::default().fg(theme.context);
    }
    Style::default().fg(theme.fg)
}

fn line_panel<'a>(theme: &Theme, spans: Vec<Span<'a>>) -> Line<'a> {
    Line::from(spans).style(Style::default().bg(theme.panel))
}

/// Wrap whitespace-delimited tokens into styled spans, keeping the style of
/// each original token even when its text spans multiple lines.
fn wrap_words<'a>(
    text: &'a str,
    width: usize,
    mut word_style: impl FnMut(&str) -> Style,
) -> Vec<Vec<Span<'a>>> {
    let mut lines = Vec::new();
    if width == 0 {
        return lines;
    }

    let mut line = Vec::new();
    let mut used = 0;
    let mut push = |text: &'a str, cells, gap, style| {
        if !line.is_empty() && used + usize::from(gap) + cells > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        if gap && !line.is_empty() {
            line.push(Span::raw(" "));
            used += 1;
        }
        line.push(Span::styled(text, style));
        used += cells;
    };

    for word in text.split_whitespace() {
        let mut style = word_style(word);
        let link = style.add_modifier.contains(Modifier::UNDERLINED);
        let whole_word = word.is_ascii() || link;
        let word_width = whole_word.then(|| word.graphemes(true).map(UnicodeWidthStr::width).sum());
        if link && word_width.is_some_and(|cells| cells > width) {
            // The OSC 8 overlay uses each underlined run as a link target.
            style = style.remove_modifier(Modifier::UNDERLINED);
        }

        let mut gap = true;
        let mut push_part = |part: &'a str, cells| {
            if cells <= width {
                push(part, cells, gap, style);
                gap = false;
            } else {
                for grapheme in part.graphemes(true) {
                    push(grapheme, grapheme.width(), gap, style);
                    gap = false;
                }
            }
        };

        if let Some(cells) = word_width {
            push_part(word, cells);
        } else {
            for (part, cells) in unicode_word_parts(word) {
                push_part(part, cells);
            }
        }
    }

    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Use Unicode line-break opportunities that fall on grapheme boundaries.
/// Measure each grapheme in terminal cells, as ratatui does when painting.
fn unicode_word_parts(word: &str) -> impl Iterator<Item = (&str, usize)> {
    let mut breaks = linebreaks(word).peekable();
    let mut start = 0;
    let mut cells = 0;
    word.grapheme_indices(true)
        .filter_map(move |(index, grapheme)| {
            let end = index + grapheme.len();
            cells += grapheme.width();
            while breaks.next_if(|&(offset, _)| offset < end).is_some() {}
            breaks.next_if(|&(offset, _)| offset == end)?;
            let part = &word[start..end];
            start = end;
            Some((part, std::mem::take(&mut cells)))
        })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::MUTED;
    use crate::todo::parse_line;

    fn line_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn wrapped(text: &str, width: usize) -> Vec<String> {
        wrap_words(text, width, |_| Style::default())
            .iter()
            .map(|line| line_text(line))
            .collect()
    }

    #[test]
    fn wraps_mixed_cjk_and_latin_at_terminal_cell_width() {
        assert_eq!(
            wrapped("前文English後續，說明", 7),
            ["前文", "English", "後續，", "說明"]
        );
        assert_eq!(wrapped("中文日本語한국어", 6), ["中文日", "本語한", "국어"]);
        assert_eq!(wrapped("整理研究资料", 8), ["整理研究", "资料"]);
        assert_eq!(wrapped("整理研究资料", 4), ["整理", "研究", "资料"]);
        assert!(wrapped("整理研究资料", 0).is_empty());
    }

    #[test]
    fn preserves_latin_words_and_only_splits_overlong_words() {
        assert_eq!(
            wrapped("  read\tproject notes  ", 9),
            ["read", "project", "notes"]
        );
        assert_eq!(wrapped("read source-code", 12), ["read", "source-code"]);
        assert_eq!(wrapped("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
    }

    #[test]
    fn keeps_combining_marks_and_emoji_sequences_with_their_graphemes() {
        assert_eq!(
            wrapped("e\u{301}e\u{301}e\u{301}", 2),
            ["e\u{301}e\u{301}", "e\u{301}"]
        );
        assert_eq!(wrapped("中👩‍💻文", 2), ["中", "👩‍💻", "文"]);
    }

    #[test]
    fn raw_continuation_inherits_the_original_token_style() {
        let task = parse_line("(A) 中文测试 +研究计划 @工作 due:2026-12-01").unwrap();
        let lines = build_lines(&MUTED, Some(&task), "2026-11-19", 6);
        let raw = lines
            .iter()
            .skip_while(|line| line_text(&line.spans) != " RAW")
            .skip(2)
            .collect::<Vec<_>>();

        assert!(raw.iter().all(|line| line.width() <= 7));
        let styled_text = |color| {
            raw.iter()
                .flat_map(|line| &line.spans)
                .filter(|span| span.style.fg == Some(color))
                .map(|span| span.content.as_ref())
                .collect::<String>()
        };
        assert_eq!(styled_text(MUTED.project), "+研究计划");
        assert_eq!(styled_text(MUTED.context), "@工作");
        assert_eq!(
            raw.iter()
                .filter(|line| line.spans.iter().any(|s| s.style.fg == Some(MUTED.project)))
                .count(),
            2
        );
    }

    #[test]
    fn only_complete_urls_are_underlined() {
        let style = url_token_style(false, &MUTED);
        let url = "https://example.com";
        let intact = wrap_words(url, 20, |_| style);
        assert_eq!(intact[0][0].content, url);
        assert!(
            intact[0][0]
                .style
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );

        let split = wrap_words(url, 10, |_| style);
        assert_eq!(
            split.iter().map(|line| line_text(line)).collect::<String>(),
            url
        );
        assert!(split.iter().flatten().all(|span| {
            span.style.fg == Some(MUTED.accent)
                && !span.style.add_modifier.contains(Modifier::UNDERLINED)
        }));
    }
}
