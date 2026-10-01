//! The top bar: context, cluster, user and version, and on resource lists a line of
//! namespace shortcuts (`Namespace: (0)all (1)default ...`).

use super::*;

#[derive(Clone)]
pub struct HeaderInfo {
    pub context: String,
    pub cluster: String,
    pub user: String,
    /// What the user may do (`admin`, `read-write`, ...), or `read-only` while knav
    /// blocks changes. Empty when unknown.
    pub role: String,
    /// knav blocks changes, so the role stands out.
    pub read_only: bool,
    /// The context is one to be careful in, so its name is shown in red.
    pub highlight: bool,
    /// The namespace queries are narrowed to (`all` when none).
    pub namespace: String,
    /// What number keys 1-9 select (index 0 is key 1).
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

/// Rows the header takes on a resource list: the info line and the namespace line.
pub const HEADER_HEIGHT: u16 = 2;

/// Below this height the header is dropped, since the list matters more.
const MIN_HEIGHT_FOR_HEADER: u16 = 10;

/// The screen below the header, where the Overview or a list goes. Mouse hit-testing
/// for that layer must use this, not the whole frame.
pub fn body_area(area: Rect, shortcuts: bool) -> Rect {
    if area.height < MIN_HEIGHT_FOR_HEADER {
        return area;
    }
    let height = if shortcuts { HEADER_HEIGHT } else { 1 };
    // The last row belongs to the path bar.
    Rect { x: area.x, y: area.y + height, width: area.width, height: area.height - height - 1 }
}

/// `left` is the column the lines start at, the left edge of what is drawn below.
pub(super) fn draw_header(frame: &mut Frame, area: Rect, left: u16, info: &HeaderInfo, shortcuts_line: bool, namespace_keys_disabled: bool, dimmed: bool) {
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
        ("K8s:", info.k8s_version.as_str()),
        ("knav:", info.knav_version.as_str()),
    ];
    // The top-right corner is the help indicator's; trailing fields drop when narrow.
    let start_x = left.clamp(area.x, (area.x + area.width).saturating_sub(1));
    let available = ((area.x + area.width).saturating_sub(start_x) as usize).saturating_sub(14 + 1);
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0;
    for (name, v) in fields.iter().filter(|(_, v)| !v.is_empty()) {
        let width = cell_width(name) + 1 + cell_width(v) + if *name == "Context:" && info.highlight { 2 } else { 0 };
        let gap = if spans.is_empty() { 0 } else { 3 };
        if used + gap + width > available {
            break;
        }
        if gap > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(format!("{name} "), label));
        let warn = *name == "Role:" && info.read_only && !dimmed;
        let careful = *name == "Context:" && info.highlight && !dimmed;
        let style = if careful {
            Style::default().bg(theme().bad).fg(crate::theme::on(theme().bad)).add_modifier(Modifier::BOLD)
        } else if warn {
            value.fg(theme().warn)
        } else {
            value
        };
        // A filled red pill gets a space each side so the name doesn't touch its edges.
        spans.push(Span::styled(if careful { format!(" {v} ") } else { v.to_string() }, style));
        used += gap + width;
    }
    let line_area = Rect { x: start_x, y: area.y, width: (area.x + area.width).saturating_sub(start_x), height: 1 };
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);

    if !shortcuts_line {
        return;
    }
    // `(0)all` plus each reserved number, the active one filled in.
    let key = if dimmed { dim_style() } else { Style::default().fg(theme().warm) };
    let name = if dimmed { dim_style() } else { Style::default().fg(theme().row) };
    let active = if dimmed { dim_style() } else { Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg)).add_modifier(Modifier::BOLD) };
    // Digits do something else here (pick a sort column) or nothing, so grey the line out.
    let (key, name, active) = if namespace_keys_disabled && !dimmed {
        let muted = Style::default().fg(theme().panel_bg);
        (muted, muted, muted)
    } else {
        (key, name, active)
    };
    // The line starts under "Context:". A scope on the right keeps its room; namespaces
    // that don't fit are shortened, then left out.
    let scope_text = (!info.scope.is_empty()).then(|| format!("Scope: {}", info.scope));
    let scope_room = scope_text.as_ref().map_or(0, |t| cell_width(t) + 3);
    let start_x = start_x.min(area.x + area.width.saturating_sub(1));
    let max_width = ((area.x + area.width).saturating_sub(start_x) as usize).saturating_sub(scope_room + 1);
    let all: Vec<(usize, String)> = std::iter::once((0usize, "all".to_string())).chain(info.namespace_slots.iter().enumerate().filter_map(|(i, ns)| ns.as_ref().map(|ns| (i + 1, ns.clone())))).collect();
    let prefix = cell_width("Namespace: ");
    let (entries, trimmed) = fit_namespaces(&all, max_width.saturating_sub(prefix), &info.namespace);
    let mut shortcuts: Vec<Span> = vec![Span::styled("Namespace: ", label)];
    for (k, (n, ns)) in entries.into_iter().enumerate() {
        let is_active = all.iter().any(|(m, full)| *m == n && *full == info.namespace) || (n == 0 && info.namespace == "all");
        if k > 0 {
            shortcuts.push(Span::raw("  "));
        }
        if is_active {
            shortcuts.push(Span::styled(format!("({n}){ns}"), active));
        } else {
            shortcuts.push(Span::styled(format!("({n})"), key));
            shortcuts.push(Span::styled(ns, name));
        }
    }
    if trimmed {
        shortcuts.push(Span::styled("  …", key));
    }
    let shortcut_area = Rect { x: start_x, y: area.y + 1, width: (area.x + area.width).saturating_sub(start_x), height: 1 };
    frame.render_widget(Paragraph::new(Line::from(shortcuts)), shortcut_area);

    if let Some(text) = scope_text {
        let width = cell_width(&text) as u16;
        if width <= shortcut_area.width {
            let scope_area = Rect { x: shortcut_area.x + shortcut_area.width - width, y: shortcut_area.y, width, height: 1 };
            frame.render_widget(Paragraph::new(Line::from(vec![Span::styled("Scope: ", label), Span::styled(info.scope.clone(), value)])), scope_area);
        }
    }
}

