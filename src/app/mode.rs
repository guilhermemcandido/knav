//! What the app is currently showing (`Mode`) and the small helpers that describe it: paths, hints, filtering.

use crate::*;

/// Where the key popup on the settings screen is.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum CaptureStep {
    /// Choosing what to do with the action's keys.
    #[default]
    Menu,
    /// Waiting for a key; `replace` drops the current keys instead of adding.
    Pick { replace: bool },
}

/// The key popup on the settings screen: its step, the key pressed so far,
/// and why it can't be used, if it can't.
#[derive(Default)]
pub(crate) struct KeyCapture {
    pub step: CaptureStep,
    pub pressed: Option<String>,
    pub problem: Option<String>,
}

pub(crate) enum Mode {
    List,
    /// The `:` command line, reachable from any screen. `:q` exits, `:pods` and the
    /// like switch the view. Esc returns to `back` without acting.
    Command { input: String, selected: usize, back: Box<Mode> },
    /// The `/` live filter. It edits the persistent `search` string, so the filter
    /// stays applied once you are back in `List`.
    Search,
    /// The `:ctx` / `C` context browser. Enter checks the context is reachable, then
    /// hands control back to `main` to reconnect. `error` is why the last attempt failed.
    Context { contexts: Vec<k8s::ContextInfo>, filter: String, editing: bool, state: TableState, error: Option<String>, sort: ListSort, back: Box<Mode> },
    Menu { selected: (usize, usize) },
    /// The `n` namespace picker, from any view but the Namespaces list:
    /// choose a namespace (`/` filters), then which key it gets.
    NamespacePick { names: Vec<String>, filter: String, editing: bool, state: TableState, sort: ListSort, back: Box<Mode> },
    /// The key picker on a namespace: pick which number key (1-9) it goes
    /// on. `selected` is the highlighted key minus one. Esc cancels.
    Slots { namespace: String, selected: usize, back: Box<Mode> },
    /// A result message (see `edit`), any key or click dismisses it,
    /// returning to `back`.
    Notice { text: String, error: bool, back: Box<Mode> },
    /// The settings screen: every setting, edited in place and saved as it changes.
    Settings { tab: ui::SettingsTab, settings: Vec<crate::config::settings::Setting>, state: TableState, editing: Option<String>, capture: Option<KeyCapture>, error: Option<String>, back: Box<Mode> },
    /// The theme list, previewing each theme live as you move through it.
    ThemePicker { entries: Vec<ThemeEntry>, state: TableState, back: Box<Mode> },
    /// A shell running in a container, drawn inside knav (`Ctrl-]` closes it).
    Shell { title: String, session: Box<crate::ops::shell::ShellSession>, back: Box<Mode> },
    /// A manifest as plain YAML text, scrollable (`y`).
    Yaml { title: String, text: String, scroll: usize, back: Box<Mode> },
    /// What the selected object is related to (owners, what it uses, what uses it, ...).
    Relations { title: String, groups: Vec<k8s::RelationGroup>, selected: usize, back: Box<Mode> },
    /// Asks before a destructive action (`y`/Enter does it, `n`/Esc cancels).
    Confirm { text: String, targets: Vec<Target>, action: Action, back: Box<Mode> },
    /// Offers to open a URL in the browser (`y`/Enter does, `n`/Esc doesn't).
    OpenUrl { text: String, url: String, back: Box<Mode> },
    /// The port-forward dialog.
    Ports { target: Target, form: crate::ops::portforward::PortForm, back: Box<Mode> },
    /// Asks for a replica count (digits only) to scale to.
    Scale { targets: Vec<Target>, input: String, back: Box<Mode> },
    Spec {
        title: String,
        items: Vec<TreeItem<'static, String>>,
        state: TreeState<String>,
        // Which way `a` last left the tree, so pressing it again does the opposite.
        expanded_all: bool,
        // A leaf's full `(label, value)` by tree identifier, for `v`: the tree clips
        // long values to the box width.
        leaf_values: ui::LeafValues,
        // Set by `v`, cleared by q/Esc, which leaf's full value (if
        // any) is currently shown in its own popup on top of the tree.
        viewing: Option<(String, String)>,
        // Where Esc returns to, normally the List we opened it from,
        // or NodeDetail if 'd' was pressed from there instead.
        back: Box<Mode>,
    },
    /// Freelens-style node drill-down: that node's own metrics + the
    /// pods scheduled on it. `current_kind` stays `Nodes` throughout,
    /// this just overlays on top, same as `Containers` overlays on Pods.
    NodeDetail {
        name: String,
        state: TableState,
        sort: ListSort,
        /// `/` filters the pods table; `editing` while typing it.
        search: String,
        editing: bool,
        // Where Esc returns to: the Nodes list, or the Overview's Resources detail.
        back: Box<Mode>,
    },
    /// The full Events browser, opened by pressing Enter on the
    /// Overview's Events panel, every event, filterable by severity.
    Events { filter: k8s::EventFilter, search: String, editing: bool, state: TableState, sort: ListSort },
    /// One event's full, untruncated detail, opened from within the
    /// Events browser. `back` restores that browser's filter/scroll
    /// position exactly, same pattern as `Containers`/`Logs`.
    EventDetail { entry: k8s::EventEntry, back: Box<Mode> },
    /// The Overview's Resources panel, opened up: full-size cluster
    /// gauges. No per-node breakdown here anymore, that's what the
    /// Nodes list is for; this is cluster-wide totals only.
    ResourcesDetail,
    /// One Overview category column, opened up into a bigger grid,
    /// see `ui::Overlay::ColumnDetail`.
    ColumnDetail { col: usize, selected: usize, row_scroll: usize },
    Containers {
        title: String,
        namespace: String,
        pod: String,
        containers: Vec<k8s::ContainerInfo>,
        state: TableState,
        sort: ListSort,
        // Where Esc returns to, the Pods list normally, or the
        // NodeDetail view if this pod was opened from there.
        back: Box<Mode>,
    },
    Logs {
        title: String,
        lines: Vec<String>,
        scroll: u16,
        follow: bool,
        timestamp_format: TimestampFormat,
        order: LogOrder,
        rx: mpsc::UnboundedReceiver<String>,
        handle: tokio::task::JoinHandle<()>,
        // `/` filters the log lines by substring (log lines are prose, not identifiers).
        // `filter_editing` is true only while typing; Enter keeps the filter, Esc clears it.
        filter: String,
        filter_editing: bool,
        // What to go back to on Esc, the Containers view we came from,
        // so backing out of logs doesn't dump you all the way to the
        // pod list.
        back: Box<Mode>,
    },
}

