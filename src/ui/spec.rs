//! The manifest (`d`) tree popup and the value viewer.

use ratatui::{buffer::Buffer, widgets::StatefulWidget};

use super::*;

/// The manifest tree. `dimmed` applies when it sits behind the value popup; node
/// colours are fixed per item, so only the border, title and selection mute. `hscroll`
/// is how far it is scrolled sideways, clamped here to the widest visible line.
pub(super) fn draw_spec_popup(frame: &mut Frame, title: &str, items: &[TreeItem<'static, String>], state: &mut TreeState<String>, hscroll: &mut usize, leaf_values: &LeafValues, dimmed: bool) {
    // The whole body width, so selecting text with the mouse never takes in what is behind.
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);

    let border_style = if dimmed { dim_style() } else { Style::default() };
    let title_line = pill_title(title, dimmed, border_style);
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(border_style).title(title_line);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let highlight_style = if dimmed { dim_style() } else { Style::default().bg(theme().muted).add_modifier(Modifier::BOLD) };
    let tree = Tree::new(items)
        .expect("pod tree ids are unique per level by construction")
        .highlight_style(highlight_style)
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("  ");

    // The tree clips at its own width, so it is drawn on a buffer a screenful wider than
    // what is scrolled past, and the part in view is copied over.
    let wide = Rect { width: inner.width.saturating_add((*hscroll).min(usize::from(u16::MAX / 2)) as u16).saturating_add(inner.width), ..inner };
    let mut scratch = Buffer::empty(wide);
    StatefulWidget::render(tree, wide, &mut scratch, state);
    let content = (0..wide.height)
        .filter_map(|dy| (0..wide.width).rev().find(|dx| scratch[(wide.x + dx, wide.y + dy)].symbol().trim() != "").map(|dx| usize::from(dx) + 1))
        .max()
        .unwrap_or(0);
    *hscroll = (*hscroll).min(content.saturating_sub(usize::from(inner.width)));
    // Each row's whole text, long values in full, so a selection copies them uncut.
    let rows = tree_rows(items, state, leaf_values);
    let mut shown: Vec<super::selection::TextRow> = Vec::new();
    for dy in 0..inner.height {
        let Some(id) = state.rendered_at(ratatui::layout::Position { x: inner.x, y: inner.y + dy }) else { break };
        let Some(line) = rows.iter().position(|(row_id, _)| row_id.as_slice() == id) else { break };
        if shown.last().is_some_and(|r| r.line == line) {
            break;
        }
        shown.push(super::selection::TextRow { line, start: *hscroll, text: rows[line].1.clone().into() });
    }
    super::selection::text_rows(super::selection::Pane::Tree, inner, shown);
    for dy in 0..inner.height {
        for dx in 0..inner.width {
            let from = (wide.x + dx + *hscroll as u16, wide.y + dy);
            frame.buffer_mut()[(inner.x + dx, inner.y + dy)] = scratch[from].clone();
        }
    }
}

/// The tree's rows as the widget lays them out, with each leaf's value in full.
fn tree_rows(items: &[TreeItem<'static, String>], state: &TreeState<String>, leaf_values: &LeafValues) -> Vec<(Vec<String>, String)> {
    state
        .flatten(items)
        .into_iter()
        .map(|row| {
            let depth = row.depth();
            let id = row.identifier.last().cloned().unwrap_or_default();
            let parent = if depth == 0 { "root".to_string() } else { row.identifier[depth - 1].clone() };
            let symbol = if row.item.children().is_empty() { "  " } else if state.opened().contains(&row.identifier) { "▾ " } else { "▸ " };
            let text = match leaf_values.get(&id) {
                Some((label, value)) => format!("{label}: {}", value.replace('\n', " ⏎ ")),
                None => branch_label(&id, &parent),
            };
            let line = format!("{}{symbol}{text}", " ".repeat(depth * 2));
            (row.identifier, line)
        })
        .collect()
}

/// A branch's label from its id, `<parent>/<index>:<key>` for a map key or
/// `<parent>/[<index>]` for a list item.
fn branch_label(id: &str, parent: &str) -> String {
    let own = id.strip_prefix(parent).and_then(|rest| rest.strip_prefix('/')).unwrap_or(id);
    match own.split_once(':') {
        Some((index, key)) if index.chars().all(|c| c.is_ascii_digit()) => key.to_string(),
        _ => own.to_string(),
    }
}

/// The tree's rows as text, for copying a selection made in it.
pub fn tree_text(items: &[TreeItem<'static, String>], state: &TreeState<String>, leaf_values: &LeafValues) -> Vec<String> {
    tree_rows(items, state, leaf_values).into_iter().map(|(_, line)| line).collect()
}

/// A leaf's full value (`v`), wrapped, since the tree clips long values.
pub(super) fn draw_value_detail_popup(frame: &mut Frame, label: &str, value: &str) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);

    let block = Block::default().borders(Borders::ALL).border_set(border_set()).title(pill_title(label, false, Style::default()));
    let paragraph = Paragraph::new(value.to_string()).wrap(Wrap { trim: false }).block(block);
    frame.render_widget(paragraph, area);
}

/// Toggles the tree node at a terminal position; `TreeState` knows where it drew them.
pub fn click_tree(state: &mut TreeState<String>, column: u16, row: u16) {
    if let Some(path) = state.rendered_at(Position::new(column, row)) {
        let path = path.to_vec();
        state.select(path.clone());
        state.toggle(path);
    }
}

/// Each leaf's identifier mapped to its label and full value, for `v`.
pub type LeafValues = HashMap<String, (String, String)>;

/// The collapsible tree of a manifest. Each identifier is its full path
/// (`root/spec/containers/[0]/image`), unique even when siblings share names.
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
            let one_line = full_value.replace('\n', " ⏎ ");
            let text = Line::from(vec![Span::styled(format!("{label}: "), Style::default().fg(theme().namespace)), Span::raw(one_line)]);
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

#[cfg(test)]
mod copy_tests {
    use super::*;

    #[test]
    fn tree_rows_read_like_the_tree_with_whole_values() {
        let manifest: serde_yaml::Value = serde_yaml::from_str("metadata:\n  name: web\n  a/b: x\nspec:\n  image: a-very-long-image-name:1.2.3\n").unwrap();
        let (items, leaves) = build_manifest_tree(&manifest);
        let mut state = TreeState::default();
        state.open(vec!["root/0:metadata".to_string()]);
        let text = tree_text(&items, &state, &leaves);
        assert_eq!(text, ["▾ metadata", "    name: web", "    a/b: x", "▸ spec"]);
        assert_eq!(branch_label("root/1:a/b", "root"), "a/b");
        assert_eq!(branch_label("root/0:spec/[3]", "root/0:spec"), "[3]");
    }
}
