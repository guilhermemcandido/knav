//! `w` in a log view: the lines shown, saved to a file.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// Writes `text` into `dir` (made if missing; `~` is the home folder), named after the
/// log's `title` and the time, and returns where it went.
pub fn save(dir: &str, title: &str, text: &str) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let dir = match dir.strip_prefix("~/") {
        Some(rest) => home.context("no home folder to save the log in")?.join(rest),
        None => PathBuf::from(dir),
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let path = dir.join(file_name(title, &k8s_openapi::jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string()));
    std::fs::write(&path, text).with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(path)
}

/// `shop/web-1/nginx` at a time becomes `shop-web-1-nginx-20261001-142000.log`.
fn file_name(title: &str, stamp: &str) -> String {
    let safe: String = title.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    let safe = safe.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    format!("{safe}-{stamp}.log")
}

/// The path with the home folder as `~`, for notices.
pub fn shown(path: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from).and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_safe_file_names() {
        assert_eq!(file_name("shop/web-1/nginx", "20261001-142000"), "shop-web-1-nginx-20261001-142000.log");
        assert_eq!(file_name("shop/web (3 pods)", "1"), "shop-web-3-pods-1.log");
    }
}
