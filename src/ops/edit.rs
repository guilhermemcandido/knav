//! `e` — edit a resource in `$EDITOR`, the way `kubectl edit` does:
//! dump its manifest to a temp file, hand the terminal over to the
//! editor, and on save replace the object on the cluster. A rejected
//! edit (bad YAML, admission error, a conflicting concurrent update)
//! reopens the editor with the reason as a comment header instead of
//! throwing the changes away; saving without touching it cancels.

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

/// What happened, for the notice shown once the terminal is back.
pub struct Outcome {
    pub text: String,
    pub error: bool,
}

/// Runs the whole edit flow for one manifest. Never returns an error —
/// every failure ends up as an `Outcome` for the UI to show, since by the
/// time something goes wrong the terminal has already been handed
/// around and must be restored regardless.
pub fn edit_resource(terminal: &mut ratatui::DefaultTerminal, client: &Client, manifest: &serde_yaml::Value) -> Outcome {
    let original = match serde_yaml::to_string(manifest) {
        Ok(y) => y,
        Err(e) => return Outcome { text: format!("can't render the manifest: {e}"), error: true },
    };
    let result = edit_loop(terminal, client, &original);
    match result {
        Ok(Some(text)) => Outcome { text, error: false },
        Ok(None) => Outcome { text: "No changes".into(), error: false },
        Err(e) => Outcome { text: format!("{e:#}"), error: true },
    }
}

fn edit_loop(terminal: &mut ratatui::DefaultTerminal, client: &Client, original: &str) -> Result<Option<String>> {
    let path = std::env::temp_dir().join(format!("knav-edit-{}.yaml", std::process::id()));
    let mut current = original.to_string();
    let mut header = String::new();
    let mut last_error = String::new();
    let outcome = loop {
        std::fs::write(&path, format!("{header}{current}")).context("writing the temp file")?;
        if !run_editor(terminal, &path)? {
            // The editor quit with a non-zero status (`:q!`/`:cq` in vi) —
            // that's how you say "abort", so drop the edit quietly.
            break None;
        }
        let edited = strip_comment_header(&std::fs::read_to_string(&path).context("reading the temp file back")?);
        if edited.trim() == current.trim() {
            // Untouched — either nothing was changed, or a rejected edit
            // was saved as-is again. Both mean "give up".
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
    let _ = std::fs::remove_file(&path);
    Ok(outcome)
}

/// Drops the leading `#` lines a previous failure left (the editor may
/// or may not have kept them); real manifests start with `apiVersion`.
fn strip_comment_header(text: &str) -> String {
    text.lines().skip_while(|l| l.starts_with('#')).collect::<Vec<_>>().join("\n") + "\n"
}

/// Hands the terminal to `$VISUAL`/`$EDITOR` (falling back to `vi`) and
/// takes it back afterwards. `Ok(false)` means it exited non-zero (an
/// abort). Run through `sh -c` so editors configured
/// with arguments (`code --wait`) work.
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
    // 126/127 are the shell's "can't run it" / "not found" — a real
    // failure to launch, unlike an editor deliberately exiting non-zero.
    if matches!(status.code(), Some(126 | 127)) {
        bail!("couldn't launch the editor '{editor}'");
    }
    Ok(status.success())
}

/// Replaces the object on the cluster with the edited manifest. The
/// identity (kind, name, namespace) has to match the original — the
/// API can't rename an object, and a changed name would silently edit
/// some other one.
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
