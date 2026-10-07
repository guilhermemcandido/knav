//! The key hints shown for each mode.

use super::*;

/// Whether a hint's key changes the cluster, so read-only mode leaves it out.
pub(crate) fn changes_cluster(hint: &(&str, &str)) -> bool {
    matches!(*hint, ("e", "edit") | ("D", "delete") | ("S", "shell" | "scale") | ("r", "restart") | ("c", "cordon") | ("t", "trigger") | ("u", "suspend") | ("X", "debug"))
}

/// The keys for acting on the selected object, for the kinds each applies to.
fn action_hints(kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    let mut hints = match kind {
        ResourceKind::Pods => vec![("l", "logs"), ("L", "workload logs"), ("p", "previous logs"), ("S", "shell"), ("X", "debug"), ("F", "forward")],
        ResourceKind::Deployments => vec![("S", "scale"), ("r", "restart"), ("v", "history"), ("F", "forward"), ("L", "pod logs")],
        ResourceKind::StatefulSets => vec![("S", "scale"), ("r", "restart"), ("v", "history"), ("L", "pod logs")],
        ResourceKind::Services => vec![("F", "forward")],
        ResourceKind::ReplicaSets => vec![("S", "scale"), ("L", "pod logs")],
        ResourceKind::DaemonSets => vec![("r", "restart"), ("v", "history"), ("L", "pod logs")],
        ResourceKind::Jobs => vec![("L", "pod logs")],
        ResourceKind::Nodes => vec![("c", "cordon")],
        ResourceKind::CronJobs => vec![("t", "trigger"), ("u", "suspend")],
        ResourceKind::Secrets => vec![("x", "decode")],
        _ => Vec::new(),
    };
    hints.push(("D", "delete"));
    hints
}

