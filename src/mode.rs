//! What the app is currently showing (`Mode`) and the small helpers that describe it: breadcrumbs, hints, filtering.

use super::*;

pub(crate) enum Mode {
    List,
    /// A vim/k9s-style `:` command line, reachable from any screen —
    /// `:q`/`:quit` exits from anywhere, `:pods`/`:namespaces`/etc. (see
    /// `ResourceKind::from_command`) switches the current view. Esc
    /// cancels back to `back` without acting, same as every other
    /// overlay's "where Esc returns to."
    Command { input: String, selected: usize, back: Box<Mode> },
    /// The `/`/`f` live-filter input — editing the persistent `search`
    /// string directly (not its own copy), so the filter it produces
    /// stays applied once you're back in `List`, same as vim/fzf's own
    /// "type to narrow, Enter to keep browsing the narrowed list."
    Search,
    /// The `:ctx` / `C` context browser, laid out like `Events` — Enter
    /// on a context checks it is reachable, then hands control back to
    /// `main` to reconnect; `/` filters by name. `error` is why the last
    /// attempt failed. Esc returns to `back`.
    Context { contexts: Vec<k8s::ContextInfo>, filter: String, editing: bool, state: TableState, error: Option<String>, back: Box<Mode> },
    Menu { selected: (usize, usize) },
    /// The `s` popup on a namespace: pick which number key (1-9) it goes
    /// on. `selected` is the highlighted key minus one. Esc cancels.
    Slots { namespace: String, selected: usize, back: Box<Mode> },
    /// A result message (see `edit`) — any key or click dismisses it,
    /// returning to `back`.
    Notice { text: String, error: bool, back: Box<Mode> },
    Spec {
        title: String,
        items: Vec<TreeItem<'static, String>>,
        state: TreeState<String>,
        // Tracks which way `a` (expand/collapse everything) last left
        // the tree, so pressing it again does the opposite — toggling
        // between "everything open" and "everything closed" rather than
        // needing two separate keys for it.
        expanded_all: bool,
        // A leaf's full `(label, value)`, keyed by its own tree
        // identifier — `v` looks up whatever's selected here to show it
        // untruncated, since the tree itself clips long values to the
        // box's width with no indication or way to see the rest.
        leaf_values: ui::LeafValues,
        // Set by `v`, cleared by q/Esc — which leaf's full value (if
        // any) is currently shown in its own popup on top of the tree.
        viewing: Option<(String, String)>,
        // Where Esc returns to — normally the List we opened it from,
        // or NodeDetail if 'd' was pressed from there instead.
        back: Box<Mode>,
    },
    /// Freelens-style node drill-down: that node's own metrics + the
    /// pods scheduled on it. `current_kind` stays `Nodes` throughout —
    /// this just overlays on top, same as `Containers` overlays on Pods.
    NodeDetail {
        name: String,
        state: TableState,
        // Where Esc returns to — the Nodes list normally, or the
        // Overview's Resources detail if this node was opened from
        // there, same "remember where you came from" pattern as
        // `Containers`/`Logs`.
        back: Box<Mode>,
    },
    /// The full Events browser, opened by pressing Enter on the
    /// Overview's Events panel — every event, filterable by severity.
    Events { filter: k8s::EventFilter, state: TableState },
    /// One event's full, untruncated detail — opened from within the
    /// Events browser. `back` restores that browser's filter/scroll
    /// position exactly, same pattern as `Containers`/`Logs`.
    EventDetail { entry: k8s::EventEntry, back: Box<Mode> },
    /// The Overview's Resources panel, opened up: full-size cluster
    /// gauges. No per-node breakdown here anymore — that's what the
    /// Nodes list is for; this is cluster-wide totals only.
    ResourcesDetail,
    /// One Overview category column, opened up into a bigger grid —
    /// see `ui::Overlay::ColumnDetail`.
    ColumnDetail { col: usize, selected: usize, row_scroll: usize },
    Containers {
        title: String,
        namespace: String,
        pod: String,
        containers: Vec<k8s::ContainerInfo>,
        state: TableState,
        // Where Esc returns to — the Pods list normally, or the
        // NodeDetail view if this pod was opened from there.
        back: Box<Mode>,
    },
    Logs {
        title: String,
        lines: Vec<String>,
        scroll: u16,
        follow: bool,
        timestamp_format: TimestampFormat,
        rx: mpsc::UnboundedReceiver<String>,
        handle: tokio::task::JoinHandle<()>,
        // `/` live-filters the log lines the same way it does everywhere
        // else — a substring match here rather than fuzzy, since log
        // lines are prose, not identifiers a fuzzy scorer makes sense
        // against. `filter_editing` is only true while actually typing
        // it; Enter confirms and goes back to normal scrolling with the
        // filter applied, Esc while typing clears it instead.
        filter: String,
        filter_editing: bool,
        // What to go back to on Esc — the Containers view we came from,
        // so backing out of logs doesn't dump you all the way to the
        // pod list.
        back: Box<Mode>,
    },
}

