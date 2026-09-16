use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
};
use tui_tree_widget::{Tree, TreeItem, TreeState};

use crate::config::TimestampFormat;
use crate::k8s::{ContainerInfo, ContainerStatusKind, DeploymentRow, Overview, PodRow, ResourceKind};

pub enum Rows<'a> {
    Overview(&'a Overview),
    Pods(&'a [PodRow]),
    Deployments(&'a [DeploymentRow]),
}

pub struct MenuSection<'a> {
    pub title: &'a str,
    pub tiles: &'a [ResourceKind],
}

pub enum Overlay<'a> {
    Spec { title: &'a str, items: &'a [TreeItem<'static, String>], state: &'a mut TreeState<String> },
    Containers { title: &'a str, containers: &'a [ContainerInfo], state: &'a mut TableState },
    Logs { title: &'a str, lines: &'a [String], scroll: u16, follow: bool, timestamp_format: TimestampFormat },
    Menu { sections: &'a [MenuSection<'a>], selected: usize },
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

pub fn draw(frame: &mut Frame, rows: Rows, table_state: &mut TableState, hover: Option<Hover>, overlay: Option<Overlay>) {
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
        Rows::Overview(overview) => {
            draw_overview(frame, frame.area(), overview, table_state, dimmed);
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

/// The home screen: a row of rounded-corner count tiles, plus a Cluster
/// Issues panel below (Node warning conditions + Warning events) — this
/// specific pairing matches what Freelens's own Overview page shows
/// without metrics-server (verified against its actual source:
/// `cluster-overview.tsx` always renders `ClusterIssues` even when
/// metrics are hidden). No CPU/Mem/pie charts — those are entirely
/// metrics-server-dependent in both k9s and Freelens, and that's still
/// not wired up here.
fn draw_overview(frame: &mut Frame, area: Rect, overview: &Overview, table_state: &mut TableState, dimmed: bool) {
    let muted = Style::default().fg(Color::DarkGray);

    let chunks = Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).split(area);
    let stat_areas = Layout::horizontal([Constraint::Ratio(1, 4); 4]).split(chunks[0]);
    draw_stat_box(frame, stat_areas[0], "Pods", overview.pod_count, dimmed);
    draw_stat_box(frame, stat_areas[1], "Deployments", overview.deployment_count, dimmed);
    draw_stat_box(frame, stat_areas[2], "Nodes", overview.node_count, dimmed);
    draw_stat_box(frame, stat_areas[3], "Namespaces", overview.namespace_count, dimmed);

    let issues_area = chunks[1];
    let border_style = if dimmed { muted } else { Style::default() };

    if overview.warnings.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title("Cluster Issues  —  m: switch resource  q: quit");
        let inner = block.inner(issues_area);
        frame.render_widget(block, issues_area);

        let ok_style = if dimmed { muted } else { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) };
        let sub_style = if dimmed { muted } else { Style::default().fg(Color::Gray) };
        let text = vec![
            Line::raw(""),
            Line::styled("✓ No issues found", ok_style),
            Line::styled("Everything is fine in the cluster", sub_style),
        ];
        frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
        return;
    }

    let header_style = if dimmed { muted } else { Style::default().add_modifier(Modifier::BOLD) };
    let header = Row::new(vec!["MESSAGE", "OBJECT", "KIND", "AGE"]).style(header_style);

    let rows = overview.warnings.iter().map(|w| {
        let color = if dimmed {
            Color::DarkGray
        } else if w.kind == "Node" {
            Color::Red // a not-ready/pressured node affects everything scheduled on it
        } else {
            Color::Yellow
        };
        let style = Style::default().fg(color);
        Row::new(vec![
            Cell::from(w.message.clone()),
            Cell::from(w.object.clone()),
            Cell::from(w.kind.clone()),
            Cell::from(w.age.clone()),
        ])
        .style(style)
    });

    let widths = [Constraint::Fill(4), Constraint::Fill(2), Constraint::Fill(1), Constraint::Length(5)];
    let highlight_style = if dimmed { muted } else { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) };

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(
            format!("Cluster Issues ({})  —  j/k: move  m: switch resource  q: quit", overview.warnings.len()),
        ))
        .row_highlight_style(highlight_style)
        .highlight_symbol(if dimmed { "  " } else { "➤ " });

    frame.render_stateful_widget(table, issues_area, table_state);
}

fn draw_stat_box(frame: &mut Frame, area: Rect, label: &str, value: usize, dimmed: bool) {
    let value_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) };
    let label_style = Style::default().fg(Color::DarkGray);
    let border_style = if dimmed { Style::default().fg(Color::DarkGray) } else { Style::default() };

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style);
    let text = vec![Line::styled(value.to_string(), value_style), Line::styled(label, label_style)];
    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center).block(block), area);
}

/// The central "switch resource" menu — rounded-corner tiles grouped by
/// section, Freelens-style. Only one section exists today (`Workloads`);
/// adding another resource kind later is just adding another
/// `MenuSection`/tile, not restructuring this.
fn draw_menu_popup(frame: &mut Frame, sections: &[MenuSection], selected: usize) {
    let area = centered_rect(50, 40, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title("Switch resource  —  ←→/hl: move  enter: select  esc: cancel");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let section_heights: Vec<Constraint> = sections.iter().map(|_| Constraint::Length(5)).collect();
    let section_areas = Layout::vertical(section_heights).split(inner);

    let mut flat_index = 0;
    for (section, section_area) in sections.iter().zip(section_areas.iter()) {
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(3)]).split(*section_area);
        frame.render_widget(
            Paragraph::new(Line::styled(section.title, Style::default().add_modifier(Modifier::BOLD))),
            rows[0],
        );

        let tile_constraints: Vec<Constraint> =
            section.tiles.iter().map(|_| Constraint::Ratio(1, section.tiles.len() as u32)).collect();
        let tile_areas = Layout::horizontal(tile_constraints).split(rows[1]);

        for (tile_area, kind) in tile_areas.iter().zip(section.tiles.iter()) {
            let is_selected = flat_index == selected;
            let (border_style, text_style) = if is_selected {
                (Style::default().fg(Color::Cyan), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            } else {
                (Style::default(), Style::default())
            };
            let tile = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style);
            let label = Paragraph::new(kind.label()).alignment(Alignment::Center).style(text_style).block(tile);
            frame.render_widget(label, *tile_area);
            flat_index += 1;
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
