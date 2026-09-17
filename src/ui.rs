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
use crate::k8s::{ContainerInfo, ContainerStatusKind, CrdInfo, DeploymentRow, GenericRow, Overview, PodRow, ResourceKind, Warning};

pub enum Rows<'a> {
    Overview(&'a Overview, usize, (usize, usize)),
    Pods(&'a [PodRow]),
    Deployments(&'a [DeploymentRow]),
    /// Every other resource kind — a plain namespace/name/age table,
    /// labeled with the kind so the title bar and log line make sense.
    Generic(&'a [GenericRow], &'static str),
    /// The Custom Resources picker — every discovered CRD kind, not yet
    /// any specific kind's instances.
    CrdList(&'a [CrdInfo]),
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
    let dimmed = overlay.is_some();

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
        Rows::Overview(overview, scroll, selected) => {
            draw_overview(frame, frame.area(), overview, scroll, selected, dimmed, icons);
        }
        Rows::Generic(rows, label) => {
            draw_generic_table(frame, frame.area(), rows, label, table_state, dimmed);
        }
        Rows::CrdList(crds) => {
            draw_crd_list_table(frame, frame.area(), crds, table_state, dimmed);
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

    let block = Block::default().borders(Borders::ALL).title(format!("{}/{}", pod.namespace, pod.name));
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
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
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
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

/// The shared table for every resource kind that doesn't get specialized
/// columns — Namespace/Name/Age is all that's generically knowable about
/// an arbitrary Kubernetes object.
fn draw_generic_table(frame: &mut Frame, area: Rect, rows: &[GenericRow], label: &str, table_state: &mut TableState, dimmed: bool) {
    let muted = Style::default().fg(Color::DarkGray);
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["NAMESPACE", "NAME", "AGE"]).style(header_style);

    let table_rows = rows.iter().map(|r| {
        Row::new(vec![
            Cell::from(r.namespace.clone()).style(cell_style),
            Cell::from(r.name.clone()).style(cell_style),
            Cell::from(r.age.clone()).style(cell_style),
        ])
    });

    let widths = [Constraint::Fill(2), Constraint::Fill(3), Constraint::Length(5)];

    let title = format!("{label} ({})  —  j/k: move  d: spec  m: switch resource  q: quit", rows.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(table_rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
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
fn draw_crd_list_table(frame: &mut Frame, area: Rect, crds: &[CrdInfo], table_state: &mut TableState, dimmed: bool) {
    let muted = Style::default().fg(Color::DarkGray);
    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let border_style = if dimmed { muted } else { Style::default() };
    let cell_style = if dimmed { muted } else { Style::default() };

    let header = Row::new(vec!["GROUP", "KIND", "SCOPE"]).style(header_style);

    let rows = crds.iter().map(|c| {
        Row::new(vec![
            Cell::from(c.group).style(cell_style),
            Cell::from(c.kind).style(cell_style),
            Cell::from(if c.namespaced { "Namespaced" } else { "Cluster" }).style(cell_style),
        ])
    });

    let widths = [Constraint::Fill(3), Constraint::Fill(2), Constraint::Length(11)];
    let title = format!("Custom Resources ({})  —  j/k: move  enter: open  m: switch resource  q: quit", crds.len());

    let highlight_style = if dimmed {
        muted
    } else {
        Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_style(border_style).title(title))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, area, table_state);
}

const TILE_WIDTH: u16 = 18;
/// 2 border rows + a 3-row-tall icon area + a count line + a label line.
const TILE_HEIGHT: u16 = 7;
const TILE_ICON_HEIGHT: u16 = 3;
pub const METRICS_PANEL_HEIGHT: u16 = 7;

/// The home screen: a cluster-resources panel on top (CPU/Memory/Pods
/// gauges — "metrics unavailable" if metrics-server isn't installed,
/// same fallback k9s/Freelens use), then a scrollable, flow-wrapping
/// grid of resource-kind tiles below, grouped into sections
/// (Freelens-style categories), ending with the Cluster Issues list
/// (Node warning conditions + Warning events — verified against
/// Freelens's actual `cluster-issues.tsx` source).
#[allow(clippy::too_many_arguments)]
fn draw_overview(
    frame: &mut Frame,
    area: Rect,
    overview: &Overview,
    scroll: usize,
    selected: (usize, usize),
    dimmed: bool,
    icons: &mut IconCache,
) {
    let chunks = Layout::vertical([Constraint::Length(METRICS_PANEL_HEIGHT), Constraint::Min(0)]).split(area);
    draw_metrics_panel(frame, chunks[0], overview, dimmed);
    draw_catalog(frame, chunks[1], overview, scroll, selected, dimmed, icons);
}

/// The catalog area is whatever's left below the metrics panel — callers
/// (keyboard navigation, mouse hit-testing) need this same rectangle to
/// stay in sync with what's actually rendered.
pub fn catalog_area(frame_area: Rect) -> Rect {
    Rect {
        x: frame_area.x,
        y: frame_area.y + METRICS_PANEL_HEIGHT,
        width: frame_area.width,
        height: frame_area.height.saturating_sub(METRICS_PANEL_HEIGHT),
    }
}

pub fn tile_cols(width: u16) -> usize {
    (width / TILE_WIDTH).max(1) as usize
}

fn draw_metrics_panel(frame: &mut Frame, area: Rect, overview: &Overview, dimmed: bool) {
    let border_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title("Cluster Resources");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !overview.metrics_available {
        let text = vec![
            Line::raw(""),
            Line::styled("metrics unavailable", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
            Line::styled("install metrics-server to see CPU/Memory usage", Style::default().fg(Color::DarkGray)),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
        return;
    }

    let pod_usage = overview
        .catalog
        .iter()
        .find(|(section, _)| *section == "Workloads")
        .and_then(|(_, tiles)| tiles.iter().find(|(label, _)| *label == "Pods"))
        .map(|(_, count)| *count)
        .unwrap_or(0);

    let gauge_areas = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(inner);
    draw_gauge(
        frame,
        gauge_areas[0],
        "CPU",
        overview.cpu_usage_millicores as f64,
        overview.cpu_capacity_millicores as f64,
        |v| format!("{:.2} cores", v / 1000.0),
        dimmed,
    );
    draw_gauge(
        frame,
        gauge_areas[1],
        "Memory",
        overview.memory_usage_bytes as f64,
        overview.memory_capacity_bytes as f64,
        format_bytes,
        dimmed,
    );
    draw_gauge(
        frame,
        gauge_areas[2],
        "Pods",
        pod_usage as f64,
        overview.pod_capacity as f64,
        |v| format!("{v:.0}"),
        dimmed,
    );
}

fn draw_gauge(frame: &mut Frame, area: Rect, label: &str, used: f64, capacity: f64, format_value: impl Fn(f64) -> String, dimmed: bool) {
    let ratio = if capacity > 0.0 { (used / capacity).clamp(0.0, 1.0) } else { 0.0 };
    let color = if dimmed {
        Color::DarkGray
    } else if ratio > 0.9 {
        Color::Red
    } else if ratio > 0.7 {
        Color::Yellow
    } else {
        Color::Green
    };
    let title = format!("{label}: {} / {}", format_value(used), format_value(capacity));
    let gauge = Gauge::default()
        .block(Block::default().title(title))
        .gauge_style(Style::default().fg(color))
        .use_unicode(true)
        .ratio(ratio);
    frame.render_widget(gauge, area);
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

/// One virtual row in the scrollable catalog area — heights vary by
/// kind, so scrolling skips whole rows (`scroll` is a row *index*, not a
/// line count) rather than trying to do partial-row pixel clipping.
/// `Tiles` carries which section it belongs to and the index of its
/// first tile within that section, so a specific tile can be identified
/// as `(section, tile)` for selection/hit-testing — the same
/// `(section, tile)` coordinate space `move_tile_selection`/`tile_at`
/// use, all built from this one shared row layout so rendering,
/// keyboard navigation, and mouse hit-testing can't drift out of sync.
enum CatalogRow<'a> {
    SectionHeader(&'a str),
    Tiles { section: usize, start: usize, tiles: &'a [(&'a str, usize)] },
    IssuesHeader(usize),
    IssuesEmpty,
    Issue(&'a Warning),
}

impl CatalogRow<'_> {
    fn height(&self) -> u16 {
        match self {
            CatalogRow::SectionHeader(_) => 2,
            CatalogRow::Tiles { .. } => TILE_HEIGHT,
            CatalogRow::IssuesHeader(_) => 1,
            CatalogRow::IssuesEmpty => 3,
            CatalogRow::Issue(_) => 1,
        }
    }
}

fn build_catalog_rows<'a>(overview: &'a Overview, cols: usize) -> Vec<CatalogRow<'a>> {
    let mut rows: Vec<CatalogRow> = Vec::new();
    for (section_idx, (section, tiles)) in overview.catalog.iter().enumerate() {
        rows.push(CatalogRow::SectionHeader(section));
        for (chunk_idx, chunk) in tiles.chunks(cols.max(1)).enumerate() {
            rows.push(CatalogRow::Tiles { section: section_idx, start: chunk_idx * cols.max(1), tiles: chunk });
        }
    }

    rows.push(CatalogRow::SectionHeader("Cluster Issues"));
    if overview.warnings.is_empty() {
        rows.push(CatalogRow::IssuesEmpty);
    } else {
        rows.push(CatalogRow::IssuesHeader(overview.warnings.len()));
        for w in &overview.warnings {
            rows.push(CatalogRow::Issue(w));
        }
    }
    rows
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn section_tile_count(overview: &Overview, section: usize) -> usize {
    overview.catalog.get(section).map(|(_, tiles)| tiles.len()).unwrap_or(0)
}

/// Moves a selection one step in a direction across any 2D flow-wrapped
/// grid of sections, given just each section's tile count — shared by
/// the Overview catalog grid and the resource-switcher menu, which lay
/// out identically (sections of tiles, wrapped at `cols` per row) but
/// have different backing data types.
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
            } else if section > 0 {
                (section - 1, section_lens[section - 1].saturating_sub(1))
            } else {
                (section, tile)
            }
        }
        Direction::Right => {
            if tile + 1 < len {
                (section, tile + 1)
            } else if section + 1 < section_count {
                (section + 1, 0)
            } else {
                (section, tile)
            }
        }
        Direction::Up => {
            if row > 0 {
                (section, (row - 1) * cols + col)
            } else if section > 0 {
                let prev_len = section_lens[section - 1];
                let prev_rows = prev_len.div_ceil(cols).max(1);
                let target = ((prev_rows - 1) * cols + col).min(prev_len.saturating_sub(1));
                (section - 1, target)
            } else {
                (section, tile)
            }
        }
        Direction::Down => {
            let next = (row + 1) * cols + col;
            if next < len {
                (section, next)
            } else if section + 1 < section_count {
                let next_len = section_lens[section + 1];
                (section + 1, col.min(next_len.saturating_sub(1)))
            } else {
                (section, tile)
            }
        }
    }
}

/// Moves the tile selection one step in a direction, across the same 2D
/// flow-wrapped grid `build_catalog_rows` lays out for rendering — so a
/// keypress always lands on a tile that's actually adjacent on screen,
/// including crossing from one section into the next.
pub fn move_tile_selection(overview: &Overview, cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let lens: Vec<usize> = (0..overview.catalog.len()).map(|i| section_tile_count(overview, i)).collect();
    move_selection(&lens, cols, current, dir)
}

/// Same movement rules as `move_tile_selection`, for the resource-switcher
/// menu's own section/tile grid.
pub fn move_menu_selection(sections: &[MenuSection], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let lens: Vec<usize> = sections.iter().map(|s| s.tiles.len()).collect();
    move_selection(&lens, cols, current, dir)
}

/// The menu popup's tile-grid column count — computed the same way as
/// `tile_cols` for the Overview grid, from the popup's actual inner area
/// so keyboard navigation and mouse hit-testing can't drift from what's
/// rendered.
pub fn menu_cols(frame_area: Rect) -> usize {
    let area = centered_rect(70, 85, frame_area);
    let inner = Block::default().borders(Borders::ALL).inner(area);
    (inner.width / TILE_WIDTH).max(1) as usize
}

/// Which virtual row (in `build_catalog_rows`'s numbering) a given tile
/// lands on — used to keep the selection scrolled into view.
fn row_index_of_tile(overview: &Overview, cols: usize, target: (usize, usize)) -> usize {
    let cols = cols.max(1);
    let mut index = 0;
    for (section_idx, (_, tiles)) in overview.catalog.iter().enumerate() {
        index += 1; // section header
        if section_idx == target.0 {
            return index + target.1 / cols;
        }
        index += tiles.len().div_ceil(cols).max(1);
    }
    index
}

/// Adjusts `scroll` (if needed) so the selected tile's row is fully
/// visible within `area_height` — scrolls up immediately if the
/// selection moved above the visible window, or forward just far enough
/// if it moved below it.
pub fn scroll_to_show(overview: &Overview, cols: usize, area_height: u16, scroll: usize, selected: (usize, usize)) -> usize {
    let target_row = row_index_of_tile(overview, cols, selected);
    if target_row < scroll {
        return target_row;
    }
    let rows = build_catalog_rows(overview, cols);
    let last = target_row.min(rows.len().saturating_sub(1));
    let mut new_scroll = scroll;
    while new_scroll < last {
        let height: u16 = rows[new_scroll..=last].iter().map(|r| r.height()).sum();
        if height <= area_height {
            break;
        }
        new_scroll += 1;
    }
    new_scroll
}

/// Which tile (if any) sits under an absolute terminal position — same
/// row-walking approach as `row_at` for the pod table, replaying
/// `build_catalog_rows` and the same `Layout::horizontal` column split
/// `draw_tiles_row` actually renders with, so a click always resolves to
/// what's really on screen.
pub fn tile_at(frame_area: Rect, overview: &Overview, scroll: usize, column: u16, row: u16) -> Option<(usize, usize)> {
    let area = catalog_area(frame_area);
    if row < area.y || row >= area.y + area.height {
        return None;
    }
    let cols = tile_cols(area.width);
    let rows = build_catalog_rows(overview, cols);
    let scroll = scroll.min(rows.len().saturating_sub(1));

    let mut y = area.y;
    for r in rows.iter().skip(scroll) {
        let h = r.height();
        if y + h > area.y + area.height {
            break;
        }
        if row >= y && row < y + h {
            if let CatalogRow::Tiles { section, start, tiles } = r {
                let mut constraints: Vec<Constraint> = tiles.iter().map(|_| Constraint::Length(TILE_WIDTH)).collect();
                constraints.push(Constraint::Min(0));
                let tile_areas = Layout::horizontal(constraints).split(Rect { x: area.x, y, width: area.width, height: h });
                for (i, tile_area) in tile_areas.iter().enumerate().take(tiles.len()) {
                    if column >= tile_area.x && column < tile_area.x + tile_area.width {
                        return Some((*section, start + i));
                    }
                }
            }
            return None;
        }
        y += h;
    }
    None
}

fn draw_catalog(frame: &mut Frame, area: Rect, overview: &Overview, scroll: usize, selected: (usize, usize), dimmed: bool, icons: &mut IconCache) {
    let cols = tile_cols(area.width);
    let rows = build_catalog_rows(overview, cols);

    let scroll = scroll.min(rows.len().saturating_sub(1));
    let mut y = area.y;
    for row in rows.iter().skip(scroll) {
        let h = row.height();
        if y + h > area.y + area.height {
            break;
        }
        let row_area = Rect { x: area.x, y, width: area.width, height: h };
        match row {
            CatalogRow::SectionHeader(title) => draw_section_header(frame, row_area, title, dimmed),
            CatalogRow::Tiles { section, start, tiles } => {
                draw_tiles_row(frame, row_area, tiles, *section, *start, selected, dimmed, icons)
            }
            CatalogRow::IssuesHeader(count) => draw_issues_header(frame, row_area, *count, dimmed),
            CatalogRow::IssuesEmpty => draw_issues_empty(frame, row_area, dimmed),
            CatalogRow::Issue(warning) => draw_issue_line(frame, row_area, warning, dimmed),
        }
        y += h;
    }
}

fn draw_section_header(frame: &mut Frame, area: Rect, title: &str, dimmed: bool) {
    let style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().add_modifier(Modifier::BOLD) };
    let lines = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    frame.render_widget(Paragraph::new(Line::styled(format!("── {title} "), style)), lines[1]);
}

#[allow(clippy::too_many_arguments)]
fn draw_tiles_row(
    frame: &mut Frame,
    area: Rect,
    tiles: &[(&str, usize)],
    section: usize,
    start: usize,
    selected: (usize, usize),
    dimmed: bool,
    icons: &mut IconCache,
) {
    let mut constraints: Vec<Constraint> = tiles.iter().map(|_| Constraint::Length(TILE_WIDTH)).collect();
    constraints.push(Constraint::Min(0));
    let areas = Layout::horizontal(constraints).split(area);
    for (i, (label, count)) in tiles.iter().enumerate() {
        let is_selected = !dimmed && selected == (section, start + i);
        draw_tile(frame, areas[i], label, *count, is_selected, dimmed, icons);
    }
}

/// A fixed-size rounded tile: an icon/glyph per resource kind (not an
/// official Kubernetes symbol set — there isn't one that's terminal
/// renderable — just a distinct, recognizable emoji per kind), the live
/// count, and the kind name underneath.
fn draw_tile(frame: &mut Frame, area: Rect, label: &str, count: usize, selected: bool, dimmed: bool, icons: &mut IconCache) {
    let border_style = if dimmed {
        Style::default().fg(Color::DarkGray)
    } else if selected {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let count_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) };
    let label_style = if selected && !dimmed { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) };

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(TILE_ICON_HEIGHT), Constraint::Length(1), Constraint::Length(1)]).split(inner);

    // A real terminal-graphics image would still read as "in focus" even
    // while a modal dims everything else — fall back to the plain glyph
    // so a dimmed tile actually looks dimmed. Same fallback if the label
    // somehow isn't one of the known kinds (shouldn't happen — every
    // label drawn here comes from `Catalog::sections`, which only ever
    // uses labels `ResourceKind::from_label` recognizes).
    match (dimmed, ResourceKind::from_label(label)) {
        (false, Some(kind)) => icons.draw(frame, icons.centered_square(rows[0]), kind),
        _ => frame.render_widget(Paragraph::new(icon_for(label)).alignment(Alignment::Center), rows[0]),
    }

    frame.render_widget(Paragraph::new(Line::styled(count.to_string(), count_style)).alignment(Alignment::Center), rows[1]);
    frame.render_widget(Paragraph::new(Line::styled(label, label_style)).alignment(Alignment::Center), rows[2]);
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
        _ => "❔",
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
                let (border_style, text_style) = if is_selected {
                    (Style::default().fg(Color::Cyan), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                } else {
                    (Style::default(), Style::default())
                };
                let tile = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style);
                let label = Paragraph::new(kind.label()).alignment(Alignment::Center).style(text_style).block(tile);
                frame.render_widget(label, *tile_area);
            }
        }
    }
}

fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>) {
    let area = centered_rect(85, 85, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).title(format!(
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
        .block(Block::default().borders(Borders::ALL).title(format!("{title}  —  j/k: move  enter: logs  esc: back")))
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
    let block = Block::default().borders(Borders::ALL).title(format!(
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
mod tile_selection_tests {
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

    #[test]
    fn right_moves_within_row() {
        let overview = test_overview(vec![("A", vec![("t1", 0), ("t2", 0), ("t3", 0)])]);
        assert_eq!(move_tile_selection(&overview, 3, (0, 0), Direction::Right), (0, 1));
    }

    #[test]
    fn right_at_last_tile_of_section_crosses_into_next_section() {
        let overview = test_overview(vec![("A", vec![("t1", 0)]), ("B", vec![("t2", 0)])]);
        assert_eq!(move_tile_selection(&overview, 3, (0, 0), Direction::Right), (1, 0));
    }

    #[test]
    fn left_at_first_tile_crosses_into_previous_section_last_tile() {
        let overview = test_overview(vec![("A", vec![("t1", 0), ("t2", 0)]), ("B", vec![("t3", 0)])]);
        assert_eq!(move_tile_selection(&overview, 3, (1, 0), Direction::Left), (0, 1));
    }

    #[test]
    fn down_moves_to_next_row_within_section() {
        // cols=2: row0=[t1,t2], row1=[t3,t4]
        let overview = test_overview(vec![("A", vec![("t1", 0), ("t2", 0), ("t3", 0), ("t4", 0)])]);
        assert_eq!(move_tile_selection(&overview, 2, (0, 0), Direction::Down), (0, 2));
    }

    #[test]
    fn up_at_top_row_crosses_into_previous_section_matching_column() {
        // Section A (3 tiles, cols=2): row0=[a1,a2], row1=[a3]. Section B: row0=[b1].
        let overview = test_overview(vec![("A", vec![("a1", 0), ("a2", 0), ("a3", 0)]), ("B", vec![("b1", 0)])]);
        // From B's b1 (col 0), Up should land on A's last row at col 0 -> a3 (index 2).
        assert_eq!(move_tile_selection(&overview, 2, (1, 0), Direction::Up), (0, 2));
    }

    #[test]
    fn movement_clamps_at_the_very_first_and_last_tile() {
        let overview = test_overview(vec![("A", vec![("t1", 0), ("t2", 0)])]);
        assert_eq!(move_tile_selection(&overview, 2, (0, 0), Direction::Left), (0, 0));
        assert_eq!(move_tile_selection(&overview, 2, (0, 1), Direction::Right), (0, 1));
    }
}