/// Whether `c`, `?` and `:` should be typed as characters instead of acting as
/// global keys: `Command` and `Search` always, `Logs` while its filter is edited.
pub(crate) fn is_typing(mode: &Mode) -> bool {
    matches!(mode, Mode::Settings { editing: Some(_), .. } | Mode::Settings { capture: Some(_), .. } | Mode::Command { .. } | Mode::Search | Mode::Slots { .. } | Mode::Scale { .. } | Mode::Ports { .. } | Mode::Shell { .. } | Mode::Confirm { .. } | Mode::OpenUrl { .. } | Mode::Context { editing: true, .. } | Mode::NamespacePick { editing: true, .. } | Mode::Events { editing: true, .. } | Mode::NodeDetail { editing: true, .. }) || matches!(mode, Mode::Logs { filter_editing: true, .. })
}

pub(crate) fn title_for(namespace: Option<&str>, name: Option<&str>) -> String {
    format!("{}/{}", namespace.unwrap_or("?"), name.unwrap_or("?"))
}

/// The `/` filter: an empty query matches everything, otherwise a fuzzy
/// subsequence match against `haystack`.
pub(crate) fn row_matches(search: &str, haystack: &str) -> bool {
    search.is_empty() || fuzzy::score(search, haystack).is_some()
}

