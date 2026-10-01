//! The session's own state: what is on screen and how it is narrowed, sorted and
//! scrolled. Owned by the loop, changed by handlers.

use super::*;

/// A place in the app: which list, drilled into what.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct View {
    pub kind: ResourceKind,
    pub scope: Option<Scope>,
}

const HISTORY_LIMIT: usize = 50;

/// One step `q` or Esc undoes: a list drilled into (kind, scope and selection), or
/// another mode left behind by a jump, with the list as it stood. One stack for both,
/// so they unwind in the order they happened.
pub(crate) enum Step {
    List(ResourceKind, Option<Scope>, usize),
    Mode(Box<Mode>, ListSnapshot),
}

/// Enough of the list's state to put it back exactly as it was before a jump.
pub(crate) struct ListSnapshot {
    pub kind: ResourceKind,
    pub scope: Option<Scope>,
    pub search: String,
    pub sort: Option<SortSpec>,
    pub hscroll: usize,
    pub selected: usize,
}

impl State {
    pub(crate) fn list_snapshot(&self) -> ListSnapshot {
        ListSnapshot {
            kind: self.current_kind,
            scope: self.scope.clone(),
            search: self.search.clone(),
            sort: self.sort,
            hscroll: self.hscroll,
            selected: self.table_state.selected().unwrap_or(0),
        }
    }

    pub(crate) fn restore_list(&mut self, snap: ListSnapshot) {
        self.current_kind = snap.kind;
        self.scope = snap.scope;
        self.search = snap.search;
        self.sort = snap.sort;
        self.hscroll = snap.hscroll;
        self.table_state.select(Some(snap.selected));
    }
}

pub(super) struct State {
    /// The config as it stands now; the Settings screen edits it while knav runs.
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
    /// The sidebar and panel as last drawn, which mouse hits are measured against.
    pub chrome: ui::Chrome,
    /// The resource sidebar (`m`): shown, holding the keys, its cursor and the folded categories.
    pub sidebar: bool,
    pub sidebar_focus: bool,
    pub sidebar_cursor: usize,
    pub sidebar_folded: HashSet<&'static str>,
    /// Whether the info view shows a Secret's values; on again whenever the object changes.
    pub reveal: bool,
    /// A key to handle again on the next turn, after a job it waited for.
    pub replay: Option<crossterm::event::KeyEvent>,
    /// What was fetched around an object for the replayed key, by `object_key`.
    pub surroundings: Option<(String, crate::k8s::surroundings::Fetched)>,
    pub current_kind: ResourceKind,
    /// The namespace every namespaced list is narrowed to. Enter on a namespace sets
    /// it, `0` clears it, and it sticks across kinds.
    pub namespace: Option<String>,
    /// What the current list is drilled into, like a Service's Pods.
    pub scope: Option<Scope>,
    /// How to get back out of drill-downs and detours, one step per `q` or Esc.
    pub back_stack: Vec<Step>,
    /// The list's sort column and direction.
    pub sort: Option<SortSpec>,
    /// Whether the next digit is choosing a sort column.
    pub sort_choosing: bool,
    /// The column the sort cursor is on while choosing.
    pub sort_cursor: usize,
    /// How many columns the list is scrolled right when they don't all fit.
    pub hscroll: usize,
    pub favorites: Favorites,
    /// The active `/` filter, empty for everything. Cleared when the kind changes.
    pub search: String,
    /// Whether the help (`?`) is open.
    pub show_hints_panel: bool,
    pub icons: icons::IconCache,
    /// Running port-forwards; dropping one stops it.
    pub forwards: Vec<portforward::Forward>,
    /// Rows marked with Space, by `ui::mark_key`, for bulk actions. They belong to
    /// `marked_kind`'s list and are dropped when it changes.
    pub marked: std::collections::HashSet<String>,
    pub marked_kind: ResourceKind,
    /// `Ctrl-z`: list only the rows that need a look.
    pub faults_only: bool,
    /// `Ctrl-w`: show the extra columns.
    pub wide: bool,
    /// The views visited, oldest first, and where in that trail we are (`[` and `]`).
    pub history: Vec<View>,
    pub history_pos: usize,
    /// The view before this one, for `-`.
    pub last_view: Option<View>,
    pub overview_selection: ui::OverviewSelection,
    /// Horizontal scroll into the Overview's category columns.
    pub overview_col_scroll: usize,
    /// Vertical scroll into the column holding the selection.
    pub overview_item_scroll: usize,
    /// How far the open extension dashboard is scrolled.
    pub dashboard_scroll: usize,
    /// `--read-only` was given.
    pub read_only_flag: bool,
    /// The context this session is on, for `read_only.contexts`.
    pub context: String,
    /// What RBAC allows (`admin`, `read-write`, ...), empty when unknown.
    pub role: String,
    /// Your own commands with a key, for the help: the key and the command's name.
    pub command_hints: Vec<(String, &'static str)>,
    /// Chosen in the Permissions menu when something else decides otherwise, until
    /// knav restarts.
    pub read_only_override: Option<bool>,
}

/// Whether a click on `id` follows the last one closely enough to be a double click,
/// updating `last_click` either way. It takes the field alone, so a handler holding
/// `&mut st.mode` can still call it.
pub(super) fn double_click(last_click: &mut Option<(std::time::Instant, usize)>, id: usize) -> bool {
    let now = std::time::Instant::now();
    let ms = crate::config::tunables::tunables().double_click_ms;
    let again = last_click.is_some_and(|(at, prev)| prev == id && now.duration_since(at) < std::time::Duration::from_millis(ms));
    *last_click = if again { None } else { Some((now, id)) };
    again
}

/// The help's entries for your commands. Their names live as long as knav: hints are
/// `&'static str`, and the config changes rarely.
fn command_hints(config: &Config) -> Vec<(String, &'static str)> {
    crate::app::handlers::custom_hints(&config.commands).into_iter().map(|(key, name)| (key, &*Box::leak(name.into_boxed_str()))).collect()
}

impl State {
    /// `icons` must be detected after raw mode is on, since it queries the terminal,
    /// and before the loop starts reading stdin.
    pub fn new(mut icons: icons::IconCache, favorites: Favorites, config: Config) -> Self {
        icons.set_enabled(config.ui.icons);
        let keymap = crate::input::keymap::Keymap::from_app_config(&config).0;
        crate::input::keymap::set_current(&keymap);
        let command_hints = command_hints(&config);
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
            chrome: ui::Chrome::default(),
            info_key: String::new(),
            sidebar: false,
            sidebar_focus: false,
            sidebar_cursor: 0,
            sidebar_folded: crate::app::sidebar::folded_by_default(),
            reveal: true,
            replay: None,
            surroundings: None,
            current_kind: ResourceKind::Overview,
            namespace: None,
            scope: None,
            back_stack: Vec::new(),
            sort: None,
            sort_choosing: false,
            sort_cursor: 0,
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
            dashboard_scroll: 0,
            read_only_flag: false,
            context: String::new(),
            role: String::new(),
            command_hints,
            read_only_override: None,
        }
    }

