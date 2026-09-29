//! `e`: edit a resource in `$EDITOR` like `kubectl edit`. A rejected edit reopens
//! the editor with the reason as a comment header; saving unchanged cancels.

use std::io::stdout;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use super::{NoticeTone, Outcome};
use kube::{
    Client,
    api::{Api, DynamicObject, PostParams},
    core::GroupVersionKind,
    discovery::{Scope, pinned_kind},
};

/// Runs the edit flow for one manifest. Failures become an `Outcome` for the UI,
/// since the terminal must be restored either way.
pub fn edit_resource(terminal: &mut ratatui::DefaultTerminal, client: &Client, manifest: &serde_yaml::Value) -> Outcome {
    let original = match serde_yaml::to_string(manifest) {
        Ok(y) => y,
        Err(e) => return Outcome { text: format!("Can't render the manifest: {e}"), tone: NoticeTone::Failed },
    };
    let result = edit_loop(terminal, client, &original);
    match result {
        Ok(Some(text)) => Outcome { text, tone: NoticeTone::Done },
        Ok(None) => Outcome { text: "No changes".into(), tone: NoticeTone::Info },
        Err(e) => Outcome { text: format!("{e:#}"), tone: NoticeTone::Failed },
    }
}

fn edit_loop(terminal: &mut ratatui::DefaultTerminal, client: &Client, original: &str) -> Result<Option<String>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let file = TempFile(std::env::temp_dir().join(format!("knav-edit-{}-{nanos}.yaml", std::process::id())));
    let path = file.0.clone();
    let mut current = original.to_string();
    let mut header = String::new();
    let mut last_error = String::new();
    let outcome = loop {
        write_private(&path, &format!("{header}{current}")).context("writing the temp file")?;
        if !run_editor(terminal, &path)? {
            // The editor quit with a non-zero status (`:q!`/`:cq` in vi),
            // that's how you say "abort", so drop the edit quietly.
            break None;
        }
        let edited = strip_comment_header(&std::fs::read_to_string(&path).context("reading the temp file back")?);
        if edited.trim() == current.trim() {
            // Nothing changed, or a rejected edit was saved as-is: give up.
            break if current.trim() == original.trim() { None } else { bail!("{last_error}\n(cancelled)") };
        }
        current = edited;
        match apply(client, original, &current) {
            Ok(text) => break Some(text),
            Err(e) => {
                last_error = format!("{e:#}");
                header = format!("# Edit failed: {last_error}\n# Save unchanged to cancel.\n#\n");
            }
        }
    };
    Ok(outcome)
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

/// Drops the leading `#` lines a previous failure left (the editor may
/// or may not have kept them); real manifests start with `apiVersion`.
fn strip_comment_header(text: &str) -> String {
    text.lines().skip_while(|l| l.starts_with('#')).collect::<Vec<_>>().join("\n") + "\n"
}

/// Hands the terminal to `$VISUAL`/`$EDITOR` (else `vi`) through `sh -c`, so
/// editors with arguments work. `Ok(false)` means a non-zero exit.
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
    // 126/127 are the shell's "can't run it" / "not found", a real
    // failure to launch, unlike an editor deliberately exiting non-zero.
    if matches!(status.code(), Some(126 | 127)) {
        bail!("couldn't launch the editor '{editor}'");
    }
    Ok(status.success())
}

/// Replaces the object with the edited manifest. The kind, name and namespace must
/// match, since the API can't rename.
fn apply(client: &Client, original: &str, edited: &str) -> Result<String> {
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

    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let gvk = GroupVersionKind::try_from(&types)?;
            let (resource, caps) = pinned_kind(client, &gvk).await?;
            let api: Api<DynamicObject> = match (caps.scope, new.metadata.namespace.as_deref()) {
                (Scope::Namespaced, Some(ns)) => Api::namespaced_with(client.clone(), ns, &resource),
                _ => Api::all_with(client.clone(), &resource),
            };
            api.replace(&name, &PostParams::default(), &new).await?;
            Ok(format!("{}/{name} edited", types.kind.to_lowercase()))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_only_the_leading_comment_header() {
        let text = "# Edit failed: nope\n#\napiVersion: v1\n# keep me\nkind: ConfigMap\n";
        assert_eq!(strip_comment_header(text), "apiVersion: v1\n# keep me\nkind: ConfigMap\n");
    }
}
