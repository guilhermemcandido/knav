use std::collections::HashSet;

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::config::TimestampFormat;
use crate::icons::IconCache;
use crate::k8s::{ContainerInfo, ContainerStatusKind, CrdInfo, DeploymentRow, GenericRow, NodeRow, Overview, PodRow, ResourceKind, Warning};

pub enum Rows<'a> {
    /// `usize` is the horizontal column scroll offset; the last field is
    /// which columns are collapsed.
    Overview(&'a Overview, OverviewSelection, usize, &'a HashSet<usize>),
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
    Logs { title: &'a str, lines: &'a [String], scroll: u16, follow: bool, timestamp_format: TimestampFormat },
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
        pods: &'a [PodRow],
        state: &'a mut TableState,
    },
    /// A vim/k9s-style `:` command line, drawn as a plain bottom bar —
    /// unlike every other overlay it doesn't dim the background, since
    /// you're still looking at (and can still see) the view you're
    /// about to switch away from.
    Command { input: &'a str },
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

#[allow(clippy::too_many_arguments)]
pub fn draw(
    frame: &mut Frame,
    rows: Rows,
    table_state: &mut TableState,
    hover: Option<Hover>,
    overlay: Option<Overlay>,
    icons: &mut IconCache,
) {
    // The command line doesn't dim the background — you're still meant
    // to see (and read) the view you're about to switch away from, same
    // as k9s's own `:` prompt.
    let dimmed = matches!(
        overlay,
        Some(Overlay::Spec { .. })
            | Some(Overlay::Containers { .. })
            | Some(Overlay::Logs { .. })
            | Some(Overlay::Menu { .. })
            | Some(Overlay::NodeDetail { .. })
    );

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
        Rows::Overview(overview, selection, col_scroll, collapsed) => {
            draw_overview(frame, frame.area(), overview, selection, col_scroll, collapsed, dimmed, icons);
        }
        Rows::Generic(rows, label) => {
            draw_generic_table(frame, frame.area(), rows, label, table_state, dimmed);
        }
        Rows::CrdList(crds, heading) => {
            draw_crd_list_table(frame, frame.area(), crds, heading, table_state, dimmed);
        }
    }

    if let Some(overlay) = overlay {
        match overlay {
            Overlay::Spec { title, items, state } => draw_spec_popup(frame, title, items, state),
            Overlay::Containers { title, containers, state } => draw_containers_popup(frame, title, containers, state),
            Overlay::Logs { title, lines, scroll, follow, timestamp_format } => {
                draw_logs_popup(frame, title, lines, scroll, follow, timestamp_format)
            }
            Overlay::Menu { sections, selected } => draw_menu_popup(frame, sections, selected),
            Overlay::NodeDetail { name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, pods, state } => {
                draw_node_detail_popup(frame, name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, pods, state)
            }
            Overlay::Command { input } => draw_command_bar(frame, input),
        }
    }
}

fn container_dot(status: ContainerStatusKind) -> (&'static str, Color) {
    match status {
        ContainerStatusKind::Running => ("●", Color::Green),
        ContainerStatusKind::Waiting => ("●", Color::Yellow),
        ContainerStatusKind::Terminated => ("●", Color::Red),
        ContainerStatusKind::Unknown => ("●", Color::Gray),
    }
}

