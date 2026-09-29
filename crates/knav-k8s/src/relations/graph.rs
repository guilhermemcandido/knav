//! The diagram model: which objects sit in which column, and how they connect.

use super::*;

/// One box of the diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphNode {
    pub kind: String,
    pub namespace: Option<String>,
    pub name: String,
    pub detail: String,
    /// Columns left (negative) and right (positive) of the object in the middle.
    pub layer: i32,
}

/// The relations as boxes and arrows (from, to). Node 0 is the object itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<(usize, usize)>,
}

/// Lays `groups` out as a flow from left to right, arrows pointing from what provides
/// to what depends: callers, owners and what the object uses on the left, the object
/// in the middle, what it owns, selects or what uses it on the right.
pub fn graph(target: &Value, groups: &[Group]) -> Graph {
    let mut g = Graph::default();
    let Some(t) = obj(target) else { return g };
    g.nodes.push(GraphNode { kind: t.kind.to_string(), namespace: t.namespace.map(String::from), name: t.name.to_string(), detail: String::new(), layer: 0 });
    fn node(g: &mut Graph, e: &Entry, layer: i32) -> usize {
        if let Some(at) = g.nodes.iter().position(|n| n.kind == e.kind && n.name == e.name && n.namespace == e.namespace) {
            return at;
        }
        g.nodes.push(GraphNode { kind: e.kind.clone(), namespace: e.namespace.clone(), name: e.name.clone(), detail: e.detail.clone(), layer });
        g.nodes.len() - 1
    }
    for group in groups {
        match group.title {
            "Owned by" => {
                let mut previous = 0;
                for e in &group.entries {
                    let at = node(&mut g, e, -(e.depth as i32) - 1);
                    g.edges.push((at, previous));
                    previous = at;
                }
            }
            // Arrows run from what provides to what depends: a Node, ConfigMap or
            // Secret points at the pod that uses it.
            "Uses" => {
                for e in &group.entries {
                    let at = node(&mut g, e, -1);
                    g.edges.push((at, 0));
                }
            }
            "Exposed by" => {
                let mut service = 0;
                for e in &group.entries {
                    if e.depth == 0 {
                        service = node(&mut g, e, -1);
                        g.edges.push((service, 0));
                    } else {
                        let ingress = node(&mut g, e, -2);
                        g.edges.push((ingress, service));
                    }
                }
            }
            _ => {
                for e in &group.entries {
                    let at = node(&mut g, e, 1);
                    g.edges.push((0, at));
                }
            }
        }
    }
    g
}


/// The diagram as Mermaid text (`flowchart LR`), for pasting into docs or an issue.
pub fn mermaid(graph: &Graph) -> String {
    let clean = |text: &str| text.replace('"', "'");
    let mut out = String::from("flowchart LR\n");
    for (i, node) in graph.nodes.iter().enumerate() {
        let place = node.namespace.as_deref().map(|ns| format!("{ns}/")).unwrap_or_default();
        out.push_str(&format!("  n{i}[\"{}<br/>{}\"]\n", clean(&node.kind), clean(&format!("{place}{}", node.name))));
    }
    for (from, to) in &graph.edges {
        match graph.nodes.get(*from).map(|n| n.detail.as_str()).filter(|d| !d.is_empty()) {
            Some(why) => out.push_str(&format!("  n{from} -->|{}| n{to}\n", clean(why))),
            None => out.push_str(&format!("  n{from} --> n{to}\n")),
        }
    }
    out
}

#[cfg(test)]
mod mermaid_tests {
    use super::*;

    #[test]
    fn boxes_and_arrows_become_mermaid_lines() {
        let node = |kind: &str, name: &str, detail: &str| GraphNode { kind: kind.into(), namespace: Some("shop".into()), name: name.into(), detail: detail.into(), layer: 0 };
        let graph = Graph { nodes: vec![node("Pod", "web", ""), node("ConfigMap", "cfg", "volume")], edges: vec![(1, 0)] };
        let text = mermaid(&graph);
        assert!(text.starts_with("flowchart LR\n"));
        assert!(text.contains("n0[\"Pod<br/>shop/web\"]") && text.contains("n1 -->|volume| n0"), "{text}");
    }
}