/// The namespace shortcuts in `room` cells: names shortened to their starts while they
/// still say something, else some left out (never the active one). The flag says so.
fn fit_namespaces(all: &[(usize, String)], room: usize, active: &str) -> (Vec<(usize, String)>, bool) {
    let width = |entries: &[(usize, String)]| entries.iter().map(|(n, ns)| cell_width(&format!("({n}){ns}"))).sum::<usize>() + 2 * entries.len().saturating_sub(1);
    let longest = all.iter().map(|(_, ns)| cell_width(ns)).max().unwrap_or(0);
    for limit in (MIN_NAME..=longest.max(MIN_NAME)).rev() {
        let cut: Vec<(usize, String)> = all.iter().map(|(n, ns)| (*n, if cell_width(ns) > limit { truncate(ns, limit) } else { ns.clone() })).collect();
        if width(&cut) <= room {
            return (cut, false);
        }
    }
    // Even the shortest starts don't all fit: keep what does, always the active one.
    let mut kept: Vec<(usize, String)> = Vec::new();
    for (n, ns) in all {
        let short = (*n, if cell_width(ns) > MIN_NAME { truncate(ns, MIN_NAME) } else { ns.clone() });
        let is_active = *ns == active || (*n == 0 && active == "all");
        kept.push(short);
        if width(&kept) > room && !is_active {
            kept.pop();
        }
    }
    (kept, true)
}

/// The fewest characters of a namespace worth showing.
const MIN_NAME: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespaces_that_do_not_fit_are_cut_to_their_starts() {
        let all = vec![(0, "all".to_string()), (1, "absdnuqweasd".to_string()), (2, "dahjsdlkjhasd".to_string())];
        let (fit, trimmed) = fit_namespaces(&all, 100, "all");
        assert_eq!(fit[1].1, "absdnuqweasd");
        assert!(!trimmed);
        let (fit, trimmed) = fit_namespaces(&all, 30, "all");
        assert!(!trimmed && fit[1].1.ends_with('…') && fit[2].1.ends_with('…'), "{fit:?}");
        assert!(fit.iter().map(|(n, ns)| cell_width(&format!("({n}){ns}"))).sum::<usize>() + 4 <= 30);
        let (fit, trimmed) = fit_namespaces(&all, 12, "dahjsdlkjhasd");
        assert!(trimmed && fit.iter().any(|(n, _)| *n == 2), "the active one stays: {fit:?}");
    }

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
