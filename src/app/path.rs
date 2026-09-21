//! The breadcrumb path shown above the list.

use crate::*;

/// The path for the path bar, oldest first, e.g. `Node[worker-1]`, `Pod[default/web-1]`,
/// `Logs[nginx]`. Empty for the plain list and for modes that don't chain back.
pub(crate) fn segment(kind: &str, value: impl Into<String>) -> ui::PathSegment {
    ui::PathSegment { kind: kind.to_string(), value: Some(value.into()) }
}

pub(crate) fn plain_segment(kind: &str) -> ui::PathSegment {
    ui::PathSegment { kind: kind.to_string(), value: None }
}

pub(crate) fn mode_path(mode: &Mode) -> Vec<ui::PathSegment> {
    match mode {
        Mode::NodeDetail { name, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Node", name.clone()));
            path
        }
        Mode::Containers { title, containers, state, sort, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Pod", title.clone()));
            // The container the cursor is on, like the selected row of a list.
            if let Some(container) = state.selected().and_then(|i| sorted_containers(containers, *sort).get(i).map(|c| c.name.clone())) {
                path.push(segment("Container", container));
            }
            path
        }
        Mode::Spec { title, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Spec", title.clone()));
            path
        }
        Mode::Logs { title, back, .. } => {
            let mut path = mode_path(back);
            // The container being read replaces the one that was selected.
            if path.last().is_some_and(|s| s.kind == "Container") {
                path.pop();
            }
            // `title` is "namespace/pod/container" (see `title_for` and
            // the Containers Enter handler), just the container name is
            // enough here, the pod/node segments already came from `back`.
            let container = title.rsplit('/').next().unwrap_or(title);
            path.push(segment("Logs", container));
            path
        }
        Mode::ThemePicker { .. } => vec![plain_segment("Themes")],
        Mode::Settings { .. } => vec![plain_segment("Settings")],
        Mode::Shell { title, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Shell", title.rsplit('/').next().unwrap_or(title)));
            path
        }
        Mode::Yaml { title, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("YAML", title.clone()));
            path
        }
        Mode::Details { manifest, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Info", object_title(manifest)));
            path
        }
        Mode::Relations { target, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Related", object_title(target)));
            path
        }
        Mode::EventDetail { back, .. } => {
            let mut path = mode_path(back);
            path.push(plain_segment("Event"));
            path
        }
        Mode::Events { .. } => vec![plain_segment("Events")],
        Mode::ResourcesDetail => vec![plain_segment("Resources")],
        Mode::ColumnDetail { .. } => vec![plain_segment("Category")],
        Mode::Context { .. } => vec![plain_segment("Contexts")],
        Mode::Notice { back, .. } | Mode::Confirm { back, .. } | Mode::OpenUrl { back, .. } | Mode::Scale { back, .. } | Mode::Ports { back, .. } | Mode::Slots { back, .. } | Mode::NamespacePick { back, .. } => mode_path(back),
        Mode::Menu { .. } => vec![plain_segment("Resources")],
        Mode::List | Mode::Command { .. } | Mode::Search => Vec::new(),
    }
}

/// Where you are, for the bottom bar: what you drilled through, then the list, e.g.
/// `Deployment[web]>>ReplicaSet[web-5d9d]>>Pods`.
pub(crate) fn location(current_kind: ResourceKind, trail: &[(ResourceKind, Option<Scope>, usize)], scope: Option<&Scope>) -> Vec<ui::PathSegment> {
    // Every level's scope names the thing it's inside; together they are the path.
    let mut segments: Vec<ui::PathSegment> = trail
        .iter()
        .filter_map(|(_, sc, _)| sc.as_ref())
        .chain(scope)
        .map(|sc| {
            let (kind, name) = sc.parts();
            segment(kind, name)
        })
        .collect();
    segments.push(plain_segment(current_kind.label()));
    segments
}

/// The path bar's segments: the list's `location`, then the open
/// popups.
pub(crate) fn full_path(mode: &Mode, location: Vec<ui::PathSegment>) -> Vec<ui::PathSegment> {
    let mut segments = location;
    let path = mode_path(mode);
    // `Nodes>>Node[worker-1]` says the same thing twice: once a popup names
    // the specific one (`Node[...]`, `Pod[...]`), it replaces its list.
    if let (Some(list), Some(first)) = (segments.last(), path.first())
        && first.value.is_some()
        && list.value.is_none()
        && list.kind.strip_suffix('s') == Some(first.kind.as_str())
    {
        segments.pop();
    }
    segments.extend(path);
    segments
}

#[cfg(test)]
mod path_tests {
    use super::*;
    use crate::k8s::{ContainerInfo, ContainerStatusKind};

    fn container(name: &str) -> ContainerInfo {
        ContainerInfo { name: name.into(), status: ContainerStatusKind::Running, reason: None, restarts: 0 }
    }

    fn containers_mode(selected: usize) -> Mode {
        Mode::Containers {
            title: "default/web".into(),
            namespace: "default".into(),
            pod: "web".into(),
            containers: vec![container("app"), container("sidecar")],
            state: TableState::default().with_selected(selected),
            sort: ListSort::default(),
            back: Box::new(Mode::List),
        }
    }

    fn text(path: &[ui::PathSegment]) -> Vec<String> {
        path.iter().map(|s| format!("{}[{}]", s.kind, s.value.clone().unwrap_or_default())).collect()
    }

    #[test]
    fn the_selected_container_follows_the_pod() {
        assert_eq!(text(&mode_path(&containers_mode(1))), ["Pod[default/web]", "Container[sidecar]"]);
    }

    #[test]
    fn reading_its_logs_replaces_the_container_segment() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let logs = Mode::Logs {
            title: "default/web/sidecar".into(),
            lines: Vec::new(),
            rx,
            scroll: 0,
            follow: true,
            timestamp_format: Default::default(),
            order: Default::default(),
            filter: String::new(),
            filter_editing: false,
            handle: AbortOnDrop(runtime.spawn(async {})),
            back: Box::new(containers_mode(1)),
        };
        assert_eq!(text(&mode_path(&logs)), ["Pod[default/web]", "Logs[sidecar]"]);
    }
}