pub(crate) fn meta_search_text(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> String {
    format!("{} {}", meta.namespace.clone().unwrap_or_default(), meta.name.clone().unwrap_or_default())
}

pub(crate) fn meta_search_text_generic(row: &k8s::GenericRow) -> String {
    format!("{} {}", row.namespace, row.name)
}

/// The sort of whichever `NodeDetail` sits in `mode`'s back-chain, the
/// pods table behind Containers/Logs keeps its order.
pub(crate) fn node_detail_search(mode: &Mode) -> &str {
    match mode {
        Mode::NodeDetail { search, .. } => search,
        Mode::Containers { back, .. } | Mode::Logs { back, .. } | Mode::Spec { back, .. } => node_detail_search(back),
        _ => "",
    }
}

pub(crate) fn node_detail_sort(mode: &Mode) -> Option<SortSpec> {
    match mode {
        Mode::NodeDetail { sort, .. } => sort.spec,
        Mode::Containers { back, .. } | Mode::Logs { back, .. } | Mode::Spec { back, .. } => node_detail_sort(back),
        _ => None,
    }
}

/// The name of the `NodeDetail` anywhere in `mode`'s back-chain, itself included,
/// so `node_detail_pods` stays right while it is a dimmed background layer.
pub(crate) fn node_detail_name(mode: &Mode) -> Option<&str> {
    match mode {
        Mode::NodeDetail { name, .. } => Some(name),
        Mode::Containers { back, .. } | Mode::Logs { back, .. } | Mode::Spec { back, .. } => node_detail_name(back),
        _ => None,
    }
}

/// The path for the path bar, oldest first, e.g. `Node[worker-1]`, `Pod[default/web-1]`,
/// `Logs[nginx]`. Empty for the plain list and for modes that don't chain back.
pub(crate) fn segment(kind: &str, value: impl Into<String>) -> ui::PathSegment {
    ui::PathSegment { kind: kind.to_string(), value: Some(value.into()) }
}

pub(crate) fn plain_segment(kind: &str) -> ui::PathSegment {
    ui::PathSegment { kind: kind.to_string(), value: None }
}

/// Each segment is `Kind[identifier]`, so the path reads as an address in the cluster.
/// One theme in the picker, with the colours to show as its swatch.
pub(crate) struct ThemeEntry {
    pub name: String,
    pub swatch: Vec<ratatui::style::Color>,
}

/// The picker's rows, and where the current theme is among them.
pub(crate) fn theme_entries(current: &str) -> (Vec<ThemeEntry>, usize) {
    let entries: Vec<ThemeEntry> = crate::theme::all_names()
        .into_iter()
        .map(|name| {
            let t = crate::theme::lookup_theme(&name).unwrap_or_default();
            ThemeEntry { swatch: vec![t.background, t.foreground, t.header, t.ok, t.warn, t.bad, t.accent, t.container, t.select_bg], name }
        })
        .collect();
    let at = entries.iter().position(|e| e.name == current).unwrap_or(0);
    (entries, at)
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
        Mode::Relations { title, back, .. } => {
            let mut path = mode_path(back);
            path.push(segment("Related", title.clone()));
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

/// The keys available on the focused screen as (key, description) pairs, shown
/// in their own bar. `Command` and `Search` return nothing: they use that bar.
/// The keys for acting on the selected object, for the kinds each applies to.
fn action_hints(kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    let mut hints = match kind {
        ResourceKind::Pods => vec![("l", "logs"), ("p", "previous logs"), ("S", "shell"), ("F", "forward")],
        ResourceKind::Deployments => vec![("S", "scale"), ("r", "restart"), ("F", "forward")],
        ResourceKind::StatefulSets => vec![("S", "scale"), ("r", "restart")],
        ResourceKind::Services => vec![("F", "forward")],
        ResourceKind::ReplicaSets => vec![("S", "scale")],
        ResourceKind::DaemonSets => vec![("r", "restart")],
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
            hints.push(("m", "resources"));
            hints.push(("C", "contexts"));
            hints.push(("T", "themes"));
            hints.push((",", "settings"));
            hints.push(("q/esc", "back"));
            hints
        }
        Mode::Command { .. } | Mode::Search | Mode::Notice { .. } | Mode::Slots { .. } | Mode::Confirm { .. } | Mode::OpenUrl { .. } | Mode::Scale { .. } | Mode::Ports { .. } => Vec::new(),
        Mode::Shell { .. } => vec![("ctrl-]", "close the shell")],
        Mode::ThemePicker { .. } => vec![("↑↓/jk", "preview"), ("enter", "keep"), ("esc", "cancel")],
        Mode::Settings { editing: Some(_), .. } | Mode::Settings { capture: Some(_), .. } => Vec::new(),
        Mode::Settings { tab: ui::SettingsTab::Overview, .. } => vec![("↑↓/jk", "move"), ("1-9", "place"), ("J/K", "nudge"), ("space", "show/hide"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Settings { .. } => vec![("↑↓/jk", "move"), ("←→/enter", "change"), ("r", "reset"), ("tab", "next tab"), ("q/esc", "back")],
        Mode::Relations { .. } => vec![("↑↓/jk", "move"), ("enter", "open"), ("q/esc", "back")],
        Mode::Yaml { .. } => vec![("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("c", "copy"), ("q/esc", "back")],
        Mode::Context { editing: true, .. } | Mode::NamespacePick { editing: true, .. } => Vec::new(),
        Mode::NamespacePick { .. } => vec![("↑↓/jk", "move"), ("1-9", "assign key"), ("d", "clear key"), ("enter", "key list"), ("/", "filter"), ("q/esc", "back")],
        Mode::Context { .. } => vec![("↑↓/jk", "move"), ("enter", "connect"), ("/", "filter"), ("q/esc", "back")],
        Mode::Menu { .. } => vec![("←↑↓→/hjkl", "move"), ("enter", "select"), ("esc", "cancel")],
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
            vec![("↑↓/jk", "scroll"), ("G", "follow"), ("t", "timestamps"), ("o", "order"), ("/", "filter"), ("q/esc", "back")]
        }
    }
}

pub(crate) fn open_spec<T: serde::Serialize>(mode: &mut Mode, title: String, item: &T) {
    open_spec_value(mode, title, k8s::manifest_value(item));
}

pub(crate) fn open_spec_value(mode: &mut Mode, title: String, value: serde_yaml::Value) {
    let (items, leaf_values) = ui::build_manifest_tree(&value);
    let mut state = TreeState::default();
    for item in &items {
        state.open(vec![item.identifier().clone()]);
    }
    // Captures the current mode as `back` so Esc returns to whichever place opened this.
    let back = Box::new(std::mem::replace(mode, Mode::List));
    *mode = Mode::Spec { title, items, state, expanded_all: false, leaf_values, viewing: None, back };
}

/// Every identifier path in the tree, depth-first, used by `a` (see the
/// `Mode::Spec` keyboard handler) to expand every node at once, since
/// `TreeState` only exposes a bulk `close_all`, not its `open` opposite.
pub(crate) fn all_tree_identifiers(items: &[TreeItem<'static, String>], prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for item in items {
        prefix.push(item.identifier().clone());
        out.push(prefix.clone());
        all_tree_identifiers(item.children(), prefix, out);
        prefix.pop();
    }
}

pub(crate) fn select_next(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let next = state.selected().map(|i| (i + 1).min(len - 1)).unwrap_or(0);
    state.select(Some(next));
}

pub(crate) fn select_prev(state: &mut TableState, len: usize) {
    if len == 0 {
        return;
    }
    let prev = state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
    state.select(Some(prev));
}

/// `g`/`G`/Home/End jump to the top or bottom of a table, `Ctrl-f`/`Ctrl-b`
/// and PageDown/PageUp move a page. False for any other key.
pub(crate) fn jump_select(key: &KeyEvent, state: &mut TableState, len: usize, page: usize) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let current = state.selected().unwrap_or(0);
    let last = len.saturating_sub(1);
    let target = match key.code {
        KeyCode::Char('g') | KeyCode::Home if !ctrl => 0,
        KeyCode::Char('G') | KeyCode::End if !ctrl => last,
        KeyCode::Char('f') if ctrl => (current + page).min(last),
        KeyCode::PageDown => (current + page).min(last),
        KeyCode::Char('b') if ctrl => current.saturating_sub(page),
        KeyCode::PageUp => current.saturating_sub(page),
        _ => return false,
    };
    if len > 0 {
        state.select(Some(target));
    }
    true
}

/// Moves a table's selection for a mouse wheel notch; false for any other
/// mouse event.
pub(crate) fn wheel_select(kind: MouseEventKind, state: &mut TableState, len: usize) -> bool {
    let step: fn(&mut TableState, usize) = match kind {
        MouseEventKind::ScrollDown => select_next,
        MouseEventKind::ScrollUp => select_prev,
        _ => return false,
    };
    for _ in 0..crate::config::tunables::tunables().wheel_rows {
        step(state, len);
    }
    true
}

/// Routes `s` and the digits to whichever popup table has focus; `true`
/// if the key was a sort key and is used up.
pub(crate) fn popup_sort_key(mode: &mut Mode, code: KeyCode) -> bool {
    match mode {
        Mode::Events { sort, editing, .. } => sort.handle(code, EVENT_COLUMNS, *editing),
        Mode::Containers { sort, .. } => sort.handle(code, CONTAINER_COLUMNS, false),
        Mode::Context { sort, editing, .. } => sort.handle(code, CONTEXT_COLUMNS, *editing),
        Mode::NamespacePick { sort, editing, .. } => sort.handle(code, NAMESPACE_PICKER_COLUMNS, *editing),
        Mode::NodeDetail { sort, editing, .. } => sort.handle(code, POD_COLUMNS, *editing),
        _ => false,
    }
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
            handle: runtime.spawn(async {}),
            back: Box::new(containers_mode(1)),
        };
        assert_eq!(text(&mode_path(&logs)), ["Pod[default/web]", "Logs[sidecar]"]);
    }
}

#[cfg(test)]
mod wheel_tests {
    use super::*;

    #[test]
    fn the_wheel_moves_three_rows_and_stops_at_the_ends() {
        let mut state = TableState::default().with_selected(0);
        assert!(wheel_select(MouseEventKind::ScrollDown, &mut state, 10));
        assert_eq!(state.selected(), Some(3));
        wheel_select(MouseEventKind::ScrollDown, &mut state, 5);
        assert_eq!(state.selected(), Some(4));
        wheel_select(MouseEventKind::ScrollUp, &mut state, 5);
        assert_eq!(state.selected(), Some(1));
        wheel_select(MouseEventKind::ScrollUp, &mut state, 5);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn other_mouse_events_are_ignored() {
        let mut state = TableState::default().with_selected(2);
        assert!(!wheel_select(MouseEventKind::Moved, &mut state, 10));
        assert_eq!(state.selected(), Some(2));
    }
}

#[cfg(test)]
mod jump_tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn g_and_capital_g_go_to_the_ends() {
        let mut state = TableState::default().with_selected(4);
        assert!(jump_select(&key(KeyCode::Char('G'), KeyModifiers::SHIFT), &mut state, 20, 5));
        assert_eq!(state.selected(), Some(19));
        assert!(jump_select(&key(KeyCode::Char('g'), KeyModifiers::NONE), &mut state, 20, 5));
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn ctrl_f_and_ctrl_b_move_a_page_and_stop_at_the_ends() {
        let mut state = TableState::default().with_selected(3);
        jump_select(&key(KeyCode::Char('f'), KeyModifiers::CONTROL), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(13));
        jump_select(&key(KeyCode::PageDown, KeyModifiers::NONE), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(19));
        jump_select(&key(KeyCode::Char('b'), KeyModifiers::CONTROL), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(9));
        jump_select(&key(KeyCode::PageUp, KeyModifiers::NONE), &mut state, 20, 10);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn plain_f_and_b_are_not_jumps() {
        let mut state = TableState::default().with_selected(3);
        assert!(!jump_select(&key(KeyCode::Char('f'), KeyModifiers::NONE), &mut state, 20, 10));
        assert!(!jump_select(&key(KeyCode::Char('b'), KeyModifiers::NONE), &mut state, 20, 10));
        assert_eq!(state.selected(), Some(3));
    }

    #[test]
    fn an_empty_table_stays_unselected() {
        let mut state = TableState::default();
        assert!(jump_select(&key(KeyCode::Char('G'), KeyModifiers::NONE), &mut state, 0, 5));
        assert_eq!(state.selected(), None);
    }
}
