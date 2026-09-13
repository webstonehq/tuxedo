//! XDG Base Directory resolution shared between the `config`, `theme` and
//! history-state loaders.

use std::path::PathBuf;

/// Resolve the XDG base config directory. Per the XDG Base Directory Spec,
/// `XDG_CONFIG_HOME` MUST be an absolute path; relative values are ignored.
/// We warn once so users debugging path resolution can see why their relative
/// override didn't take effect.
pub fn config_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_CONFIG_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        eprintln!(
            "tuxedo: ignoring non-absolute XDG_CONFIG_HOME={:?} (per XDG spec)",
            p.display()
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config"))
}

/// Resolve the XDG base state directory, where the undo/redo history lives.
/// Same absolute-path rule as [`config_home`]: per the spec `XDG_STATE_HOME`
/// MUST be absolute, and a relative value is ignored with a warning.
pub fn state_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_STATE_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        eprintln!(
            "tuxedo: ignoring non-absolute XDG_STATE_HOME={:?} (per XDG spec)",
            p.display()
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local").join("state"))
}
