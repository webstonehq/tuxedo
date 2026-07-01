use std::path::{Path, PathBuf};

use crate::todo::{self, Task};

pub const DEFAULT_NOTES_SUBDIR: &str = "projects/tuxedo-tasks";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteTarget {
    pub rel: String,
    pub path: PathBuf,
    pub existed_in_task: bool,
}

pub fn notes_dir_from_config(configured: Option<&str>) -> PathBuf {
    if let Some(value) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        return expand_note_dir(value);
    }
    if let Some(value) = std::env::var_os("NOTES_DIR") {
        return PathBuf::from(value);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("notes");
    }
    PathBuf::from("notes")
}

pub fn target_for_task(task: &Task, notes_dir: &Path) -> NoteTarget {
    if let Some(rel) = note_rel_from_raw(&task.raw) {
        let path = path_for_rel(notes_dir, &rel);
        return NoteTarget {
            rel,
            path,
            existed_in_task: true,
        };
    }

    let title = todo::body_only(&task.raw);
    let slug = slugify(if title.is_empty() { "task" } else { &title });
    let rel = format!("{DEFAULT_NOTES_SUBDIR}/{slug}.md");
    let path = notes_dir.join(&rel);
    NoteTarget {
        rel,
        path,
        existed_in_task: false,
    }
}

pub fn note_rel_from_raw(raw: &str) -> Option<String> {
    raw.split_whitespace()
        .find_map(|token| token.strip_prefix("note:"))
        .map(|s| s.trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

pub fn note_template(task: &Task) -> String {
    let title = todo::body_only(&task.raw);
    let title = if title.is_empty() { "Task" } else { &title };
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(title);
    out.push_str("\n\n");
    out.push_str("## Metadata\n\n");
    if let Some(priority) = task.priority {
        out.push_str(&format!("- Priority: {priority}\n"));
    }
    if let Some(created) = &task.created_date {
        out.push_str(&format!("- Created: {created}\n"));
    }
    if let Some(due) = &task.due {
        out.push_str(&format!("- Due: {due}\n"));
    }
    if !task.projects.is_empty() {
        out.push_str("- Projects: ");
        out.push_str(
            &task
                .projects
                .iter()
                .map(|p| format!("+{p}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        out.push('\n');
    }
    if !task.contexts.is_empty() {
        out.push_str("- Contexts: ");
        out.push_str(
            &task
                .contexts
                .iter()
                .map(|c| format!("@{c}"))
                .collect::<Vec<_>>()
                .join(" "),
        );
        out.push('\n');
    }
    for key in ["clickup", "clickup_status"] {
        if let Some(value) = kv_from_raw(&task.raw, key) {
            let label = match key {
                "clickup" => "ClickUp",
                "clickup_status" => "ClickUp status",
                _ => key,
            };
            out.push_str(&format!("- {label}: {value}\n"));
        }
    }
    if let Some(url) = task
        .raw
        .split_whitespace()
        .find(|token| token.starts_with("http://") || token.starts_with("https://"))
    {
        out.push_str(&format!("- URL: {url}\n"));
    }
    out.push_str("\n## Task\n\n```todo.txt\n");
    out.push_str(&task.raw);
    out.push_str("\n```\n\n## My notes\n\n");
    out
}

fn expand_note_dir(value: &str) -> PathBuf {
    if value == "~"
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home);
    }
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(value)
}

fn path_for_rel(notes_dir: &Path, rel: &str) -> PathBuf {
    let path = Path::new(rel);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        notes_dir.join(path)
    }
}

fn kv_from_raw(raw: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    raw.split_whitespace()
        .find_map(|token| token.strip_prefix(&prefix))
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in s.chars() {
        let folded = fold_char(ch);
        for folded_ch in folded.chars() {
            if folded_ch.is_ascii_alphanumeric() {
                out.push(folded_ch.to_ascii_lowercase());
                last_dash = false;
            } else if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "task".to_string()
    } else {
        trimmed
    }
}

fn fold_char(c: char) -> String {
    match c {
        'á' | 'à' | 'ã' | 'â' | 'ä' | 'Á' | 'À' | 'Ã' | 'Â' | 'Ä' => "a".into(),
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => "e".into(),
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => "i".into(),
        'ó' | 'ò' | 'õ' | 'ô' | 'ö' | 'Ó' | 'Ò' | 'Õ' | 'Ô' | 'Ö' => "o".into(),
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => "u".into(),
        'ç' | 'Ç' => "c".into(),
        'ñ' | 'Ñ' => "n".into(),
        _ => c.to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::todo::parse_line;

    #[test]
    fn notes_dir_prefers_configured_value() {
        let dir = notes_dir_from_config(Some("/tmp/custom-notes"));
        assert_eq!(dir, PathBuf::from("/tmp/custom-notes"));
    }

    #[test]
    fn notes_dir_expands_configured_tilde() {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let dir = notes_dir_from_config(Some("~/notes-work"));
        assert_eq!(dir, PathBuf::from(home).join("notes-work"));
    }

    #[test]
    fn uses_existing_unquoted_note_token_relative_to_notes_dir() {
        let task = parse_line("Do thing +Proj @ctx note:projects/clickup-tasks/86abc.md").unwrap();
        let target = target_for_task(&task, Path::new("/home/me/notes"));

        assert_eq!(target.rel, "projects/clickup-tasks/86abc.md");
        assert_eq!(
            target.path,
            PathBuf::from("/home/me/notes/projects/clickup-tasks/86abc.md")
        );
        assert!(target.existed_in_task);
    }

    #[test]
    fn creates_default_slug_path_when_task_has_no_note_token() {
        let task = parse_line("(A) 2026-06-25 Reformular relatório diário/eventos para Slack +Event_Graphs @charlie due:2026-06-30").unwrap();
        let target = target_for_task(&task, Path::new("/home/me/notes"));

        assert_eq!(
            target.rel,
            "projects/tuxedo-tasks/reformular-relatorio-diario-eventos-para-slack.md"
        );
        assert_eq!(
            target.path,
            PathBuf::from(
                "/home/me/notes/projects/tuxedo-tasks/reformular-relatorio-diario-eventos-para-slack.md"
            )
        );
        assert!(!target.existed_in_task);
    }

    #[test]
    fn template_contains_title_metadata_and_preserved_notes_section() {
        let task = parse_line(
            "(B) Flow +EstudoViabilidade @charlie @clickup due:2026-06-23 clickup:86ahz8gcg",
        )
        .unwrap();
        let rendered = note_template(&task);

        assert!(rendered.starts_with("# Flow\n"));
        assert!(rendered.contains("- Priority: B\n"));
        assert!(rendered.contains("- Due: 2026-06-23\n"));
        assert!(rendered.contains("- Projects: +EstudoViabilidade\n"));
        assert!(rendered.contains("- Contexts: @charlie @clickup\n"));
        assert!(rendered.contains("- ClickUp: 86ahz8gcg\n"));
        assert!(rendered.contains("## My notes\n\n"));
    }
}
