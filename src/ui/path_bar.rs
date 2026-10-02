//! The bottom bar: the trail of what you drilled through, then the selected row. To fit,
//! container detail goes first, then the longest names are shortened in the middle.

use super::*;

/// The selected row, shown at the end of the bar: a pod with its containers, a
/// deployment or node with a status note, or any other object by name.
pub struct SelectedItem {
    namespace: Option<String>,
    name: String,
    /// A short coloured status after the name, dropped when space is short.
    note: Option<(Color, String)>,
    /// Dot colour, name and state per container, pods only.
    containers: Vec<(Color, String, String)>,
}

impl SelectedItem {
    pub(super) fn from_pod(pod: &PodRow) -> Self {
        SelectedItem {
            namespace: Some(pod.namespace.clone()),
            name: pod.name.clone(),
            note: None,
            containers: pod.containers.iter().map(|c| (container_dot(c).1, c.name.clone(), container_state_text(c))).collect(),
        }
    }

    pub(super) fn from_deployment(dep: &DeploymentRow) -> Self {
        SelectedItem { namespace: Some(dep.namespace.clone()), name: dep.name.clone(), note: Some((ready_color(&dep.ready), dep.ready.clone())), containers: Vec::new() }
    }

    pub(super) fn from_node(node: &NodeRow) -> Self {
        let (color, status) = match (node.ready, node.schedulable) {
            (true, true) => (theme().ok, "Ready"),
            (true, false) => (theme().warn, "Ready, cordoned"),
            (false, _) => (theme().bad, "NotReady"),
        };
        SelectedItem { namespace: None, name: node.name.clone(), note: Some((color, status.to_string())), containers: Vec::new() }
    }

    pub(super) fn from_generic(row: &GenericRow) -> Self {
        use crate::k8s::describe::Tone;
        SelectedItem {
            namespace: (row.namespace != "-").then(|| row.namespace.clone()),
            name: row.name.clone(),
            note: row.status.as_ref().map(|(tone, text)| {
                let color = match tone {
                    Tone::Plain => theme().text_soft,
                    Tone::Good => theme().ok,
                    Tone::Warn => theme().warn,
                    Tone::Bad => theme().bad,
                    Tone::Muted => theme().muted,
                };
                (color, text.clone())
            }),
            containers: Vec::new(),
        }
    }

    pub(super) fn from_crd(crd: &CrdInfo) -> Self {
        SelectedItem { namespace: Some(crd.group.to_string()), name: crd.kind.to_string(), note: None, containers: Vec::new() }
    }
}

/// Names are shortened to this length before anything is squeezed harder, and never
/// below `HARD_MIN`.
const MIN_SHORTENED: usize = 8;
const HARD_MIN: usize = 3;

/// Shortens `text` to `max` characters with an ellipsis in the middle: the start says
/// what it is, the end is where generated names differ.
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

/// How much of the containers to show: names and states, just dots, or nothing.
#[derive(Clone, Copy)]
enum Detail {
    Full,
    Dots,
    None,
}

fn build(segments: &[PathSegment], pod: Option<&SelectedItem>, caps: &[usize], detail: Detail) -> Line<'static> {
    // Each step is a pill like a list's title, so the trail reads at a glance.
    let pill = Style::default().bg(theme().pill_bg);
    let kind_style = pill.fg(theme().namespace).add_modifier(Modifier::BOLD);
    let value_style = pill.fg(theme().text_strong);
    let punct_style = Style::default().fg(theme().muted);
    let cap = |i: usize, text: &str| middle_ellipsis(text, caps.get(i).copied().unwrap_or(usize::MAX));
    let separator = || Span::styled(" › ", punct_style);

    let mut spans = vec![Span::raw(" ")];
    for (i, segment) in segments.iter().enumerate() {
        if i > 0 {
            spans.push(separator());
        }
        match &segment.value {
            Some(value) => {
                spans.push(Span::styled(format!(" {} ", segment.kind), kind_style));
                spans.push(Span::styled(format!("{} ", cap(i, value)), value_style));
            }
            None => spans.push(Span::styled(format!(" {} ", segment.kind), kind_style)),
        }
    }
    if let Some(pod) = pod {
        let (ns_cap, name_cap) = (segments.len(), segments.len() + 1);
        spans.push(separator());
        // The selected row stands out from the trail: the selection's colours.
        let current = Style::default().bg(theme().select_bg).fg(crate::theme::on(theme().select_bg));
        spans.push(Span::styled(" ", current));
        if let Some(namespace) = &pod.namespace {
            spans.push(Span::styled(cap(ns_cap, namespace), current));
            spans.push(Span::styled("/", current));
        }
        spans.push(Span::styled(format!("{} ", cap(name_cap, &pod.name)), current.add_modifier(Modifier::BOLD)));
        // The status and each container are pills too, in their own colour.
        if let (Some((color, note)), Detail::Full) = (&pod.note, detail) {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!(" ● {note} "), pill.fg(*color)));
        }
        if !matches!(detail, Detail::None) {
            for (color, name, state) in &pod.containers {
                spans.push(Span::raw(" "));
                let text = if matches!(detail, Detail::Full) { format!(" ● {name} {state} ") } else { " ● ".to_string() };
                spans.push(Span::styled(text, pill.fg(*color)));
            }
        }
    }
    Line::from(spans)
}

