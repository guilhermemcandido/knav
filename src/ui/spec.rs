//! The manifest (`d`) tree popup and the value viewer.

use super::*;

/// `dimmed` only ever applies when this is the background behind its
/// own `ValueDetail` popup (`v` on a leaf) — the tree's own per-node
/// colors (baked into each `TreeItem`'s `Line` at build time in
/// `build_manifest_tree`) aren't re-muted, just the border/title and
/// selection highlight, same lighter-touch dimming `EventDetail`'s
/// background gets.
pub(super) fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>, dimmed: bool) {
    let area = centered_rect(85, 85, frame.area());
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_line = if dimmed { Line::styled(title.to_string(), dim_style()) } else { colored_slash_title(title) };
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(border_style).title(title_line);

    let highlight_style = if dimmed { dim_style() } else { Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD) };
    let tree = Tree::new(items)
        .expect("pod tree ids are unique per level by construction")
        .block(block)
        .highlight_style(highlight_style)
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("  ");

    frame.render_stateful_widget(tree, area, state);
}

/// A tree leaf's full value, untruncated — opened by `v`. Plain wrapped
/// text, same treatment as `draw_event_detail_popup` for the same
/// reason: a narrow column/box clips long content with no indication or
/// way to see the rest.
pub(super) fn draw_value_detail_popup(frame: &mut Frame, label: &str, value: &str) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).title(label.to_string());
    let paragraph = Paragraph::new(value.to_string()).wrap(Wrap { trim: false }).block(block);
    frame.render_widget(paragraph, area);
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
/// Alongside the tree itself, a lookup from a leaf's identifier (opaque,
/// but guaranteed unique — see below) to its `(label, full value)` —
/// tree items only ever show a value clipped to the box's width with no
/// indication it's cut off or way to see the rest, so `v` (see the
/// `Mode::Spec` keyboard handler) looks it up here to show untruncated.
/// A leaf's `(label, full value)`, by its tree identifier.
pub type LeafValues = HashMap<String, (String, String)>;

pub fn build_manifest_tree(value: &serde_yaml::Value) -> (Vec<TreeItem<'static, String>>, LeafValues) {
    let mut leaf_values = HashMap::new();
    let items = children_of(value, "root", &mut leaf_values);
    (items, leaf_values)
}

pub(super) fn children_of(value: &serde_yaml::Value, path: &str, leaf_values: &mut LeafValues) -> Vec<TreeItem<'static, String>> {
    match value {
        serde_yaml::Value::Mapping(map) => map
            .iter()
            .map(|(k, v)| {
                let label = scalar_to_string(k);
                node(&format!("{path}/{label}"), &label, v, leaf_values)
            })
            .collect(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = format!("[{i}]");
                node(&format!("{path}/{label}"), &label, v, leaf_values)
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn node(id: &str, label: &str, value: &serde_yaml::Value, leaf_values: &mut LeafValues) -> TreeItem<'static, String> {
    match value {
        serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => {
            let children = children_of(value, id, leaf_values);
            let text = Line::from(Span::styled(
                label.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            TreeItem::new(id.to_string(), text, children)
                .expect("child identifiers are unique per level by construction")
        }
        scalar => {
            let full_value = scalar_to_string(scalar);
            leaf_values.insert(id.to_string(), (label.to_string(), full_value.clone()));
            let text = Line::from(vec![Span::styled(format!("{label}: "), Style::default().fg(Color::Cyan)), Span::raw(full_value)]);
            TreeItem::new_leaf(id.to_string(), text)
        }
    }
}

pub(super) fn scalar_to_string(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}