/// Compact form: just the colored dots — used for every row except the
/// one that's hovered/selected.
fn containers_cell(containers: &[ContainerInfo], muted: bool) -> Line<'static> {
    let mut spans = Vec::with_capacity(containers.len() * 2);
    for c in containers {
        let (glyph, color) = container_dot(c.status);
        let style = if muted { Style::default().fg(Color::DarkGray) } else { Style::default().fg(color) };
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

/// The always-visible status line below the table, for the
/// keyboard-selected row — works regardless of mouse/terminal support.
/// The `:` command bar — a single line pinned to the very bottom of the
/// screen, on top of whatever's there (same spot the Pods status line
/// uses, when there is one — you're not looking at container state while
/// typing a command anyway).
fn draw_command_bar(frame: &mut Frame, input: &str) {
    let area = frame.area();
    let bar = Rect { x: area.x, y: area.y + area.height.saturating_sub(1), width: area.width, height: 1 };
    frame.render_widget(Clear, bar);
    let line = Line::styled(format!(":{input}"), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    frame.render_widget(Paragraph::new(line), bar);
}

fn draw_status_line(frame: &mut Frame, area: Rect, pods: &[PodRow], row: Option<usize>, dimmed: bool) {
    let line = match (dimmed, row.and_then(|i| pods.get(i))) {
        (false, Some(pod)) => {
            let mut spans = vec![
                Span::styled(format!("{}/{}", pod.namespace, pod.name), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw("  —  "),
            ];
            for (i, c) in pod.containers.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::raw("   "));
                }
                let (glyph, color) = container_dot(c.status);
                let style = Style::default().fg(color);
                spans.push(Span::styled(format!("{glyph} "), style));
                spans.push(Span::raw(format!("{}: ", c.name)));
                spans.push(Span::styled(container_state_text(c), style));
            }
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
            let (glyph, color) = container_dot(c.status);
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
    let muted = Style::default().fg(Color::DarkGray);
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

    let title = format!("Pods ({})  —  j/k: move  enter: containers  d: spec  q: quit", pods.len());

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
    let muted = Style::default().fg(Color::DarkGray);
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

    let title = format!("Deployments ({})  —  j/k: move  d: spec  m: switch resource  q: quit", deployments.len());

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
    Line::from(vec![
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(WIDTH - filled), Style::default().fg(Color::DarkGray)),
        Span::raw(format!(" {:.0}%", ratio * 100.0)),
    ])
}

fn usage_color(ratio: f64, dimmed: bool) -> Color {
    if dimmed {
        Color::DarkGray
    } else if ratio > 0.9 {
        Color::Red
    } else if ratio > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn draw_nodes_table(frame: &mut Frame, area: Rect, nodes: &[NodeRow], table_state: &mut TableState, dimmed: bool) {
    let muted = Style::default().fg(Color::DarkGray);
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["NAME", "STATUS", "CPU", "MEMORY", "PODS", "AGE"]).style(header_style);

    let rows = nodes.iter().map(|n| {
        let status_style = if dimmed {
            muted
        } else if n.ready {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Red)
        };
        Row::new(vec![
            Cell::from(n.name.clone()).style(cell_style),
            Cell::from(if n.ready { "Ready" } else { "NotReady" }).style(status_style),
            Cell::from(usage_bar(n.cpu_millicores, n.cpu_capacity, dimmed)),
            Cell::from(usage_bar(n.memory_bytes, n.memory_capacity, dimmed)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)).style(cell_style),
            Cell::from(n.age.clone()).style(cell_style),
        ])
    });

    let widths = [
        Constraint::Fill(2),
        Constraint::Length(9),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Length(9),
        Constraint::Length(5),
    ];

    let title = format!("Nodes ({})  —  j/k: move  enter: what's running  d: spec  m: switch resource  q: quit", nodes.len());

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
    let muted = Style::default().fg(Color::DarkGray);
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

    let title = format!("{label} ({})  —  j/k: move  d: spec  m: switch resource  q: quit", rows.len());

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
    let muted = Style::default().fg(Color::DarkGray);
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
    let title = format!("{heading} ({})  —  j/k: move  enter: open  m: switch resource  q: quit", crds.len());

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
/// One catalog column's fixed width in the Overview browser.
const COLUMN_WIDTH: u16 = 26;
/// Cluster Issues is a fixed-size dashboard strip now, not a scrollable
/// section — cap how many warnings it shows directly, with a "+N more"
/// line instead of growing to fit all of them.
const MAX_VISIBLE_ISSUES: usize = 5;

