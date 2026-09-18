//! The persistent top info bar (context, cluster, user, versions), after k9s.

use super::*;

/// What the header shows — built once per connected session.
pub struct HeaderInfo {
    pub context: String,
    pub cluster: String,
    pub user: String,
    pub k8s_version: String,
    pub knav_version: String,
}

/// Rows the header takes at the top of the main screen.
pub const HEADER_HEIGHT: u16 = 3;

/// Below this height the header is dropped — the resource list matters
/// more than the context on a tiny terminal.
const MIN_HEIGHT_FOR_HEADER: u16 = 14;

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
    let rw = if dimmed { dim_style() } else { Style::default().fg(Color::Rgb(240, 160, 110)) };

    let field = |name: &str, v: &str| -> Vec<Span<'static>> {
        vec![Span::styled(format!("{name:<9}"), label), Span::styled(v.to_string(), value)]
    };
    let user = if info.user.is_empty() { String::new() } else { format!("{}@{}", info.user, info.cluster) };
    let mut context_line = field("Context:", &info.context);
    context_line.push(Span::styled(" [RW]", rw));
    let left = vec![Line::from(context_line), Line::from(field("Cluster:", &info.cluster)), Line::from(field("User:", &user))];
    let right = vec![Line::from(field("knav Rev:", &info.knav_version)), Line::from(field("K8s Rev:", &info.k8s_version)), Line::raw("")];

    let left_width = left.iter().map(|l| l.width() as u16).max().unwrap_or(0);
    let gap = 6;
    let mut left_area = Rect { x: area.x + 1, y: area.y, width: left_width.min(area.width.saturating_sub(1)), height: HEADER_HEIGHT };
    left_area.width = left_area.width.max(1);
    let right_x = left_area.x + left_area.width + gap;
    frame.render_widget(Paragraph::new(left), left_area);
    if right_x < area.x + area.width {
        let right_area = Rect { x: right_x, y: area.y, width: area.x + area.width - right_x, height: HEADER_HEIGHT };
        frame.render_widget(Paragraph::new(right), right_area);
    }
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
        let full = Rect { x: 0, y: 0, width: 80, height: 10 };
        assert_eq!(body_area(full), full);
    }
}