/// Whether `c`/`?`/`:` (each bound globally, see the top of the event
/// loop) should instead just be typed as a character — `Command`/
/// `Search` always, `Logs` only while its own `/` filter is actively
/// being edited.
pub(crate) fn is_typing(mode: &Mode) -> bool {
    matches!(mode, Mode::Command { .. } | Mode::Search | Mode::Slots { .. } | Mode::Context { editing: true, .. }) || matches!(mode, Mode::Logs { filter_editing: true, .. })
}

pub(crate) fn title_for(namespace: Option<&str>, name: Option<&str>) -> String {
    format!("{}/{}", namespace.unwrap_or("?"), name.unwrap_or("?"))
}

/// The `/`/`f` filter: an empty query matches everything (no filter
/// active); otherwise a fuzzy subsequence match against `haystack`,
/// reusing the exact same scorer the cluster picker's own type-to-filter
/// search already uses.
pub(crate) fn row_matches(search: &str, haystack: &str) -> bool {
    search.is_empty() || fuzzy::score(search, haystack).is_some()
}

pub(crate) fn meta_search_text(meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> String {
    format!("{} {}", meta.namespace.clone().unwrap_or_default(), meta.name.clone().unwrap_or_default())
}

pub(crate) fn meta_search_text_generic(row: &k8s::GenericRow) -> String {
    format!("{} {}", row.namespace, row.name)
}

/// The name of whichever `NodeDetail` sits anywhere in `mode`'s own
/// back-chain (including `mode` itself) — not just when it's the
/// topmost/focused mode. Needed so `node_detail_pods` stays correct even
/// while NodeDetail is being drawn as a dimmed background layer behind
/// something opened from it (Containers, Spec, and transitively Logs).
pub(crate) fn node_detail_name(mode: &Mode) -> Option<&str> {
    match mode {
        Mode::NodeDetail { name, .. } => Some(name),
        Mode::Containers { back, .. } | Mode::Logs { back, .. } | Mode::Spec { back, .. } => node_detail_name(back),
        _ => None,
    }
}

/// The full "how did I get here" path for the breadcrumb bar, oldest
/// first — e.g. `["Node: worker-1", "Pod: default/web-1", "Logs:
/// nginx"]`. `None` for `Mode::List` itself (nothing to show — you're
/// already home) and for modes that don't chain back further than the
/// base list (Menu, Command, Events, ResourcesDetail, ColumnDetail),
/// since their own overlay title already says what they are.
pub(crate) fn segment(kind: &str, value: impl Into<String>) -> ui::BreadcrumbSegment {
    ui::BreadcrumbSegment { kind: kind.to_string(), value: Some(value.into()) }
}

pub(crate) fn plain_segment(kind: &str) -> ui::BreadcrumbSegment {
    ui::BreadcrumbSegment { kind: kind.to_string(), value: None }
}

/// Each segment carries `Kind[identifier]` — e.g. `Node[worker-1]`,
/// `Pod[default/web-1]`, `Logs[nginx]` — so the breadcrumb reads as a
/// literal address into the cluster, not just a label trail.
pub(crate) fn breadcrumb_path(mode: &Mode) -> Vec<ui::BreadcrumbSegment> {
    match mode {
        Mode::NodeDetail { name, back, .. } => {
            let mut path = breadcrumb_path(back);
            path.push(segment("Node", name.clone()));
            path
        }
        Mode::Containers { title, back, .. } => {
            let mut path = breadcrumb_path(back);
            path.push(segment("Pod", title.clone()));
            path
        }
        Mode::Spec { title, back, .. } => {
            let mut path = breadcrumb_path(back);
            path.push(segment("Spec", title.clone()));
            path
        }
        Mode::Logs { title, back, .. } => {
            let mut path = breadcrumb_path(back);
            // `title` is "namespace/pod/container" (see `title_for` and
            // the Containers Enter handler) — just the container name is
            // enough here, the pod/node segments already came from `back`.
            let container = title.rsplit('/').next().unwrap_or(title);
            path.push(segment("Logs", container));
            path
        }
        Mode::EventDetail { back, .. } => {
            let mut path = breadcrumb_path(back);
            path.push(plain_segment("Event"));
            path
        }
        Mode::Events { .. } => vec![plain_segment("Events")],
        Mode::ResourcesDetail => vec![plain_segment("Resources")],
        Mode::ColumnDetail { .. } => vec![plain_segment("Category")],
        Mode::Context { .. } => vec![plain_segment("Contexts")],
        Mode::Notice { back, .. } | Mode::Slots { back, .. } => breadcrumb_path(back),
        Mode::List | Mode::Command { .. } | Mode::Search | Mode::Menu { .. } => Vec::new(),
    }
}

/// The breadcrumb bar's segments, or `None` when there's nothing worth
/// showing (plain `List`, or a shallow overlay whose own title already
/// says everything — Menu/Command/Events/ResourcesDetail/ColumnDetail).
pub(crate) fn breadcrumb(mode: &Mode, current_kind: ResourceKind) -> Option<Vec<ui::BreadcrumbSegment>> {
    let path = breadcrumb_path(mode);
    if path.is_empty() {
        return None;
    }
    let mut segments = vec![plain_segment(current_kind.label())];
    segments.extend(path);
    Some(segments)
}

/// The keybindings actually available on whatever's currently focused —
/// (key, description) pairs, handed to `ui::draw` to render in their own
/// bar instead of crammed into each screen's title. `Command`/`Search`
/// return nothing: both already occupy that bar themselves with the
/// input being typed, which matters more than a hint list right then.
pub(crate) fn hints_for(mode: &Mode, current_kind: ResourceKind) -> Vec<(&'static str, &'static str)> {
    match mode {
        // Nothing on the main screen — deliberately kept clean. The
        // commands panel only exists once you've actually entered some
        // resource view.
        Mode::List if current_kind == ResourceKind::Overview => Vec::new(),
        Mode::List => {
            let mut hints = match current_kind {
                ResourceKind::Pods => vec![("j/k", "move"), ("enter", "containers"), ("d", "spec")],
                ResourceKind::Deployments => vec![("j/k", "move"), ("enter", "replicasets"), ("d", "spec")],
                ResourceKind::Namespaces => vec![("j/k", "move"), ("enter", "pods"), ("s", "assign a number key"), ("d", "spec")],
                ResourceKind::CronJobs => vec![("j/k", "move"), ("enter", "jobs"), ("d", "spec")],
                ResourceKind::ReplicaSets
                | ResourceKind::StatefulSets
                | ResourceKind::DaemonSets
                | ResourceKind::Jobs
                | ResourceKind::Services => vec![("j/k", "move"), ("enter", "pods"), ("d", "spec")],
                ResourceKind::Nodes => vec![("j/k", "move"), ("enter", "what's running"), ("d", "spec")],
                ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_) => vec![("j/k", "move"), ("enter", "open")],
                _ => vec![("j/k", "move"), ("enter", "spec"), ("d", "spec")],
            };
            if !matches!(current_kind, ResourceKind::CustomResourceList | ResourceKind::CustomResourceGroup(_)) {
                hints.push(("e", "edit"));
            }
            hints.push(("0-9", "namespace"));
            hints.push(("/", "search"));
            hints.push(("m", "switch resource"));
            hints.push(("C", "switch context"));
            hints.push(("q/esc", "back"));
            hints
        }
        Mode::Command { .. } | Mode::Search | Mode::Notice { .. } | Mode::Slots { .. } => Vec::new(),
        Mode::Context { editing: true, .. } => Vec::new(),
        Mode::Context { .. } => vec![("j/k", "move"), ("enter", "connect"), ("/", "filter"), ("q/esc", "back")],
        Mode::Menu { .. } => vec![("arrows/hjkl", "move"), ("enter", "select"), ("esc", "cancel")],
        Mode::Spec { .. } => {
            vec![("j/k", "move"), ("enter", "toggle"), ("v", "view full value"), ("a", "expand/collapse all"), ("q/esc", "back")]
        }
        Mode::NodeDetail { .. } => vec![("j/k", "move"), ("enter", "containers"), ("d", "spec"), ("e", "edit"), ("q/esc", "back")],
        Mode::Events { .. } => vec![("j/k", "move"), ("enter", "detail"), ("a/w/n", "filter"), ("q/esc", "back")],
        Mode::EventDetail { .. } => vec![("q/esc", "back")],
        Mode::ResourcesDetail => vec![("q/esc", "back")],
        Mode::ColumnDetail { .. } => vec![("arrows/hjkl", "move"), ("enter", "open"), ("q/esc", "back")],
        Mode::Containers { .. } => vec![("j/k", "move"), ("enter", "logs"), ("q/esc", "back")],
        Mode::Logs { .. } => {
            vec![("j/k", "scroll"), ("G", "resume follow"), ("t", "toggle timestamp"), ("/", "filter"), ("q/esc", "back")]
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
    // Captures whatever `mode` actually was (List, or NodeDetail if 'd'
    // was pressed from there) as `back`, so Esc returns to the right
    // place regardless of which of `open_spec`'s several call sites
    // opened this.
    let back = Box::new(std::mem::replace(mode, Mode::List));
    *mode = Mode::Spec { title, items, state, expanded_all: false, leaf_values, viewing: None, back };
}

/// Every identifier path in the tree, depth-first — used by `a` (see the
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
