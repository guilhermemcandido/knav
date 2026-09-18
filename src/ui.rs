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
    pub tiles: &'a [ResourceKind],
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
    Command { input: &'a str, suggestions: &'a [ResourceKind], selected: usize },
    /// The dedicated Events browser, opened by pressing Enter on the
    /// Overview's Events panel — every event (not capped, unlike the
    /// dashboard preview), filterable by severity with a/w/n.
    Events { events: &'a [EventEntry], filter: EventFilter, state: &'a mut TableState },
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
    /// The `/`/`f` live-filter input bar — still doesn't dim the
    /// background, since you're meant to see the list narrowing as you
    /// type, unlike `Command`'s modal jump.
    Search { query: &'a str, matches: usize },
    /// A tree leaf's full, untruncated value — `v` in `Mode::Spec`,
    /// since a long value (a cert blob, a long annotation) just gets
    /// silently clipped by the box's width otherwise, with no way to
    /// see the rest of it.
    ValueDetail { label: &'a str, value: &'a str },
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
                | Some(Overlay::Events { .. })
                | Some(Overlay::EventDetail { .. })
                | Some(Overlay::ResourcesDetail { .. })
                | Some(Overlay::ColumnDetail { .. })
                | Some(Overlay::ValueDetail { .. })
        );
    let suppress_hints = matches!(overlay, Some(Overlay::Command { .. }) | Some(Overlay::Search { .. }));

    // Terminals can't literally blur, so a modal "recedes" the usual way
    // these things fake depth in a TUI: mute every color in the
    // background down to gray while something's on top of it, so
    // whatever's in full color is the only thing that reads as "in focus."
    match rows {
        Rows::Pods(pods) => {
            // A persistent status line below the table for the
            // keyboard-selected row's container breakdown — always
            // there, keyboard-driven, works regardless of mouse/terminal
            // support.
            let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(frame.area());
            draw_table(frame, chunks[0], pods, table_state, dimmed);
            draw_status_line(frame, chunks[1], pods, table_state.selected(), dimmed);

            // The mouse-hover popup is separate from the status line and
            // only appears while actively hovering over a container dot
            // specifically — a real floating box "in front," near the
            // cursor, on top of everything else.
            if !dimmed
                && let Some(hover) = &hover
                && let Some(pod) = pods.get(hover.row)
            {
                draw_hover_popup(frame, pod, hover.column, hover.row_on_screen, frame.area());
            }
        }
        Rows::Deployments(deployments) => {
            draw_deployment_table(frame, frame.area(), deployments, table_state, dimmed);
        }
        Rows::Nodes(nodes) => {
            draw_nodes_table(frame, frame.area(), nodes, table_state, dimmed);
        }
        Rows::Overview(overview, selection, col_scroll, item_scroll) => {
            draw_overview(frame, frame.area(), overview, selection, col_scroll, item_scroll, dimmed, icons);
        }
        Rows::Generic(rows, label) => {
            draw_generic_table(frame, frame.area(), rows, label, table_state, dimmed);
        }
        Rows::CrdList(crds, heading) => {
            draw_crd_list_table(frame, frame.area(), crds, heading, table_state, dimmed);
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
fn draw_overlay(frame: &mut Frame, overlay: Overlay, dimmed: bool, icons: &mut IconCache) {
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
        Overlay::Events { events, filter, state } => draw_events_popup(frame, events, filter, state, dimmed),
        Overlay::EventDetail { entry } => draw_event_detail_popup(frame, entry),
        Overlay::ResourcesDetail { overview } => draw_resources_detail_popup(frame, overview, dimmed),
        Overlay::ColumnDetail { title, items, selected, row_scroll } => {
            draw_column_detail_popup(frame, title, items, selected, row_scroll, icons)
        }
        Overlay::Search { query, matches } => draw_search_bar(frame, query, matches),
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
fn draw_breadcrumb_bar(frame: &mut Frame, segments: &[BreadcrumbSegment]) {
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
fn draw_hints(frame: &mut Frame, hints: &[(&str, &str)], open: bool) {
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

/// The color everything in a dimmed "background" layer is muted down
/// to — deliberately darker than plain ANSI `DarkGray` (which most
/// terminals render as a fairly legible mid-gray) plus the `DIM`
/// modifier on top, so a screen sitting behind a popup reads as
/// unmistakably out of focus rather than just "a bit gray."
fn dim_style() -> Style {
    Style::default().fg(Color::Rgb(40, 40, 40)).add_modifier(Modifier::DIM)
}

/// A small floating input box, centered on the screen both ways — mac
/// Spotlight style — used for the `/` filter and `:` command line
/// instead of pinning either to an edge nobody's looking at.
fn centered_input_box(area: Rect) -> Rect {
    centered_box(area, 3)
}

/// Same centering as `centered_input_box`, but for a box that grows —
/// the `:` command line rises as its autocomplete list grows underneath
/// it, since centering a taller box moves its top edge up while its
/// bottom edge moves down, instead of just growing downward off-center.
fn centered_box(area: Rect, height: u16) -> Rect {
    let width = (area.width * 3 / 5).max(20).min(area.width);
    let height = height.min(area.height).max(1);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width, height }
}

/// The `/`/`f` live-filter box — no dimming, since the point is
/// watching the list narrow (right underneath, in the same spot) while
/// you type.
fn draw_search_bar(frame: &mut Frame, query: &str, matches: usize) {
    let bar = centered_input_box(frame.area());
    frame.render_widget(Clear, bar);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title("Search");
    let inner = block.inner(bar);
    frame.render_widget(block, bar);
    let line = Line::from(vec![
        Span::styled(format!("/{query}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(format!("  ({matches} match{})", if matches == 1 { "" } else { "es" }), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

/// A `Terminated` container isn't necessarily a problem — a Job/init
/// container that ran to completion and exited 0 gets this same status
/// kind, distinguished only by `reason` being "Completed" rather than
/// something like "Error"/"OOMKilled". Red is for the latter; a clean
/// completion gets the same blue k9s/kubectl use for it.
fn container_dot(c: &ContainerInfo) -> (&'static str, Color) {
    match c.status {
        ContainerStatusKind::Running => ("●", Color::Green),
        ContainerStatusKind::Waiting => ("●", Color::Yellow),
        ContainerStatusKind::Terminated if c.reason.as_deref() == Some("Completed") => ("●", Color::Blue),
        ContainerStatusKind::Terminated => ("●", Color::Red),
        ContainerStatusKind::Unknown => ("●", Color::Gray),
    }
}

/// Compact form: just the colored dots — used for every row except the
/// one that's hovered/selected.
fn containers_cell(containers: &[ContainerInfo], muted: bool) -> Line<'static> {
    let mut spans = Vec::with_capacity(containers.len() * 2);
    for c in containers {
        let (glyph, color) = container_dot(c);
        let style = if muted { dim_style() } else { Style::default().fg(color) };
        spans.push(Span::styled(glyph, style));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn container_state_text(c: &ContainerInfo) -> String {
    match c.status {
        ContainerStatusKind::Running => "Running".to_string(),
        ContainerStatusKind::Waiting => c.reason.clone().unwrap_or_else(|| "Waiting".into()),
        ContainerStatusKind::Terminated => c.reason.clone().unwrap_or_else(|| "Terminated".into()),
        ContainerStatusKind::Unknown => "Unknown".into(),
    }
}

/// The `:` command line — a floating box centered on the screen, same
/// as the search box, on top of whatever's there.
/// The `:` command line plus its live autocomplete list — one suggestion
/// per matching resource kind, best match first, growing the box
/// downward (and, since it's centered, rising upward too) as you type.
fn draw_command_bar(frame: &mut Frame, input: &str, suggestions: &[ResourceKind], selected: usize) {
    let box_height = 3 + suggestions.len() as u16;
    let bar = centered_box(frame.area(), box_height);
    frame.render_widget(Clear, bar);
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title("Command");
    let inner = block.inner(bar);
    frame.render_widget(block, bar);

    let rows = Layout::vertical([Constraint::Length(1)].repeat(inner.height.max(1) as usize)).split(inner);
    let line = Line::styled(format!(":{input}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    frame.render_widget(Paragraph::new(line), rows[0]);

    for (i, kind) in suggestions.iter().enumerate() {
        let Some(row) = rows.get(i + 1) else { break };
        let is_selected = i == selected;
        let style = if is_selected {
            Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let text = format!("{:width$}", kind.label(), width = row.width as usize);
        frame.render_widget(Paragraph::new(Span::styled(text, style)), *row);
    }
}

/// Shared namespace/name coloring — namespace in the app's cyan accent,
/// name in plain bold, `/` muted — the same "kind vs value" split the
/// breadcrumb uses, reused everywhere a `namespace/name` pair shows up
/// (this status line, the Containers/Logs popup titles) so it's one
/// defined color pairing rather than a different pick per screen.
fn namespace_name_spans(namespace: &str, name: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(namespace.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled("/", Style::default().fg(Color::DarkGray)),
        Span::styled(name.to_string(), Style::default().add_modifier(Modifier::BOLD)),
    ]
}

fn draw_status_line(frame: &mut Frame, area: Rect, pods: &[PodRow], row: Option<usize>, dimmed: bool) {
    let line = match (dimmed, row.and_then(|i| pods.get(i))) {
        (false, Some(pod)) => {
            let mut spans = namespace_name_spans(&pod.namespace, &pod.name);
            spans.push(Span::raw(" ["));
            for (i, c) in pod.containers.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(" : ", Style::default().fg(Color::DarkGray)));
                }
                let (glyph, color) = container_dot(c);
                let style = Style::default().fg(color);
                spans.push(Span::styled(format!("{glyph} "), style));
                spans.push(Span::styled(c.name.clone(), style));
                spans.push(Span::styled(format!("({})", container_state_text(c)), style));
            }
            spans.push(Span::raw("]"));
            Line::from(spans)
        }
        _ => Line::raw(""),
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// A real floating popup, positioned right next to the cursor — "in
/// front," on top of everything, only while actively hovering.
fn draw_hover_popup(frame: &mut Frame, pod: &PodRow, column: u16, row: u16, bounds: Rect) {
    let lines: Vec<Line> = pod
        .containers
        .iter()
        .map(|c| {
            let (glyph, color) = container_dot(c);
            let style = Style::default().fg(color);
            Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::raw(format!("{}: ", c.name)),
                Span::styled(container_state_text(c), style),
            ])
        })
        .collect();

    let width = lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap_or(10)
        .max(pod.name.len() + pod.namespace.len() + 1)
        .saturating_add(4) as u16;
    let height = (lines.len() as u16).saturating_add(2).max(3);

    let area = popup_near(column, row, width, height, bounds);
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!("{}/{}", pod.namespace, pod.name));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Places a small box near a screen position, nudged so it never renders
/// past the right/bottom edge of the terminal.
fn popup_near(column: u16, row: u16, width: u16, height: u16, bounds: Rect) -> Rect {
    let x = (column + 1).min(bounds.width.saturating_sub(width));
    let y = (row + 1).min(bounds.height.saturating_sub(height));
    Rect { x, y, width: width.min(bounds.width), height: height.min(bounds.height) }
}

fn draw_table(frame: &mut Frame, area: Rect, pods: &[PodRow], table_state: &mut TableState, dimmed: bool) {
    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["NAMESPACE", "NAME", "READY", "STATUS", "RESTARTS", "NODE", "AGE", "CONTAINERS"])
        .style(header_style);

    let rows = pods.iter().map(|p| {
        let status_style = if dimmed {
            muted
        } else {
            let color = match p.phase.as_str() {
                "Running" => Color::Green,
                "Pending" => Color::Yellow,
                "Failed" => Color::Red,
                _ => Color::Gray,
            };
            Style::default().fg(color)
        };
        let cell_style = if dimmed { muted } else { Style::default() };
        Row::new(vec![
            Cell::from(p.namespace.clone()).style(cell_style),
            Cell::from(p.name.clone()).style(cell_style),
            Cell::from(p.ready.clone()).style(cell_style),
            Cell::from(p.phase.clone()).style(status_style),
            Cell::from(p.restarts.to_string()).style(cell_style),
            Cell::from(p.node.clone()).style(cell_style),
            Cell::from(p.age.clone()).style(cell_style),
            Cell::from(containers_cell(&p.containers, dimmed)),
        ])
    });

    let title = format!("Pods ({})", pods.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, pod_table_widths())
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

fn pod_table_widths() -> [Constraint; 8] {
    [
        Constraint::Fill(2),   // namespace
        Constraint::Fill(3),   // name
        Constraint::Length(6), // ready
        Constraint::Fill(2),   // status
        Constraint::Length(9), // restarts
        Constraint::Fill(2),   // node
        Constraint::Length(5), // age
        Constraint::Fill(3),   // containers
    ]
}

/// Which pod row sits under an absolute terminal position, restricted to
/// the CONTAINERS column specifically — hovering anywhere else in the row
/// shouldn't trigger the popup, only the dots themselves. Reuses
/// `Table`'s own column constraints through a real `Layout` solve (same
/// widths, same default 1-cell `column_spacing`) rather than
/// hand-guessing pixel math that could silently drift out of sync with
/// what's actually rendered.
pub fn row_at(frame_area: Rect, table_state: &TableState, row_count: usize, column: u16, row: u16) -> Option<usize> {
    let table_area =
        Rect { x: frame_area.x, y: frame_area.y, width: frame_area.width, height: frame_area.height.saturating_sub(1) };

    let inner = Rect {
        x: table_area.x.saturating_add(1),
        y: table_area.y.saturating_add(2), // border + header
        width: table_area.width.saturating_sub(2),
        height: table_area.height.saturating_sub(3), // top border + header + bottom border
    };

    if row < inner.y || row >= inner.y + inner.height {
        return None;
    }

    let columns = Layout::horizontal(pod_table_widths()).spacing(1).split(inner);
    let containers_col = columns[7];
    if column < containers_col.x || column >= containers_col.x + containers_col.width {
        return None;
    }

    let offset = table_state.offset();
    let index = offset + usize::from(row - inner.y);
    (index < row_count).then_some(index)
}

fn draw_deployment_table(frame: &mut Frame, area: Rect, deployments: &[DeploymentRow], table_state: &mut TableState, dimmed: bool) {
    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header =
        Row::new(vec!["NAMESPACE", "NAME", "READY", "UP-TO-DATE", "AVAILABLE", "AGE"]).style(header_style);

    let rows = deployments.iter().map(|d| {
        Row::new(vec![
            Cell::from(d.namespace.clone()).style(cell_style),
            Cell::from(d.name.clone()).style(cell_style),
            Cell::from(d.ready.clone()).style(cell_style),
            Cell::from(d.up_to_date.to_string()).style(cell_style),
            Cell::from(d.available.to_string()).style(cell_style),
            Cell::from(d.age.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Fill(2),
        Constraint::Fill(3),
        Constraint::Length(6),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Length(5),
    ];

    let title = format!("Deployments ({})", deployments.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

/// A compact inline usage bar for a table cell: `▓▓▓░░░░░ 34%`, or
/// `n/a` in gray when metrics-server isn't installed. Same block-style
/// bar `draw_meter` uses for the full-width Cluster Resources meters,
/// just narrow enough to fit a column.
fn usage_bar(used: Option<i64>, capacity: i64, dimmed: bool) -> Line<'static> {
    const WIDTH: usize = 10;
    let Some(used) = used else {
        return Line::styled("n/a", Style::default().fg(Color::DarkGray));
    };
    let ratio = if capacity > 0 { (used as f64 / capacity as f64).clamp(0.0, 1.0) } else { 0.0 };
    let filled = (ratio * WIDTH as f64).round() as usize;
    let color = usage_color(ratio, dimmed);
    let bracket = if dimmed { dim_style() } else { Style::default().fg(Color::DarkGray) };
    Line::from(vec![
        Span::styled("[", bracket),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(WIDTH - filled), Style::default().fg(Color::DarkGray)),
        Span::styled("]", bracket),
        Span::raw(format!(" {:.0}%", ratio * 100.0)),
    ])
}

fn usage_color(ratio: f64, dimmed: bool) -> Color {
    if dimmed {
        Color::Rgb(40, 40, 40)
    } else if ratio > 0.9 {
        Color::Red
    } else if ratio > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn draw_nodes_table(frame: &mut Frame, area: Rect, nodes: &[NodeRow], table_state: &mut TableState, dimmed: bool) {
    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["NAME", "STATUS", "ROLES", "CPU", "MEMORY", "PODS", "AGE", "VERSION"]).style(header_style);

    let rows = nodes.iter().map(|n| {
        let status_style = if dimmed {
            muted
        } else if n.ready && n.schedulable {
            Style::default().fg(Color::Green)
        } else if n.ready {
            Style::default().fg(Color::Yellow) // cordoned, but otherwise healthy
        } else {
            Style::default().fg(Color::Red)
        };
        // kubectl's own convention: append ",SchedulingDisabled" to STATUS
        // rather than a separate column.
        let status = match (n.ready, n.schedulable) {
            (true, true) => "Ready".to_string(),
            (true, false) => "Ready,SchedulingDisabled".to_string(),
            (false, true) => "NotReady".to_string(),
            (false, false) => "NotReady,SchedulingDisabled".to_string(),
        };
        Row::new(vec![
            Cell::from(n.name.clone()).style(cell_style),
            Cell::from(status).style(status_style),
            Cell::from(n.roles.clone()).style(cell_style),
            Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
            Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)).style(cell_style),
            Cell::from(n.age.clone()).style(cell_style),
            Cell::from(n.version.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Fill(2),
        Constraint::Length(24),
        Constraint::Fill(1),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(5),
        Constraint::Length(12),
    ];

    let title = format!("Nodes ({})", nodes.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

/// The shared table for every resource kind that doesn't get specialized
/// columns — Namespace/Name/Age is all that's generically knowable about
/// an arbitrary Kubernetes object.
/// Cluster-scoped kinds (Nodes, ClusterRoles, PVs, StorageClasses, ...)
/// show "-" for every row's namespace — a column that's all dashes isn't
/// telling anyone anything, so `draw_generic_table` drops it entirely
/// when this is false.
fn any_row_has_namespace(rows: &[GenericRow]) -> bool {
    rows.iter().any(|r| r.namespace != "-")
}

fn draw_generic_table(frame: &mut Frame, area: Rect, rows: &[GenericRow], label: &str, table_state: &mut TableState, dimmed: bool) {
    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let show_namespace = any_row_has_namespace(rows);

    let (header, widths): (Row, Vec<Constraint>) = if show_namespace {
        (Row::new(vec!["NAMESPACE", "NAME", "AGE"]), vec![Constraint::Fill(2), Constraint::Fill(3), Constraint::Length(5)])
    } else {
        (Row::new(vec!["NAME", "AGE"]), vec![Constraint::Fill(1), Constraint::Length(5)])
    };
    let header = header.style(header_style);

    let table_rows = rows.iter().map(|r| {
        let mut cells = Vec::with_capacity(3);
        if show_namespace {
            cells.push(Cell::from(r.namespace.clone()).style(cell_style));
        }
        cells.push(Cell::from(r.name.clone()).style(cell_style));
        cells.push(Cell::from(r.age.clone()).style(cell_style));
        Row::new(cells)
    });

    let title = format!("{label} ({})", rows.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(table_rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

/// The Custom Resources picker: every discovered CRD kind, grouped
/// visually just by sorting on GROUP (already the order `discover_crds`
/// returns them in) rather than a nested per-group tile browser — simpler,
/// and still scannable since same-group kinds land next to each other.
/// Selecting a row and pressing Enter is what actually starts watching
/// that kind (see the CustomResourceList Enter handler in main.rs) —
/// nothing here is live-watched itself, consistent with the "list only
/// until opened" design.
fn draw_crd_list_table(frame: &mut Frame, area: Rect, crds: &[(usize, CrdInfo)], heading: &str, table_state: &mut TableState, dimmed: bool) {
    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["GROUP", "KIND", "SCOPE"]).style(header_style);

    let rows = crds.iter().map(|(_, c)| {
        Row::new(vec![
            Cell::from(c.group).style(cell_style),
            Cell::from(c.kind).style(cell_style),
            Cell::from(if c.namespaced { "Namespaced" } else { "Cluster" }).style(cell_style),
        ])
    });

    let widths = [Constraint::Fill(3), Constraint::Fill(2), Constraint::Length(11)];
    let title = format!("{heading} ({})", crds.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

/// Column width for the resource-switcher menu's own tile grid (see
/// `menu_cols`/`draw_menu_popup`) — the Overview page no longer uses
/// fixed-size tiles at all, but the menu still does.
const TILE_WIDTH: u16 = 22;
/// One catalog column's fixed width in the Overview browser, including
/// its own rounded border. A 1-cell gap is inserted between columns (see
/// `column_layout`) so each reads as a distinct bordered pane, herdr-style,
/// rather than boxes sharing an edge.
const COLUMN_WIDTH: u16 = 28;
/// Each item card's fixed height: a rounded-border top edge, one content
/// row (icon on the left, name + live count filling the rest), and a
/// rounded-border bottom edge.
const ITEM_HEIGHT: u16 = 3;
/// The Events panel is a fixed-size dashboard strip, not a scrollable
/// section — cap how many entries it shows directly, with a "+N more"
/// line instead of growing to fit all of them. The full, uncapped,
/// filterable feed is one Enter away (see `Overlay::Events`).
const MAX_VISIBLE_EVENTS: usize = 5;
/// Width of the left/right scroll-affordance gutters flanking the
/// columns area (see `columns_inner`) — just wide enough for a single
/// arrow glyph.
const SCROLL_ARROW_WIDTH: u16 = 1;

/// The home screen: a fixed-size dashboard strip up top (Resources, then
/// Events — each its own rounded-border box, mirroring the column boxes
/// below), and below it a horizontally-scrollable set of columns, one per
/// resource category (Cluster, Workloads, Config, ...), each listing that
/// category's kinds vertically — Miller-columns style.
#[allow(clippy::too_many_arguments)]
fn draw_overview(
    frame: &mut Frame,
    area: Rect,
    overview: &Overview,
    selection: OverviewSelection,
    col_scroll: usize,
    item_scroll: usize,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let top_h = top_area_height(overview);
    let chunks = Layout::vertical([Constraint::Length(top_h), Constraint::Length(1), Constraint::Min(0)]).split(area);
    draw_top_panel(frame, chunks[0], overview, selection, dimmed);
    draw_columns(frame, chunks[2], overview, selection, col_scroll, item_scroll, dimmed, icons);
}

/// How tall the Resources box is: a rounded border top/bottom (2) plus
/// either 3 meter lines or the 2-line "unavailable" message.
fn resources_box_height(overview: &Overview) -> u16 {
    2 + if overview.metrics_available { 3 } else { 2 }
}

/// How tall the Events box is: a rounded border top/bottom (2) plus
/// `events_content_height`.
fn events_box_height(overview: &Overview) -> u16 {
    2 + events_content_height(overview)
}

/// How tall the fixed top dashboard strip is: the Resources box, a 1-row
/// gap, then the Events box. Callers (mouse hit-testing, the columns area
/// below it) can't drift out of sync with what's actually rendered since
/// they all go through this and the two box-height functions above.
fn top_area_height(overview: &Overview) -> u16 {
    resources_box_height(overview) + 1 + events_box_height(overview)
}

/// Height of the Events box's content only (below its border) — either
/// the 2-line "no events" message, or the column-header row plus up to
/// `MAX_VISIBLE_EVENTS` entries plus a "+N more" line if there are more
/// than that. `events_box_height` and `draw_top_panel` both use this so
/// they can't drift apart.
fn events_content_height(overview: &Overview) -> u16 {
    if overview.events.is_empty() {
        return 2;
    }
    let shown = overview.events.len().min(MAX_VISIBLE_EVENTS);
    let more = usize::from(overview.events.len() > MAX_VISIBLE_EVENTS);
    1 + (shown + more) as u16
}

/// The columns area is whatever's left below the fixed dashboard strip
/// — callers (keyboard navigation, mouse hit-testing) need this same
/// rectangle to stay in sync with what's actually rendered.
pub fn columns_area(frame_area: Rect, overview: &Overview) -> Rect {
    // +1 for the same gap `draw_overview` puts between the top strip and
    // the columns — Resources-to-Events and top-strip-to-columns are now
    // both a single blank row, not one bigger than the other.
    let top_h = top_area_height(overview) + 1;
    Rect { x: frame_area.x, y: frame_area.y + top_h, width: frame_area.width, height: frame_area.height.saturating_sub(top_h) }
}

/// The columns area minus its left/right scroll-arrow gutters — every
/// place that lays out or hit-tests the column boxes themselves
/// (`draw_columns`, `column_hit`) works within this narrower rect so the
/// arrows always sit outside the boxes rather than overlapping them.
fn columns_inner(area: Rect) -> Rect {
    let shrink = SCROLL_ARROW_WIDTH * 2;
    Rect { x: area.x + SCROLL_ARROW_WIDTH, y: area.y, width: area.width.saturating_sub(shrink), height: area.height }
}

pub fn visible_columns(width: u16, total_columns: usize) -> usize {
    // Each column takes `COLUMN_WIDTH` plus a 1-cell gap before the next
    // one (see `column_layout`) — so `n` columns actually need
    // `n * (COLUMN_WIDTH + 1) - 1` cells, not `n * COLUMN_WIDTH`.
    let cols = ((width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize;
    cols.min(total_columns.max(1))
}

/// How many item cards fit vertically inside one column, given the whole
/// columns area's height — every column shares that same height
/// regardless of how many items it actually holds, so this one number is
/// right for all of them. Used both to size the keyboard auto-scroll
/// window and (implicitly, via the same math in `draw_column`) to decide
/// how many cards actually get drawn.
pub fn visible_items_per_column(columns_area_height: u16) -> usize {
    (columns_area_height.saturating_sub(2) / ITEM_HEIGHT).max(1) as usize
}

/// The shared column-rect layout — `draw_columns` and `column_hit` must
/// agree on exactly where each column's box sits, or clicks stop lining
/// up with what's on screen.
fn column_layout(area: Rect, cols_visible: usize) -> std::rc::Rc<[Rect]> {
    let constraints: Vec<Constraint> = (0..cols_visible).map(|_| Constraint::Length(COLUMN_WIDTH)).collect();
    Layout::horizontal(constraints).spacing(1).split(area)
}

/// Resources and Events, each its own rounded-border box — same visual
/// language as the column boxes below, per the explicit request to make
/// the dashboard read as boxes/cards throughout rather than plain labeled
/// regions. Selecting one (see `OverviewSelection`) highlights its whole
/// border, herdr-style, same as a column header/item; otherwise the
/// Events box's border reflects cluster health at a glance (green/
/// yellow/red) the same way an individual event line already did.
fn draw_top_panel(frame: &mut Frame, area: Rect, overview: &Overview, selection: OverviewSelection, dimmed: bool) {
    let resources_h = resources_box_height(overview);
    let chunks = Layout::vertical([Constraint::Length(resources_h), Constraint::Length(1), Constraint::Min(0)]).split(area);

    let highlight = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let resources_border = if dimmed {
        dim_style()
    } else if selection == OverviewSelection::Resources {
        highlight
    } else {
        Style::default()
    };
    let resources_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(resources_border)
        .title(Line::styled(" Resources ", if dimmed { resources_border } else { Style::default().add_modifier(Modifier::BOLD) }));
    let resources_inner = resources_block.inner(chunks[0]);
    frame.render_widget(resources_block, chunks[0]);
    draw_metrics_lines(frame, resources_inner, overview, dimmed);

    // Same plain default styling as the Resources box — no status color
    // on the box chrome itself, only the highlight when selected. Each
    // event's own line (in this preview and the full browser) still
    // carries its own severity color.
    let events_border = if dimmed {
        dim_style()
    } else if selection == OverviewSelection::Events {
        highlight
    } else {
        Style::default()
    };
    let events_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(events_border)
        .title(Line::styled(format!(" Events ({}) ", overview.events.len()), if dimmed { events_border } else { Style::default().add_modifier(Modifier::BOLD) }));
    let events_inner = events_block.inner(chunks[2]);
    frame.render_widget(events_block, chunks[2]);

    if overview.events.is_empty() {
        draw_events_empty(frame, events_inner, dimmed);
    } else {
        let shown = overview.events.len().min(MAX_VISIBLE_EVENTS);
        let has_more = overview.events.len() > MAX_VISIBLE_EVENTS;
        let lines = Layout::vertical((0..1 + shown + usize::from(has_more)).map(|_| Constraint::Length(1))).split(events_inner);
        draw_events_header(frame, lines[0], dimmed);
        for (i, e) in overview.events.iter().take(shown).enumerate() {
            draw_event_line(frame, lines[i + 1], e, dimmed);
        }
        if has_more {
            let more = overview.events.len() - shown;
            let style = if dimmed { dim_style() } else { Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC) };
            frame.render_widget(Paragraph::new(Line::styled(format!("… and {more} more"), style)).alignment(Alignment::Center), lines[1 + shown]);
        }
    }
}

/// The CPU/Memory/Pods meters, one per line.
/// Live pod count, read from the catalog rather than duplicated as its
/// own field on `Overview` — Pods already has one live reflector feeding
/// the catalog tile, so this just reads the same number back out.
fn workloads_pod_count(overview: &Overview) -> usize {
    overview
        .catalog
        .iter()
        .find(|(section, _)| *section == "Workloads")
        .and_then(|(_, tiles)| tiles.iter().find(|(label, _)| *label == "Pods"))
        .map(|(_, count)| *count)
        .unwrap_or(0)
}

fn draw_metrics_lines(frame: &mut Frame, area: Rect, overview: &Overview, dimmed: bool) {
    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(Color::DarkGray)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
        return;
    }

    let pod_usage = workloads_pod_count(overview);

    let lines = Layout::vertical([Constraint::Length(1); 3]).split(area);
    draw_meter(
        frame,
        lines[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_meter(
        frame,
        lines[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_meter(frame, lines[2], "Pods", pod_usage as f64, overview.pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
}

/// A single-line usage meter: `CPU     ▓▓▓▓▓▓░░░░░░░░░░░░░░░░  71m / 2000m (3%)`.
/// Hand-built instead of ratatui's `Gauge` widget, which bakes in its own
/// centered percentage label — impossible to turn off without also
/// losing the ability to show the actual used/capacity numbers, so the
/// two labels ended up overlapping/duplicating. The bar width adapts to
/// whatever space is actually available instead of being fixed.
fn draw_meter(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let color = usage_color(ratio, dimmed);
    let detail = format!("{} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0);

    let label_text = format!("{label:<8}");
    let reserved = label_text.chars().count() as u16 + detail.chars().count() as u16 + 5;
    let bar_width = area.width.saturating_sub(reserved).max(4) as usize;
    let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);

    let label_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let detail_style = if dimmed { dim_style() } else { Style::default() };
    let bracket = if dimmed { dim_style() } else { Style::default().fg(Color::DarkGray) };

    let line = Line::from(vec![
        Span::styled(label_text, label_style),
        Span::styled("[", bracket),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_width - filled), Style::default().fg(Color::DarkGray)),
        Span::styled("]", bracket),
        Span::styled(format!(" {detail}"), detail_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn format_bytes(bytes: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1}{}", UNITS[unit])
}

/// Selection across the whole Overview page: the Resources box, the
/// Events box, a column's own header (selectable so its name reads
/// clearly even without a mouse), or a specific item within a column.
/// `Resources`/`Events` sit "above" every column — Up from any column
/// header lands on `Events`, and Down from `Events` returns to the first
/// column's header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverviewSelection {
    Resources,
    Events,
    Header(usize),
    Item(usize, usize),
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn column_len(overview: &Overview, col: usize) -> usize {
    overview.catalog.get(col).map(|(_, items)| items.len()).unwrap_or(0)
}

/// Moves the Overview selection one step in a direction. Up/Down move
/// into and out of a column's own header (pressing Up at a column's
/// first item lands on its header, pressing Up again from the header
/// lands on `Events`; pressing Down on a header enters its first item,
/// or does nothing if the column is empty) — Left/Right move directly
/// between columns at the same item index, landing on the target
/// column's header instead if it has nothing at that index.
/// `Resources`/`Events` only respond to Up/Down (there's nothing beside
/// them to move to horizontally).
pub fn move_overview_selection(overview: &Overview, selection: OverviewSelection, dir: Direction) -> OverviewSelection {
    let total = overview.catalog.len();
    match selection {
        OverviewSelection::Resources => match dir {
            Direction::Down => OverviewSelection::Events,
            _ => selection,
        },
        OverviewSelection::Events => match dir {
            Direction::Up => OverviewSelection::Resources,
            Direction::Down => {
                if total == 0 { selection } else { OverviewSelection::Header(0) }
            }
            _ => selection,
        },
        OverviewSelection::Header(col) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            match dir {
                Direction::Down => {
                    if column_len(overview, col) > 0 { OverviewSelection::Item(col, 0) } else { selection }
                }
                Direction::Up => OverviewSelection::Events,
                Direction::Left => if col > 0 { OverviewSelection::Header(col - 1) } else { selection },
                Direction::Right => if col + 1 < total { OverviewSelection::Header(col + 1) } else { selection },
            }
        }
        OverviewSelection::Item(col, item) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            let len = column_len(overview, col).max(1);
            let item = item.min(len - 1);
            match dir {
                Direction::Up => {
                    if item > 0 { OverviewSelection::Item(col, item - 1) } else { OverviewSelection::Header(col) }
                }
                Direction::Down => {
                    if item + 1 < len { OverviewSelection::Item(col, item + 1) } else { selection }
                }
                Direction::Left => {
                    if col == 0 {
                        selection
                    } else {
                        let target_len = column_len(overview, col - 1);
                        if target_len == 0 { OverviewSelection::Header(col - 1) } else { OverviewSelection::Item(col - 1, item.min(target_len - 1)) }
                    }
                }
                Direction::Right => {
                    if col + 1 >= total {
                        selection
                    } else {
                        let target_len = column_len(overview, col + 1);
                        if target_len == 0 { OverviewSelection::Header(col + 1) } else { OverviewSelection::Item(col + 1, item.min(target_len - 1)) }
                    }
                }
            }
        }
    }
}

/// Same movement rules as before, for the resource-switcher menu's own
/// section/tile grid — unrelated to the Overview's column browser, which
/// doesn't wrap tiles into rows at all anymore.
fn next_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (from + 1..lens.len()).find(|&i| lens[i] > 0)
}

fn prev_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (0..from).rev().find(|&i| lens[i] > 0)
}

fn move_selection(section_lens: &[usize], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let section_count = section_lens.len();
    if section_count == 0 {
        return current;
    }
    let section = current.0.min(section_count - 1);
    let len = section_lens[section].max(1);
    let tile = current.1.min(len - 1);
    let cols = cols.max(1);
    let row = tile / cols;
    let col = tile % cols;

    match dir {
        Direction::Left => {
            if col > 0 {
                (section, tile - 1)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => (s, section_lens[s].saturating_sub(1)),
                    None => (section, tile),
                }
            }
        }
        Direction::Right => {
            if tile + 1 < len {
                (section, tile + 1)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, 0),
                    None => (section, tile),
                }
            }
        }
        Direction::Up => {
            if row > 0 {
                (section, (row - 1) * cols + col)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => {
                        let prev_len = section_lens[s];
                        let prev_rows = prev_len.div_ceil(cols).max(1);
                        let target = ((prev_rows - 1) * cols + col).min(prev_len.saturating_sub(1));
                        (s, target)
                    }
                    None => (section, tile),
                }
            }
        }
        Direction::Down => {
            let next = (row + 1) * cols + col;
            if next < len {
                (section, next)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, col.min(section_lens[s].saturating_sub(1))),
                    None => (section, tile),
                }
            }
        }
    }
}

/// Same movement rules as `move_selection`, for the resource-switcher
/// menu's own section/tile grid.
pub fn move_menu_selection(sections: &[MenuSection], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let lens: Vec<usize> = sections.iter().map(|s| s.tiles.len()).collect();
    move_selection(&lens, cols, current, dir)
}

/// The menu popup's tile-grid column count — computed from the popup's
/// actual inner area so keyboard navigation and mouse hit-testing can't
/// drift from what's rendered.
pub fn menu_cols(frame_area: Rect) -> usize {
    let area = centered_rect(70, 85, frame_area);
    let inner = Block::default().borders(Borders::ALL).inner(area);
    (inner.width / TILE_WIDTH).max(1) as usize
}

/// The area a column-detail popup (see `Overlay::ColumnDetail`) actually
/// renders into — one place so its own draw pass, the grid column count,
/// and the visible-row count can't drift apart.
fn column_detail_area(frame_area: Rect) -> Rect {
    centered_rect(85, 80, frame_area)
}

/// How many item cards fit per row in a column-detail popup — same
/// card width the compact Overview columns use, so a kind's card looks
/// identical whether you're looking at it there or here.
pub fn column_detail_cols(frame_area: Rect) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    ((inner.width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize
}

/// How many grid rows of item cards fit vertically in a column-detail
/// popup at once.
pub fn column_detail_visible_rows(frame_area: Rect) -> usize {
    let inner = Block::default().borders(Borders::ALL).inner(column_detail_area(frame_area));
    (inner.height / ITEM_HEIGHT).max(1) as usize
}

/// Same movement rules as `move_selection`/`move_menu_selection`, for a
/// column-detail popup's own single-list item grid — reuses `move_selection`
/// with exactly one "section" (there's nothing to jump to when you run off
/// an edge, so it just clamps there, which is exactly what a single list
/// needs).
pub fn move_column_detail_selection(items_len: usize, cols: usize, selected: usize, dir: Direction) -> usize {
    move_selection(&[items_len], cols, (0, selected), dir).1
}

/// Adjusts a scroll offset (if needed) so `target` is fully within the
/// `visible` window currently on screen — scrolls back immediately if the
/// selection moved before the window, or forward just far enough if it
/// moved past it. Dimension-agnostic: used both for the Overview's
/// horizontal column scroll and its vertical within-column item scroll.
pub fn scroll_columns_to_show(col_scroll: usize, cols_visible: usize, target_col: usize) -> usize {
    if target_col < col_scroll {
        target_col
    } else if target_col >= col_scroll + cols_visible {
        target_col + 1 - cols_visible
    } else {
        col_scroll
    }
}

/// Which column header or item (if any) sits under an absolute terminal
/// position, or the Resources/Events box above them — same layout
/// `draw_top_panel`/`draw_columns` actually render with, so a click
/// always resolves to what's really on screen. `active_col`/`item_scroll`
/// must be whatever was actually passed to the last `draw_columns` call —
/// only the active column's items are vertically scrolled, everything
/// else always renders starting from its own first item.
pub fn column_hit(
    frame_area: Rect,
    overview: &Overview,
    col_scroll: usize,
    active_col: usize,
    item_scroll: usize,
    column: u16,
    row: u16,
) -> Option<OverviewSelection> {
    if row < frame_area.y {
        return None;
    }
    let resources_h = resources_box_height(overview);
    let rel = row - frame_area.y;
    if rel < resources_h {
        return Some(OverviewSelection::Resources);
    }
    let events_start = resources_h + 1;
    if rel >= events_start && rel < events_start + events_box_height(overview) {
        return Some(OverviewSelection::Events);
    }

    let area = columns_inner(columns_area(frame_area, overview));
    if row < area.y || row >= area.y + area.height || column < area.x || column >= area.x + area.width {
        return None;
    }
    let total = overview.catalog.len();
    if total == 0 {
        return None;
    }
    let cols_visible = visible_columns(area.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let areas = column_layout(area, cols_visible);
    let col_i = areas.iter().position(|r| column >= r.x && column < r.x + r.width)?;
    let col_area = areas[col_i];
    let col_idx = col_scroll + col_i;

    if row == col_area.y {
        return Some(OverviewSelection::Header(col_idx));
    }
    let inner = Block::default().borders(Borders::ALL).inner(col_area);
    if row < inner.y || row >= inner.y + inner.height {
        return None;
    }
    let scroll = if col_idx == active_col { item_scroll } else { 0 };
    let item_i = ((row - inner.y) / ITEM_HEIGHT) as usize + scroll;
    let (_, items) = &overview.catalog[col_idx];
    if item_i < items.len() { Some(OverviewSelection::Item(col_idx, item_i)) } else { None }
}

/// Draws the columns themselves plus, in the 1-cell gutters flanking
/// them, a "◀"/"▶" arrow whenever scrolling that way would actually
/// reveal another column — the replacement for the old per-column
/// collapse toggle as the way to signal "there's more here."
#[allow(clippy::too_many_arguments)]
fn draw_columns(frame: &mut Frame, area: Rect, overview: &Overview, selection: OverviewSelection, col_scroll: usize, item_scroll: usize, dimmed: bool, icons: &mut IconCache) {
    let total = overview.catalog.len();
    if total == 0 {
        return;
    }
    let inner = columns_inner(area);
    let cols_visible = visible_columns(inner.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let areas = column_layout(inner, cols_visible);
    let active_col = match selection {
        OverviewSelection::Header(c) | OverviewSelection::Item(c, _) => c,
        OverviewSelection::Resources | OverviewSelection::Events => usize::MAX,
    };
    for (i, col_area) in areas.iter().enumerate() {
        let col_idx = col_scroll + i;
        let (title, items) = &overview.catalog[col_idx];
        let scroll = if col_idx == active_col { item_scroll } else { 0 };
        draw_column(frame, *col_area, col_idx, title, items, selection, scroll, dimmed, icons);
    }

    let arrow_style = if dimmed { dim_style() } else { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) };
    if col_scroll > 0 {
        let left = Rect { x: area.x, y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("◀", arrow_style)), left);
    }
    if col_scroll + cols_visible < total {
        let right = Rect { x: area.x + area.width - SCROLL_ARROW_WIDTH, y: area.y, width: SCROLL_ARROW_WIDTH, height: 1 };
        frame.render_widget(Paragraph::new(Span::styled("▶", arrow_style)), right);
    }
}

/// One column: a rounded-border box — herdr-style, the whole box's border
/// takes on the highlight color when its header is selected — carrying
/// the category name as its title, with that category's kinds listed
/// vertically inside as their own item cards (see `draw_column_item`).
/// `item_scroll` is only meaningful for whichever column is actually the
/// current selection's — every other column always renders from its own
/// first item.
#[allow(clippy::too_many_arguments)]
fn draw_column(
    frame: &mut Frame,
    area: Rect,
    col_idx: usize,
    title: &str,
    items: &[(&str, usize)],
    selection: OverviewSelection,
    item_scroll: usize,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let header_selected = matches!(selection, OverviewSelection::Header(c) if c == col_idx);
    let highlight = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let (border_style, title_style) = if dimmed {
        (dim_style(), dim_style())
    } else if header_selected {
        (highlight, highlight)
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD))
    };

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Line::styled(format!(" {title} "), title_style));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if items.is_empty() || inner.height < ITEM_HEIGHT {
        return;
    }

    let visible = visible_items_per_column(area.height);
    let scroll = item_scroll.min(items.len().saturating_sub(visible));
    let shown: Vec<(usize, &(&str, usize))> = items.iter().enumerate().skip(scroll).take(visible).collect();

    let constraints: Vec<Constraint> = shown.iter().map(|_| Constraint::Length(ITEM_HEIGHT)).collect();
    let rows = Layout::vertical(constraints).split(inner);

    for (slot, (i, (label, count))) in shown.into_iter().enumerate() {
        let selected = matches!(selection, OverviewSelection::Item(c, it) if c == col_idx && it == i);
        draw_column_item(frame, rows[slot], label, *count, title, selected, dimmed, icons);
    }
}

/// A label reaching here is a fixed kind name for every item except the
/// dynamically discovered CRD-group ones (raw API group strings like
/// "gateway.networking.k8s.io", which `from_label` can't know about
/// ahead of time) — those live only in the "Custom Resources" column, so
/// that's the signal to fall back to the generic CRD icon instead of a
/// "no icon" glyph.
fn resolve_icon_kind(label: &str, column_title: &str) -> Option<ResourceKind> {
    ResourceKind::from_label(label).or_else(|| (column_title == "Custom Resources").then_some(ResourceKind::CustomResourceList))
}

/// One item card: a rounded-border box, the kind's icon on the left of a
/// single content row, its name and live count filling the rest —
/// `<image> Pods           17`. Selecting it, herdr-style, turns the
/// whole card's border into a solid highlight color rather than just
/// tinting the background.
#[allow(clippy::too_many_arguments)]
fn draw_column_item(frame: &mut Frame, area: Rect, label: &str, count: usize, column_title: &str, selected: bool, dimmed: bool, icons: &mut IconCache) {
    let highlight = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let (border_style, text_style, count_style) = if dimmed {
        let muted = dim_style();
        (muted, muted, muted)
    } else if selected {
        (highlight, highlight, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    } else {
        (Style::default(), Style::default().add_modifier(Modifier::BOLD), Style::default().fg(Color::Cyan))
    };

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let icon_w = 3u16.min(inner.width);
    let split = Layout::horizontal([Constraint::Length(icon_w), Constraint::Min(0)]).split(inner);

    // A real vendored icon image where the terminal can render one; the
    // small emoji glyph is a fallback for when it can't (halfblocks
    // rendering). Skipped entirely while dimmed — a color emoji glyph
    // can't be muted via ANSI styling the way everything else here is,
    // so it would just sit there in full color on top of a background
    // that's supposed to read as out of focus.
    if !dimmed {
        match resolve_icon_kind(label, column_title) {
            Some(kind) => icons.draw(frame, icons.centered_square(split[0]), kind),
            None => frame.render_widget(Paragraph::new(icon_for(label)).alignment(Alignment::Center), split[0]),
        }
    }

    let count_text = count.to_string();
    let label_width = (split[1].width as usize).saturating_sub(count_text.chars().count() + 1).max(1);
    let line = Line::from(vec![
        Span::styled(format!("{:<label_width$}", truncate(label, label_width)), text_style),
        Span::styled(count_text, count_style),
    ]);
    frame.render_widget(Paragraph::new(line), split[1]);
}

fn icon_for(label: &str) -> &'static str {
    match label {
        "Nodes" => "🖥",
        "Namespaces" => "🗂",
        "Pods" => "📦",
        "Deployments" => "🚀",
        "ReplicaSets" => "📑",
        "StatefulSets" => "🧱",
        "DaemonSets" => "👻",
        "Jobs" => "⚙",
        "CronJobs" => "⏰",
        "ConfigMaps" => "🔧",
        "Secrets" => "🔐",
        "HPAs" => "📈",
        "Services" => "🔌",
        "Endpoints" => "🎯",
        "Ingresses" => "🚪",
        "NetworkPolicies" => "🛡",
        "PVCs" => "💿",
        "PVs" => "💾",
        "StorageClasses" => "🗄",
        "ServiceAccounts" => "🪪",
        "Roles" | "ClusterRoles" => "📜",
        "RoleBindings" | "ClusterRoleBindings" => "🔗",
        "Custom Resources" => "🧩",
        // Any other label reaching here is a dynamically discovered CRD
        // group name — same reasoning as `resolve_icon_kind`'s fallback.
        _ => "🧩",
    }
}

fn draw_events_header(frame: &mut Frame, area: Rect, dimmed: bool) {
    let style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let line = format!("{:<8}{:<44} {:<18} {:<12} AGE", "TYPE", "MESSAGE", "OBJECT", "KIND");
    frame.render_widget(Paragraph::new(Line::styled(line, style)), area);
}

fn draw_events_empty(frame: &mut Frame, area: Rect, dimmed: bool) {
    let ok_style = if dimmed { dim_style() } else { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) };
    let sub_style = Style::default().fg(Color::DarkGray);
    let text = vec![Line::styled("✓ No events yet", ok_style), Line::styled("Nothing has happened on the cluster", sub_style)];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
}

/// One dashboard-preview line: color reflects severity — a not-ready/
/// pressured node (Red) affects everything scheduled on it, an ordinary
/// Warning event (Yellow) is worth a look, and a Normal event (a muted
/// green) is just routine activity, not a problem.
fn draw_event_line(frame: &mut Frame, area: Rect, entry: &EventEntry, dimmed: bool) {
    let color = if dimmed {
        Color::Rgb(40, 40, 40)
    } else {
        match (entry.severity, entry.kind.as_str()) {
            (crate::k8s::EventSeverity::Warning, "Node") => Color::Red,
            (crate::k8s::EventSeverity::Warning, _) => Color::Yellow,
            (crate::k8s::EventSeverity::Normal, _) => Color::Green,
        }
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let message = truncate(&entry.message, 42);
    let line = format!("{type_text:<8}{message:<44} {:<18} {:<12} {}", entry.object, entry.kind, entry.age);
    frame.render_widget(Paragraph::new(Line::styled(line, Style::default().fg(color))), area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
    } else {
        s.to_string()
    }
}

/// The central "switch resource" menu — rounded-corner tiles grouped by
/// section, Freelens-style. Only one section exists today (`Workloads`);
/// adding another resource kind later is just adding another
/// `MenuSection`/tile, not restructuring this.
fn draw_menu_popup(frame: &mut Frame, sections: &[MenuSection], selected: (usize, usize)) {
    let area = centered_rect(70, 85, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title("Switch resource");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Sections can now hold up to ~7 tiles (Workloads) — one fixed-width
    // row per section, like the old layout, would squeeze those down to
    // unreadable slivers. Wrap each section's tiles the same way the
    // Overview catalog wraps its own tile grid.
    let cols = menu_cols(frame.area());
    let section_heights: Vec<Constraint> = sections
        .iter()
        .map(|s| Constraint::Length(1 + s.tiles.len().div_ceil(cols).max(1) as u16 * 3))
        .collect();
    let section_areas = Layout::vertical(section_heights).split(inner);

    for (section_idx, (section, section_area)) in sections.iter().zip(section_areas.iter()).enumerate() {
        let rows_needed = section.tiles.len().div_ceil(cols).max(1);
        let row_heights: Vec<Constraint> =
            std::iter::once(Constraint::Length(1)).chain((0..rows_needed).map(|_| Constraint::Length(3))).collect();
        let row_areas = Layout::vertical(row_heights).split(*section_area);

        frame.render_widget(
            Paragraph::new(Line::styled(section.title, Style::default().add_modifier(Modifier::BOLD))),
            row_areas[0],
        );

        for (row, row_area) in row_areas[1..].iter().enumerate() {
            let start = row * cols;
            let row_tiles = &section.tiles[start..(start + cols).min(section.tiles.len())];
            let tile_constraints: Vec<Constraint> =
                row_tiles.iter().map(|_| Constraint::Ratio(1, row_tiles.len() as u32)).collect();
            let tile_areas = Layout::horizontal(tile_constraints).split(*row_area);

            for (col, (tile_area, kind)) in tile_areas.iter().zip(row_tiles.iter()).enumerate() {
                let is_selected = selected == (section_idx, start + col);
                // A colored border alone read as too subtle to notice at
                // a glance — the selected tile now gets a solid filled
                // background instead, unmistakable regardless of terminal
                // theme.
                let (border_style, text_style) = if is_selected {
                    (Style::default().fg(Color::Cyan), Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD))
                } else {
                    (Style::default(), Style::default())
                };
                let tile = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).style(text_style);
                let label = Paragraph::new(kind.label()).alignment(Alignment::Center).style(text_style).block(tile);
                frame.render_widget(label, *tile_area);
            }
        }
    }
}

/// Freelens-style node drill-down: that node's own CPU/Memory/Pods
/// gauges (reusing the exact same `draw_gauge` the Overview panel uses)
/// above the pods actually scheduled on it (reusing the exact same pod
/// table Pods' own list view uses, including its container dots).
#[allow(clippy::too_many_arguments)]
fn draw_node_detail_popup(
    frame: &mut Frame,
    name: &str,
    cpu_usage: Option<i64>,
    cpu_capacity: i64,
    memory_usage: Option<i64>,
    memory_capacity: i64,
    pod_capacity: i64,
    info: Option<&crate::k8s::NodeDetailInfo>,
    pods: &[PodRow],
    state: &mut TableState,
    dimmed: bool,
) {
    let area = centered_rect(94, 92, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(format!("Node: {name}"));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let info_h = info.map(node_info_height).unwrap_or(0);
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Length(info_h), Constraint::Min(0)]).split(inner);

    match (cpu_usage, memory_usage) {
        (Some(cpu), Some(mem)) => {
            let lines = Layout::vertical([Constraint::Length(1); 3]).split(chunks[0]);
            draw_meter(frame, lines[0], "CPU", cpu as f64, cpu_capacity as f64, |v| format!("{:.2} cores", v / 1000.0), dimmed);
            draw_meter(frame, lines[1], "Memory", mem as f64, memory_capacity as f64, format_bytes, dimmed);
            draw_meter(frame, lines[2], "Pods", pods.len() as f64, pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
        }
        _ => {
            let text = Paragraph::new(Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Center);
            frame.render_widget(text, chunks[0]);
        }
    }

    if let Some(info) = info {
        draw_node_info_panel(frame, chunks[1], info, dimmed);
    }

    draw_table(frame, chunks[2], pods, state, dimmed);
}

/// How tall the node-info panel is: three summary lines, a blank
/// separator, the conditions header + one row per condition, then (if
/// there are any) a blank separator and a taints line — computed once so
/// sizing and drawing can't drift apart, same pattern as the Overview's
/// `top_area_height`/`issues_content_height`.
fn node_info_height(info: &crate::k8s::NodeDetailInfo) -> u16 {
    let base = 3 + 1 + 1 + info.conditions.len() as u16;
    if info.taints.is_empty() { base } else { base + 1 + info.taints.len() as u16 }
}

/// Freelens-style node summary: schedulability/roles/version, network
/// addresses and host OS/runtime details, the full condition list
/// (healthy conditions included — unlike the Cluster Issues panel, this
/// is a diagnostic view), and any taints.
fn draw_node_info_panel(frame: &mut Frame, area: Rect, info: &crate::k8s::NodeDetailInfo, dimmed: bool) {
    let label = Style::default().fg(Color::DarkGray);
    let value = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let field = |l: &'static str, v: String| vec![Span::styled(format!("{l}: "), label), Span::styled(v, value)];

    let schedulable_text = if info.schedulable { "Schedulable".to_string() } else { "Cordoned".to_string() };
    let schedulable_style = if dimmed {
        dim_style()
    } else if info.schedulable {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Yellow)
    };

    let mut line1 = field("Roles", info.roles.clone());
    line1.push(Span::raw("   "));
    line1.extend(vec![Span::styled("Status: ", label), Span::styled(schedulable_text, schedulable_style)]);
    line1.push(Span::raw("   "));
    line1.extend(field("Kubelet", info.kubelet_version.clone()));

    let mut line2 = field("Internal IP", info.internal_ip.clone());
    line2.push(Span::raw("   "));
    line2.extend(field("External IP", info.external_ip.clone()));

    let mut line3 = field("OS", info.os_image.clone());
    line3.push(Span::raw("   "));
    line3.extend(field("Kernel", info.kernel_version.clone()));
    line3.push(Span::raw("   "));
    line3.extend(field("Runtime", info.container_runtime.clone()));

    let mut lines = vec![Line::from(line1), Line::from(line2), Line::from(line3), Line::raw("")];

    lines.push(Line::styled("CONDITIONS", value));
    for c in &info.conditions {
        let is_healthy = (c.type_ == "Ready") == (c.status == "True");
        let color = if dimmed {
            Color::Rgb(40, 40, 40)
        } else if is_healthy {
            Color::Green
        } else {
            Color::Red
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:<20}", c.type_), if dimmed { dim_style() } else { Style::default() }),
            Span::styled(format!("{:<8}", c.status), Style::default().fg(color)),
            Span::styled(c.reason.clone(), label),
        ]));
    }

    if !info.taints.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![Span::styled("Taints: ", label), Span::styled(info.taints.join(", "), value)]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// The full Events browser — every event (not capped, unlike the
/// dashboard preview), filterable by severity. Kubernetes only defines
/// `Normal`/`Warning` as event types, so that's the full set of filters;
/// there's no separate "Errors" bucket to add since the API doesn't have
/// one.
fn draw_events_popup(frame: &mut Frame, events: &[EventEntry], filter: EventFilter, state: &mut TableState, dimmed: bool) {
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);

    let muted = dim_style();
    let filtered: Vec<&EventEntry> = events.iter().filter(|e| filter.matches(e)).collect();

    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let header = Row::new(vec!["TYPE", "REASON", "OBJECT", "KIND", "MESSAGE", "AGE"]).style(header_style);
    let cell_style = if dimmed { muted } else { Style::default() };
    let rows = filtered.iter().map(|e| {
        let color = if dimmed {
            Color::Rgb(40, 40, 40)
        } else {
            match (e.severity, e.kind.as_str()) {
                (crate::k8s::EventSeverity::Warning, "Node") => Color::Red,
                (crate::k8s::EventSeverity::Warning, _) => Color::Yellow,
                (crate::k8s::EventSeverity::Normal, _) => Color::Green,
            }
        };
        let type_text = match e.severity {
            crate::k8s::EventSeverity::Normal => "Normal",
            crate::k8s::EventSeverity::Warning => "Warning",
        };
        Row::new(vec![
            Cell::from(type_text).style(Style::default().fg(color)),
            Cell::from(e.reason.clone()).style(cell_style),
            Cell::from(e.object.clone()).style(cell_style),
            Cell::from(e.kind.clone()).style(cell_style),
            Cell::from(e.message.clone()).style(cell_style),
            Cell::from(e.age.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Length(9),
        Constraint::Fill(2),
        Constraint::Fill(2),
        Constraint::Length(12),
        Constraint::Fill(4),
        Constraint::Length(5),
    ];

    let title = format!(
        "Events ({}/{})  —  filter: {}",
        filtered.len(),
        events.len(),
        filter.label()
    );

    let border_style = if dimmed { muted } else { Style::default() };
    let highlight_style = if dimmed { muted } else { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) };
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    if let Some(selected) = state.selected() {
        state.select(Some(selected.min(filtered.len().saturating_sub(1))));
    }
    frame.render_stateful_widget(table, area, state);
}

/// Which row of the Events browser's table (if any) sits under an
/// absolute terminal position — same `centered_rect(94, 88, ...)` and
/// border/header layout `draw_events_popup` actually renders with.
/// `offset` must be the table's own current scroll offset (`TableState::
/// offset()`, valid only after that state has actually been rendered
/// with once — same reasoning `row_at` already relies on for Pods).
pub fn event_row_at(frame_area: Rect, filtered_len: usize, offset: usize, row: u16) -> Option<usize> {
    let area = centered_rect(94, 88, frame_area);
    let top = area.y + 2; // top border + header row
    let bottom = area.y + area.height.saturating_sub(1); // bottom border
    if row < top || row >= bottom {
        return None;
    }
    let index = offset + usize::from(row - top);
    (index < filtered_len).then_some(index)
}

/// One event's full detail — a plain wrapped-text popup rather than a
/// table row, since the point is showing the *un*truncated message a
/// narrow MESSAGE column would otherwise clip.
fn draw_event_detail_popup(frame: &mut Frame, entry: &EventEntry) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let color = match (entry.severity, entry.kind.as_str()) {
        (crate::k8s::EventSeverity::Warning, "Node") => Color::Red,
        (crate::k8s::EventSeverity::Warning, _) => Color::Yellow,
        (crate::k8s::EventSeverity::Normal, _) => Color::Green,
    };
    let type_text = match entry.severity {
        crate::k8s::EventSeverity::Normal => "Normal",
        crate::k8s::EventSeverity::Warning => "Warning",
    };
    let label = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let lines = vec![
        Line::from(vec![Span::styled("Type:   ", label), Span::styled(type_text, Style::default().fg(color).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled("Reason: ", label), Span::styled(entry.reason.clone(), bold)]),
        Line::from(vec![Span::styled("Object: ", label), Span::raw(format!("{} ({})", entry.object, entry.kind))]),
        Line::from(vec![Span::styled("Age:    ", label), Span::raw(entry.age.clone())]),
        Line::raw(""),
        Line::styled("Message:", bold),
        Line::raw(entry.message.clone()),
    ];

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title("Event detail");
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

/// The Overview's Resources panel, opened up — the exact same
/// cluster-wide gauges (`draw_metrics_lines`) and per-node usage table
/// (`draw_nodes_table`) the compact panel and the Nodes list already
/// draw, just given a full-screen popup's worth of room instead of three
/// cramped lines.
/// One cluster-wide meter as a real gauge, not a hand-drawn bar — the
/// popup has room to spare, unlike the compact Overview panel's
/// three cramped lines (`draw_meter`), so there's no need for the
/// label-clipping tradeoffs that ruled `Gauge` out there. The used/
/// capacity/percentage text lives inside the gauge's own centered
/// label instead of needing a separate line for it.
fn draw_gauge_box(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_style = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let gauge_style = if dimmed { dim_style() } else { Style::default().fg(usage_color(ratio, dimmed)) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title(Line::styled(format!(" {label} "), title_style));
    let label_text = format!("{} / {} ({:.0}%)", format_value(used), format_value(capacity), ratio * 100.0);
    let gauge = Gauge::default().block(block).gauge_style(gauge_style).ratio(ratio).label(label_text);
    frame.render_widget(gauge, area);
}

/// The Overview's Resources panel, opened up — cluster-wide CPU/Memory/
/// Pods only, as real gauges now there's room for them. Deliberately
/// doesn't repeat the per-node breakdown the Nodes list already owns —
/// that duplication was the actual complaint, not "the bars aren't
/// gauge-shaped enough."
fn draw_resources_detail_popup(frame: &mut Frame, overview: &Overview, dimmed: bool) {
    let area = centered_rect(60, 30, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title("Resources");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(Color::DarkGray)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
        return;
    }

    let pod_usage = workloads_pod_count(overview);
    let chunks = Layout::vertical([Constraint::Length(3); 3]).split(inner);
    draw_gauge_box(
        frame,
        chunks[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_gauge_box(
        frame,
        chunks[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_gauge_box(frame, chunks[2], "Pods", pod_usage as f64, overview.pod_capacity as f64, |v| format!("{v:.0}"), dimmed);
}

/// One category column, opened up into a bigger grid of the exact same
/// item cards `draw_column` draws in the compact Overview — for a
/// category with more kinds than fit in that narrow column at once
/// (Custom Resources, with many discovered API groups, is the case this
/// exists for). Scrolls vertically the same way the compact column does,
/// just over grid rows instead of single items.
fn draw_column_detail_popup(frame: &mut Frame, title: &str, items: &[(&str, usize)], selected: usize, row_scroll: usize, icons: &mut IconCache) {
    let area = column_detail_area(frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title.to_string());
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if items.is_empty() {
        frame.render_widget(Paragraph::new("Nothing here.").alignment(Alignment::Center), inner);
        return;
    }

    let cols = ((inner.width + 1) / (COLUMN_WIDTH + 1)).max(1) as usize;
    let total_rows = items.len().div_ceil(cols);
    let visible_rows = (inner.height / ITEM_HEIGHT).max(1) as usize;
    let row_scroll = row_scroll.min(total_rows.saturating_sub(visible_rows));
    let rows_shown = visible_rows.min(total_rows.saturating_sub(row_scroll));

    let row_constraints: Vec<Constraint> = (0..rows_shown).map(|_| Constraint::Length(ITEM_HEIGHT)).collect();
    let row_areas = Layout::vertical(row_constraints).split(inner);

    for (slot, row_area) in row_areas.iter().enumerate() {
        let row_idx = row_scroll + slot;
        let start = row_idx * cols;
        let row_items = &items[start..(start + cols).min(items.len())];
        let col_constraints: Vec<Constraint> = row_items.iter().map(|_| Constraint::Length(COLUMN_WIDTH)).collect();
        let col_areas = Layout::horizontal(col_constraints).spacing(1).split(*row_area);
        for (i, (item_area, (label, count))) in col_areas.iter().zip(row_items.iter()).enumerate() {
            let idx = start + i;
            draw_column_item(frame, *item_area, label, *count, title, idx == selected, false, icons);
        }
    }
}

/// Colors a `/`-joined title (`namespace/name`, or `namespace/pod/
/// container` for Logs) the same way as the breadcrumb: the outermost
/// segment (namespace) in the app's cyan accent, the innermost (a
/// container name, when there is one) in a distinct accent of its own,
/// everything else plain bold — joined by muted `/`s instead of one
/// flat-colored string. Falls back to plain bold for a title with no
/// `/` at all (a bare node name, say).
fn colored_slash_title(title: &str) -> Line<'static> {
    let parts: Vec<&str> = title.split('/').collect();
    if parts.len() < 2 {
        return Line::styled(title.to_string(), Style::default().add_modifier(Modifier::BOLD));
    }
    let sep = Style::default().fg(Color::DarkGray);
    let plain = Style::default().add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(parts[0].to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))];
    for (i, part) in parts[1..].iter().enumerate() {
        spans.push(Span::styled("/", sep));
        let is_last = i == parts.len() - 2;
        let style = if is_last && parts.len() > 2 { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) } else { plain };
        spans.push(Span::styled((*part).to_string(), style));
    }
    Line::from(spans)
}

/// `dimmed` only ever applies when this is the background behind its
/// own `ValueDetail` popup (`v` on a leaf) — the tree's own per-node
/// colors (baked into each `TreeItem`'s `Line` at build time in
/// `build_manifest_tree`) aren't re-muted, just the border/title and
/// selection highlight, same lighter-touch dimming `EventDetail`'s
/// background gets.
fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>, dimmed: bool) {
    let area = centered_rect(85, 85, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_line = if dimmed { Line::styled(title.to_string(), dim_style()) } else { colored_slash_title(title) };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title_line);

    let highlight_style = if dimmed { dim_style() } else { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) };
    let tree = Tree::new(items)
        .expect("pod tree ids are unique per level by construction")
        .block(block)
        .highlight_style(highlight_style)
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("  ");

    frame.render_stateful_widget(tree, area, state);
}

/// A tree leaf's full value, untruncated — opened by `v`. Plain wrapped
/// text, same treatment as `draw_event_detail_popup` for the same
/// reason: a narrow column/box clips long content with no indication or
/// way to see the rest.
fn draw_value_detail_popup(frame: &mut Frame, label: &str, value: &str) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!("{label}  —  q/esc: back"));
    let paragraph = Paragraph::new(value.to_string()).wrap(Wrap { trim: false }).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_containers_popup(frame: &mut Frame, title: &str, containers: &[ContainerInfo], state: &mut TableState, dimmed: bool) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let muted = dim_style();
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let header = Row::new(vec!["", "NAME", "STATE", "RESTARTS"]).style(header_style);
    let cell_style = if dimmed { muted } else { Style::default() };
    let rows = containers.iter().map(|c| {
        let (glyph, color) = container_dot(c);
        let dot_style = if dimmed { muted } else { Style::default().fg(color) };
        let state_text = c.reason.clone().unwrap_or_else(|| match c.status {
            ContainerStatusKind::Running => "Running".into(),
            ContainerStatusKind::Waiting => "Waiting".into(),
            ContainerStatusKind::Terminated => "Terminated".into(),
            ContainerStatusKind::Unknown => "Unknown".into(),
        });
        Row::new(vec![
            Cell::from(Span::styled(glyph, dot_style)),
            Cell::from(c.name.clone()).style(cell_style),
            Cell::from(state_text).style(dot_style),
            Cell::from(c.restarts.to_string()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(45),
        Constraint::Percentage(35),
        Constraint::Percentage(20),
    ];

    let border_style = if dimmed { muted } else { Style::default() };
    let highlight_style = if dimmed { muted } else { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) };
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style)
                .title(if dimmed { Line::styled(title.to_string(), muted) } else { colored_slash_title(title) }),
        )
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, state);
}

#[allow(clippy::too_many_arguments)]
fn draw_logs_popup(
    frame: &mut Frame,
    title: &str,
    lines: &[String],
    scroll: u16,
    follow: bool,
    timestamp_format: TimestampFormat,
    filter: &str,
    filter_editing: bool,
) {
    let area = centered_rect(90, 90, frame.area());
    frame.render_widget(Clear, area);

    // A plain substring match, not the fuzzy scorer the rest of the app
    // uses — log lines are prose to scan, not identifiers to narrow.
    let needle = filter.to_lowercase();
    let filtered: Vec<&str> =
        if filter.is_empty() { lines.iter().map(String::as_str).collect() } else { lines.iter().map(String::as_str).filter(|l| l.to_lowercase().contains(&needle)).collect() };

    // Just the live state, not how to control it — the keybindings for
    // pausing/resuming/toggling timestamps live in the `?` commands
    // panel now instead of being spelled out here every time.
    let follow_status = if follow { "following" } else { "paused" };
    let filter_status = if filter.is_empty() { String::new() } else { format!(", {}/{} match \"{filter}\"", filtered.len(), lines.len()) };
    let mut title_line = colored_slash_title(title);
    title_line.push_span(Span::raw(format!("  —  {follow_status}  ({} lines{filter_status})", lines.len())));
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(title_line);

    // When following, always show exactly the tail that fits the visible
    // area — simpler and more robust than trusting Paragraph's own scroll
    // clamping to not show blank space past the end of the content.
    let (text, effective_scroll): (Vec<Line>, u16) = if follow {
        let visible = area.height.saturating_sub(2) as usize; // minus borders
        let start = filtered.len().saturating_sub(visible);
        (filtered[start..].iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), 0)
    } else {
        (filtered.iter().copied().map(|l| colorize_log_line(l, timestamp_format, filter)).collect(), scroll)
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false }).scroll((effective_scroll, 0));

    frame.render_widget(paragraph, area);

    // The filter's own input line, pinned just inside the bottom border
    // while actively being typed — same treatment as the `/` search box
    // elsewhere, just scoped to this popup instead of floating over it.
    if filter_editing {
        let bar = Rect { x: area.x + 1, y: area.y + area.height.saturating_sub(2), width: area.width.saturating_sub(2), height: 1 };
        frame.render_widget(Clear, bar);
        let line = Line::styled(format!("/{filter}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        frame.render_widget(Paragraph::new(line), bar);
    }
}

/// Kubernetes' log API merges stdout/stderr into one stream and doesn't
/// preserve which one a line came from — there's no real "is this
/// stderr" signal to key off. This is the practical substitute: split
/// off the leading server-side timestamp (see `k8s::stream_logs`,
/// `timestamps: true`), bracket it and give it its own color (cyan,
/// matching the metadata/key color used in the spec tree view) so it
/// doesn't compete with gray — gray is reserved for normal-severity
/// message text. The message itself is heuristically colored by
/// scanning for error/warning keywords: substring match on
/// error/fatal/panic/fail → red, warn → yellow, else gray. It's a naive
/// heuristic, not a real log-level parser — "no errors occurred" would
/// still show red, since it's just checking for the substring "error."
/// Same approach most terminal log viewers fall back to in the absence
/// of real stream/level metadata.
fn colorize_log_line(raw: &str, timestamp_format: TimestampFormat, filter: &str) -> Line<'static> {
    let (timestamp, message) = match raw.split_once(' ') {
        Some((ts, rest)) if looks_like_timestamp(ts) => (Some(ts), rest),
        _ => (None, raw),
    };

    let lower = message.to_ascii_lowercase();
    let level_color = if ["error", "fatal", "panic", "fail"].iter().any(|kw| lower.contains(kw)) {
        Color::Red
    } else if lower.contains("warn") {
        Color::Yellow
    } else {
        Color::Gray
    };

    let mut spans = Vec::new();
    if let Some(ts) = timestamp {
        let display = match timestamp_format {
            TimestampFormat::Short => short_timestamp(ts),
            TimestampFormat::Full => ts.to_string(),
        };
        spans.push(Span::styled(format!("[{display}] "), Style::default().fg(Color::Cyan)));
    }
    spans.extend(highlight_matches(message, filter, Style::default().fg(level_color)));
    Line::from(spans)
}

/// Splits `text` around every case-insensitive occurrence of `needle`,
/// highlighting the matched part — otherwise a live filter narrows
/// *which* lines show up but gives no indication of *where* in each one
/// it actually matched. `needle` empty means no filter is active, so
/// the whole text just gets `base_style` unchanged.
fn highlight_matches(text: &str, needle: &str, base_style: Style) -> Vec<Span<'static>> {
    if needle.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let highlight_style = Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD);
    let lower_text = text.to_lowercase();
    let lower_needle = needle.to_lowercase();
    // Lowercasing can change byte lengths for some Unicode; byte offsets
    // wouldn't line up with `text`, so skip highlighting rather than panic.
    if lower_text.len() != text.len() || lower_needle.len() != needle.len() {
        return vec![Span::styled(text.to_string(), base_style)];
    }
    let mut spans = Vec::new();
    let mut rest = text;
    let mut rest_lower = lower_text.as_str();
    let mut consumed = 0;
    while let Some(pos) = rest_lower.find(&lower_needle) {
        if pos > 0 {
            spans.push(Span::styled(rest[..pos].to_string(), base_style));
        }
        spans.push(Span::styled(rest[pos..pos + needle.len()].to_string(), highlight_style));
        consumed += pos + needle.len();
        rest = &text[consumed..];
        rest_lower = &lower_text[consumed..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest.to_string(), base_style));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }
    spans
}

/// `2026-09-16T18:36:38.477289255Z` -> `18:36:38.477` — drops the date
/// (a live pod-log view is almost always "recent" logs, and if you're
/// scrolled back far enough for that to matter that's a rare edge case)
/// and truncates nanoseconds down to milliseconds, which is as much
/// precision as a human can actually use when reading logs by eye.
fn short_timestamp(ts: &str) -> String {
    let time_part = ts.split('T').nth(1).unwrap_or(ts).trim_end_matches('Z');
    match time_part.split_once('.') {
        Some((secs, frac)) => format!("{secs}.{}", &frac[..frac.len().min(3)]),
        None => time_part.to_string(),
    }
}

/// Cheap shape check for the RFC3339 timestamp `timestamps: true` adds
/// (e.g. `2026-09-16T18:36:38.477289255Z`) — not a full parse, just
/// enough to avoid misidentifying an ordinary line that happens to have
/// an early space (like our own `[failed to start log stream: ...]`
/// messages, which have no timestamp prefix at all).
fn looks_like_timestamp(s: &str) -> bool {
    s.len() >= 20 && s.as_bytes().get(4) == Some(&b'-') && s.contains('T') && s.ends_with('Z')
}

/// Click-to-toggle at an absolute terminal position — `TreeState` already
/// knows where everything was last rendered, so no manual hit-testing.
pub fn click_tree(state: &mut TreeState<String>, column: u16, row: u16) {
    if let Some(path) = state.rendered_at(Position::new(column, row)) {
        let path = path.to_vec();
        state.select(path.clone());
        state.toggle(path);
    }
}

/// Builds the collapsible tree for any k8s object's manifest, from its
/// generic YAML value tree (see `k8s::manifest_value`) — works for any
/// resource kind. Every level's identifier is its full path from the
/// root (e.g. `root/spec/containers/[0]/image`), which is what
/// `TreeState` uses to track open/closed and selection — so it stays
/// unique even though sibling branches reuse field names like `name`.
/// Alongside the tree itself, a lookup from a leaf's identifier (opaque,
/// but guaranteed unique — see below) to its `(label, full value)` —
/// tree items only ever show a value clipped to the box's width with no
/// indication it's cut off or way to see the rest, so `v` (see the
/// `Mode::Spec` keyboard handler) looks it up here to show untruncated.
/// A leaf's `(label, full value)`, by its tree identifier.
pub type LeafValues = HashMap<String, (String, String)>;

pub fn build_manifest_tree(value: &serde_yaml::Value) -> (Vec<TreeItem<'static, String>>, LeafValues) {
    let mut leaf_values = HashMap::new();
    let items = children_of(value, "root", &mut leaf_values);
    (items, leaf_values)
}

fn children_of(value: &serde_yaml::Value, path: &str, leaf_values: &mut LeafValues) -> Vec<TreeItem<'static, String>> {
    match value {
        serde_yaml::Value::Mapping(map) => map
            .iter()
            .map(|(k, v)| {
                let label = scalar_to_string(k);
                node(&format!("{path}/{label}"), &label, v, leaf_values)
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = format!("[{i}]");
                node(&format!("{path}/{label}"), &label, v, leaf_values)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn node(id: &str, label: &str, value: &serde_yaml::Value, leaf_values: &mut LeafValues) -> TreeItem<'static, String> {
    match value {
        serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => {
            let children = children_of(value, id, leaf_values);
            let text = Line::from(Span::styled(
                label.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            TreeItem::new(id.to_string(), text, children)
                .expect("child identifiers are unique per level by construction")
        }
        scalar => {
            let full_value = scalar_to_string(scalar);
            leaf_values.insert(id.to_string(), (label.to_string(), full_value.clone()));
            let text = Line::from(vec![Span::styled(format!("{label}: "), Style::default().fg(Color::Cyan)), Span::raw(full_value)]);
            TreeItem::new_leaf(id.to_string(), text)
        }
    }
}

fn scalar_to_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
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
mod generic_table_tests {
    use super::*;

    fn row(namespace: &str) -> GenericRow {
        GenericRow { namespace: namespace.to_string(), name: "x".to_string(), age: "1d".to_string() }
    }

    #[test]
    fn namespace_column_hidden_when_every_row_is_cluster_scoped() {
        assert!(!any_row_has_namespace(&[row("-"), row("-")]));
    }

    #[test]
    fn namespace_column_shown_when_any_row_has_a_real_namespace() {
        assert!(any_row_has_namespace(&[row("-"), row("default")]));
        assert!(!any_row_has_namespace(&[]));
    }
}

#[cfg(test)]
mod events_popup_tests {
    use super::*;

    fn entry(severity: crate::k8s::EventSeverity) -> EventEntry {
        EventEntry {
            message: "m".into(),
            reason: "r".into(),
            object: "o".into(),
            kind: "Pod".into(),
            age: "1m".into(),
            age_secs: 60,
            severity,
        }
    }

    #[test]
    fn workloads_pod_count_reads_the_pods_tile_from_the_catalog() {
        let overview = Overview {
            events: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog: vec![("Workloads", vec![("Pods", 17), ("Deployments", 4)])],
        };
        assert_eq!(workloads_pod_count(&overview), 17);
    }

    #[test]
    fn event_row_at_resolves_the_first_row_and_respects_offset() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 5, 0, top), Some(0));
        assert_eq!(event_row_at(frame_area, 5, 2, top), Some(2));
        assert_eq!(event_row_at(frame_area, 5, 0, top - 1), None); // header row, not a data row
    }

    #[test]
    fn event_row_at_is_none_past_the_filtered_list_or_the_table() {
        let frame_area = Rect { x: 0, y: 0, width: 100, height: 40 };
        let area = centered_rect(94, 88, frame_area);
        let top = area.y + 2;
        assert_eq!(event_row_at(frame_area, 1, 0, top + 1), None); // only 1 row exists
        let bottom = area.y + area.height - 1;
        assert_eq!(event_row_at(frame_area, 100, 0, bottom), None); // bottom border row
    }

    #[test]
    fn event_filter_matches_the_right_severities() {
        let normal = entry(crate::k8s::EventSeverity::Normal);
        let warning = entry(crate::k8s::EventSeverity::Warning);
        assert!(EventFilter::All.matches(&normal) && EventFilter::All.matches(&warning));
        assert!(EventFilter::Warnings.matches(&warning) && !EventFilter::Warnings.matches(&normal));
        assert!(EventFilter::Normal.matches(&normal) && !EventFilter::Normal.matches(&warning));
    }
}

#[cfg(test)]
mod column_detail_tests {
    use super::*;

    #[test]
    fn move_column_detail_selection_wraps_rows_within_a_single_grid() {
        // 5 items, 2 per row: [0 1] [2 3] [4]
        assert_eq!(move_column_detail_selection(5, 2, 0, Direction::Right), 1);
        assert_eq!(move_column_detail_selection(5, 2, 1, Direction::Down), 3);
        assert_eq!(move_column_detail_selection(5, 2, 4, Direction::Right), 4); // clamps, nothing after
        assert_eq!(move_column_detail_selection(5, 2, 0, Direction::Left), 0); // clamps, nothing before
    }

    #[test]
    fn column_detail_cols_and_visible_rows_are_at_least_one() {
        let tiny = Rect { x: 0, y: 0, width: 1, height: 1 };
        assert!(column_detail_cols(tiny) >= 1);
        assert!(column_detail_visible_rows(tiny) >= 1);
    }
}

#[cfg(test)]
mod log_color_tests {
    use super::*;

    #[test]
    fn timestamped_error_line_splits_and_colors_red() {
        let line = colorize_log_line(
            "2026-09-16T18:36:38.477289255Z connection refused: ERROR dialing upstream",
            TimestampFormat::Full,
            ""
        );
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "[2026-09-16T18:36:38.477289255Z] ");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn short_format_truncates_to_millisecond_time_of_day() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Short, "");
        assert_eq!(line.spans[0].content, "[18:36:38.477] ");
    }

    #[test]
    fn warning_line_colors_yellow() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z WARN: retrying in 5s", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn plain_line_colors_gray() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Full, "");
        assert_eq!(line.spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn filter_match_is_highlighted_case_insensitively() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z hello World", TimestampFormat::Full, "world");
        let hl: Vec<_> = line.spans.iter().filter(|s| s.style.bg == Some(Color::Yellow)).collect();
        assert_eq!(hl.len(), 1);
        assert_eq!(hl[0].content, "World");
    }

    #[test]
    fn line_without_timestamp_has_no_timestamp_span() {
        let line = colorize_log_line("[failed to start log stream: connection reset]", TimestampFormat::Short, "");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, Some(Color::Red)); // "failed" matches
    }
}

#[cfg(test)]
mod overview_selection_tests {
    use super::*;

    fn test_overview(catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>) -> Overview {
        Overview {
            events: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog,
        }
    }

    #[test]
    fn down_moves_to_next_item_within_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 0), Direction::Down),
            OverviewSelection::Item(0, 1)
        );
    }

    #[test]
    fn down_stops_at_the_last_item_of_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let last = OverviewSelection::Item(0, 1);
        assert_eq!(move_overview_selection(&overview, last, Direction::Down), last);
    }

    #[test]
    fn up_at_the_first_item_goes_to_the_columns_own_header() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 0), Direction::Up),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn down_from_a_header_enters_its_first_item() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Down),
            OverviewSelection::Item(0, 0)
        );
    }

    #[test]
    fn up_from_any_column_header_goes_to_events() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Up), OverviewSelection::Events);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Up), OverviewSelection::Events);
    }

    #[test]
    fn events_and_resources_navigate_vertically_into_each_other_and_the_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Down), OverviewSelection::Events);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Down), OverviewSelection::Header(0));
    }

    #[test]
    fn resources_and_events_ignore_left_right_and_resources_ignores_up() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Up), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Resources, Direction::Left), OverviewSelection::Resources);
        assert_eq!(move_overview_selection(&overview, OverviewSelection::Events, Direction::Right), OverviewSelection::Events);
    }

    #[test]
    fn left_right_move_headers_directly_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(0), Direction::Right),
            OverviewSelection::Header(1)
        );
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Header(1), Direction::Left),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn left_right_move_items_at_the_same_index_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, OverviewSelection::Item(0, 1), Direction::Right),
            OverviewSelection::Item(1, 1)
        );
    }

    #[test]
    fn movement_clamps_at_the_first_and_last_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        let only = OverviewSelection::Header(0);
        assert_eq!(move_overview_selection(&overview, only, Direction::Left), only);
        assert_eq!(move_overview_selection(&overview, only, Direction::Right), only);
    }

    #[test]
    fn scroll_columns_to_show_brings_a_column_off_either_edge_into_view() {
        assert_eq!(scroll_columns_to_show(0, 3, 5), 3); // off the right edge
        assert_eq!(scroll_columns_to_show(3, 3, 1), 1); // off the left edge
        assert_eq!(scroll_columns_to_show(2, 3, 3), 2); // already visible
    }

    #[test]
    fn column_hit_resolves_resources_events_header_and_item_rows() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        // +1 for the gap `columns_area` now puts between the top strip
        // and the columns, matching the Resources-to-Events gap.
        let top_h = top_area_height(&overview) + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, 0), Some(OverviewSelection::Resources));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, resources_box_height(&overview) + 1), Some(OverviewSelection::Events));
        // Row 0 of the columns area is the column box's top border (the
        // header); rows 1-3 are the first item card (border/content/border).
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, top_h), Some(OverviewSelection::Header(0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, top_h + 1), Some(OverviewSelection::Item(0, 0)));
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 0, 1, top_h + 4), Some(OverviewSelection::Item(0, 1)));
    }

    #[test]
    fn column_hit_uses_item_scroll_only_for_the_active_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0), ("a3", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        let top_h = top_area_height(&overview) + 1;
        // Column 0 is active with item_scroll 1: its first visible card is
        // actually item index 1, not 0.
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, 1, top_h + 1), Some(OverviewSelection::Item(0, 1)));
        // Column 1 isn't active, so it always renders from item 0
        // regardless of the (irrelevant, for it) item_scroll value. The
        // columns area has a 1-cell left scroll-arrow gutter before the
        // first column box starts.
        let col1_x = 1 + COLUMN_WIDTH + 1;
        assert_eq!(column_hit(frame_area, &overview, 0, 0, 1, col1_x, top_h + 1), Some(OverviewSelection::Item(1, 0)));
    }

    #[test]
    fn visible_columns_accounts_for_the_inter_column_gap() {
        // Two columns need 2*COLUMN_WIDTH + 1 cells (one gap between them),
        // not 2*COLUMN_WIDTH.
        let two_cols_width = COLUMN_WIDTH * 2 + 1;
        assert_eq!(visible_columns(two_cols_width, 5), 2);
        assert_eq!(visible_columns(two_cols_width - 1, 5), 1);
    }
}
