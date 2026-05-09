use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::header;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.bg));

    let [header_area, _sp, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: Some("settings"),
            // title: None,
            // file: "settings",
            count: app.tasks().len(),
            sort: app.sort_label(),
            filter: None,
        },
    );

    let mut lines: Vec<Line> = Vec::new();
    let density = match app.prefs.density {
        crate::app::Density::Compact => "compact",
        crate::app::Density::Comfortable => "comfortable",
        crate::app::Density::Cozy => "cozy",
    };
    let on = |b: bool| if b { "on" } else { "off" };

    let config_path = crate::config::Config::path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(unavailable)".into());

    let items: &[(&str, Option<String>)] = &[
        ("FILES", None),
        ("  todo file", Some(app.file_path.display().to_string())),
        ("  config file", Some(config_path)),
        ("", Some("".into())),
        ("DISPLAY", None),
        ("  theme", Some(format!("{} ▾  (T to cycle)", theme.name))),
        ("  density", Some(format!("{} ▾  (D to cycle)", density))),
        (
            "  line numbers",
            Some(format!("{}  (L to toggle)", on(app.prefs.layout.line_num))),
        ),
        ("  status bar", Some(on(app.prefs.layout.status_bar).into())),
        (
            "  filter sidebar",
            Some(format!("{}  ([ to toggle)", on(app.prefs.layout.left))),
        ),
        (
            "  detail sidebar",
            Some(format!("{}  (] to toggle)", on(app.prefs.layout.right))),
        ),
        (
            "  show done in list",
            Some(format!("{}  (H to toggle)", on(app.prefs.show_done))),
        ),
        ("", Some("".into())),
        ("BEHAVIOR", None),
        (
            "  default sort",
            Some(format!("{} (s to cycle)", app.sort_label())),
        ),
        ("", Some("".into())),
        ("KEYBINDINGS", None),
        ("  ", Some("press ? for the full list".into())),
    ];

    for (k, v) in items {
        match v {
            None => {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        k.to_string(),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            Some(val) if k.is_empty() => {
                lines.push(Line::raw(" "));
                let _ = val;
            }
            Some(val) => {
                let mut padded = k.to_string();
                let len = padded.chars().count();
                if len < 30 {
                    padded.push_str(&" ".repeat(30 - len));
                }
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(padded, Style::default().fg(theme.fg)),
                    Span::styled(val.clone(), Style::default().fg(theme.dim)),
                ]));
            }
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}
