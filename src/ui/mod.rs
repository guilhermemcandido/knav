//! Everything drawn on screen. This file owns the shared types (`Rows`, `Overlay`,
//! `Chrome`) and the top-level `draw`.

use std::collections::{HashMap, HashSet};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, TableState, Wrap},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::config::{LogOrder, TimestampFormat};
use crate::theme::theme;
use crate::ui::icons::IconCache;
use crate::k8s::{
    ContainerInfo, ContainerStatusKind, CrdInfo, DeploymentRow, EventEntry, EventFilter, GenericRow, NodeRow, Overview, PodRow, ResourceKind,
};

mod path_bar;
pub mod icons;
mod columns;
mod dashboard;
mod details;
mod graph;
mod header;
mod health;
mod help;
mod layout;
mod loading;
mod logs;
mod nav;
mod overview;
mod popups;
mod sidebar;
mod spec;
mod tables;
mod style;

pub use self::columns::*;
use self::dashboard::draw_dashboard;
pub use self::nav::*;
pub use self::sidebar::{SIDEBAR_MIN_WIDTH, Sidebar, SidebarRow, beside_sidebar, sidebar_area, sidebar_row_at};

/// The area the relations diagram is drawn in.
pub fn relations_inner(frame_area: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(body_area(frame_area, true))
}
use self::health::*;
pub use self::details::{SidePanel, side_panel_max_scroll, details_max_scroll, list_body, side_panel_width, SIDE_PANEL_MIN_WIDTH};
pub use self::graph::{DEFAULT_ZOOM, Move, graph_hit, layout as graph_layout, neighbor as graph_neighbor, zoom_in, zoom_out};
pub use self::header::*;
pub use self::loading::{Loading, draw_loading};
use self::help::draw_help;
pub use self::layout::{configure_columns, set_data_version};
use self::layout::*;
pub use self::path_bar::SelectedItem;
use self::path_bar::*;
pub use self::logs::*;
use self::overview::*;
use crate::util::text::{format_bytes, truncate};
pub use self::popups::*;
pub use self::spec::*;
pub use self::tables::*;
use self::style::*;
pub use self::style::{border_set, configure_border, mark_key};