    /// Whether changes to the cluster are blocked.
    pub fn read_only(&self) -> bool {
        self.read_only_override.unwrap_or_else(|| self.read_only_flag || self.config.read_only.applies_to(&self.context))
    }

    /// Shows why a change was refused, when read-only. Returns whether it was.
    pub fn refuse_if_read_only(&mut self) -> bool {
        if !self.read_only() {
            return false;
        }
        let back = std::mem::replace(&mut self.mode, Mode::List);
        self.mode = Mode::Notice { text: "Read-only mode is on".into(), tone: crate::ops::NoticeTone::Info, back: Box::new(back) };
        true
    }

    /// Takes a changed config into use: colours, box lines, numbers and keys.
    pub fn reload(&mut self, config: Config) {
        self.command_hints = command_hints(&config);
        self.config = config;
        crate::app::settings::apply(&self.config);
        self.icons.set_enabled(self.config.ui.icons);
        self.keymap = crate::input::keymap::Keymap::from_app_config(&self.config).0;
        crate::input::keymap::set_current(&self.keymap);
    }

    fn here(&self) -> View {
        View { kind: self.current_kind, scope: self.scope.clone() }
    }

    /// Notes the current view in the history when it changed. A new view drops
    /// whatever was ahead of it.
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
        self.back_stack.clear();
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
        self.show(view);
        self.record_view();
    }

    /// Goes to `kind` filtered to `search`, remembering where it came from so Esc returns.
    pub fn jump_to(&mut self, kind: ResourceKind, search: String) {
        let selected = self.table_state.selected().unwrap_or(0);
        self.back_stack.push(Step::List(self.current_kind, self.scope.take(), selected));
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

    /// Switches to another kind with a clean slate: no drill-down, sort, scroll or search.
    pub fn switch_kind(&mut self, kind: ResourceKind) {
        self.current_kind = kind;
        self.scope = None;
        self.back_stack.clear();
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
    fn a_second_click_on_the_same_id_right_after_is_a_double_click() {
        let mut last: Option<(std::time::Instant, usize)> = None;
        assert!(!double_click(&mut last, 7), "the first click never is");
        assert!(double_click(&mut last, 7), "the second, on the same id, right after, is");
        assert!(last.is_none(), "used up, so a third click starts over");
    }

    #[test]
    fn read_only_refuses_with_a_notice_that_goes_back() {
        let mut st = state();
        assert!(!st.refuse_if_read_only());
        assert!(matches!(st.mode, Mode::List));
        st.config.read_only.enabled = true;
        assert!(st.refuse_if_read_only());
        assert!(matches!(&st.mode, Mode::Notice { back, .. } if matches!(**back, Mode::List)));
        let mut flagged = state();
        flagged.read_only_flag = true;
        assert!(flagged.read_only(), "--read-only holds whatever Settings says");
    }

    #[test]
    fn a_context_pattern_applies_as_soon_as_it_is_set() {
        let mut st = state();
        st.context = "prod-eu".into();
        assert!(!st.read_only());
        st.config.read_only.contexts = vec!["prod*".into()];
        assert!(st.read_only());
        st.read_only_override = Some(false);
        assert!(!st.read_only(), "the Permissions menu wins until knav restarts");
    }

    #[test]
    fn a_click_on_a_different_id_is_not_a_double_click() {
        let mut last: Option<(std::time::Instant, usize)> = None;
        double_click(&mut last, 1);
        assert!(!double_click(&mut last, 2));
    }

    #[test]
    fn switching_kind_clears_drill_down_sort_scroll_and_search() {
        let mut st = state();
        st.scope = Some(Scope::Namespace { name: "kube-system".into() });
        st.back_stack.push(Step::List(ResourceKind::Deployments, None, 2));
        st.sort = Some(SortSpec::pressed(None, 1));
        st.hscroll = 3;
        st.search = "core".into();
        st.table_state.select(Some(5));

        st.switch_kind(ResourceKind::Services);

        assert_eq!(st.current_kind, ResourceKind::Services);
        assert!(st.scope.is_none() && st.back_stack.is_empty() && st.sort.is_none());
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
