//! What the app is currently showing (`Mode`) and the small helpers that describe it: paths, hints, filtering.

use crate::*;

pub(crate) use super::{hints::*, nav::*, path::*};

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
    /// A readable summary of one object (name, labels, status, containers, ...).
    Details { manifest: serde_yaml::Value, sections: Vec<k8s::details::Section>, scroll: usize, hscroll: usize, back: Box<Mode> },
    /// What the selected object is related to (owners, what it uses, what uses it, ...).
    Relations { target: serde_yaml::Value, all: Vec<serde_yaml::Value>, graph: k8s::relations::Graph, selected: usize, previous: Vec<serde_yaml::Value>, back: Box<Mode> },
    /// Asks before a destructive action (`y`/Enter does it, `n`/Esc cancels).
    Confirm { spec: actions::ConfirmSpec, targets: Vec<Target>, action: Action, back: Box<Mode> },
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
        // Held so the stream stops with the view.
        #[allow(dead_code)]
        handle: AbortOnDrop,
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

/// Whether the mode takes every key itself (text entry, dialogs, the shell), so
/// the global keys such as `c`, `?` and `:` must not fire.
pub(crate) fn owns_keys(mode: &Mode) -> bool {
    matches!(
        mode,
        Mode::Settings { editing: Some(_), .. }
            | Mode::Settings { capture: Some(_), .. }
            | Mode::Command { .. }
            | Mode::Search
            | Mode::Slots { .. }
            | Mode::Scale { .. }
            | Mode::Ports { .. }
            | Mode::Shell { .. }
            | Mode::Confirm { .. }
            | Mode::OpenUrl { .. }
            | Mode::Context { editing: true, .. }
            | Mode::NamespacePick { editing: true, .. }
            | Mode::Events { editing: true, .. }
            | Mode::NodeDetail { editing: true, .. }
            | Mode::Logs { filter_editing: true, .. }
    )
}

/// A background task that stops when the mode holding it is dropped.
pub(crate) struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) fn title_for(namespace: Option<&str>, name: Option<&str>) -> String {
    format!("{}/{}", namespace.unwrap_or("?"), name.unwrap_or("?"))
}

/// `Kind namespace/name` for a manifest.
pub(crate) fn object_title(manifest: &serde_yaml::Value) -> String {
    let text = |path: &[&str]| path.iter().try_fold(manifest, |v, key| v.get(*key)).and_then(|v| v.as_str()).map(String::from);
    let place = match text(&["metadata", "namespace"]) {
        Some(ns) => format!("{ns}/"),
        None => String::new(),
    };
    format!("{} {place}{}", text(&["kind"]).unwrap_or_default(), text(&["metadata", "name"]).unwrap_or_default())
}

/// The `/` filter: an empty query matches everything, otherwise a fuzzy
/// subsequence match against `haystack`.
/// A search starting with `=` matches exactly (the jumps use it to land on one object).
pub(crate) fn row_matches(search: &str, haystack: &str) -> bool {
    match search.strip_prefix('=') {
        Some(exact) => haystack == exact,
        None => search.is_empty() || fuzzy::score(search, haystack).is_some(),
    }
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
