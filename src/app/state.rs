//! The interactive session's own state: what is on screen and how it is
//! narrowed, sorted and scrolled. Owned by the loop, changed by handlers.

use super::*;

pub(super) struct State {
    pub table_state: TableState,
    pub mode: Mode,
    pub hovered: Option<ui::Hover>,
    pub current_kind: ResourceKind,
    /// The namespace every namespaced list is narrowed to (`Enter` on a
    /// namespace sets it, `0` clears it) — sticks across kind switches.
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
    /// The active `/` filter — empty means "show everything". Cleared
    /// whenever the resource kind changes.
    pub search: String,
    /// Mouse reporting makes hover/click work but stops the terminal's own
    /// text selection, so `c` toggles it.
    pub mouse_capture_enabled: bool,
    /// Whether the commands panel (`?`) is open.
    pub show_hints_panel: bool,
    pub icons: icons::IconCache,
    pub overview_selection: ui::OverviewSelection,
    /// Horizontal scroll into the Overview's category columns.
    pub overview_col_scroll: usize,
    /// Vertical scroll into the column holding the selection.
    pub overview_item_scroll: usize,
}

impl State {
    /// Must be created after raw mode is on (the icon cache queries the
    /// terminal) and before the event loop starts reading stdin.
    pub fn new(active_context: &str) -> Self {
        State {
            table_state: TableState::default().with_selected(0),
            mode: Mode::List,
            hovered: None,
            current_kind: ResourceKind::Overview,
            namespace: None,
            scope: None,
            nav_stack: Vec::new(),
            sort: None,
            sort_choosing: false,
            hscroll: 0,
            favorites: Favorites::load(active_context),
            search: String::new(),
            mouse_capture_enabled: true,
            show_hints_panel: false,
            icons: icons::IconCache::detect(),
            overview_selection: ui::OverviewSelection::Resources,
            overview_col_scroll: 0,
            overview_item_scroll: 0,
        }
    }

    /// Switches to another resource kind with a clean slate: no drill-down,
    /// sort, scroll or search carried over.
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
