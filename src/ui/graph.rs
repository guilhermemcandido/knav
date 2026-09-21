//! The relations diagram: boxes in columns, joined by arrows that flow left to
//! right, like a Mermaid `flowchart LR`.

use super::*;
use crate::k8s::relations::Graph;

pub const BOX_W: u16 = 26;
pub const BOX_H: u16 = 4;
/// Room between columns for the arrows.
pub const GAP: u16 = 7;
const ROW_GAP: u16 = 1;

/// Where each box sits on the canvas, and how big the canvas is.
pub struct GraphLayout {
    pub pos: Vec<(u16, u16)>,
    pub width: u16,
    pub height: u16,
}

pub fn layout(graph: &Graph) -> GraphLayout {
    let layers: Vec<i32> = {
        let mut l: Vec<i32> = graph.nodes.iter().map(|n| n.layer).collect();
        l.sort_unstable();
        l.dedup();
        l
    };
    let column_of = |layer: i32| layers.iter().position(|l| *l == layer).unwrap_or(0) as u16;
    let members = |layer: i32| graph.nodes.iter().enumerate().filter(move |(_, n)| n.layer == layer).map(|(i, _)| i);
    let height = layers.iter().map(|l| members(*l).count() as u16 * (BOX_H + ROW_GAP)).max().unwrap_or(BOX_H).saturating_sub(ROW_GAP).max(BOX_H);
    let mut pos = vec![(0, 0); graph.nodes.len()];
    for layer in &layers {
        let indices: Vec<usize> = members(*layer).collect();
        let column_height = (indices.len() as u16 * (BOX_H + ROW_GAP)).saturating_sub(ROW_GAP);
        let top = (height - column_height) / 2;
        for (row, i) in indices.into_iter().enumerate() {
            pos[i] = (column_of(*layer) * (BOX_W + GAP), top + row as u16 * (BOX_H + ROW_GAP));
        }
    }
    GraphLayout { pos, width: layers.len() as u16 * (BOX_W + GAP) - GAP, height }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
}

