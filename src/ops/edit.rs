//! `e`: edit a resource in `$EDITOR` like `kubectl edit`. The app shows the changes
//! before anything is applied; a rejected edit reopens with the reason on top.

use std::io::stdout;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use kube::{
    Client,
    api::{Api, DynamicObject, PostParams},
    core::GroupVersionKind,
    discovery::{Scope, pinned_kind},
};

/// Opens `$EDITOR` on `text`, with `error` from a rejected try as a comment header.
/// `None` when the editor exits non-zero (`:cq` in vi), which means abort.
pub fn open_editor(terminal: &mut ratatui::DefaultTerminal, text: &str, error: Option<&str>) -> Result<Option<String>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let file = TempFile(std::env::temp_dir().join(format!("knav-edit-{}-{nanos}.yaml", std::process::id())));
    let header = error.map(|e| format!("# Edit failed: {}\n#\n", e.replace('\n', "\n# "))).unwrap_or_default();
    write_private(&file.0, &format!("{header}{text}")).context("writing the temp file")?;
    if !run_editor(terminal, &file.0)? {
        return Ok(None);
    }
    let edited = std::fs::read_to_string(&file.0).context("reading the temp file back")?;
    Ok(Some(strip_comment_header(&edited)))
}

/// One line of a diff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffKind {
    Removed,
    Added,
    Same,
    /// Unchanged lines left out between two changes.
    Gap,
}

/// What changed from `original` to `edited`, with three lines of context around it.
pub fn diff(original: &str, edited: &str) -> Vec<(DiffKind, String)> {
    let text = similar::TextDiff::from_lines(original, edited);
    let mut lines = Vec::new();
    for (i, group) in text.grouped_ops(3).iter().enumerate() {
        if i > 0 {
            lines.push((DiffKind::Gap, String::new()));
        }
        for change in group.iter().flat_map(|op| text.iter_changes(op)) {
            let kind = match change.tag() {
                similar::ChangeTag::Delete => DiffKind::Removed,
                similar::ChangeTag::Insert => DiffKind::Added,
                similar::ChangeTag::Equal => DiffKind::Same,
            };
            lines.push((kind, change.value().trim_end_matches('\n').to_string()));
        }
    }
    lines
}

/// The edit buffer on disk, removed however the edit ends.
struct TempFile(std::path::PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Writes a file only its owner can read: an edited Secret sits in it.
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

/// Drops the `#` lines a previous failure added; a manifest starts with `apiVersion`.
fn strip_comment_header(text: &str) -> String {
    text.lines().skip_while(|l| l.starts_with('#')).collect::<Vec<_>>().join("\n") + "\n"
}

/// Hands the terminal to `$VISUAL`, `$EDITOR` or `vi` through `sh -c`, so editors with
/// arguments work. `Ok(false)` means a non-zero exit.
fn run_editor(terminal: &mut ratatui::DefaultTerminal, path: &std::path::Path) -> Result<bool> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|e| !e.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    let status = Command::new("sh").arg("-c").arg(format!("{editor} '{}'", path.display())).status();
    *terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    let status = status.with_context(|| format!("couldn't launch the editor '{editor}'"))?;
    // 126 and 127 mean the shell couldn't run it, unlike an editor exiting non-zero on purpose.
    if matches!(status.code(), Some(126 | 127)) {
        bail!("couldn't launch the editor '{editor}'");
    }
    Ok(status.success())
}

/// Replaces the object with the edited manifest. The kind, name and namespace must
/// match, since the API can't rename.
pub async fn apply(client: Client, original: String, edited: String) -> Result<String> {
    let (original, edited) = (original.as_str(), edited.as_str());
    let old: DynamicObject = serde_yaml::from_str(original).context("original manifest")?;
    let new: DynamicObject = serde_yaml::from_str(edited).context("the edited YAML is not a valid manifest")?;
    let types = new.types.clone().context("apiVersion and kind are required")?;
    if Some(&types) != old.types.as_ref() {
        bail!("apiVersion/kind can't be changed");
    }
    let name = new.metadata.name.clone().context("metadata.name is required")?;
    if new.metadata.name != old.metadata.name || new.metadata.namespace != old.metadata.namespace {
        bail!("metadata.name and metadata.namespace can't be changed");
    }

    let gvk = GroupVersionKind::try_from(&types)?;
    let (resource, caps) = pinned_kind(&client, &gvk).await?;
    let api: Api<DynamicObject> = match (caps.scope, new.metadata.namespace.as_deref()) {
        (Scope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, &resource),
        _ => Api::all_with(client.clone(), &resource),
    };
    api.replace(&name, &PostParams::default(), &new).await?;
    Ok(format!("{}/{name} edited", types.kind.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_the_leading_comment_header() {
        let text = "# Edit failed: nope\n#\napiVersion: v1\n# keep me\nkind: ConfigMap\n";
        assert_eq!(strip_comment_header(text), "apiVersion: v1\n# keep me\nkind: ConfigMap\n");
    }

    #[test]
    fn a_diff_shows_the_change_with_context_and_skips_the_rest() {
        let original: String = (1..=20).map(|i| format!("line{i}\n")).collect();
        let edited = original.replace("line10\n", "line10 changed\n");
        let lines = diff(&original, &edited);
        assert!(lines.contains(&(DiffKind::Removed, "line10".into())));
        assert!(lines.contains(&(DiffKind::Added, "line10 changed".into())));
        assert_eq!(lines.iter().filter(|(k, _)| *k == DiffKind::Same).count(), 6, "three lines each side: {lines:?}");
    }

    #[test]
    fn two_far_apart_changes_are_split_by_a_gap() {
        let original: String = (1..=30).map(|i| format!("line{i}\n")).collect();
        let edited = original.replace("line2\n", "two\n").replace("line28\n", "twenty-eight\n");
        assert_eq!(diff(&original, &edited).iter().filter(|(k, _)| *k == DiffKind::Gap).count(), 1);
    }
}
