//! The interactive session's own state: what is on screen and how it is
//! narrowed, sorted and scrolled. Owned by the loop, changed by handlers.

use super::*;

/// A place in the app: which list, drilled into what.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct View {
    pub kind: ResourceKind,
    pub scope: Option<Scope>,
}

const HISTORY_LIMIT: usize = 50;

pub(super) struct State {
    /// The config as it stands now: the config screen edits it while knav runs.
    pub config: Config,
    /// Turns the keys you press into the built-in keys the handlers know.
    pub keymap: crate::input::keymap::Keymap,
    pub table_state: TableState,
    pub mode: Mode,
    pub hovered: Option<ui::Hover>,
    /// When and where the last click landed, to recognise a double-click.
    pub last_click: Option<(std::time::Instant, usize)>,
    /// The info panel beside the list (`i`), and how far it is scrolled.
    pub info_panel: bool,
    pub info_scroll: usize,
    pub info_hscroll: usize,
    /// The keys scroll the info panel rather than move in the list.
    pub info_focus: bool,
    /// The object the panel showed last, to restart its scroll when the selection moves.
    pub info_key: String,
    /// Whether the info view shows a Secret's values; off whenever the object changes.
    pub reveal: bool,
    /// A key to handle again on the next turn, after a job it waited for.
    pub replay: Option<crossterm::event::KeyEvent>,
    pub current_kind: ResourceKind,
    /// The namespace every namespaced list is narrowed to (`Enter` on a
    /// namespace sets it, `0` clears it), sticks across kind switches.
    pub namespace: Option<String>,
    /// What the current list is drilled into (a Deployment's ReplicaSets,
    /// a Service's Pods, ...).
    pub scope: Option<Scope>,
    /// How to get back out of a drill-down, one level per entry: the kind,
    /// scope and selected row we came from.
    pub nav_stack: Vec<(ResourceKind, Option<Scope>, usize)>,
    /// The list's sort column/direction (`s` then a column number).
    pub sort: Option<SortSpec>,
    /// Whether the next digit is choosing a sort column.
    pub sort_choosing: bool,
    /// How many columns the list is scrolled to the right (←/→ or h/l)
    /// when its columns don't all fit the screen.
    pub hscroll: usize,
    /// Namespaces reserved to number keys 1-9.
    pub favorites: Favorites,
    /// The active `/` filter, empty means "show everything". Cleared
    /// whenever the resource kind changes.
    pub search: String,
    /// Whether the commands panel (`?`) is open.
    pub show_hints_panel: bool,
    pub icons: icons::IconCache,
    /// Running port-forwards; dropping one stops it.
    pub forwards: Vec<portforward::Forward>,
    /// Rows marked with Space, by `ui::mark_key`; bulk actions apply to
    /// them. They belong to `marked_kind`'s list and are dropped when the
    /// list changes.
    pub marked: std::collections::HashSet<String>,
    pub marked_kind: ResourceKind,
    /// `Ctrl-z`: list only the rows that need a look.
    pub faults_only: bool,
    /// `Ctrl-w`: show the extra columns.
    pub wide: bool,
    /// The views visited, oldest first, and where in that trail we are
    /// (`[` and `]` move along it).
    pub history: Vec<View>,
    pub history_pos: usize,
    /// The view before this one, for `-`.
    pub last_view: Option<View>,
    pub overview_selection: ui::OverviewSelection,
    /// Horizontal scroll into the Overview's category columns.
    pub overview_col_scroll: usize,
    /// Vertical scroll into the column holding the selection.
    pub overview_item_scroll: usize,
}

impl State {
    /// `icons` must be detected after raw mode is on (it queries the
    /// terminal) and before the event loop starts reading stdin.
    pub fn new(icons: icons::IconCache, favorites: Favorites, config: Config) -> Self {
        let keymap = crate::input::keymap::Keymap::from_app_config(&config).0;
        crate::input::keymap::set_current(&keymap);
        State {
            keymap,
            faults_only: config.tables.faults_by_default,
            wide: config.tables.wide_by_default,
            config,
            table_state: TableState::default().with_selected(0),
            mode: Mode::List,
            hovered: None,
            last_click: None,
            info_panel: false,
            info_scroll: 0,
            info_hscroll: 0,
            info_focus: false,
            info_key: String::new(),
            reveal: false,
            replay: None,
            current_kind: ResourceKind::Overview,
            namespace: None,
            scope: None,
            nav_stack: Vec::new(),
            sort: None,
            sort_choosing: false,
            hscroll: 0,
            favorites,
            search: String::new(),
            show_hints_panel: false,
            icons,
            forwards: Vec::new(),
            marked: Default::default(),
            marked_kind: ResourceKind::Overview,
            history: vec![View { kind: ResourceKind::Overview, scope: None }],
            history_pos: 0,
            last_view: None,
            overview_selection: ui::OverviewSelection::Resources,
            overview_col_scroll: 0,
            overview_item_scroll: 0,
        }
    }

    /// Takes a changed config into use: colours, box lines, numbers and keys.
    pub fn reload(&mut self, config: Config) {
        self.config = config;
        crate::config::settings::apply(&self.config);
        self.keymap = crate::input::keymap::Keymap::from_app_config(&self.config).0;
        crate::input::keymap::set_current(&self.keymap);
    }

    fn here(&self) -> View {
        View { kind: self.current_kind, scope: self.scope.clone() }
    }

    /// Notes the current view in the history when it changed since last
    /// looked (a new view drops whatever was ahead of it).
    pub fn record_view(&mut self) {
        let here = self.here();
        if self.history.get(self.history_pos) == Some(&here) {
            return;
        }
        self.last_view = self.history.get(self.history_pos).cloned();
        self.history.truncate(self.history_pos + 1);
        self.history.push(here);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.history_pos = self.history.len() - 1;
    }