/// The home screen: a fixed-size dashboard strip up top (CPU/Memory/Pods
/// meters, then Cluster Issues — verified against Freelens's actual
/// `cluster-issues.tsx` source for what belongs there), and below it a
/// horizontally-scrollable set of columns, one per resource category
/// (Cluster, Workloads, Config, ...), each listing that category's kinds
/// vertically — Miller-columns style, replacing the previous flow-
/// wrapping tile grid.
#[allow(clippy::too_many_arguments)]
fn draw_overview(
    frame: &mut Frame,
    area: Rect,
    overview: &Overview,
    selection: OverviewSelection,
    col_scroll: usize,
    collapsed: &HashSet<usize>,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let top_h = top_area_height(overview);
    let chunks = Layout::vertical([Constraint::Length(top_h), Constraint::Min(0)]).split(area);
    draw_top_panel(frame, chunks[0], overview, dimmed);
    draw_columns(frame, chunks[1], overview, selection, col_scroll, collapsed, dimmed, icons);
}

/// How tall the fixed top dashboard strip is — depends on how many
/// warnings there actually are (up to `MAX_VISIBLE_ISSUES`), so callers
/// (mouse hit-testing, the columns area below it) can't drift out of
/// sync with what's actually rendered.
fn top_area_height(overview: &Overview) -> u16 {
    1 + if overview.metrics_available { 3 } else { 2 } + 1 + issues_content_height(overview)
}

/// Height of the Issues section's content only (below its "Cluster
/// Issues" label line) — either the 2-line "no issues" message, or the
/// column-header row plus up to `MAX_VISIBLE_ISSUES` warnings plus a
/// "+N more" line if there are more than that. `top_area_height` and
/// `draw_top_panel` both use this so they can't drift apart.
fn issues_content_height(overview: &Overview) -> u16 {
    if overview.warnings.is_empty() {
        return 2;
    }
    let shown = overview.warnings.len().min(MAX_VISIBLE_ISSUES);
    let more = usize::from(overview.warnings.len() > MAX_VISIBLE_ISSUES);
    1 + (shown + more) as u16
}

/// The columns area is whatever's left below the fixed dashboard strip
/// — callers (keyboard navigation, mouse hit-testing) need this same
/// rectangle to stay in sync with what's actually rendered.
pub fn columns_area(frame_area: Rect, overview: &Overview) -> Rect {
    let top_h = top_area_height(overview);
    Rect { x: frame_area.x, y: frame_area.y + top_h, width: frame_area.width, height: frame_area.height.saturating_sub(top_h) }
}

pub fn visible_columns(width: u16, total_columns: usize) -> usize {
    ((width / COLUMN_WIDTH).max(1) as usize).min(total_columns.max(1))
}

fn draw_top_panel(frame: &mut Frame, area: Rect, overview: &Overview, dimmed: bool) {
    let label_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().add_modifier(Modifier::BOLD) };
    let metrics_h = 1 + if overview.metrics_available { 3 } else { 2 };
    let chunks = Layout::vertical([Constraint::Length(metrics_h), Constraint::Min(0)]).split(area);

    let metrics_rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(chunks[0]);
    frame.render_widget(Paragraph::new(Line::styled("Cluster Resources", label_style)), metrics_rows[0]);
    draw_metrics_lines(frame, metrics_rows[1], overview, dimmed);

    let issues_rows = Layout::vertical([Constraint::Length(1), Constraint::Length(issues_content_height(overview))]).split(chunks[1]);
    frame.render_widget(Paragraph::new(Line::styled("Cluster Issues", label_style)), issues_rows[0]);
    if overview.warnings.is_empty() {
        draw_issues_empty(frame, issues_rows[1], dimmed);
    } else {
        let shown = overview.warnings.len().min(MAX_VISIBLE_ISSUES);
        let has_more = overview.warnings.len() > MAX_VISIBLE_ISSUES;
        let lines = Layout::vertical((0..1 + shown + usize::from(has_more)).map(|_| Constraint::Length(1))).split(issues_rows[1]);
        draw_issues_header(frame, lines[0], overview.warnings.len(), dimmed);
        for (i, w) in overview.warnings.iter().take(shown).enumerate() {
            draw_issue_line(frame, lines[i + 1], w, dimmed);
        }
        if has_more {
            let more = overview.warnings.len() - shown;
            let style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC) };
            frame.render_widget(Paragraph::new(Line::styled(format!("… and {more} more"), style)).alignment(Alignment::Center), lines[1 + shown]);
        }
    }
}

