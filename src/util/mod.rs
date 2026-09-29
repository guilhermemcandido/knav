//! Small helpers shared by several modules, with no knav types of their own.

pub mod fuzzy;
pub mod text;

use std::path::PathBuf;

/// `$XDG_CONFIG_HOME/knav`, else `~/.config/knav`: config, themes and saved state.
pub fn config_dir() -> PathBuf {
    let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = set("XDG_CONFIG_HOME").or_else(|| set("HOME").map(|home| home.join(".config"))).unwrap_or_else(std::env::temp_dir);
    base.join("knav")
}
