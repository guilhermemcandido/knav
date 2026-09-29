
use super::{Action, Target};

/// What the confirmation dialog shows: what is about to happen, to what, and what follows.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfirmSpec {
    /// The question, e.g. `Delete pod?`.
    pub title: String,
    /// The word on the yes button.
    pub verb: String,
    /// Destructive: drawn in red, and Enter alone doesn't confirm.
    pub danger: bool,
    /// Kind and `namespace/name` of what it applies to, at most a handful.
    pub subjects: Vec<(String, String)>,
    /// What to know first, and whether it is a warning.
    pub notes: Vec<(String, bool)>,
}

fn plural(kind: &str) -> String {
    let kind = kind.to_lowercase();
    if kind.ends_with('s') { format!("{kind}es") } else if let Some(stem) = kind.strip_suffix('y') { format!("{stem}ies") } else { format!("{kind}s") }
}

/// The dialog for running `action` on `targets`, for the actions that ask first.
pub fn confirm_spec(action: Action, targets: &[Target]) -> Option<ConfirmSpec> {
    let first = targets.first()?;
    let (verb, question, danger) = match action {
        Action::Delete => ("Delete", "Delete", true),
        Action::Restart => ("Restart", "Restart", false),
        Action::Trigger => ("Run", "Run", false),
        Action::Suspend(true) => ("Suspend", "Suspend", false),
        Action::Suspend(false) => ("Resume", "Resume", false),
        _ => return None,
    };
    // Only some actions make sense on many at once.
    if targets.len() > 1 && !matches!(action, Action::Delete | Action::Restart) {
        return None;
    }
    let title = match (targets.len(), action) {
        (1, Action::Trigger) => format!("Run {} now?", first.kind),
        (1, _) => format!("{question} {}?", first.kind.to_lowercase()),
        (n, _) => format!("{question} {n} {}?", plural(&first.kind)),
    };
    let place = |t: &Target| match &t.namespace {
        Some(ns) => format!("{ns}/{}", t.name),
        None => t.name.clone(),
    };
    let mut subjects: Vec<(String, String)> = targets.iter().take(6).map(|t| (t.kind.clone(), place(t))).collect();
    if targets.len() > 6 {
        subjects.push((String::new(), format!("+{} more", targets.len() - 6)));
    }
    let mut notes: Vec<(String, bool)> = Vec::new();
    match action {
        Action::Delete => {
            if targets.iter().any(|t| t.kind == "Namespace") {
                notes.push(("Everything in the namespace is removed with it.".into(), true));
            }
            if let [one] = targets
                && one.kind == "Pod"
            {
                match one.manifest.get("metadata").and_then(|m| m.get("ownerReferences")).and_then(|o| o.as_sequence()).and_then(|o| o.first()) {
                    Some(owner) => notes.push((format!("{} {} controls it and will normally start a replacement.", owner.get("kind").and_then(|k| k.as_str()).unwrap_or("Its owner"), owner.get("name").and_then(|n| n.as_str()).unwrap_or("")), false)),
                    None => notes.push(("Nothing controls this pod, so nothing will bring it back.".into(), true)),
                }
            }
            notes.push(("This can't be undone.".into(), true));
        }
        Action::Restart => notes.push(("Pods are replaced one at a time, a rolling restart.".into(), false)),
        Action::Trigger => notes.push(("Creates a Job right now from the CronJob's template.".into(), false)),
        Action::Suspend(true) => notes.push(("No new Jobs are created until you resume it.".into(), false)),
        Action::Suspend(false) => notes.push(("Jobs are created on schedule again.".into(), false)),
        _ => {}
    }
    Some(ConfirmSpec { title, verb: verb.to_string(), danger, subjects, notes })
}