pub enum Rows<'a> {
    /// With the column scroll and the item scroll within the selected column.
    Overview(&'a Overview, OverviewSelection, usize, usize),
    Pods(&'a [std::sync::Arc<PodRow>]),
    Deployments(&'a [std::sync::Arc<DeploymentRow>]),
    /// Nodes have their own columns, with CPU and memory usage in the list.
    Nodes(&'a [NodeRow]),
    /// Any other kind: rows, the kind's label, and its extra headers (for an empty list).
    Generic(&'a [std::sync::Arc<GenericRow>], &'static str, &'a [&'static str]),
    /// The Custom Resources picker: CRD kinds with their catalog index, object
    /// counts, and the heading.
    CrdList(&'a [(usize, CrdInfo)], &'a [crate::k8s::Count], &'a str),
    /// An extension dashboard: title, rendered content and scroll.
    Dashboard(&'a str, &'a [Line<'static>], usize),
}

pub struct MenuSection<'a> {
    #[allow(dead_code)]
    pub title: &'a str,
    pub tiles: Vec<ResourceKind>,
}

pub enum Overlay<'a> {
    Spec { title: &'a str, items: &'a [TreeItem<'static, String>], state: &'a mut TreeState<String> },
    Containers { title: &'a str, containers: &'a [ContainerInfo], state: &'a mut TableState, sort: SortState },
    Logs { title: &'a str, lines: &'a [String], scroll: usize, follow: bool, timestamp_format: TimestampFormat, order: LogOrder, filter: &'a str, filter_editing: bool },
    /// A node's gauges and the pods on it. Usage is `None` without metrics-server.
    NodeDetail {
        name: &'a str,
        cpu_usage: Option<i64>,
        cpu_capacity: i64,
        memory_usage: Option<i64>,
        memory_capacity: i64,
        pod_capacity: i64,
        /// `None` briefly when the node vanished between frames.
        info: Option<&'a crate::k8s::NodeDetailInfo>,
        pods: &'a [std::sync::Arc<PodRow>],
        state: &'a mut TableState,
        sort: SortState,
        search: Search<'a>,
    },
    /// The `:` command line with its sorted suggestions. Unlike `Search` it dims the page.
    Command { input: &'a str, suggestions: &'a [SuggestionView], selected: usize },
    /// The context browser: `(name, cluster, is_current)` rows, already filtered, and why
    /// the last connect failed.
    Context { items: &'a [(String, String, bool)], total: usize, filter: &'a str, editing: bool, state: &'a mut TableState, error: Option<&'a str>, sort: SortState },
    /// The Events browser: every event, filterable by severity with a/w/n.
    Events { events: &'a [EventEntry], filter: EventFilter, search: &'a str, editing: bool, state: &'a mut TableState, sort: SortState },
    /// One event in full, since the browser clips long messages.
    EventDetail { entry: &'a EventEntry },
    /// The Resources panel opened up: cluster gauges plus per-node usage.
    ResourcesDetail { overview: &'a Overview, nodes: &'a [crate::k8s::NodeRow] },
    /// One category column opened into a bigger grid.
    ColumnDetail { title: &'a str, items: &'a [(&'a str, usize)], health: &'a std::collections::HashMap<&'static str, crate::k8s::Health>, selected: usize, row_scroll: usize },
    /// A short result message; any key closes it.
    Notice { text: &'a str, tone: crate::ops::NoticeTone },
    /// A yes/no question about a destructive action.
    Confirm { spec: &'a crate::ops::actions::ConfirmSpec },
    /// A background job: what, for how long, and progress (`total` 0 when unknown).
    Working { title: &'a str, elapsed: std::time::Duration, done: usize, total: usize, cancellable: bool },
    Details { title: &'a str, sections: &'a [crate::k8s::details::Section], scroll: usize, hscroll: usize },
    Relations { title: &'a str, graph: &'a crate::k8s::relations::Graph, selected: usize, zoom: usize },
    Settings { tab: SettingsTab, rows: &'a [SettingView], layout: &'a [LayoutRow], state: &'a mut TableState, error: Option<&'a str>, capture: Option<CaptureView> },
    Extensions { rows: &'a [ExtensionRow], state: &'a mut TableState, error: Option<&'a str>, filter: &'a str, filter_editing: bool },
    ThemePicker { entries: &'a [crate::theme::ThemeEntry], state: &'a mut TableState, saved: &'a str },
    Shell { title: &'a str, screen: &'a vt100::Screen, exited: bool },
    Yaml { title: &'a str, text: &'a str, scroll: usize },
    PortForward { title: &'a str, form: &'a crate::ops::portforward::PortForm },
    /// A number being typed.
    Prompt { title: &'a str, value: &'a str, hint: &'a str },
    /// The `n` namespace picker: every namespace with its number key, if any.
    NamespacePicker { items: &'a [(String, Option<usize>)], total: usize, filter: &'a str, editing: bool, state: &'a mut TableState, sort: SortState },
    /// Keys 1-9 (and `0` for all) with what each holds, to choose one for a namespace.
    Slots { namespace: &'a str, slots: &'a [Option<String>], selected: usize },
    ValueDetail { label: &'a str, value: &'a str },
}

/// The `/` search on the main list, and whether it is still being typed.
#[derive(Clone, Copy, Default)]
pub struct Search<'a> {
    pub text: &'a str,
    pub editing: bool,
}

const COMMAND_BAR_HEIGHT: u16 = 3;

/// How the main list is sorted, and whether a column is being chosen (`s`).
#[derive(Clone, Copy, Default)]
pub struct SortState {
    pub column: Option<usize>,
    pub descending: bool,
    pub choosing: bool,
    /// The column the cursor is on while choosing.
    pub cursor: Option<usize>,
}

impl SortState {
    pub fn spec(self) -> Option<crate::k8s::sort::SortSpec> {
        self.column.map(|column| crate::k8s::sort::SortSpec { column, descending: self.descending })
    }
}

/// The row the mouse is over and its position, to place the Pods hover popup.
#[derive(Clone, Copy)]
pub struct Hover {
    pub row: usize,
    pub column: u16,
    pub row_on_screen: u16,
}

/// The line in the middle of an empty list, saying why when a search or the
/// faults filter emptied it.
pub(super) fn empty_list_message(label: &str, search: &str, faults_only: bool) -> Line<'static> {
    let label = label.to_lowercase();
    if !search.is_empty() {
        Line::styled(format!("No {label} match '{search}'"), Style::default().fg(theme().warn).add_modifier(Modifier::BOLD))
    } else if faults_only {
        Line::styled(format!("✔ No {label} need attention"), Style::default().fg(theme().ok).add_modifier(Modifier::BOLD))
    } else {
        Line::styled(format!("No {label} found"), Style::default().fg(theme().warn).add_modifier(Modifier::BOLD))
    }
}

pub struct SettingView {
    pub section: &'static str,
    pub label: String,
    pub value: String,
    /// A colour setting's current colour, shown as a swatch.
    pub swatch: Option<Color>,
    /// Set by the config file itself.
    pub customised: bool,
    /// Takes effect the next time knav starts.
    pub restart: bool,
    pub editing: bool,
    /// Shown under the list while selected.
    pub help: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Keys,
    Overview,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 3] = [SettingsTab::General, SettingsTab::Keys, SettingsTab::Overview];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::General => "General",
            SettingsTab::Keys => "Keys",
            SettingsTab::Overview => "Layout",
        }
    }

    pub fn next(self) -> SettingsTab {
        Self::ALL[(Self::ALL.iter().position(|t| *t == self).unwrap_or(0) + 1) % Self::ALL.len()]
    }

    pub fn previous(self) -> SettingsTab {
        Self::ALL[(Self::ALL.iter().position(|t| *t == self).unwrap_or(0) + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

pub struct LayoutRow {
    pub name: String,
    /// Its place from the left, from 1.
    pub number: usize,
    pub hidden: bool,
}

/// One row of the Extensions screen.
pub struct ExtensionRow {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Bundled with knav, versus added from `~/.config/knav/extensions/`.
    pub bundled: bool,
    /// Whether the cluster has any of its kinds; `None` while it is off or broken.
    pub present: Option<bool>,
    pub error: Option<String>,
}

pub struct CaptureView {
    pub label: String,
    pub keys: Vec<String>,
    pub stage: CaptureStage,
    pub problem: Option<String>,
}

pub enum CaptureStage {
    Menu,
    Waiting,
    /// A key was pressed, to replace the others or be added to them.
    Confirm { key: String, replace: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionIcon {
    Kind(crate::k8s::ResourceKind),
    /// A drawn icon that isn't a resource (`door`, `bell`, `switch`).
    Named(&'static str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestionView {
    pub label: String,
    pub icon: SuggestionIcon,
}

/// One path segment: a kind ("Node") and its value ("worker-1"), each in its own
/// colour. `value` is `None` for plain labels.
#[derive(Debug, PartialEq, Eq)]
pub struct PathSegment {
    pub kind: String,
    pub value: Option<String>,
}

#[allow(clippy::too_many_arguments)]
/// What sits around the main content this frame. The app builds it before
/// drawing and hit-tests the mouse against the same one.
#[derive(Default)]
pub struct Chrome {
    pub sidebar: Option<Sidebar>,
    pub panel: Option<SidePanel>,
    /// The list has the keys while a panel is open beside it.
    pub list_focused: bool,
    /// The sidebar has the keys, so what is beside it shows no selection.
    pub content_unfocused: bool,
}

/// Whether `rows` is a resource list, with selectable rows and a side panel.
fn is_list_kind(rows: &Rows) -> bool {
    !matches!(rows, Rows::Overview(..) | Rows::Dashboard(..))
}

pub fn draw(
    frame: &mut Frame,
    rows: Rows,
    table_state: &mut TableState,
    hover: Option<Hover>,
    // The parent screen, drawn dimmed under `overlay`; `None` for the base list.
    background: Option<Overlay>,
    overlay: Option<Overlay>,
    // The screen's key hints, and whether the help is open.
    hints: &[(&str, &str)],
    show_hints_panel: bool,
    // The breadcrumb path in the bottom bar, like "Nodes › worker-1 › Logs".
    path: Option<&[PathSegment]>,
    icons: &mut IconCache,
    header: &HeaderInfo,
    search: Search,
    sort: SortState,
    // How far the main list is scrolled right, clamped here to what its columns allow.
    hscroll: &mut usize,
    marked: &HashSet<String>,
    chrome: &Chrome,
) {
    // Popups dim what's behind them. The `:` line only softens it (below), and `/`
    // search leaves the list in full colour.
    let dimmed = background.is_some()
        || show_hints_panel
        || matches!(
            overlay,
            Some(Overlay::Spec { .. })
                | Some(Overlay::Containers { .. })
                | Some(Overlay::Logs { .. })
                | Some(Overlay::NodeDetail { .. })
                | Some(Overlay::Context { .. })
                | Some(Overlay::Notice { .. })
                | Some(Overlay::Confirm { .. })
                | Some(Overlay::Working { .. })
                | Some(Overlay::Prompt { .. })
                | Some(Overlay::Yaml { .. })
                | Some(Overlay::Shell { .. })
                | Some(Overlay::ThemePicker { .. })
                | Some(Overlay::Settings { .. })
                | Some(Overlay::Extensions { .. })
                | Some(Overlay::PortForward { .. })
                | Some(Overlay::Slots { .. })
                | Some(Overlay::NamespacePicker { .. })
                | Some(Overlay::Events { .. })
                | Some(Overlay::EventDetail { .. })
                | Some(Overlay::ResourcesDetail { .. })
                | Some(Overlay::ColumnDetail { .. })
                | Some(Overlay::ValueDetail { .. })
        );
    let suppress_hints = matches!(overlay, Some(Overlay::Command { .. }) | Some(Overlay::Context { .. }));

    let full = frame.area();
    // Room for the three badges at the right of the list's top border, so the
    // search text stays put as they come and go.
    let title_reserve = if !is_list_kind(&rows) || dimmed { 0 } else { [" sorting ", " faults ", " wide "].iter().map(|b| b.chars().count() as u16 + 1).sum::<u16>() + 2 };
    let look = ListLook { dimmed, focused: chrome.list_focused, unfocused: chrome.content_unfocused, title_reserve };
    // The namespace shortcut line is for resource lists only.
    let shortcuts_line = is_list_kind(&rows);
    let side = sidebar_area(full, shortcuts_line, chrome);
    sidebar::draw_sidebar(frame, side, dimmed, chrome);
    let body = beside_sidebar(full, shortcuts_line, chrome);
    let full_body = body;
    let body = if is_list_kind(&rows) { Rect { width: body.width - side_panel_width(full.width, chrome).min(body.width), ..body } } else { body };
    // The `:` command line takes a bar under the header and pushes the list down.
    let (command_bar, body) = match &overlay {
        Some(Overlay::Command { .. }) if body.height > COMMAND_BAR_HEIGHT + 4 => (
            Some(Rect { height: COMMAND_BAR_HEIGHT, ..body }),
            Rect { y: body.y + COMMAND_BAR_HEIGHT, height: body.height - COMMAND_BAR_HEIGHT, ..body },
        ),
        _ => (None, body),
    };
    // Only the focused list highlights matches; behind a popup it's dimmed.
    let search = if dimmed { Search::default() } else { search };
    // Header lines align with the boxes' content as it sits without the sidebar,
    // so opening it doesn't push them aside.
    let header_left = 1 + match &rows {
        Rows::Overview(overview, ..) => columns_span(body_area(full, false), overview.catalog.len()).x,
        _ => body_area(full, true).x,
    };
    // Digits only switch namespace on the plain list, so the line reads as
    // unavailable under an overlay or while picking a sort column.
    draw_header(frame, full, header_left, header, shortcuts_line, sort.choosing || overlay.is_some(), dimmed);
    // The selected row, named at the end of the path bar while nothing is on top.
    let selected_row = table_state.selected();
    let selected_pod: Option<SelectedItem> = match &overlay {
        Some(Overlay::NodeDetail { pods, state, .. }) if background.is_none() => {
            state.selected().and_then(|i| pods.get(i)).map(|r| SelectedItem::from_pod(r))
        }
        _ if dimmed => None,
        _ => match &rows {
            Rows::Pods(pods) => selected_row.and_then(|i| pods.get(i)).map(|r| SelectedItem::from_pod(r)),
            Rows::Deployments(deployments) => selected_row.and_then(|i| deployments.get(i)).map(|r| SelectedItem::from_deployment(r)),
            Rows::Nodes(nodes) => selected_row.and_then(|i| nodes.get(i)).map(SelectedItem::from_node),
            Rows::Generic(rows, _, _) => selected_row.and_then(|i| rows.get(i)).map(|r| SelectedItem::from_generic(r)),
            Rows::CrdList(crds, _, _) => selected_row.and_then(|i| crds.get(i)).map(|(_, crd)| SelectedItem::from_crd(crd)),
            Rows::Overview(..) | Rows::Dashboard(..) => None,
        },
    };
    let is_overview = matches!(rows, Rows::Overview(..));
    if is_list_kind(&rows) && overlay.is_none() {
        details::draw_side_panel(frame, full_body, chrome);
    }
    let empty_message = match &rows {
        Rows::Pods(r) if r.is_empty() => Some("pods"),
        Rows::Deployments(r) if r.is_empty() => Some("deployments"),
        Rows::Nodes(r) if r.is_empty() => Some("nodes"),
        Rows::Generic(r, label, _) if r.is_empty() => Some(*label),
        Rows::CrdList(r, _, heading) if r.is_empty() => Some(*heading),
        _ => None,
    }
    .map(|label| empty_list_message(label, search.text, header.faults_only));
    match rows {
        Rows::Pods(pods) => {
            draw_table(frame, body, pods, table_state, search, sort, hscroll, marked, header.wide, look);

            // Floats near the cursor while it is over a container dot.
            if !dimmed
                && let Some(hover) = &hover
                && let Some(pod) = pods.get(hover.row)
            {
                draw_hover_popup(frame, pod, hover.column, hover.row_on_screen, full);
            }
        }
        Rows::Deployments(deployments) => {
            draw_deployment_table(frame, body, deployments, table_state, search, sort, hscroll, marked, header.wide, look);
        }
        Rows::Nodes(nodes) => {
            draw_nodes_table(frame, body, nodes, table_state, search, sort, hscroll, marked, header.wide, look);
        }
        Rows::Overview(overview, selection, col_scroll, item_scroll) => {
            // With the keys in the sidebar nothing on Home is selected.
            let selection = if chrome.content_unfocused { OverviewSelection::Header(usize::MAX) } else { selection };
            draw_overview(frame, body, overview, selection, col_scroll, item_scroll, dimmed, icons);
        }
        Rows::Generic(rows, label, kind_headers) => {
            draw_generic_table(frame, body, rows, label, kind_headers, table_state, search, sort, hscroll, marked, header.wide, look);
        }
        Rows::CrdList(crds, counts, heading) => {
            draw_crd_list_table(frame, body, crds, counts, heading, table_state, search, sort, hscroll, look);
        }
        Rows::Dashboard(title, content, scroll) => {
            draw_dashboard(frame, body, title, content, scroll, dimmed);
        }
    }

    if let Some(message) = empty_message.filter(|_| !dimmed) {
        // The middle of the list, inside the border and below the header row.
        let inner = Rect { x: body.x + 1, y: body.y + 2, width: body.width.saturating_sub(2), height: body.height.saturating_sub(3) };
        if inner.height > 0 {
            frame.render_widget(Paragraph::new(message).centered(), Rect { y: inner.y + inner.height / 2, height: 1, ..inner });
        }
    }

    // What is filtering or widening the list, in the title bar's corner.
    if !is_overview && (header.faults_only || header.wide || sort.choosing) && !dimmed {
        let mut badges = Vec::new();
        if sort.choosing {
            badges.push(Span::styled(" sorting ", Style::default().bg(theme().accent).fg(crate::theme::on(theme().accent)).add_modifier(Modifier::BOLD)));
        }
        if header.faults_only {
            badges.push(Span::styled(" faults ", Style::default().bg(theme().warn).fg(crate::theme::on(theme().warn)).add_modifier(Modifier::BOLD)));
        }
        if header.wide {
            badges.push(Span::styled(" wide ", Style::default().bg(theme().key).fg(crate::theme::on(theme().key)).add_modifier(Modifier::BOLD)));
        }
        let width: u16 = badges.iter().map(|b| b.width() as u16 + 1).sum();
        let rect = Rect { x: body.x + body.width.saturating_sub(width + 2), y: body.y, width: width.min(body.width), height: 1 };
        let mut spans = Vec::new();
        for b in badges {
            spans.push(b);
            spans.push(Span::raw(" "));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), rect);
    }

    if let Some(bg) = background {
        draw_overlay(frame, bg, true, icons);
    }
    // Behind the command line the page recedes a little, without vanishing.
    if matches!(overlay, Some(Overlay::Command { .. })) {
        frame.buffer_mut().set_style(full, Style::default().add_modifier(Modifier::DIM));
    }
    match overlay {
        Some(Overlay::Command { input, suggestions, selected }) => {
            // Under the header, or over the top of the list on a short screen.
            let bar = command_bar.unwrap_or(Rect { height: COMMAND_BAR_HEIGHT.min(body.height), ..body });
            draw_command_line(frame, bar, input, suggestions, selected, icons);
        }
        Some(overlay) => draw_overlay(frame, overlay, false, icons),
        None => {}
    }
    if !suppress_hints && (!hints.is_empty() || show_hints_panel) {
        draw_hints(frame, hints, show_hints_panel, &header.namespace_slots, shortcuts_line);
    }
    if let Some(segments) = path {
        draw_path_bar(frame, segments, selected_pod);
    }
    paint_theme_base(frame);
}

/// The theme's background and text colour wherever nothing else set one.
/// Runs last, since popups clear their area.
fn paint_theme_base(frame: &mut Frame) {
    let (background, foreground) = (theme().background, theme().foreground);
    if background == Color::Reset && foreground == Color::Reset {
        return;
    }
    for cell in frame.buffer_mut().content.iter_mut() {
        if background != Color::Reset && cell.bg == Color::Reset {
            cell.bg = background;
        }
        if foreground != Color::Reset && cell.fg == Color::Reset {
            cell.fg = foreground;
        }
    }
}

/// Draws one overlay, focused or as a dimmed background. `dimmed` only matters for
/// overlays that can be backgrounds.
pub(super) fn draw_overlay(frame: &mut Frame, overlay: Overlay, dimmed: bool, icons: &mut IconCache) {
    match overlay {
        Overlay::Spec { title, items, state } => draw_spec_popup(frame, title, items, state, dimmed),
        Overlay::Containers { title, containers, state, sort } => draw_containers_popup(frame, title, containers, state, sort, dimmed),
        Overlay::Logs { title, lines, scroll, follow, timestamp_format, order, filter, filter_editing } => {
            draw_logs_popup(frame, title, lines, scroll, follow, timestamp_format, order, filter, filter_editing)
        }
        Overlay::NodeDetail { name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, info, pods, state, sort, search } => {
            draw_node_detail_popup(frame, name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, info, pods, state, sort, search, dimmed)
        }
        // Drawn by `draw` itself, in its own bar.
        Overlay::Command { .. } => {}
        Overlay::Context { items, total, filter, editing, state, error, sort } => draw_context_popup(frame, items, total, filter, editing, state, error, sort),
        Overlay::Events { events, filter, search, editing, state, sort } => draw_events_popup(frame, events, filter, search, editing, state, sort, dimmed),
        Overlay::EventDetail { entry } => draw_event_detail_popup(frame, entry),
        Overlay::ResourcesDetail { overview, nodes } => draw_resources_detail_popup(frame, overview, nodes, dimmed),
        Overlay::ColumnDetail { title, items, health, selected, row_scroll } => {
            draw_column_detail_popup(frame, title, items, health, selected, row_scroll, icons)
        }
        Overlay::Notice { text, tone } => draw_notice_popup(frame, text, tone),
        Overlay::Details { title, sections, scroll, hscroll } => details::draw_details(frame, title, sections, scroll, hscroll),
        Overlay::Relations { title, graph, selected, zoom } => draw_relations(frame, title, graph, selected, zoom),
        Overlay::Settings { tab, rows, layout, state, error, capture } => draw_settings(frame, popups::SettingsView { tab, rows, layout, error, capture: capture.as_ref() }, state),
        Overlay::Extensions { rows, state, error, filter, filter_editing } => draw_extensions_popup(frame, rows, filter, filter_editing, error, state),
        Overlay::ThemePicker { entries, state, saved } => draw_theme_picker(frame, entries, state, saved),
        Overlay::Shell { title, screen, exited } => draw_shell_popup(frame, title, screen, exited),
        Overlay::Yaml { title, text, scroll } => draw_yaml_popup(frame, title, text, scroll),
        Overlay::PortForward { title, form } => draw_port_forward_popup(frame, title, form),
        Overlay::Confirm { spec } => draw_confirm_popup(frame, spec),
        Overlay::Working { title, elapsed, done, total, cancellable } => draw_working_popup(frame, title, elapsed, done, total, cancellable),
        Overlay::Prompt { title, value, hint } => draw_prompt_popup(frame, title, value, hint),
        Overlay::Slots { namespace, slots, selected } => draw_slots_popup(frame, namespace, slots, selected),
        Overlay::NamespacePicker { items, total, filter, editing, state, sort } => draw_namespace_picker(frame, items, total, filter, editing, state, sort),
        Overlay::ValueDetail { label, value } => draw_value_detail_popup(frame, label, value),
    }
}

/// The help indicator at the top right, and the help itself when `open`.
pub(super) fn draw_hints(frame: &mut Frame, hints: &[(&str, &str)], open: bool, slots: &[Option<String>], shortcuts_line: bool) {
    let key_style = Style::default().fg(theme().highlight).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(theme().text_soft);
    let sep_style = Style::default().fg(theme().muted);

    let indicator = Line::from(vec![
        Span::styled(if open { "close" } else { "help" }, desc_style),
        Span::styled(": ", sep_style),
        Span::styled(crate::input::keymap::keys_now("help").first().map(|k| crate::input::keymap::glyph(k)).unwrap_or_else(|| "?".into()), key_style),
    ]);
    let area = frame.area();
    let indicator_width = (indicator.width() as u16).min(area.width);
    let indicator_rect = Rect { x: area.x + area.width - indicator_width, y: area.y, width: indicator_width, height: 1 };
    frame.render_widget(Clear, indicator_rect);
    frame.render_widget(Paragraph::new(indicator), indicator_rect);

    if !open {
        return;
    }

    // Some widgets keep their colours when dimmed (warning rows, selection bars),
    // so every cell under the help is muted as well.
    for cell in &mut frame.buffer_mut().content {
        cell.set_fg(theme().dim).set_bg(theme().background);
        cell.modifier = Modifier::DIM;
    }
    draw_help(frame, hints, slots, shortcuts_line);
}

/// A floating box, centred, its top a quarter down the screen. The top stays put so
/// the input doesn't jump as suggestions change.
pub(super) fn centered_box(area: Rect, height: u16) -> Rect {
    let width = (area.width * 3 / 5).max(20).min(area.width);
    let height = height.min(area.height).max(1);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = (area.y + area.height / 4).min(area.y + area.height.saturating_sub(height));
    Rect { x, y, width, height }
}

/// A small box near a screen position, kept inside the terminal.
pub(super) fn popup_near(column: u16, row: u16, width: u16, height: u16, bounds: Rect) -> Rect {
    let x = (column + 1).min(bounds.width.saturating_sub(width));
    let y = (row + 1).min(bounds.height.saturating_sub(height));
    Rect { x, y, width: width.min(bounds.width), height: height.min(bounds.height) }
}

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

#[cfg(test)]
mod empty_message_tests {
    use super::*;

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn an_empty_list_says_nothing_was_found_in_yellow() {
        let line = empty_list_message("PVCs", "", false);
        assert_eq!(text(&line), "No pvcs found");
        assert_eq!(line.style.fg, Some(theme().warn));
    }

    #[test]
    fn a_search_that_matches_nothing_says_so() {
        assert_eq!(text(&empty_list_message("Pods", "zzz", false)), "No pods match 'zzz'");
    }

    #[test]
    fn an_empty_faults_list_is_good_news_in_green() {
        let line = empty_list_message("Pods", "", true);
        assert_eq!(text(&line), "✔ No pods need attention");
        assert_eq!(line.style.fg, Some(theme().ok));
    }
}

#[cfg(test)]
mod help_backdrop_tests {
    use super::*;

    #[test]
    fn everything_behind_the_open_help_is_muted() {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| {
                let bright = Style::default().fg(Color::Yellow).bg(Color::Blue).add_modifier(Modifier::BOLD);
                frame.render_widget(Paragraph::new(vec![Line::styled("Warning x".repeat(20), bright); 40]), frame.area());
                draw_hints(frame, &[], true, &[], false);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        // The top-left corner is never under the centred help box.
        let corner = &buffer[(0, 0)];
        assert_eq!((corner.fg, corner.bg, corner.modifier), (theme().dim, theme().background, Modifier::DIM));
    }
}