/// The CPU/Memory/Pods meters, one per line.
fn draw_metrics_lines(frame: &mut Frame, area: Rect, overview: &Overview, dimmed: bool) {
    if !overview.metrics_available {
        let text = vec![
            Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(Color::DarkGray)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
        return;
    }

    let pod_usage = overview
        .catalog
        .iter()
        .find(|(section, _)| *section == "Workloads")
        .and_then(|(_, tiles)| tiles.iter().find(|(label, _)| *label == "Pods"))
        .map(|(_, count)| *count)
        .unwrap_or(0);

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
    let reserved = label_text.chars().count() as u16 + detail.chars().count() as u16 + 3;
    let bar_width = area.width.saturating_sub(reserved).max(4) as usize;
    let filled = ((ratio * bar_width as f64).round() as usize).min(bar_width);

    let label_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().add_modifier(Modifier::BOLD) };
    let detail_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default() };

    let line = Line::from(vec![
        Span::styled(label_text, label_style),
        Span::styled("▓".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_width - filled), Style::default().fg(Color::DarkGray)),
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

/// Selection within the Overview's column browser — either a column's
/// own header (selectable so it can be toggled without a mouse) or a
/// specific item within a column.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverviewSelection {
    Header(usize),
    Item(usize, usize),
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn column_len(overview: &Overview, collapsed: &HashSet<usize>, col: usize) -> usize {
    if collapsed.contains(&col) { 0 } else { overview.catalog.get(col).map(|(_, items)| items.len()).unwrap_or(0) }
}

/// Moves the Overview selection one step in a direction across the
/// column browser. Up/Down move into and out of a column's own header
/// (pressing Up at a column's first item lands on its header; pressing
/// Down on a header enters its first item, or does nothing if the
/// column is collapsed/empty) — Left/Right move directly between
/// columns at the same item index, landing on the target column's
/// header instead if it's collapsed or has nothing at that index.
pub fn move_overview_selection(overview: &Overview, collapsed: &HashSet<usize>, selection: OverviewSelection, dir: Direction) -> OverviewSelection {
    let total = overview.catalog.len();
    if total == 0 {
        return selection;
    }
    match selection {
        OverviewSelection::Header(col) => {
            let col = col.min(total - 1);
            match dir {
                Direction::Down => {
                    if column_len(overview, collapsed, col) > 0 { OverviewSelection::Item(col, 0) } else { selection }
                }
                Direction::Up => selection,
                Direction::Left => if col > 0 { OverviewSelection::Header(col - 1) } else { selection },
                Direction::Right => if col + 1 < total { OverviewSelection::Header(col + 1) } else { selection },
            }
        }
        OverviewSelection::Item(col, item) => {
            let col = col.min(total - 1);
            let len = column_len(overview, collapsed, col).max(1);
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
                        let target_len = column_len(overview, collapsed, col - 1);
                        if target_len == 0 { OverviewSelection::Header(col - 1) } else { OverviewSelection::Item(col - 1, item.min(target_len - 1)) }
                    }
                }
                Direction::Right => {
                    if col + 1 >= total {
                        selection
                    } else {
                        let target_len = column_len(overview, collapsed, col + 1);
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

/// Adjusts `col_scroll` (if needed) so `target_col` is fully within the
/// `cols_visible` columns currently on screen — scrolls left immediately
/// if the selection moved off the left edge, or right just far enough if
/// it moved off the right edge.
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
/// position — same column layout `draw_columns` actually renders with,
/// so a click always resolves to what's really on screen.
pub fn column_hit(frame_area: Rect, overview: &Overview, col_scroll: usize, collapsed: &HashSet<usize>, column: u16, row: u16) -> Option<OverviewSelection> {
    let area = columns_area(frame_area, overview);
    if row < area.y || row >= area.y + area.height || column < area.x || column >= area.x + area.width {
        return None;
    }
    let total = overview.catalog.len();
    if total == 0 {
        return None;
    }
    let cols_visible = visible_columns(area.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let col_i = ((column - area.x) / COLUMN_WIDTH) as usize;
    if col_i >= cols_visible {
        return None;
    }
    let col_idx = col_scroll + col_i;
    let rel_y = row - area.y;
    if rel_y == 0 {
        return Some(OverviewSelection::Header(col_idx));
    }
    if collapsed.contains(&col_idx) {
        return None;
    }
    let item_i = (rel_y - 1) as usize;
    let (_, items) = &overview.catalog[col_idx];
    if item_i < items.len() { Some(OverviewSelection::Item(col_idx, item_i)) } else { None }
}

#[allow(clippy::too_many_arguments)]
fn draw_columns(
    frame: &mut Frame,
    area: Rect,
    overview: &Overview,
    selection: OverviewSelection,
    col_scroll: usize,
    collapsed: &HashSet<usize>,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let total = overview.catalog.len();
    if total == 0 {
        return;
    }
    let cols_visible = visible_columns(area.width, total);
    let col_scroll = col_scroll.min(total - cols_visible);
    let constraints: Vec<Constraint> = (0..cols_visible).map(|_| Constraint::Length(COLUMN_WIDTH)).collect();
    let areas = Layout::horizontal(constraints).split(area);
    for (i, col_area) in areas.iter().enumerate() {
        let col_idx = col_scroll + i;
        let (title, items) = &overview.catalog[col_idx];
        draw_column(frame, *col_area, col_idx, title, items, selection, collapsed.contains(&col_idx), dimmed, icons);
    }
}

/// One column: a centered, selectable header (`▾`/`▸` indicator, same
/// convention as the spec tree) followed by that category's kinds
/// listed vertically, one compact row each.
#[allow(clippy::too_many_arguments)]
fn draw_column(
    frame: &mut Frame,
    area: Rect,
    col_idx: usize,
    title: &str,
    items: &[(&str, usize)],
    selection: OverviewSelection,
    collapsed: bool,
    dimmed: bool,
    icons: &mut IconCache,
) {
    let header_selected = matches!(selection, OverviewSelection::Header(c) if c == col_idx);
    let indicator = if collapsed { "▸" } else { "▾" };
    let header_style = if dimmed {
        Style::default().fg(Color::DarkGray)
    } else if header_selected {
        Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    let visible_items = if collapsed { 0 } else { items.len() };
    let mut constraints = vec![Constraint::Length(1)];
    constraints.extend((0..visible_items).map(|_| Constraint::Length(1)));
    let rows = Layout::vertical(constraints).split(area);

    if header_selected && !dimmed {
        frame.render_widget(Block::default().style(Style::default().bg(Color::Cyan)), rows[0]);
    }
    frame.render_widget(Paragraph::new(Line::styled(format!("{indicator} {title}"), header_style)).alignment(Alignment::Center), rows[0]);

    if !collapsed {
        for (i, (label, count)) in items.iter().enumerate() {
            let selected = matches!(selection, OverviewSelection::Item(c, it) if c == col_idx && it == i);
            draw_column_item(frame, rows[i + 1], label, *count, title, selected, dimmed, icons);
        }
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

/// One compact, single-line item: a small icon, the kind name, and its
/// live count — deliberately short (one row, one small icon) so a whole
/// column's worth of kinds reads at a glance without scrolling.
#[allow(clippy::too_many_arguments)]
fn draw_column_item(frame: &mut Frame, area: Rect, label: &str, count: usize, column_title: &str, selected: bool, dimmed: bool, icons: &mut IconCache) {
    if selected && !dimmed {
        frame.render_widget(Block::default().style(Style::default().bg(Color::Cyan)), area);
    }

    let icon_w = 3u16.min(area.width);
    let split = Layout::horizontal([Constraint::Length(icon_w), Constraint::Min(0)]).split(area);

    match (dimmed, resolve_icon_kind(label, column_title)) {
        (false, Some(kind)) => icons.draw(frame, icons.centered_square(split[0]), kind),
        _ => frame.render_widget(Paragraph::new(icon_for(label)).alignment(Alignment::Center), split[0]),
    }

    let text_style = if selected && !dimmed {
        Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)
    } else if dimmed {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };
    let count_style = if selected && !dimmed {
        Style::default().bg(Color::Cyan).fg(Color::Black)
    } else if dimmed {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::Cyan)
    };

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

fn draw_issues_header(frame: &mut Frame, area: Rect, count: usize, dimmed: bool) {
    let style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().add_modifier(Modifier::BOLD) };
    let line = format!("{:<50} {:<18} {:<12} AGE  ({count})", "MESSAGE", "OBJECT", "KIND");
    frame.render_widget(Paragraph::new(Line::styled(line, style)), area);
}

fn draw_issues_empty(frame: &mut Frame, area: Rect, dimmed: bool) {
    let ok_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) };
    let sub_style = Style::default().fg(Color::DarkGray);
    let text = vec![
        Line::styled("✓ No issues found", ok_style),
        Line::styled("Everything is fine in the cluster", sub_style),
    ];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), area);
}

fn draw_issue_line(frame: &mut Frame, area: Rect, warning: &Warning, dimmed: bool) {
    let color = if dimmed {
        Color::DarkGray
    } else if warning.kind == "Node" {
        Color::Red // a not-ready/pressured node affects everything scheduled on it
    } else {
        Color::Yellow
    };
    let message = truncate(&warning.message, 48);
    let line = format!("{message:<50} {:<18} {:<12} {}", warning.object, warning.kind, warning.age);
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
        .title("Switch resource  —  arrows/hjkl: move  enter: select  esc: cancel");
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
    pods: &[PodRow],
    state: &mut TableState,
) {
    let area = centered_rect(90, 88, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(format!("Node: {name}  —  j/k: move  enter: containers  d: spec  esc: back"));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(inner);

    match (cpu_usage, memory_usage) {
        (Some(cpu), Some(mem)) => {
            let lines = Layout::vertical([Constraint::Length(1); 3]).split(chunks[0]);
            draw_meter(frame, lines[0], "CPU", cpu as f64, cpu_capacity as f64, |v| format!("{:.2} cores", v / 1000.0), false);
            draw_meter(frame, lines[1], "Memory", mem as f64, memory_capacity as f64, format_bytes, false);
            draw_meter(frame, lines[2], "Pods", pods.len() as f64, pod_capacity as f64, |v| format!("{v:.0}"), false);
        }
        _ => {
            let text = Paragraph::new(Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Center);
            frame.render_widget(text, chunks[0]);
        }
    }

    draw_table(frame, chunks[1], pods, state, false);
}

fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>) {
    let area = centered_rect(85, 85, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!(
        "{title}  —  ↑↓/jk: move  ←→/hl: collapse/expand  enter/click: toggle  esc: back"
    ));

    let tree = Tree::new(items)
        .expect("pod tree ids are unique per level by construction")
        .block(block)
        .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("  ");

    frame.render_stateful_widget(tree, area, state);
}

fn draw_containers_popup(frame: &mut Frame, title: &str, containers: &[ContainerInfo], state: &mut TableState) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let header = Row::new(vec!["", "NAME", "STATE", "RESTARTS"]).style(Style::default().add_modifier(Modifier::BOLD));
    let rows = containers.iter().map(|c| {
        let (glyph, color) = container_dot(c.status);
        let state_text = c.reason.clone().unwrap_or_else(|| match c.status {
            ContainerStatusKind::Running => "Running".into(),
            ContainerStatusKind::Waiting => "Waiting".into(),
            ContainerStatusKind::Terminated => "Terminated".into(),
            ContainerStatusKind::Unknown => "Unknown".into(),
        });
        Row::new(vec![
            Cell::from(Span::styled(glyph, Style::default().fg(color))),
            Cell::from(c.name.clone()),
            Cell::from(state_text).style(Style::default().fg(color)),
            Cell::from(c.restarts.to_string()),
        ])
    });

    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(45),
        Constraint::Percentage(35),
        Constraint::Percentage(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!("{title}  —  j/k: move  enter: logs  esc: back")))
        .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol("➤ ");

    frame.render_stateful_widget(table, area, state);
}

