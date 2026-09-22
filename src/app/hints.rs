//! The key hints shown for each mode.

use crate::*;

/// The keys available on the focused screen as (key, description) pairs, shown
/// in their own bar. `Command` and `Search` return nothing: they use that bar.
/// The keys for acting on the selected object, for the kinds each applies to.
fn action_hints(kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    let mut hints = match kind {
        ResourceKind::Pods => vec![("l", "logs"), ("L", "workload logs"), ("p", "previous logs"), ("S", "shell"), ("F", "forward")],
        ResourceKind::Deployments => vec![("S", "scale"), ("r", "restart"), ("F", "forward"), ("L", "pod logs")],
        ResourceKind::StatefulSets => vec![("S", "scale"), ("r", "restart"), ("L", "pod logs")],
        ResourceKind::Services => vec![("F", "forward")],
        ResourceKind::ReplicaSets => vec![("S", "scale"), ("L", "pod logs")],
        ResourceKind::DaemonSets => vec![("r", "restart"), ("L", "pod logs")],
        ResourceKind::Jobs => vec![("L", "pod logs")],
        ResourceKind::Nodes => vec![("c", "cordon")],
        ResourceKind::CronJobs => vec![("t", "trigger"), ("u", "suspend")],
        ResourceKind::Secrets => vec![("x", "decode")],
        _ => Vec::new(),
    };
    hints.push(("D", "delete"));
    hints
}

pub(crate) fn hints_for(mode: &Mode, current_kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    match mode {
        // Nothing on the main screen, deliberately kept clean. The
        // commands panel only exists once you've actually entered some
        // resource view.
        Mode::List if current_kind == ResourceKind::Overview => Vec::new(),
        Mode::List if current_kind == ResourceKind::ApiResources => {
            vec![("↑↓/jk", "move"), ("enter", "open"), ("s", "sort"), ("/", "search"), ("q/esc", "back")]
        }
        Mode::List if current_kind == ResourceKind::PortForwards => {
            vec![("↑↓/jk", "move"), ("enter/o", "open in browser"), ("D", "stop"), ("s", "sort"), ("/", "search"), ("q/esc", "back")]
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
            hints.push(("T", "themes"));
            hints.push((",", "settings"));
            hints.push(("q/esc", "back"));
            hints
        }
        Mode::Command { .. } | Mode::Search | Mode::Notice { .. } | Mode::Working { .. } | Mode::Slots { .. } | Mode::Confirm { .. } | Mode::OpenUrl { .. } | Mode::Scale { .. } | Mode::Ports { .. } => Vec::new(),
        Mode::Shell { .. } => vec![("ctrl-]", "close the shell")],
        Mode::ThemePicker { .. } => vec![("↑↓/jk", "preview"), ("enter", "keep"), ("esc", "cancel")],
        Mode::Settings { editing: Some(_), .. } | Mode::Settings { capture: Some(_), .. } => Vec::new(),
        Mode::Settings { tab: ui::SettingsTab::Overview, .. } => vec![("↑↓/jk", "move"), ("1-9", "place"), ("J/K", "nudge"), ("space", "show/hide"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Settings { .. } => vec![("↑↓/jk", "move"), ("←→/enter", "change"), ("r", "reset"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Details { .. } => vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("enter", "open list"), ("y", "yaml"), ("q/esc", "back")],
        // The minus sign (U+2212), not a hyphen: next to the closing `>` a hyphen
        // ligatures into an arrow in fonts like Fira Code.
        Mode::Relations { .. } => vec![("←↑↓→/hjkl", "move"), ("enter", "info"), ("o", "open list"), ("space", "follow"), ("backspace", "back"), ("+/\u{2212}", "zoom"), ("m", "copy as Mermaid"), ("q/esc", "close")],
        Mode::Yaml { .. } => vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("c", "copy"), ("q/esc", "back")],
        Mode::Context { editing: true, .. } | Mode::NamespacePick { editing: true, .. } => Vec::new(),
        Mode::NamespacePick { .. } => vec![("↑↓/jk", "move"), ("1-9", "assign key"), ("d", "clear key"), ("enter", "key list"), ("/", "filter"), ("q/esc", "back")],
        Mode::Context { .. } => vec![("↑↓/jk", "move"), ("enter", "connect"), ("/", "filter"), ("q/esc", "back")],
        Mode::Spec { .. } => {
            vec![("↑↓/jk", "move"), ("enter", "toggle"), ("v", "value"), ("a", "expand all"), ("q/esc", "back")]
        }
        Mode::NodeDetail { editing: true, .. } => Vec::new(),
        Mode::NodeDetail { .. } => vec![("↑↓/jk", "move"), ("enter", "containers"), ("d", "spec"), ("e", "edit"), ("s", "sort"), ("/", "search"), ("q/esc", "back")],
        Mode::Events { editing: true, .. } => Vec::new(),
        Mode::Events { .. } => vec![("↑↓/jk", "move"), ("enter", "detail"), ("a/w/n", "filter"), ("/", "search"), ("q/esc", "back")],
        Mode::EventDetail { .. } => vec![("q/esc", "back")],
        Mode::ResourcesDetail => vec![("q/esc", "back")],
        Mode::ColumnDetail { .. } => vec![("←↑↓→/hjkl", "move"), ("enter", "open"), ("q/esc", "back")],
        Mode::Containers { .. } => vec![("↑↓/jk", "move"), ("enter/l", "logs"), ("p", "previous"), ("S", "shell"), ("s", "sort"), ("q/esc", "back")],
        Mode::Logs { .. } => {
            vec![("↑↓/jk", "scroll"), ("G", "follow"), ("t", "timestamps"), ("o", "order"), ("/", "filter"), ("c", "copy"), ("q/esc", "back")]
        }
    }
}