    /// Goes to `view` with a clean slate, without adding to the history.
    fn show(&mut self, view: View) {
        self.last_view = Some(self.here());
        self.current_kind = view.kind;
        self.scope = view.scope;
        self.nav_stack.clear();
        self.sort = None;
        self.hscroll = 0;
        self.table_state.select(Some(0));
        self.search.clear();
    }

    /// `[`: the view before this one.
    pub fn history_back(&mut self) {
        if self.history_pos > 0 {
            self.history_pos -= 1;
            self.show(self.history[self.history_pos].clone());
        }
    }

    /// `]`: the view after this one, when we have gone back.
    pub fn history_forward(&mut self) {
        if self.history_pos + 1 < self.history.len() {
            self.history_pos += 1;
            self.show(self.history[self.history_pos].clone());
        }
    }

    /// `-`: the view we were just in, and back again.
    pub fn toggle_last_view(&mut self) {
        if let Some(view) = self.last_view.clone() {
            self.switch_to(view);
        }
    }

    fn switch_to(&mut self, view: View) {
        // `show` notes where we were, so `-` again comes back.
        self.show(view);
        self.record_view();
    }

    /// Switches to another resource kind with a clean slate: no drill-down,
    /// sort, scroll or search carried over.
    /// Goes to `kind` filtered to `search`, remembering where it came from so Esc returns.
    pub fn jump_to(&mut self, kind: ResourceKind, search: String) {
        let selected = self.table_state.selected().unwrap_or(0);
        self.nav_stack.push((self.current_kind, self.scope.take(), selected));
        self.current_kind = kind;
        self.sort = None;
        self.hscroll = 0;
        self.table_state.select(Some(0));
        self.search = search;
    }

    /// Goes to the list of `kind` showing exactly the object `namespace`/`name`.
    pub fn jump_to_object(&mut self, kind: ResourceKind, namespace: Option<&str>, name: &str) {
        let search = match (kind, namespace) {
            (ResourceKind::Nodes, _) => format!("={name}"),
            (_, Some(ns)) => format!("={ns} {name}"),
            (_, None) => format!("=- {name}"),
        };
        self.jump_to(kind, search);
    }

    pub fn switch_kind(&mut self, kind: ResourceKind) {
        self.current_kind = kind;
        self.scope = None;
        self.nav_stack.clear();
        self.sort = None;
        self.hscroll = 0;
        self.table_state.select(Some(0));
        self.search.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(icons::IconCache::halfblocks(), Favorites::default(), Config::default())
    }

    #[test]
    fn switching_kind_clears_drill_down_sort_scroll_and_search() {
        let mut st = state();
        st.scope = Some(Scope::Namespace { name: "kube-system".into() });
        st.nav_stack.push((ResourceKind::Deployments, None, 2));
        st.sort = Some(SortSpec::pressed(None, 1));
        st.hscroll = 3;
        st.search = "core".into();
        st.table_state.select(Some(5));

        st.switch_kind(ResourceKind::Services);

        assert_eq!(st.current_kind, ResourceKind::Services);
        assert!(st.scope.is_none() && st.nav_stack.is_empty() && st.sort.is_none());
        assert_eq!((st.hscroll, st.search.as_str(), st.table_state.selected()), (0, "", Some(0)));
    }

    #[test]
    fn switching_kind_keeps_the_namespace() {
        let mut st = state();
        st.namespace = Some("kube-system".into());
        st.switch_kind(ResourceKind::Pods);
        assert_eq!(st.namespace.as_deref(), Some("kube-system"));
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    fn state() -> State {
        State::new(icons::IconCache::halfblocks(), Favorites::default(), Config::default())
    }

    fn visit(st: &mut State, kind: ResourceKind) {
        st.switch_kind(kind);
        st.record_view();
    }

    #[test]
    fn back_and_forward_walk_the_views_visited() {
        let mut st = state();
        visit(&mut st, ResourceKind::Pods);
        visit(&mut st, ResourceKind::Services);
        st.history_back();
        assert_eq!(st.current_kind, ResourceKind::Pods);
        st.history_back();
        assert_eq!(st.current_kind, ResourceKind::Overview);
        st.history_back();
        assert_eq!(st.current_kind, ResourceKind::Overview, "nothing before the first view");
        st.history_forward();
        st.history_forward();
        assert_eq!(st.current_kind, ResourceKind::Services);
        st.history_forward();
        assert_eq!(st.current_kind, ResourceKind::Services);
    }

    #[test]
    fn going_back_does_not_add_to_the_history() {
        let mut st = state();
        visit(&mut st, ResourceKind::Pods);
        st.history_back();
        st.record_view();
        assert_eq!(st.history.len(), 2);
    }

    #[test]
    fn a_new_view_after_going_back_drops_what_was_ahead() {
        let mut st = state();
        visit(&mut st, ResourceKind::Pods);
        visit(&mut st, ResourceKind::Services);
        st.history_back();
        visit(&mut st, ResourceKind::Jobs);
        let kinds: Vec<_> = st.history.iter().map(|v| v.kind).collect();
        assert_eq!(kinds, [ResourceKind::Overview, ResourceKind::Pods, ResourceKind::Jobs]);
    }

    #[test]
    fn dash_toggles_between_the_last_two_views() {
        let mut st = state();
        visit(&mut st, ResourceKind::Pods);
        visit(&mut st, ResourceKind::Services);
        st.toggle_last_view();
        assert_eq!(st.current_kind, ResourceKind::Pods);
        st.toggle_last_view();
        assert_eq!(st.current_kind, ResourceKind::Services);
    }
}