fn draw_logs_popup(frame: &mut Frame, title: &str, lines: &[String], scroll: u16, follow: bool, timestamp_format: TimestampFormat) {
    let area = centered_rect(90, 90, frame.area());
    frame.render_widget(Clear, area);

    let follow_status = if follow { "following — j/k or ↑↓ to pause" } else { "paused — G to resume following" };
    let ts_status = match timestamp_format {
        TimestampFormat::Short => "short ts",
        TimestampFormat::Full => "full ts",
    };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(format!(
        "{title}  —  {follow_status}  t: toggle timestamp ({ts_status})  esc: back  ({} lines)",
        lines.len()
    ));

    // When following, always show exactly the tail that fits the visible
    // area — simpler and more robust than trusting Paragraph's own scroll
    // clamping to not show blank space past the end of the content.
    let (text, effective_scroll): (Vec<Line>, u16) = if follow {
        let visible = area.height.saturating_sub(2) as usize; // minus borders
        let start = lines.len().saturating_sub(visible);
        (lines[start..].iter().map(|l| colorize_log_line(l, timestamp_format)).collect(), 0)
    } else {
        (lines.iter().map(|l| colorize_log_line(l, timestamp_format)).collect(), scroll)
    };

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false }).scroll((effective_scroll, 0));

    frame.render_widget(paragraph, area);
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
fn colorize_log_line(raw: &str, timestamp_format: TimestampFormat) -> Line<'static> {
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
    spans.push(Span::styled(message.to_string(), Style::default().fg(level_color)));
    Line::from(spans)
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
pub fn build_manifest_tree(value: &serde_yaml::Value) -> Vec<TreeItem<'static, String>> {
    children_of(value, "root")
}

