//! What the app is showing (`Mode`) and the small helpers that describe it.

use super::*;


/// Where the key popup on the settings screen is.
#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) enum CaptureStep {
    /// Choosing what to do with the action's keys.
    #[default]
    Menu,
    /// Waiting for a key; `replace` drops the current keys instead of adding.
    Pick { replace: bool },
}

/// The key popup on the settings screen: its step, the key pressed, and why it can't
/// be used, if it can't.
#[derive(Default)]
pub(crate) struct KeyCapture {
    pub step: CaptureStep,
    pub pressed: Option<String>,
    pub problem: Option<String>,
}

/// An edit in progress: the object as it was, the text as edited, and why the
/// cluster refused the last try.
pub(crate) struct EditDraft {
    pub title: String,
    pub original: String,
    pub edited: String,
    pub error: Option<String>,
}

pub(crate) enum Mode {
    List,
    /// The `:` command line. `:q` exits, `:pods` and the like switch the view, Esc returns
    /// to `back`.
    Command { input: String, selected: usize, back: Box<Mode> },
    /// The `/` filter. It edits the persistent `search`, so it stays applied afterwards.
    Search,
    /// The context browser (`C`). Enter checks the context is reachable, then hands back
    /// to `main` to reconnect. `error` is why the last attempt failed.
    Context { contexts: Vec<k8s::ContextInfo>, filter: String, editing: bool, state: TableState, error: Option<String>, back: Box<Mode> },
    /// The `n` namespace picker: choose a namespace, then which key it gets.
    NamespacePick { names: Vec<String>, filter: String, editing: bool, state: TableState, sort: ListSort, back: Box<Mode> },
    /// The key picker for a namespace. `selected` is the highlighted key minus one.
    Slots { namespace: String, selected: usize, back: Box<Mode> },
    /// A result message; any key or click returns to `back`.
    Notice { text: String, tone: crate::ops::NoticeTone, back: Box<Mode> },
    /// The settings screen, saving each setting as it changes.
    Settings { tab: ui::SettingsTab, settings: Vec<crate::app::settings::Setting>, state: TableState, editing: Option<String>, capture: Option<KeyCapture>, error: Option<String>, back: Box<Mode> },
    /// The Extensions screen (`E`). `/` types a filter, Enter keeps it, Esc clears it.
    Extensions { filter: String, filter_editing: bool, state: TableState, error: Option<String>, back: Box<Mode> },
    /// The theme list, previewing each theme live as you move through it.
    ThemePicker { entries: Vec<crate::theme::ThemeEntry>, state: TableState, back: Box<Mode> },
    /// A shell running in a container, drawn inside knav (`Ctrl-]` closes it).
    Shell { title: String, session: Box<crate::ops::shell::ShellSession>, back: Box<Mode> },
    /// A manifest as plain YAML text, scrollable (`y`).
    /// Text to read and scroll: an object's YAML, or a command's output. `label` names
    /// it in the path line.
    Yaml { label: &'static str, title: String, text: String, scroll: usize, back: Box<Mode> },
    /// A readable summary of one object.
    Details { manifest: serde_yaml::Value, sections: Vec<k8s::details::Section>, scroll: usize, hscroll: usize, back: Box<Mode> },
    /// The selected object's relations (owners, what it uses, what uses it, ...).
    Relations { target: serde_yaml::Value, all: Vec<serde_yaml::Value>, graph: k8s::relations::Graph, selected: usize, previous: Vec<serde_yaml::Value>, zoom: usize, back: Box<Mode> },
    /// A background job (an action, a connection check, a port-forward) is running.
    Working { job: crate::app::jobs::Job, back: Box<Mode> },
    /// Asks before an action. `yes` is the focused button, which the arrows move and
    /// Enter presses; `y` and `n` work anywhere.
    Confirm { spec: actions::ConfirmSpec, targets: Vec<Target>, action: Action, yes: bool, back: Box<Mode> },
    /// The changes an edit makes, before they are applied. `focus` is the button:
    /// 0 Apply, 1 Edit again, 2 Cancel.
    EditReview { draft: EditDraft, diff: Vec<(crate::ops::edit::DiffKind, String)>, scroll: usize, focus: usize, back: Box<Mode> },
    /// Everything that needs a look, across kinds; `search` filters it.
    Problems { state: TableState, search: String, editing: bool, back: Box<Mode> },
    /// A Deployment's revisions, newest first; `scroll` is into the changes below.
    History { target: Target, revisions: Vec<k8s::rollout::Revision>, cursor: usize, scroll: usize, back: Box<Mode> },
    /// `P`: this cluster's permission mode, and which contexts are always read-only.
    /// `input` is a context pattern being typed: which one it replaces, and the text.
    Permissions { tab: usize, cursor: usize, input: Option<(Option<usize>, String)>, error: Option<String>, back: Box<Mode> },
    /// Offers to open a URL in the browser, with buttons like `Confirm`.
    OpenUrl { text: String, url: String, yes: bool, back: Box<Mode> },
    /// The port-forward dialog.
    Ports { target: Target, form: crate::ops::portforward::PortForm, back: Box<Mode> },
    /// Asks for a replica count, stepped with the arrows or typed. `fresh` means the
    /// number is still the current count, so the first digit typed replaces it. The
    /// arrows change the number, so Tab moves between the buttons (`yes` is Scale).
    Scale { targets: Vec<Target>, input: String, fresh: bool, yes: bool, back: Box<Mode> },
    Spec {
        title: String,
        items: Vec<TreeItem<'static, String>>,
        state: TreeState<String>,
        // Which way `a` last left the tree, so pressing it again does the opposite.
        expanded_all: bool,
        // Each leaf's full label and value by identifier, for `v`, since the tree clips them.
        leaf_values: ui::LeafValues,
        // The leaf whose full value `v` is showing over the tree, if any.
        viewing: Option<(String, String)>,
        // Where Esc returns to: the list, or NodeDetail if opened from there.
        back: Box<Mode>,
    },
    /// A node's metrics and the pods on it, over the Nodes list like Containers over Pods.
    NodeDetail {
        name: String,
        state: TableState,
        sort: ListSort,
        /// The pods table's `/` filter; `editing` while it is typed.
        search: String,
        editing: bool,
        // Where Esc returns to: the Nodes list, or the Overview's Resources detail.
        back: Box<Mode>,
    },
    /// The Events browser: every event, filterable by severity.
    Events { filter: k8s::EventFilter, search: String, editing: bool, state: TableState, sort: ListSort },
    /// One event in full. `back` restores the browser exactly.
    EventDetail { entry: k8s::EventEntry, back: Box<Mode> },
    /// The Overview's Resources panel opened up.
    ResourcesDetail,
    /// One Overview category column opened into a bigger grid.
    ColumnDetail { col: usize, selected: usize, row_scroll: usize },
    Containers {
        title: String,
        namespace: String,
        pod: String,
        containers: Vec<k8s::ContainerInfo>,
        state: TableState,
        sort: ListSort,
        // Where Esc returns to: the Pods list, or NodeDetail if opened from there.
        back: Box<Mode>,
    },
    Logs {
        title: String,
        lines: Vec<String>,
        scroll: usize,
        follow: bool,
        timestamp_format: TimestampFormat,
        order: LogOrder,
        rx: mpsc::UnboundedReceiver<String>,
        // Held so the streams stop with the view: one per container shown.
        #[allow(dead_code)]
        handles: Vec<AbortOnDrop>,
        // `/` filters lines by substring; `filter_editing` only while typing.
        // Enter keeps the filter, Esc clears it.
        filter: String,
        filter_editing: bool,
        // Where Esc returns to: the Containers view it came from.
        back: Box<Mode>,
    },
}

/// Whether the mode takes every key itself (text entry, dialogs, the shell), so global
/// keys like `?` and `:` must not fire.
pub(crate) fn owns_keys(mode: &Mode) -> bool {
    matches!(
        mode,
        Mode::Settings { editing: Some(_), .. }
            | Mode::Settings { capture: Some(_), .. }
            | Mode::Extensions { filter_editing: true, .. }
            | Mode::Command { .. }
            | Mode::Search
            | Mode::Slots { .. }
            | Mode::Scale { .. }
            | Mode::Ports { .. }
            | Mode::Shell { .. }
            | Mode::Confirm { .. }
            | Mode::Working { .. }
            | Mode::OpenUrl { .. }
            | Mode::EditReview { .. }
            | Mode::Permissions { .. }
            | Mode::Problems { editing: true, .. }
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

/// The `/` filter: empty matches everything, `=` matches exactly (jumps use it to land on
/// one object), anything else is a fuzzy match.
pub(crate) fn row_matches(search: &str, haystack: &str) -> bool {
    match search.strip_prefix('=') {
        Some(exact) => haystack == exact,
        None => search.is_empty() || fuzzy::score(search, haystack).is_some(),
    }
}

/// `row_matches` against `namespace name`, built only when there is a search.
pub(crate) fn meta_matches(search: &str, meta: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta) -> bool {
    search.is_empty() || row_matches(search, &format!("{} {}", meta.namespace.as_deref().unwrap_or_default(), meta.name.as_deref().unwrap_or_default()))
}

pub(crate) fn generic_matches(search: &str, row: &k8s::GenericRow) -> bool {
    search.is_empty() || row_matches(search, &format!("{} {}", row.namespace, row.name))
}

/// The search of the `NodeDetail` in `mode`'s back-chain, so its pods table keeps its
/// filter behind Containers and Logs.
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

/// The name of the `NodeDetail` in `mode`'s back-chain, itself included, so its pods
/// stay right while it is a dimmed background.
pub(crate) fn node_detail_name(mode: &Mode) -> Option<&str> {
    match mode {
        Mode::NodeDetail { name, .. } => Some(name),
        Mode::Containers { back, .. } | Mode::Logs { back, .. } | Mode::Spec { back, .. } => node_detail_name(back),
        _ => None,
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
    // The current mode becomes `back`, so Esc returns to whatever opened this.
    let back = Box::new(std::mem::replace(mode, Mode::List));
    *mode = Mode::Spec { title, items, state, expanded_all: false, leaf_values, viewing: None, back };
}

/// Every identifier path in the tree, depth first, for `a` to expand all:
/// `TreeState` can close all but not open all.
pub(crate) fn all_tree_identifiers(items: &[TreeItem<'static, String>], prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for item in items {
        prefix.push(item.identifier().clone());
        out.push(prefix.clone());
        all_tree_identifiers(item.children(), prefix, out);
        prefix.pop();
    }
}

/// The screen whose keys a mode uses, or `None` while typing text or in fixed prompts,
/// which are never remapped.
pub(crate) fn screen_of(mode: &Mode, kind: ResourceKind) -> Option<crate::input::keymap::Screen> {
    use crate::input::keymap::Screen::*;
    Some(match mode {
        Mode::List if kind == ResourceKind::Overview => Overview,
        Mode::List => List,
        Mode::ColumnDetail { .. } => Column,
        Mode::Events { editing: false, .. } => Events,
        Mode::NamespacePick { editing: false, .. } => Namespaces,
        Mode::Context { editing: false, .. } => Contexts,
        Mode::Containers { .. } => Containers,
        Mode::NodeDetail { editing: false, .. } => NodeDetail,
        Mode::Logs { filter_editing: false, .. } => Logs,
        Mode::Spec { viewing: None, .. } => Spec,
        Mode::Yaml { .. } => Yaml,
        Mode::Settings { editing: None, capture: None, .. } => Settings,
        Mode::Extensions { filter_editing: false, .. } => Extensions,
        Mode::ThemePicker { .. } => Themes,
        Mode::EventDetail { .. } | Mode::ResourcesDetail | Mode::Relations { .. } | Mode::Details { .. } => Other,
        _ => return None,
    })
}

#[cfg(test)]
mod screen_tests {
    use super::*;
    use crate::input::keymap::Screen::{List, Overview};

    #[test]
    fn screens_that_take_text_are_not_remapped() {
        assert_eq!(screen_of(&Mode::Search, ResourceKind::Pods), None);
        assert_eq!(screen_of(&Mode::List, ResourceKind::Overview), Some(Overview));
        assert_eq!(screen_of(&Mode::List, ResourceKind::Pods), Some(List));
    }
}
