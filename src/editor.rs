use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn create(content: &str) -> Result<Self> {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "tuxedo-edit-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&path, content)
            .with_context(|| format!("writing temp file {}", path.display()))?;
        Ok(Self { path })
    }

    fn read(&self) -> Result<String> {
        std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading temp file {}", self.path.display()))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn which(name: &str) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') {
        let path = PathBuf::from(name);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!("editor '{name}' not found")
    }
    #[cfg(unix)]
    {
        let path_env = std::env::var_os("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path_env) {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    #[cfg(not(unix))]
    {
        // PATH-based lookup not implemented for this platform
    }
    anyhow::bail!("editor '{name}' not found in PATH")
}

fn resolve_editor() -> Result<String> {
    if let Ok(editor) = std::env::var("EDITOR") {
        let editor = editor.trim();
        if !editor.is_empty() {
            return Ok(editor.to_string());
        }
    }
    let fallbacks = ["nvim", "vim", "vi", "nano", "emacs", "helix"];
    for name in fallbacks {
        if which(name).is_ok() {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!("no editor found (set $EDITOR or install vim/nano)")
}

pub fn edit_in_editor(content: &str) -> Result<Option<String>> {
    let tf = TempFile::create(content)?;
    let editor = resolve_editor()?;
    let status = Command::new(&editor)
        .arg(&tf.path)
        .status()
        .with_context(|| format!("spawning editor: {editor}"))?;
    if !status.success() {
        anyhow::bail!("editor exited with {}", status);
    }
    let new_content = tf.read()?;
    if new_content.trim() == content.trim() {
        return Ok(None);
    }
    Ok(Some(new_content))
}