/// The box to go to from `from`: the nearest one in that direction.
pub fn neighbor(graph: &Graph, layout: &GraphLayout, from: usize, direction: Move) -> Option<usize> {
    let here = &graph.nodes[from];
    let centre = |i: usize| i32::from(layout.pos[i].1) + i32::from(BOX_H) / 2;
    let candidates: Vec<usize> = match direction {
        Move::Up => (0..graph.nodes.len()).filter(|&i| graph.nodes[i].layer == here.layer && layout.pos[i].1 < layout.pos[from].1).collect(),
        Move::Down => (0..graph.nodes.len()).filter(|&i| graph.nodes[i].layer == here.layer && layout.pos[i].1 > layout.pos[from].1).collect(),
        Move::Left | Move::Right => {
            let sign = if direction == Move::Left { -1 } else { 1 };
            let next = graph.nodes.iter().map(|n| n.layer).filter(|l| (l - here.layer) * sign > 0).min_by_key(|l| (l - here.layer).abs());
            match next {
                Some(layer) => (0..graph.nodes.len()).filter(|&i| graph.nodes[i].layer == layer).collect(),
                None => Vec::new(),
            }
        }
    };
    candidates.into_iter().min_by_key(|&i| (centre(i) - centre(from)).abs())
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// The line glyph for the directions (up, down, left, right) a cell joins.
fn glyph(mask: u8) -> char {
    match mask {
        m if m == UP => '╵',
        m if m == DOWN => '╷',
        m if m == LEFT => '╴',
        m if m == RIGHT => '╶',
        m if m == UP | DOWN => '│',
        m if m == LEFT | RIGHT => '─',
        m if m == UP | RIGHT => '└',
        m if m == UP | LEFT => '┘',
        m if m == DOWN | RIGHT => '┌',
        m if m == DOWN | LEFT => '┐',
        m if m == UP | DOWN | RIGHT => '├',
        m if m == UP | DOWN | LEFT => '┤',
        m if m == LEFT | RIGHT | UP => '┴',
        m if m == LEFT | RIGHT | DOWN => '┬',
        m if m == UP | DOWN | LEFT | RIGHT => '┼',
        _ => ' ',
    }
}

/// The whole diagram on a character grid.
fn canvas(graph: &Graph, layout: &GraphLayout, selected: usize) -> Vec<Vec<(char, Style)>> {
    let (w, h) = (usize::from(layout.width), usize::from(layout.height));
    let mut masks = vec![vec![0u8; w]; h];
    let mut lit = vec![vec![false; w]; h];
    let mut arrows: Vec<(usize, usize, char, bool)> = Vec::new();
    let join = |masks: &mut Vec<Vec<u8>>, lit: &mut Vec<Vec<bool>>, from: (usize, usize), to: (usize, usize), highlighted: bool| {
        // A straight run from one cell to another, each cell joined to its neighbours.
        let (dx, dy) = ((to.0 as i32 - from.0 as i32).signum(), (to.1 as i32 - from.1 as i32).signum());
        let (mut x, mut y) = (from.0 as i32, from.1 as i32);
        loop {
            let (cx, cy) = (x as usize, y as usize);
            if cx < w && cy < h {
                if (x, y) != (from.0 as i32, from.1 as i32) {
                    masks[cy][cx] |= if dx > 0 { LEFT } else if dx < 0 { RIGHT } else if dy > 0 { UP } else { DOWN };
                }
                if (x, y) != (to.0 as i32, to.1 as i32) {
                    masks[cy][cx] |= if dx > 0 { RIGHT } else if dx < 0 { LEFT } else if dy > 0 { DOWN } else { UP };
                }
                lit[cy][cx] |= highlighted;
            }
            if (x, y) == (to.0 as i32, to.1 as i32) {
                break;
            }
            x += dx;
            y += dy;
        }
    };
    for &(a, b) in &graph.edges {
        let (from, to) = (&graph.nodes[a], &graph.nodes[b]);
        let (pa, pb) = (layout.pos[a], layout.pos[b]);
        let rightwards = from.layer <= to.layer;
        let (ya, yb) = (usize::from(pa.1 + BOX_H / 2 - 1), usize::from(pb.1 + BOX_H / 2 - 1));
        let highlighted = a == selected || b == selected;
        if rightwards {
            let start = usize::from(pa.0 + BOX_W);
            let trunk = start + usize::from(GAP / 2);
            let end = usize::from(pb.0).saturating_sub(1);
            masks[ya.min(h - 1)][start.min(w - 1)] |= LEFT;
            join(&mut masks, &mut lit, (start, ya), (trunk.min(end), ya), highlighted);
            join(&mut masks, &mut lit, (trunk.min(end), ya), (trunk.min(end), yb), highlighted);
            join(&mut masks, &mut lit, (trunk.min(end), yb), (end, yb), highlighted);
            arrows.push((end, yb, '▶', highlighted));
        } else {
            let start = usize::from(pa.0).saturating_sub(1);
            let end = usize::from(pb.0 + BOX_W);
            let trunk = start.saturating_sub(usize::from(GAP / 2)).max(end);
            join(&mut masks, &mut lit, (start, ya), (trunk, ya), highlighted);
            join(&mut masks, &mut lit, (trunk, ya), (trunk, yb), highlighted);
            join(&mut masks, &mut lit, (trunk, yb), (end, yb), highlighted);
            arrows.push((end, yb, '◀', highlighted));
        }
    }
    let muted = Style::default().fg(theme().muted);
    let bright = Style::default().fg(theme().accent);
    let mut grid: Vec<Vec<(char, Style)>> = (0..h).map(|y| (0..w).map(|x| (glyph(masks[y][x]), if lit[y][x] { bright } else { muted })).collect()).collect();
    for (x, y, ch, highlighted) in arrows {
        if y < h && x < w {
            grid[y][x] = (ch, if highlighted { bright } else { muted });
        }
    }
    // Boxes go over the arrows.
    let set = border_set();
    for (i, node) in graph.nodes.iter().enumerate() {
        let (x0, y0) = (usize::from(layout.pos[i].0), usize::from(layout.pos[i].1));
        let is_selected = i == selected;
        let border = if is_selected {
            Style::default().fg(theme().accent).add_modifier(Modifier::BOLD)
        } else if i == 0 {
            Style::default().fg(theme().warm)
        } else if node.openable {
            Style::default().fg(theme().border)
        } else {
            muted
        };
        let inner_w = usize::from(BOX_W) - 2;
        let fit = |text: &str, room: usize| -> String {
            let n = text.chars().count();
            if n <= room { text.to_string() } else { format!("{}…", text.chars().take(room.saturating_sub(1)).collect::<String>()) }
        };
        let put = |grid: &mut Vec<Vec<(char, Style)>>, x: usize, y: usize, text: &str, style: Style| {
            for (k, ch) in text.chars().enumerate() {
                if y < h && x + k < w {
                    grid[y][x + k] = (ch, style);
                }
            }
        };
        let horizontal = |edge: &str| edge.repeat(inner_w);
        put(&mut grid, x0, y0, &format!("{}{}{}", set.top_left, horizontal(set.horizontal_top), set.top_right), border);
        put(&mut grid, x0, y0 + usize::from(BOX_H) - 1, &format!("{}{}{}", set.bottom_left, horizontal(set.horizontal_bottom), set.bottom_right), border);
        for row in 1..usize::from(BOX_H) - 1 {
            put(&mut grid, x0, y0 + row, &format!("{}{}{}", set.vertical_left, " ".repeat(inner_w), set.vertical_right), border);
        }
        // First line: the kind, and why it is here on the right.
        let kind = fit(&node.kind, inner_w);
        let room = inner_w.saturating_sub(kind.chars().count() + 1);
        let detail = if room >= 4 { fit(&node.detail, room) } else { String::new() };
        let kind_style = if is_selected { Style::default().fg(theme().accent) } else { muted };
        put(&mut grid, x0 + 1, y0 + 1, &kind, kind_style);
        put(&mut grid, x0 + 1 + inner_w - detail.chars().count(), y0 + 1, &detail, muted);
        let name_style = if is_selected { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
        put(&mut grid, x0 + 1, y0 + 2, &fit(&node.name, inner_w), name_style);
    }
    grid
}

/// Draws the diagram into `area`, panned to keep the selected box in view.
pub(super) fn draw_graph(frame: &mut Frame, area: Rect, graph: &Graph, selected: usize) {
    let layout = layout(graph);
    let grid = canvas(graph, &layout, selected);
    let (view_w, view_h) = (area.width, area.height);
    let (sx, sy) = layout.pos.get(selected).copied().unwrap_or((0, 0));
    let scroll = |centre: u16, view: u16, whole: u16| centre.saturating_sub(view / 2).min(whole.saturating_sub(view));
    let (ox, oy) = (scroll(sx + BOX_W / 2, view_w, layout.width), scroll(sy + BOX_H / 2, view_h, layout.height));
    // Small diagrams sit in the middle of the space.
    let (pad_x, pad_y) = (view_w.saturating_sub(layout.width) / 2, view_h.saturating_sub(layout.height) / 2);
    let buffer = frame.buffer_mut();
    for y in 0..view_h.saturating_sub(pad_y) {
        for x in 0..view_w.saturating_sub(pad_x) {
            let (gx, gy) = (usize::from(x + ox), usize::from(y + oy));
            if let Some((ch, style)) = grid.get(gy).and_then(|row| row.get(gx)) {
                if *ch != ' ' {
                    buffer.set_string(area.x + pad_x + x, area.y + pad_y + y, ch.to_string(), *style);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::k8s::relations::GraphNode;

    fn node(kind: &str, layer: i32) -> GraphNode {
        GraphNode { kind: kind.into(), namespace: None, name: kind.to_lowercase(), detail: String::new(), layer, openable: true }
    }

    fn chain() -> Graph {
        // Deployment -> ReplicaSet -> Pod -> ConfigMap, and a Service feeding the Pod.
        Graph { nodes: vec![node("Pod", 0), node("ReplicaSet", -1), node("Deployment", -2), node("Service", -1), node("ConfigMap", 1)], edges: vec![(1, 0), (2, 1), (3, 0), (0, 4)] }
    }

    #[test]
    fn columns_run_left_to_right_by_layer() {
        let g = chain();
        let l = layout(&g);
        assert!(l.pos[2].0 < l.pos[1].0 && l.pos[1].0 < l.pos[0].0 && l.pos[0].0 < l.pos[4].0);
        assert_eq!(l.pos[1].0, l.pos[3].0, "same layer, same column");
        assert_ne!(l.pos[1].1, l.pos[3].1, "stacked, not overlapping");
    }

    #[test]
    fn the_arrows_reach_the_next_box() {
        let g = chain();
        let l = layout(&g);
        let grid = canvas(&g, &l, 0);
        let (x, y) = (usize::from(l.pos[0].0) - 1, usize::from(l.pos[0].1 + BOX_H / 2 - 1));
        assert_eq!(grid[y][x].0, '▶', "an arrow enters the Pod box");
    }

    #[test]
    fn moving_goes_to_the_nearest_box_in_that_direction() {
        let g = chain();
        let l = layout(&g);
        assert!(matches!(neighbor(&g, &l, 0, Move::Left), Some(1) | Some(3)));
        assert_eq!(neighbor(&g, &l, 0, Move::Right), Some(4));
        assert_eq!(neighbor(&g, &l, 1, Move::Left), Some(2));
        assert_eq!(neighbor(&g, &l, 1, Move::Down), Some(3));
        assert_eq!(neighbor(&g, &l, 4, Move::Right), None);
    }
}
