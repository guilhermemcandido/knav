//! The persistent top bar, after k9s: a line of context/cluster/user/version
//! info, and, on the resource lists, not the main Overview, a line of
//! namespace shortcuts (`Namespace: (0)all (1)default ...`).

use super::*;

/// What the header shows, built once per connected session.
#[derive(Clone)]
pub struct HeaderInfo {
    pub context: String,
    pub cluster: String,
    pub user: String,
    /// What knav is allowed to do: `read-and-write` today; a future
    /// read-only mode would show `read-only`.
    pub role: String,
    /// The namespace queries are narrowed to (`all` when none).
    pub namespace: String,
    /// What number keys 1-9 select (index 0 is key 1); `0` is always all.
    pub namespace_slots: Vec<Option<String>>,
    /// What the current list is drilled into (`Deployment/web`), if anything.
    pub scope: String,
    pub k8s_version: String,
    pub knav_version: String,
    /// Only rows that need a look are listed (`Ctrl-z`).
    pub faults_only: bool,
    /// Extra columns are shown (`Ctrl-w`).
    pub wide: bool,
}

/// Rows the header takes at the top of a resource list: the info line
/// plus the namespace-shortcut line. The Overview only has the first.
pub const HEADER_HEIGHT: u16 = 2;

/// Below this height the header is dropped, the resource list matters
/// more than the context on a tiny terminal.
const MIN_HEIGHT_FOR_HEADER: u16 = 10;

/// The part of the screen below the header, where the main layer (the
/// Overview or a resource list) lives. Mouse hit-testing for that layer
/// must use this, not the full frame area.
pub fn body_area(area: Rect, shortcuts: bool) -> Rect {
    if area.height < MIN_HEIGHT_FOR_HEADER {
        return area;
    }
    let height = if shortcuts { HEADER_HEIGHT } else { 1 };
    // The last row belongs to the path bar.
    Rect { x: area.x, y: area.y + height, width: area.width, height: area.height - height - 1 }
}

pub(super) fn draw_header(frame: &mut Frame, area: Rect, info: &HeaderInfo, shortcuts_line: bool, sort_mode: bool, dimmed: bool) {
    if area.height < MIN_HEIGHT_FOR_HEADER {
        return;
    }
    let label = if dimmed { dim_style() } else { Style::default().fg(theme().info_label) };
    let value = if dimmed { dim_style() } else { Style::default().fg(theme().text_strong).add_modifier(Modifier::BOLD) };

    let fields = [
        ("Context:", info.context.as_str()),
        ("Cluster:", info.cluster.as_str()),
        ("User:", info.user.as_str()),
        ("Role:", info.role.as_str()),
        ("K8s Version:", info.k8s_version.as_str()),
        ("knav Version:", info.knav_version.as_str()),
    ];
    // Leave the top-right corner to the `help: ?` indicator; when the
    // terminal is too narrow for everything, the trailing fields drop.
    let available = area.width.saturating_sub(1 + 14) as usize;
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0;
    for (name, v) in fields.iter().filter(|(_, v)| !v.is_empty()) {
        let width = cell_width(name) + 1 + cell_width(v);
        let gap = if spans.is_empty() { 0 } else { 3 };
        if used + gap + width > available {
            break;
        }
        if gap > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!("{name} "), label));
        spans.push(Span::styled(v.to_string(), value));
        used += gap + width;
    }
    let line_area = Rect { x: area.x + 1, y: area.y, width: area.width.saturating_sub(1), height: 1 };
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);

    if !shortcuts_line {
        return;
    }
    // Namespace shortcuts: `(0)all` plus each reserved number. The
    // active one is filled in.
    let key = if dimmed { dim_style() } else { Style::default().fg(theme().warm) };
    let name = if dimmed { dim_style() } else { Style::default().fg(theme().row) };
    let active = if dimmed { dim_style() } else { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) };
    // In sort mode the digits pick columns, not namespaces, grey the line
    // out so it reads as unavailable.
    let (key, name, active) = if sort_mode && !dimmed {
        let muted = Style::default().fg(theme().panel_bg);
        (muted, muted, muted)
    } else {
        (key, name, active)
    };
    let mut shortcuts: Vec<Span> = vec![Span::styled("Namespace: ", label)];
    let entries = std::iter::once((0usize, "all".to_string())).chain(
        info.namespace_slots.iter().enumerate().filter_map(|(i, ns)| ns.as_ref().map(|ns| (i + 1, ns.clone()))),
    );
    for (n, ns) in entries {
        let is_active = if n == 0 { info.namespace == "all" } else { info.namespace == ns };
        if shortcuts.len() > 1 {
            shortcuts.push(Span::raw("  "));
        }
        if is_active {
            shortcuts.push(Span::styled(format!("({n}){ns}"), active));
        } else {
            shortcuts.push(Span::styled(format!("({n})"), key));
            shortcuts.push(Span::styled(ns, name));
        }
    }
    let shortcut_area = Rect { x: area.x + 1, y: area.y + 1, width: area.width.saturating_sub(1), height: 1 };
    let shortcuts_width = Line::from(shortcuts.clone()).width() as u16;
    frame.render_widget(Paragraph::new(Line::from(shortcuts)), shortcut_area);

    // What the list is drilled into (`Deployment/web`), right-aligned on
    // the same row, only if it fits, so it never pushes the shortcuts or
    // the fixed first line around.
    if !info.scope.is_empty() {
        let text = format!("Scope: {}", info.scope);
        let width = cell_width(&text) as u16;
        if shortcuts_width + 3 + width <= shortcut_area.width {
            let scope_area = Rect { x: shortcut_area.x + shortcut_area.width - width, y: shortcut_area.y, width, height: 1 };
            frame.render_widget(Paragraph::new(Line::from(vec![Span::styled("Scope: ", label), Span::styled(info.scope.clone(), value)])), scope_area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_area_leaves_room_for_the_header() {
        let full = Rect { x: 0, y: 0, width: 100, height: 40 };
        assert_eq!(body_area(full, true), Rect { x: 0, y: HEADER_HEIGHT, width: 100, height: 40 - HEADER_HEIGHT - 1 });
        // Without the shortcut line (the Overview) only the info line is taken.
        assert_eq!(body_area(full, false), Rect { x: 0, y: 1, width: 100, height: 38 });
    }

    #[test]
    fn tiny_terminals_drop_the_header() {
        let full = Rect { x: 0, y: 0, width: 80, height: 9 };
        assert_eq!(body_area(full, true), full);
    }
}
