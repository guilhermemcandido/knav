//! The node detail popup and its info panel.

use super::*;

/// A node's gauges above its pods, in the same table as the Pods list.
pub(in crate::ui) fn draw_node_detail_popup(frame: &mut Frame, view: NodeDetailView, dimmed: bool) {
    let NodeDetailView { name, cpu_usage, cpu_capacity, memory_usage, memory_capacity, pod_capacity, info, pods, pod_usage, state, sort, search } = view;
    let area = centered_rect(94, 92, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(border_style)
        .title(pill_title(&format!("Node: {name}"), dimmed, border_style));
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
            let text = Paragraph::new(Line::styled("metrics unavailable", Style::default().fg(theme().muted).add_modifier(Modifier::BOLD)))
                .alignment(Alignment::Center);
            frame.render_widget(text, chunks[0]);
        }
    }

    if let Some(info) = info {
        draw_node_info_panel(frame, chunks[1], info, dimmed);
    }

    let marked = HashSet::new();
    let view = ListView { state, search: if dimmed { Search::default() } else { search }, sort: if dimmed { SortState::default() } else { sort }, hscroll: &mut 0, marked: &marked, wide: false, look: ListLook::dimmed(dimmed) };
    draw_table(frame, chunks[2], pods, pod_usage, view);
}

/// The node info panel's height, shared by sizing and drawing.
pub(in crate::ui) fn node_info_height(info: &crate::k8s::NodeDetailInfo) -> u16 {
    let base = 3 + 1 + 1 + info.conditions.len() as u16;
    if info.taints.is_empty() { base } else { base + 1 + info.taints.len() as u16 }
}

/// The node summary: schedulability, roles, version, addresses, all conditions and taints.
pub(in crate::ui) fn draw_node_info_panel(frame: &mut Frame, area: Rect, info: &crate::k8s::NodeDetailInfo, dimmed: bool) {
    let label = Style::default().fg(theme().muted);
    let value = if dimmed { dim_style() } else { Style::default().add_modifier(Modifier::BOLD) };
    let field = |l: &'static str, v: String| vec![Span::styled(format!("{l}: "), label), Span::styled(v, value)];

    let schedulable_text = if info.schedulable { "Schedulable".to_string() } else { "Cordoned".to_string() };
    let schedulable_style = if dimmed {
        dim_style()
    } else if info.schedulable {
        Style::default().fg(theme().ok)
    } else {
        Style::default().fg(theme().highlight)
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
            theme().dim
        } else if is_healthy {
            theme().ok
        } else {
            theme().bad
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