fn children_of(value: &serde_yaml::Value, path: &str) -> Vec<TreeItem<'static, String>> {
    match value {
        serde_yaml::Value::Mapping(map) => map
            .iter()
            .map(|(k, v)| {
                let label = scalar_to_string(k);
                node(&format!("{path}/{label}"), &label, v)
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = format!("[{i}]");
                node(&format!("{path}/{label}"), &label, v)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn node(id: &str, label: &str, value: &serde_yaml::Value) -> TreeItem<'static, String> {
    match value {
        serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => {
            let children = children_of(value, id);
            let text = Line::from(Span::styled(
                label.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            TreeItem::new(id.to_string(), text, children)
                .expect("child identifiers are unique per level by construction")
        }
        scalar => {
            let text = Line::from(vec![
                Span::styled(format!("{label}: "), Style::default().fg(Color::Cyan)),
                Span::raw(scalar_to_string(scalar)),
            ]);
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
mod log_color_tests {
    use super::*;

    #[test]
    fn timestamped_error_line_splits_and_colors_red() {
        let line = colorize_log_line(
            "2026-09-16T18:36:38.477289255Z connection refused: ERROR dialing upstream",
            TimestampFormat::Full,
        );
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "[2026-09-16T18:36:38.477289255Z] ");
        assert_eq!(line.spans[0].style.fg, Some(Color::Cyan));
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn short_format_truncates_to_millisecond_time_of_day() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Short);
        assert_eq!(line.spans[0].content, "[18:36:38.477] ");
    }

    #[test]
    fn warning_line_colors_yellow() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z WARN: retrying in 5s", TimestampFormat::Full);
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn plain_line_colors_gray() {
        let line = colorize_log_line("2026-09-16T18:36:38.477289255Z line 0", TimestampFormat::Full);
        assert_eq!(line.spans[1].style.fg, Some(Color::Gray));
    }

    #[test]
    fn line_without_timestamp_has_no_timestamp_span() {
        let line = colorize_log_line("[failed to start log stream: connection reset]", TimestampFormat::Short);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, Some(Color::Red)); // "failed" matches
    }
}

#[cfg(test)]
mod overview_selection_tests {
    use super::*;

    fn test_overview(catalog: Vec<(&'static str, Vec<(&'static str, usize)>)>) -> Overview {
        Overview {
            warnings: vec![],
            cpu_usage_millicores: 0,
            cpu_capacity_millicores: 0,
            memory_usage_bytes: 0,
            memory_capacity_bytes: 0,
            pod_capacity: 0,
            metrics_available: false,
            catalog,
        }
    }

    fn none_collapsed() -> HashSet<usize> {
        HashSet::new()
    }

    #[test]
    fn down_moves_to_next_item_within_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Item(0, 0), Direction::Down),
            OverviewSelection::Item(0, 1)
        );
    }

    #[test]
    fn down_stops_at_the_last_item_of_a_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let last = OverviewSelection::Item(0, 1);
        assert_eq!(move_overview_selection(&overview, &none_collapsed(), last, Direction::Down), last);
    }

    #[test]
    fn up_at_the_first_item_goes_to_the_columns_own_header() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Item(0, 0), Direction::Up),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn down_from_a_header_enters_its_first_item() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Header(0), Direction::Down),
            OverviewSelection::Item(0, 0)
        );
    }

    #[test]
    fn down_from_a_collapsed_columns_header_does_nothing() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let collapsed: HashSet<usize> = [0].into_iter().collect();
        let header = OverviewSelection::Header(0);
        assert_eq!(move_overview_selection(&overview, &collapsed, header, Direction::Down), header);
    }

    #[test]
    fn left_right_move_headers_directly_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Header(0), Direction::Right),
            OverviewSelection::Header(1)
        );
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Header(1), Direction::Left),
            OverviewSelection::Header(0)
        );
    }

    #[test]
    fn left_right_move_items_at_the_same_index_between_columns() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)]), ("B", vec![("b1", 0), ("b2", 0)])]);
        assert_eq!(
            move_overview_selection(&overview, &none_collapsed(), OverviewSelection::Item(0, 1), Direction::Right),
            OverviewSelection::Item(1, 1)
        );
    }

    #[test]
    fn right_into_a_collapsed_column_lands_on_its_header_instead_of_a_nonexistent_item() {
        let overview = test_overview(vec![("A", vec![("a1", 0)]), ("B", vec![("b1", 0)])]);
        let collapsed: HashSet<usize> = [1].into_iter().collect();
        assert_eq!(
            move_overview_selection(&overview, &collapsed, OverviewSelection::Item(0, 0), Direction::Right),
            OverviewSelection::Header(1)
        );
    }

    #[test]
    fn movement_clamps_at_the_first_and_last_column() {
        let overview = test_overview(vec![("A", vec![("a1", 0)])]);
        let only = OverviewSelection::Header(0);
        assert_eq!(move_overview_selection(&overview, &none_collapsed(), only, Direction::Left), only);
        assert_eq!(move_overview_selection(&overview, &none_collapsed(), only, Direction::Right), only);
    }

    #[test]
    fn scroll_columns_to_show_brings_a_column_off_either_edge_into_view() {
        assert_eq!(scroll_columns_to_show(0, 3, 5), 3); // off the right edge
        assert_eq!(scroll_columns_to_show(3, 3, 1), 1); // off the left edge
        assert_eq!(scroll_columns_to_show(2, 3, 3), 2); // already visible
    }

    #[test]
    fn column_hit_resolves_header_and_item_rows() {
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0)])]);
        let frame_area = Rect { x: 0, y: 0, width: 80, height: 40 };
        let top_h = top_area_height(&overview);
        // Row 0 of the columns area is the header; row 1 is the first item.
        assert_eq!(column_hit(frame_area, &overview, 0, &none_collapsed(), 1, top_h), Some(OverviewSelection::Header(0)));
        assert_eq!(column_hit(frame_area, &overview, 0, &none_collapsed(), 1, top_h + 1), Some(OverviewSelection::Item(0, 0)));
    }
}