pub(super) fn path_line(segments: &[PathSegment], pod: Option<&SelectedItem>, width: u16) -> Line<'static> {
    let width = usize::from(width);
    // Give up container detail before touching any name.
    for detail in [Detail::Full, Detail::Dots, Detail::None] {
        let line = build(segments, pod, &[], detail);
        if line.width() <= width {
            return line;
        }
    }
    // Then shorten the longest value/name a character at a time.
    let mut texts: Vec<usize> = segments.iter().map(|s| s.value.as_ref().map_or(0, |v| cell_width(v))).collect();
    texts.push(pod.and_then(|p| p.namespace.as_ref()).map_or(0, |n| cell_width(n)));
    texts.push(pod.map_or(0, |p| cell_width(&p.name)));
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

/// Draws the bar on the last row, over everything.
pub(super) fn draw_path_bar(frame: &mut Frame, segments: &[PathSegment], pod: Option<SelectedItem>) {
    let area = frame.area();
    let bar = Rect { x: area.x, y: area.y + area.height.saturating_sub(1), width: area.width, height: 1 };
    frame.render_widget(Clear, bar);
    frame.render_widget(Paragraph::new(path_line(segments, pod.as_ref(), bar.width)), bar);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(kind: &str, value: Option<&str>) -> PathSegment {
        PathSegment { kind: kind.into(), value: value.map(String::from) }
    }

    fn pod() -> SelectedItem {
        SelectedItem {
            namespace: Some("kube-system".into()),
            name: "local-path-provisioner-5d9d9885bc-f".into(),
            note: None,
            containers: vec![(theme().ok, "local-path-provisioner".into(), "Running".into())],
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn non_pod_rows_show_by_name_with_a_note_that_goes_first() {
        let node = SelectedItem { namespace: None, name: "worker-1".into(), note: Some((theme().ok, "Ready".into())), containers: Vec::new() };
        let wide = text(&path_line(&[seg("Nodes", None)], Some(&node), 100));
        assert!(wide.ends_with("worker-1   ● Ready "), "{wide}");
        let tight = text(&path_line(&[seg("Nodes", None)], Some(&node), 22));
        assert!(tight.ends_with("worker-1 "), "{tight}");
        let configmap = SelectedItem { namespace: Some("default".into()), name: "kube-root-ca.crt".into(), note: None, containers: Vec::new() };
        assert!(text(&path_line(&[seg("ConfigMaps", None)], Some(&configmap), 100)).ends_with("default/kube-root-ca.crt "));
    }

    #[test]
    fn middle_ellipsis_keeps_both_ends() {
        assert_eq!(middle_ellipsis("local-path-provisioner", 12), "local-…ioner");
        assert_eq!(middle_ellipsis("short", 12), "short");
    }

    #[test]
    fn a_wide_terminal_shows_everything() {
        let segments = [seg("Deployment", Some("web")), seg("Pods", None)];
        let line = path_line(&segments, Some(&pod()), 300);
        assert!(text(&line).contains("● local-path-provisioner Running"));
    }

    #[test]
    fn container_detail_goes_before_names_are_touched() {
        let segments = [seg("Deployment", Some("local-path-provisioner")), seg("ReplicaSet", Some("local-path-provisioner-5d9d9885bc")), seg("Pods", None)];
        let full = path_line(&segments, Some(&pod()), 500).width();
        // A little narrower: the container text goes, the names stay whole.
        let line = path_line(&segments, Some(&pod()), (full - 10) as u16);
        let t = text(&line);
        assert!(t.contains("local-path-provisioner-5d9d9885bc "), "{t}");
        assert!(!t.contains("Running"), "{t}");
    }

    #[test]
    fn it_always_fits_by_shortening_the_longest_names() {
        let segments = [seg("Deployment", Some("local-path-provisioner")), seg("ReplicaSet", Some("local-path-provisioner-5d9d9885bc")), seg("Pods", None)];
        for width in [120u16, 100, 80, 60] {
            let line = path_line(&segments, Some(&pod()), width);
            assert!(line.width() <= usize::from(width), "width {width}: {} > {width} in {:?}", line.width(), text(&line));
        }
        assert!(text(&path_line(&segments, Some(&pod()), 80)).contains('…'));
    }
}