/// The focused screen's keys as (key, description) pairs. `Command` and `Search`
/// return nothing, since they use the hint bar themselves.
pub(crate) fn hints_for(mode: &Mode, current_kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    match mode {
        // The Overview has no hint bar, to keep it clean.
        Mode::List if current_kind == ResourceKind::Overview => Vec::new(),
        Mode::List if current_kind == ResourceKind::ApiResources => {
            vec![("↑↓/jk", "move"), ("enter", "open"), ("s", "sort"), ("/", "search"), ("q/esc", "back")]
        }
        Mode::List if current_kind == ResourceKind::PortForwards => {
            vec![("↑↓/jk", "move"), ("enter/o", "open in browser"), ("D", "stop"), ("s", "sort"), ("/", "search"), ("q/esc", "back")]
        }
        Mode::List if matches!(current_kind, ResourceKind::ExtensionDashboard(_)) => {
            vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("q/esc", "back")]
        }
        Mode::List => {
            let mut hints = match current_kind {
                ResourceKind::Pods => vec![("↑↓/jk", "move"), ("enter", "containers"), ("d", "spec")],
                ResourceKind::Deployments => vec![("↑↓/jk", "move"), ("enter", "replicasets"), ("d", "spec")],
                ResourceKind::Namespaces => vec![("↑↓/jk", "move"), ("enter", "pods"), ("d", "spec")],
                ResourceKind::CronJobs => vec![("↑↓/jk", "move"), ("enter", "jobs"), ("d", "spec")],
                ResourceKind::ReplicaSets
                | ResourceKind::StatefulSets
                | ResourceKind::DaemonSets
                | ResourceKind::Jobs
                | ResourceKind::Services => vec![("↑↓/jk", "move"), ("enter", "pods"), ("d", "spec")],
                ResourceKind::Nodes => vec![("↑↓/jk", "move"), ("enter", "pods"), ("d", "spec")],
                ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => vec![("↑↓/jk", "move"), ("enter", "open")],
                _ => vec![("↑↓/jk", "move"), ("enter/d", "spec")],
            };
            if !matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) {
                hints.push(("e", "edit"));
                hints.push(("y", "yaml"));
                hints.push(("i", "info"));
                hints.push(("R", "related"));
                hints.push(("Y", "copy name"));
                hints.push(("O", "owner"));
                hints.extend(action_hints(current_kind));
            }
            hints.push(("space", "mark"));
            hints.push(("g/G", "top/bottom"));
            hints.push(("n", "namespaces"));
            hints.push(("0-9", "namespace"));
            hints.push(("s", "sort"));
            hints.push(("/", "search"));
            hints.push(("b/m", "sidebar"));
            hints.push(("C", "contexts"));
            hints.push(("E", "extensions"));
            hints.push(("T", "themes"));
            hints.push((",", "settings"));
            hints.push(("q/esc", "back"));
            hints
        }
        Mode::Command { .. } | Mode::Search | Mode::Notice { .. } | Mode::Working { .. } | Mode::Slots { .. } | Mode::Confirm { .. } | Mode::OpenUrl { .. } | Mode::Scale { .. } | Mode::Ports { .. } | Mode::EditReview { .. } | Mode::Permissions { .. } | Mode::History { .. } | Mode::Problems { .. } => Vec::new(),
        Mode::Shell { .. } => vec![("ctrl-]", "close the shell")],
        Mode::ThemePicker { .. } => vec![("↑↓/jk", "preview"), ("enter", "keep"), ("esc", "cancel")],
        Mode::Settings { editing: Some(_), .. } | Mode::Settings { capture: Some(_), .. } => Vec::new(),
        Mode::Settings { tab: ui::SettingsTab::Overview, .. } => vec![("↑↓/jk", "move"), ("1-9", "place"), ("J/K", "nudge"), ("space", "show/hide"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Settings { .. } => vec![("↑↓/jk", "move"), ("←→/enter", "change"), ("r", "reset"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Extensions { .. } => vec![("↑↓/jk", "move"), ("space/enter", "on/off"), ("/", "filter"), ("q/esc", "back")],
        Mode::Details { .. } => vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("enter", "open list"), ("y", "yaml"), ("q/esc", "back")],
        // U+2212 minus, not a hyphen, which some fonts fuse with `>` into an arrow.
        Mode::Relations { .. } => vec![("←↑↓→/hjkl", "move"), ("enter", "info"), ("o", "open list"), ("space", "follow"), ("backspace", "back"), ("+/\u{2212}", "zoom"), ("m", "copy as Mermaid"), ("q/esc", "close")],
        Mode::Yaml { .. } => vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("c", "copy"), ("q/esc", "back")],
        Mode::NamespacePick { editing: true, .. } => Vec::new(),
        Mode::NamespacePick { .. } => vec![("↑↓/jk", "move"), ("1-9", "assign key"), ("d", "clear key"), ("enter", "key list"), ("/", "filter"), ("q/esc", "back")],
        // No typing mode: typing filters, arrows, wheel or clicks move.
        Mode::Context { .. } => vec![("type", "filter"), ("↑↓", "move"), ("enter", "connect"), ("esc", "back")],
        Mode::Spec { .. } => {
            vec![("↑↓/jk", "move"), ("←→", "scroll"), ("enter", "toggle"), ("v", "value"), ("a", "expand all"), ("q/esc", "back")]
        }
        Mode::NodeDetail { editing: true, .. } => Vec::new(),
        Mode::NodeDetail { .. } => vec![("↑↓/jk", "move"), ("enter", "containers"), ("d", "spec"), ("e", "edit"), ("s", "sort"), ("/", "search"), ("q/esc", "back")],
        Mode::Events { editing: true, .. } => Vec::new(),
        Mode::Events { .. } => vec![("↑↓/jk", "move"), ("enter", "detail"), ("a/w/n", "filter"), ("/", "search"), ("q/esc", "back")],
        Mode::EventDetail { .. } => vec![("q/esc", "back")],
        Mode::ResourcesDetail => vec![("q/esc", "back")],
        Mode::ColumnDetail { .. } => vec![("←↑↓→/hjkl", "move"), ("enter", "open"), ("q/esc", "back")],
        Mode::Containers { .. } => vec![("↑↓/jk", "move"), ("enter/l", "logs"), ("p", "previous"), ("S", "shell"), ("X", "debug"), ("s", "sort"), ("q/esc", "back")],
        Mode::Logs { .. } => {
            vec![("↑↓/jk", "scroll"), ("G", "follow"), ("t", "timestamps"), ("o", "order"), ("/", "filter"), ("c", "copy"), ("w", "save"), ("q/esc", "back")]
        }
    }
}
