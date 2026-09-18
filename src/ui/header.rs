//! The persistent one-line top bar (context, cluster, user, versions), after k9s.

use super::*;

/// What the header shows — built once per connected session.
pub struct HeaderInfo {
    pub context: String,
    pub cluster: String,
    pub user: String,
    /// What knav is allowed to do: `read-and-write` today; a future
    /// read-only mode would show `read-only`.
    pub role: String,
    pub k8s_version: String,
    pub knav_version: String,
}

/// Rows the header takes at the top of the main screen.
pub const HEADER_HEIGHT: u16 = 1;

/// Below this height the header is dropped — the resource list matters
/// more than the context on a tiny terminal.
const MIN_HEIGHT_FOR_HEADER: u16 = 10;

/// The part of the screen below the header, where the main layer (the
/// Overview or a resource list) lives. Mouse hit-testing for that layer
/// must use this, not the full frame area.
pub fn body_area(area: Rect) -> Rect {
    if area.height < MIN_HEIGHT_FOR_HEADER {
        return area;
    }
    Rect { x: area.x, y: area.y + HEADER_HEIGHT, width: area.width, height: area.height - HEADER_HEIGHT }
}

pub(super) fn draw_header(frame: &mut Frame, area: Rect, info: &HeaderInfo, dimmed: bool) {
    if area.height < MIN_HEIGHT_FOR_HEADER {
        return;
    }
    let label = if dimmed { dim_style() } else { Style::default().fg(Color::Rgb(122, 140, 170)) };
    let value = if dimmed { dim_style() } else { Style::default().fg(Color::Rgb(226, 232, 240)).add_modifier(Modifier::BOLD) };

    let fields = [
        ("Context:", info.context.as_str()),
        ("Cluster:", info.cluster.as_str()),
        ("User:", info.user.as_str()),
        ("Role:", info.role.as_str()),
        ("K8s Version:", info.k8s_version.as_str()),
        ("knav Version:", info.knav_version.as_str()),
    ];
    // Leave the top-right corner to the `commands: ?` indicator; when the
    // terminal is too narrow for everything, the trailing fields drop.
    let available = area.width.saturating_sub(1 + 14) as usize;
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0;
    for (name, v) in fields.iter().filter(|(_, v)| !v.is_empty()) {
        let width = name.chars().count() + 1 + v.chars().count();
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
    let line_area = Rect { x: area.x + 1, y: area.y, width: area.width.saturating_sub(1), height: HEADER_HEIGHT };
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_area_leaves_room_for_the_header() {
        let full = Rect { x: 0, y: 0, width: 100, height: 40 };
        assert_eq!(body_area(full), Rect { x: 0, y: HEADER_HEIGHT, width: 100, height: 40 - HEADER_HEIGHT });
    }

    #[test]
    fn tiny_terminals_drop_the_header() {
        let full = Rect { x: 0, y: 0, width: 80, height: 9 };
        assert_eq!(body_area(full), full);
    }
}
