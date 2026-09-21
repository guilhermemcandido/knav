//! Everything drawn on screen, split by concern (`theme`, `tables`, `overview`,
//! `columns`, `menu`, `popups`, `spec`, `logs`). This file owns the shared
//! types (`Rows`, `Overlay`) and the top-level `draw`.

use std::collections::HashMap;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, TableState, Wrap},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::config::TimestampFormat;
use crate::icons::IconCache;
use crate::k8s::{
    ContainerInfo, ContainerStatusKind, CrdInfo, DeploymentRow, EventEntry, EventFilter, GenericRow, NodeRow, Overview, PodRow, ResourceKind,
};

mod columns;
mod header;
mod logs;
mod menu;
mod overview;
mod popups;
mod spec;
mod tables;
mod theme;

pub use self::columns::*;
pub use self::header::*;
pub use self::logs::*;
pub use self::menu::*;
pub use self::overview::*;
pub use self::popups::*;
pub use self::spec::*;
pub use self::tables::*;
use self::theme::*;

pub enum Rows<'a> {
    /// The two `usize`s are the horizontal column scroll offset and the
    /// vertical item scroll offset (within whichever column is currently
    /// selected).
    Overview(&'a Overview, OverviewSelection, usize, usize),
    Pods(&'a [PodRow]),
    Deployments(&'a [DeploymentRow]),
    /// Nodes get their own specialized columns (CPU/Memory usage right
    /// in the list, not just after drilling into one) instead of the
    /// generic Namespace/Name/Age table every other kind uses.
    Nodes(&'a [NodeRow]),
    /// Every other resource kind — a plain namespace/name/age table,
    /// labeled with the kind so the title bar and log line make sense.
    Generic(&'a [GenericRow], &'static str),
    /// The Custom Resources picker — every discovered CRD kind (or just
    /// one API group's), not yet any specific kind's instances. Each
    /// entry keeps its real index into `Catalog`'s full discovered list
    /// (needed to open the right one on Enter, since this may be a
    /// filtered subset) alongside a heading describing what's shown
    /// ("Custom Resources" for everything, or the group name).
    CrdList(&'a [(usize, CrdInfo)], &'a str),
}

pub struct MenuSection<'a> {
    pub title: &'a str,
    pub tiles: Vec<ResourceKind>,
}

pub enum Overlay<'a> {
    Spec { title: &'a str, items: &'a [TreeItem<'static, String>], state: &'a mut TreeState<String> },
    Containers { title: &'a str, containers: &'a [ContainerInfo], state: &'a mut TableState },
    Logs { title: &'a str, lines: &'a [String], scroll: u16, follow: bool, timestamp_format: TimestampFormat, filter: &'a str, filter_editing: bool },
    Menu { sections: &'a [MenuSection<'a>], selected: (usize, usize) },
    /// A single node's own CPU/Memory/Pods gauges plus the pods actually
    /// scheduled on it — Freelens-style node drill-down. `cpu_usage`/
    /// `memory_usage` are `None` when metrics-server isn't installed,
    /// same "unavailable" fallback as the Overview's own panel.
    NodeDetail {
        name: &'a str,
        cpu_usage: Option<i64>,
        cpu_capacity: i64,
        memory_usage: Option<i64>,
        memory_capacity: i64,
        pod_capacity: i64,
        /// `None` only in the brief window where the node has vanished
        /// from the store between frames (e.g. right after deletion).
        info: Option<&'a crate::k8s::NodeDetailInfo>,
        pods: &'a [PodRow],
        state: &'a mut TableState,
    },
    /// A vim/k9s-style `:` command line with live autocomplete —
    /// `suggestions` are already fuzzy-matched and sorted (see
    /// `command_suggestions` in `main.rs`), `selected` is which one
    /// Up/Down has highlighted. Unlike `Search`, this one *does* dim the
    /// background — it's a real modal jump, not a live-narrowing filter
    /// you're meant to keep watching.
    Command { input: &'a str, suggestions: &'a [String], selected: usize },
    /// The kubeconfig context browser (`:ctx` / `C`) — a full-size
    /// table like the Events browser, one row per `(name, cluster,
    /// is_current)`, already filtered. `error` is why the last attempt
    /// to connect to a chosen context failed, if it did.
    Context { items: &'a [(String, String, bool)], total: usize, filter: &'a str, editing: bool, state: &'a mut TableState, error: Option<&'a str> },
    /// The dedicated Events browser, opened by pressing Enter on the
    /// Overview's Events panel — every event (not capped, unlike the
    /// dashboard preview), filterable by severity with a/w/n.
    Events { events: &'a [EventEntry], filter: EventFilter, search: &'a str, editing: bool, state: &'a mut TableState },
    /// One event's full detail — opened by pressing Enter or clicking a
    /// row in the Events browser, since the browser's own MESSAGE column
    /// clips long messages to fit the table.
    EventDetail { entry: &'a EventEntry },
    /// The Overview's Resources panel, opened up: the same cluster-wide
    /// CPU/Memory/Pods gauges, full-size, plus a per-node usage
    /// breakdown — reuses the exact same gauge/table drawing the compact
    /// panel and the Nodes list already use, just with more room.
    ResourcesDetail { overview: &'a Overview },
    /// One category column (Workloads, Config, ...), opened up — its
    /// items laid out as a bigger grid of the exact same cards, for when
    /// a category has more kinds than the compact column can show at
    /// once (e.g. Custom Resources with many discovered groups).
    ColumnDetail { title: &'a str, items: &'a [(&'a str, usize)], selected: usize, row_scroll: usize },
    /// A short result message (e.g. after an edit) — any key closes it.
    Notice { text: &'a str, error: bool },
    /// The `n` namespace picker: every namespace in the cluster with the
    /// number key it already has (if any), for choosing which one to give a
    /// key to. Same table layout as `Context`.
    NamespacePicker { items: &'a [(String, Option<usize>)], total: usize, filter: &'a str, editing: bool, state: &'a mut TableState },
    /// The key picker: keys 1-9 (and the fixed `0` = all) with what each
    /// currently holds, for choosing where a namespace goes.
    Slots { namespace: &'a str, slots: &'a [Option<String>], selected: usize },
    ValueDetail { label: &'a str, value: &'a str },
}

/// The `/` search on the main list: what's typed, and whether it's still
/// being typed (which shows the cursor).
#[derive(Clone, Copy, Default)]
pub struct Search<'a> {
    pub text: &'a str,
    pub editing: bool,
}

/// Mouse hover state: which row it's over, and the raw cursor position
/// (needed to place the floating popup right next to the cursor). Only
/// meaningful for the Pods view — Deployments have no per-row containers.
#[derive(Clone, Copy)]
pub struct Hover {
    pub row: usize,
    pub column: u16,
    pub row_on_screen: u16,
}

/// One breadcrumb segment — `kind` (e.g. "Node", "Pod") in one color,
/// its bracketed `value` (e.g. "worker-1") in another, so the two read
/// as visually distinct without either one shouting. `value` is `None`
/// for segments that are just a label with no specific identifier
/// (`Resources`, `Events`, `Category`).
#[derive(Debug, PartialEq, Eq)]
pub struct BreadcrumbSegment {
    pub kind: String,
    pub value: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    rows: Rows,
    table_state: &mut TableState,
    hover: Option<Hover>,
    // The immediate parent screen, drawn dimmed just underneath `overlay`
    // — e.g. Containers behind Logs, or NodeDetail behind Containers —
    // so what's showing through is what you actually came from, not
    // always the base list. `None` when the current overlay's parent
    // *is* the base list (nothing more to show).
    background: Option<Overlay>,
    overlay: Option<Overlay>,
    // The current screen's keybinding hints and whether the panel
    // showing them is currently toggled open — see `draw_hints`.
    // Suppressed whenever `Command`/`Search` is the active overlay —
    // `hints_for` already returns nothing for either, since neither is
    // really "a screen" with its own commands to look up mid-typing.
    hints: &[(&str, &str)],
    show_hints_panel: bool,
    // The full "how did I get here" path, rendered as a bottom bar on
    // top of everything — e.g. "Nodes › Node: worker-1 › Pod: web-1 ›
    // Container: nginx › Logs".
    breadcrumb: Option<&[BreadcrumbSegment]>,
    icons: &mut IconCache,
    header: &HeaderInfo,
    // The `/` search on the main list: shown in its title and highlighted
    // in the rows.
    search: Search,
) {
    // `Command` is a real modal jump now, so it dims like everything
    // else; `Search` stays undimmed — you're meant to see (and read) the
    // list actually narrowing as you type.
    let dimmed = background.is_some()
        || matches!(
            overlay,
            Some(Overlay::Spec { .. })
                | Some(Overlay::Containers { .. })
                | Some(Overlay::Logs { .. })
                | Some(Overlay::Menu { .. })
                | Some(Overlay::NodeDetail { .. })
                | Some(Overlay::Command { .. })
                | Some(Overlay::Context { .. })
                | Some(Overlay::Notice { .. })
                | Some(Overlay::Slots { .. })
                | Some(Overlay::NamespacePicker { .. })
                | Some(Overlay::Events { .. })
                | Some(Overlay::EventDetail { .. })
                | Some(Overlay::ResourcesDetail { .. })
                | Some(Overlay::ColumnDetail { .. })
                | Some(Overlay::ValueDetail { .. })
        );
    let suppress_hints = matches!(overlay, Some(Overlay::Command { .. }) | Some(Overlay::Context { .. }));

    // Terminals can't literally blur, so a modal "recedes" the usual way
    // these things fake depth in a TUI: mute every color in the
    // background down to gray while something's on top of it, so
    // whatever's in full color is the only thing that reads as "in focus."
    let full = frame.area();
    // The namespace-shortcut line is for the resource lists; the main
    // Overview keeps just the info line.
    let shortcuts_line = !matches!(rows, Rows::Overview(..));
    let body = body_area(full, shortcuts_line);
    // Only the focused list highlights matches; behind a popup it's dimmed.
    let search = if dimmed { Search::default() } else { search };
    draw_header(frame, full, header, shortcuts_line, dimmed);
    match rows {
        Rows::Pods(pods) => {
            // A persistent status line below the table for the
            // keyboard-selected row's container breakdown — always
            // there, keyboard-driven, works regardless of mouse/terminal
            // support.
            let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(body);
            draw_table(frame, chunks[0], pods, table_state, search, dimmed);
            draw_status_line(frame, chunks[1], pods, table_state.selected(), dimmed);

            // The mouse-hover popup is separate from the status line and
            // only appears while actively hovering over a container dot
            // specifically — a real floating box "in front," near the
            // cursor, on top of everything else.
            if !dimmed
                && let Some(hover) = &hover
                && let Some(pod) = pods.get(hover.row)
            {
                draw_hover_popup(frame, pod, hover.column, hover.row_on_screen, full);
            }
        }
        Rows::Deployments(deployments) => {
            draw_deployment_table(frame, body, deployments, table_state, search, dimmed);
        }
        Rows::Nodes(nodes) => {
            draw_nodes_table(frame, body, nodes, table_state, search, dimmed);
        }
        Rows::Overview(overview, selection, col_scroll, item_scroll) => {
            draw_overview(frame, body, overview, selection, col_scroll, item_scroll, dimmed, icons);
        }
        Rows::Generic(rows, label) => {
            draw_generic_table(frame, body, rows, label, table_state, search, dimmed);
        }
        Rows::CrdList(crds, heading) => {
            draw_crd_list_table(frame, body, crds, heading, table_state, search, dimmed);
        }
    }

    if let Some(bg) = background {
        draw_overlay(frame, bg, true, icons);
    }
    if let Some(overlay) = overlay {
        draw_overlay(frame, overlay, false, icons);
    }
    if !suppress_hints && !hints.is_empty() {
        draw_hints(frame, hints, show_hints_panel);
    }
    if let Some(segments) = breadcrumb {
        draw_breadcrumb_bar(frame, segments);
    }
}

/// Dispatches one `Overlay` value to its actual draw function — shared
/// between the focused (topmost, `dimmed: false`) and background
/// (immediate-parent-preview, `dimmed: true`) render passes in `draw`.
/// `dimmed` only actually changes anything for the handful of overlay
/// kinds that can ever be used as a background layer (Containers,
/// NodeDetail, Events, ResourcesDetail) — the rest just ignore it, since
/// they're never drawn as anyone's background.
pub(super) fn draw_overlay(frame: &mut Frame, overlay: Overlay, dimmed: bool, icons: &mut IconCache) {
    match overlay {
        Overlay::Spec { title, items, state } => draw_spec_popup(frame, title, items, state, dimmed),
        Overlay::Containers { title, containers, state } => draw_containers_popup(frame, title, containers, state, dimmed),
        Overlay::Logs { title, lines, scroll, follow, timestamp_format, filter, filter_editing } => {
            draw_logs_popup(frame, title, lines, scroll, follow, timestamp_format, filter, filter_editing)
        }
        Overlay::Menu { sections, selected } => draw_menu_popup(frame, sections, selected),
        Overlay::NodeDetail { name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, info, pods, state } => {
            draw_node_detail_popup(frame, name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, info, pods, state, dimmed)
        }
        Overlay::Command { input, suggestions, selected } => draw_command_bar(frame, input, suggestions, selected),
        Overlay::Context { items, total, filter, editing, state, error } => draw_context_popup(frame, items, total, filter, editing, state, error),
        Overlay::Events { events, filter, search, editing, state } => draw_events_popup(frame, events, filter, search, editing, state, dimmed),
        Overlay::EventDetail { entry } => draw_event_detail_popup(frame, entry),
        Overlay::ResourcesDetail { overview } => draw_resources_detail_popup(frame, overview, dimmed),
        Overlay::ColumnDetail { title, items, selected, row_scroll } => {
            draw_column_detail_popup(frame, title, items, selected, row_scroll, icons)
        }
        Overlay::Notice { text, error } => draw_notice_popup(frame, text, error),
        Overlay::Slots { namespace, slots, selected } => draw_slots_popup(frame, namespace, slots, selected),
        Overlay::NamespacePicker { items, total, filter, editing, state } => draw_namespace_picker(frame, items, total, filter, editing, state),
        Overlay::ValueDetail { label, value } => draw_value_detail_popup(frame, label, value),
    }
}

/// The bottom-row navigation path, e.g. "Nodes>>Node[worker-1]>>
/// Pod[default/web-1]>>Logs[nginx]" — drawn on top of everything
/// (including a dimmed background layer), so "where am I and how did I
/// get here" is always answerable at a glance. Each segment's kind and
/// value get their own color (kind in the app's cyan accent, value in a
/// calmer gray) so they read as visually distinct without either one
/// shouting; `>>` between segments is muted so it doesn't compete with
/// either.
pub(super) fn draw_breadcrumb_bar(frame: &mut Frame, segments: &[BreadcrumbSegment]) {
    let area = frame.area();
    let bar = Rect { x: area.x, y: area.y + area.height.saturating_sub(1), width: area.width, height: 1 };
    frame.render_widget(Clear, bar);

    let kind_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let value_style = Style::default().fg(Color::Gray);
    let punct_style = Style::default().fg(Color::DarkGray);

    let mut spans = vec![Span::raw(" ")];
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(">>", punct_style));
        }
        spans.push(Span::styled(segment.kind.clone(), kind_style));
        if let Some(value) = &segment.value {
            spans.push(Span::styled("[", punct_style));
            spans.push(Span::styled(value.clone(), value_style));
            spans.push(Span::styled("]", punct_style));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), bar);
}

/// The current screen's keybinding hints — kept out of the way until
/// asked for. A small "commands: ?" indicator sits in the top-right
/// corner always (whenever there's anything to show); pressing `?`
/// toggles a bordered panel open just underneath it, off to the side,
/// rather than cluttering the screen with a permanent hint list. Each
/// hint's key and its description get their own color, same reasoning
/// as the breadcrumb's kind/value split — a flat run of same-colored
/// text reads as one undifferentiated blob, not a list of distinct
/// commands.
pub(super) fn draw_hints(frame: &mut Frame, hints: &[(&str, &str)], open: bool) {
    let key_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::Gray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let indicator = Line::from(vec![
        Span::styled(if open { "close" } else { "commands" }, desc_style),
        Span::styled(": ", sep_style),
        Span::styled("?", key_style),
    ]);
    let area = frame.area();
    let indicator_width = (indicator.width() as u16).min(area.width);
    let indicator_rect = Rect { x: area.x + area.width - indicator_width, y: area.y, width: indicator_width, height: 1 };
    frame.render_widget(Clear, indicator_rect);
    frame.render_widget(Paragraph::new(indicator), indicator_rect);

    if !open {
        return;
    }

    let content_width = hints.iter().map(|(key, desc)| key.chars().count() + 2 + desc.chars().count()).max().unwrap_or(0) as u16;
    let panel_width = (content_width + 4).min(area.width);
    let panel_height = (hints.len() as u16 + 2).min(area.height.saturating_sub(1));
    let panel = Rect {
        x: area.x + area.width.saturating_sub(panel_width),
        y: area.y + 1,
        width: panel_width,
        height: panel_height,
    };
    frame.render_widget(Clear, panel);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(" Commands ");
    let inner = block.inner(panel);
    frame.render_widget(block, panel);

    let lines: Vec<Line> = hints
        .iter()
        .map(|(key, desc)| Line::from(vec![Span::styled(*key, key_style), Span::styled(": ", sep_style), Span::styled(*desc, desc_style)]))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// A floating box, horizontally centered with its top edge a quarter of the
/// way down the screen — the `:` command line and its autocomplete list. The top edge stays pinned
/// at the quarter-mark (so the input line doesn't jump as suggestions
/// come and go) and the box grows downward.
pub(super) fn centered_box(area: Rect, height: u16) -> Rect {
    let width = (area.width * 3 / 5).max(20).min(area.width);
    let height = height.min(area.height).max(1);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = (area.y + area.height / 4).min(area.y + area.height.saturating_sub(height));
    Rect { x, y, width, height }
}

/// Places a small box near a screen position, nudged so it never renders
/// past the right/bottom edge of the terminal.
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
