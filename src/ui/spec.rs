//! The manifest (`d`) tree popup and the value viewer.

use super::*;

/// `dimmed` applies only when this is the background of its `ValueDetail` popup.
/// Node colours are baked into each `TreeItem`, so only the border, title and
/// selection are muted.
pub(super) fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>, dimmed: bool) {
    // Full width, so selecting text with the mouse never takes in what is behind.
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_line = pill_title(title, dimmed, border_style);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title_line);

    let highlight_style = if dimmed { dim_style() } else { Style::default().bg(theme().muted).add_modifier(Modifier::BOLD) };
    let tree = Tree::new(items)
        .expect("pod tree ids are unique per level by construction")
        .block(block)
        .highlight_style(highlight_style)
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("  ");

    frame.render_stateful_widget(tree, area, state);
}

/// A leaf's full value, opened by `v`. Wrapped plain text, since the tree clips
/// long values.
pub(super) fn draw_value_detail_popup(frame: &mut Frame, label: &str, value: &str) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title(pill_title(label, false, Style::default()));
    let paragraph = Paragraph::new(value.to_string()).wrap(Wrap { trim: false }).block(block);
    frame.render_widget(paragraph, area);
}

/// Click-to-toggle at an absolute terminal position, `TreeState` already
/// knows where everything was last rendered, so no manual hit-testing.
pub fn click_tree(state: &mut TreeState<String>, column: u16, row: u16) {
    if let Some(path) = state.rendered_at(Position::new(column, row)) {
        let path = path.to_vec();
        state.select(path.clone());
        state.toggle(path);
    }
}

/// Builds the collapsible tree for a manifest from its value tree. Each identifier
/// is its full path (`root/spec/containers/[0]/image`), unique even when siblings
/// reuse names. Also returns leaf identifier to `(label, full value)`, for `v`.
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
            .enumerate()
            .map(|(i, (k, v))| {
                let label = scalar_to_string(k);
                // The index keeps ids unique when keys such as `1` and "1" print alike.
                node(&format!("{path}/{i}:{label}"), &label, v, leaf_values)
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
                Style::default().fg(theme().namespace).add_modifier(Modifier::BOLD),
            ));
            TreeItem::new(id.to_string(), text, children)
                .expect("child identifiers are unique per level by construction")
        }
        scalar => {
            let full_value = scalar_to_string(scalar);
            leaf_values.insert(id.to_string(), (label.to_string(), full_value.clone()));
            let text = Line::from(vec![Span::styled(format!("{label}: "), Style::default().fg(theme().namespace)), Span::raw(full_value)]);
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
