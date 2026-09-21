//! The bottom bar saying where you are: the trail of what you drilled
//! through, then the selected pod. It always fits the terminal width: the
//! container detail goes first, then the longest names are shortened in
//! the middle (`local-pa…d9885bc`).

use super::*;

/// The pod highlighted in the Pods list, shown at the end of the bar.
pub struct BreadcrumbPod {
    namespace: String,
    name: String,
    /// `(dot colour, container name, state)` per container.
    containers: Vec<(Color, String, String)>,
}

impl BreadcrumbPod {
    pub(super) fn from_row(pod: &PodRow) -> Self {
        BreadcrumbPod {
            namespace: pod.namespace.clone(),
            name: pod.name.clone(),
            containers: pod.containers.iter().map(|c| (container_dot(c).1, c.name.clone(), container_state_text(c))).collect(),
        }
    }
}

/// Names are shortened to this many characters before anything gets
/// squeezed harder, and never below `HARD_MIN`.
const MIN_SHORTENED: usize = 8;
const HARD_MIN: usize = 3;

/// Shortens `text` to at most `max` characters with an ellipsis in the
/// middle, keeping both ends — the start says what it is, the end is where
/// generated names differ.
fn middle_ellipsis(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 2 {
        return text.to_string();
    }
    let keep = max - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    format!("{}…{}", chars[..head].iter().collect::<String>(), chars[chars.len() - tail..].iter().collect::<String>())
}

/// How much of the containers to show: every name and state, just the
/// coloured dots, or nothing.
#[derive(Clone, Copy)]
enum Detail {
    Full,
    Dots,
    None,
}

fn build(segments: &[BreadcrumbSegment], pod: Option<&BreadcrumbPod>, caps: &[usize], detail: Detail) -> Line<'static> {
    let kind_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let value_style = Style::default().fg(Color::Gray);
    let punct_style = Style::default().fg(Color::DarkGray);
    let cap = |i: usize, text: &str| middle_ellipsis(text, caps.get(i).copied().unwrap_or(usize::MAX));

    let mut spans = vec![Span::raw(" ")];
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(">>", punct_style));
        }
        spans.push(Span::styled(segment.kind.clone(), kind_style));
        if let Some(value) = &segment.value {
            spans.push(Span::styled("[", punct_style));
            spans.push(Span::styled(cap(i, value), value_style));
            spans.push(Span::styled("]", punct_style));
        }
    }
    if let Some(pod) = pod {
        let (ns_cap, name_cap) = (segments.len(), segments.len() + 1);
        spans.push(Span::styled(">>", punct_style));
        spans.extend(namespace_name_spans(&cap(ns_cap, &pod.namespace), &cap(name_cap, &pod.name)));
        if !matches!(detail, Detail::None) && !pod.containers.is_empty() {
            spans.push(Span::raw(" ["));
            for (i, (color, name, state)) in pod.containers.iter().enumerate() {
                if i > 0 {
                    spans.push(Span::styled(if matches!(detail, Detail::Full) { " : " } else { " " }, punct_style));
                }
                let style = Style::default().fg(*color);
                spans.push(Span::styled("●", style));
                if matches!(detail, Detail::Full) {
                    spans.push(Span::styled(format!(" {name}({state})"), style));
                }
            }
            spans.push(Span::raw("]"));
        }
    }
    Line::from(spans)
}

/// The bar's line for a terminal `width` cells wide.
pub(super) fn breadcrumb_line(segments: &[BreadcrumbSegment], pod: Option<&BreadcrumbPod>, width: u16) -> Line<'static> {
    let width = usize::from(width);
    // Give up container detail before touching any name.
    for detail in [Detail::Full, Detail::Dots, Detail::None] {
        let line = build(segments, pod, &[], detail);
        if line.width() <= width {
            return line;
        }
    }
    // Then shorten the longest value/name a character at a time.
    let mut texts: Vec<usize> = segments.iter().map(|s| s.value.as_ref().map_or(0, |v| v.chars().count())).collect();
    texts.push(pod.map_or(0, |p| p.namespace.chars().count()));
    texts.push(pod.map_or(0, |p| p.name.chars().count()));
    let mut caps = texts.clone();
    loop {
        let line = build(segments, pod, &caps, Detail::None);
        if line.width() <= width {
            return line;
        }
        let widest = |floor: usize| (0..caps.len()).filter(|&i| caps[i] > floor).max_by_key(|&i| caps[i]);
        match widest(MIN_SHORTENED).or_else(|| widest(HARD_MIN)) {
            Some(i) => caps[i] -= 1,
            None => return line,
        }
    }
}

/// The bar's line drawn on the last row of the screen, over everything.
pub(super) fn draw_breadcrumb_bar(frame: &mut Frame, segments: &[BreadcrumbSegment], pod: Option<BreadcrumbPod>) {
    let area = frame.area();
    let bar = Rect { x: area.x, y: area.y + area.height.saturating_sub(1), width: area.width, height: 1 };
    frame.render_widget(Clear, bar);
    frame.render_widget(Paragraph::new(breadcrumb_line(segments, pod.as_ref(), bar.width)), bar);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: &str, value: Option<&str>) -> BreadcrumbSegment {
        BreadcrumbSegment { kind: kind.into(), value: value.map(String::from) }
    }

    fn pod() -> BreadcrumbPod {
        BreadcrumbPod {
            namespace: "kube-system".into(),
            name: "local-path-provisioner-5d9d9885bc-f".into(),
            containers: vec![(Color::Green, "local-path-provisioner".into(), "Running".into())],
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn middle_ellipsis_keeps_both_ends() {
        assert_eq!(middle_ellipsis("local-path-provisioner", 12), "local-…ioner");
        assert_eq!(middle_ellipsis("short", 12), "short");
    }

    #[test]
    fn a_wide_terminal_shows_everything() {
        let segments = [seg("Deployment", Some("web")), seg("Pods", None)];
        let line = breadcrumb_line(&segments, Some(&pod()), 300);
        assert!(text(&line).contains("● local-path-provisioner(Running)"));
    }

    #[test]
    fn container_detail_goes_before_names_are_touched() {
        let segments = [seg("Deployment", Some("local-path-provisioner")), seg("ReplicaSet", Some("local-path-provisioner-5d9d9885bc")), seg("Pods", None)];
        let full = breadcrumb_line(&segments, Some(&pod()), 500).width();
        // A little narrower than everything: the container text is dropped, the names are intact.
        let line = breadcrumb_line(&segments, Some(&pod()), (full - 10) as u16);
        let t = text(&line);
        assert!(t.contains("local-path-provisioner-5d9d9885bc]"), "{t}");
        assert!(!t.contains("(Running)"), "{t}");
    }

    #[test]
    fn it_always_fits_by_shortening_the_longest_names() {
        let segments = [seg("Deployment", Some("local-path-provisioner")), seg("ReplicaSet", Some("local-path-provisioner-5d9d9885bc")), seg("Pods", None)];
        for width in [120u16, 100, 80, 60] {
            let line = breadcrumb_line(&segments, Some(&pod()), width);
            assert!(line.width() <= usize::from(width), "width {width}: {} > {width} in {:?}", line.width(), text(&line));
        }
        assert!(text(&breadcrumb_line(&segments, Some(&pod()), 80)).contains('…'));
    }
}
