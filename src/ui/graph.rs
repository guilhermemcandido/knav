//! The relations diagram: boxes in columns, joined by arrows that flow left to
//! right, like a Mermaid `flowchart LR`.

use super::*;
use crate::k8s::relations::Graph;

/// Box width, height, the gap between columns (for the arrows) and the gap between
/// boxes stacked in a column, at each zoom level: compact, normal (the default), large.
#[derive(Clone, Copy)]
struct Dims {
    w: u16,
    h: u16,
    gap: u16,
    row_gap: u16,
}

const LEVELS: [Dims; 3] = [Dims { w: 18, h: 3, gap: 4, row_gap: 0 }, Dims { w: 26, h: 4, gap: 7, row_gap: 1 }, Dims { w: 34, h: 5, gap: 9, row_gap: 1 }];

pub const DEFAULT_ZOOM: usize = 1;
const MAX_ZOOM: usize = LEVELS.len() - 1;

fn dims(zoom: usize) -> Dims {
    LEVELS[zoom.min(MAX_ZOOM)]
}

/// One step closer, up to the largest boxes.
pub fn zoom_in(zoom: usize) -> usize {
    (zoom + 1).min(MAX_ZOOM)
}

/// One step further out, down to the most compact boxes.
pub fn zoom_out(zoom: usize) -> usize {
    zoom.saturating_sub(1)
}

/// Where each box sits on the canvas, and how big the canvas is.
pub struct GraphLayout {
    pub pos: Vec<(u16, u16)>,
    pub width: u16,
    pub height: u16,
}

/// Most boxes stacked in one column, so the canvas size stays in range.
const MAX_PER_LAYER: usize = 1000;

pub fn layout(graph: &Graph, zoom: usize) -> GraphLayout {
    let d = dims(zoom);
    let layers: Vec<i32> = {
        let mut l: Vec<i32> = graph.nodes.iter().map(|n| n.layer).collect();
        l.sort_unstable();
        l.dedup();
        l
    };
    let column_of = |layer: i32| layers.iter().position(|l| *l == layer).unwrap_or(0) as u16;
    let members = |layer: i32| graph.nodes.iter().enumerate().filter(move |(_, n)| n.layer == layer).map(|(i, _)| i);
    let height = layers.iter().map(|l| (members(*l).count().min(MAX_PER_LAYER) as u16) * (d.h + d.row_gap)).max().unwrap_or(d.h).saturating_sub(d.row_gap).max(d.h);
    let mut pos = vec![(0, 0); graph.nodes.len()];
    for layer in &layers {
        let indices: Vec<usize> = members(*layer).collect();
        let column_height = ((indices.len().min(MAX_PER_LAYER) as u16) * (d.h + d.row_gap)).saturating_sub(d.row_gap);
        let top = height.saturating_sub(column_height) / 2;
        for (row, i) in indices.into_iter().enumerate() {
            pos[i] = (column_of(*layer) * (d.w + d.gap), top + (row.min(MAX_PER_LAYER) as u16) * (d.h + d.row_gap));
        }
    }
    GraphLayout { pos, width: (layers.len() as u16 * (d.w + d.gap)).saturating_sub(d.gap), height }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
}

/// The box to go to from `from`: the nearest one in that direction.
pub fn neighbor(graph: &Graph, layout: &GraphLayout, from: usize, direction: Move, zoom: usize) -> Option<usize> {
    let here = &graph.nodes[from];
    let box_h = dims(zoom).h;
    let centre = |i: usize| i32::from(layout.pos[i].1) + i32::from(box_h) / 2;
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
fn canvas(graph: &Graph, layout: &GraphLayout, selected: usize, zoom: usize) -> Vec<Vec<(char, Style)>> {
    let d = dims(zoom);
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
        let (ya, yb) = (usize::from(pa.1 + d.h / 2 - 1), usize::from(pb.1 + d.h / 2 - 1));
        let highlighted = a == selected || b == selected;
        if rightwards {
            let start = usize::from(pa.0 + d.w);
            let trunk = start + usize::from(d.gap / 2);
            let end = usize::from(pb.0).saturating_sub(1);
            masks[ya.min(h - 1)][start.min(w - 1)] |= LEFT;
            join(&mut masks, &mut lit, (start, ya), (trunk.min(end), ya), highlighted);
            join(&mut masks, &mut lit, (trunk.min(end), ya), (trunk.min(end), yb), highlighted);
            join(&mut masks, &mut lit, (trunk.min(end), yb), (end, yb), highlighted);
            arrows.push((end, yb, '▶', highlighted));
        } else {
            let start = usize::from(pa.0).saturating_sub(1);
            let end = usize::from(pb.0 + d.w);
            let trunk = start.saturating_sub(usize::from(d.gap / 2)).max(end);
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
        } else {
            Style::default().fg(theme().border)
        };
        let inner_w = usize::from(d.w) - 2;
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
        put(&mut grid, x0, y0 + usize::from(d.h) - 1, &format!("{}{}{}", set.bottom_left, horizontal(set.horizontal_bottom), set.bottom_right), border);
        for row in 1..usize::from(d.h) - 1 {
            put(&mut grid, x0, y0 + row, &format!("{}{}{}", set.vertical_left, " ".repeat(inner_w), set.vertical_right), border);
        }
        let name_style = if is_selected { Style::default().fg(theme().accent).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
        // The kind, and why it is here on the right; shared by the two multi-line sizes.
        let kind_row = |grid: &mut Vec<Vec<(char, Style)>>, y: usize| {
            let kind = fit(&node.kind, inner_w);
            let room = inner_w.saturating_sub(kind.chars().count() + 1);
            let detail = if room >= 4 { fit(&node.detail, room) } else { String::new() };
            let kind_style = if is_selected { Style::default().fg(theme().accent) } else { muted };
            put(grid, x0 + 1, y, &kind, kind_style);
            put(grid, x0 + 1 + inner_w - detail.chars().count(), y, &detail, muted);
        };
        if d.h >= 5 {
            // Zoomed in: the kind, then the namespace, then the name, each on its own
            // line instead of getting truncated together.
            kind_row(&mut grid, y0 + 1);
            if let Some(namespace) = &node.namespace {
                put(&mut grid, x0 + 1, y0 + 2, &fit(namespace, inner_w), muted);
            }
            put(&mut grid, x0 + 1, y0 + 3, &fit(&node.name, inner_w), name_style);
        } else if d.h >= 4 {
            // First line: the kind. Second: the name.
            kind_row(&mut grid, y0 + 1);
            put(&mut grid, x0 + 1, y0 + 2, &fit(&node.name, inner_w), name_style);
        } else {
            // Zoomed out: one line only, the kind's first letter ahead of the name.
            let prefix = node.kind.chars().next().map(|c| format!("{c} ")).unwrap_or_default();
            put(&mut grid, x0 + 1, y0 + 1, &fit(&format!("{prefix}{}", node.name), inner_w), name_style);
        }
    }
    grid
}

/// Where the diagram sits in `area`: how far it is panned (to keep the selected box in view)
/// and how much room is left around a small one, which sits in the middle.
fn viewport(area: Rect, layout: &GraphLayout, selected: usize, zoom: usize) -> (u16, u16, u16, u16) {
    let d = dims(zoom);
    let (sx, sy) = layout.pos.get(selected).copied().unwrap_or((0, 0));
    let scroll = |centre: u16, view: u16, whole: u16| centre.saturating_sub(view / 2).min(whole.saturating_sub(view));
    let (ox, oy) = (scroll(sx + d.w / 2, area.width, layout.width), scroll(sy + d.h / 2, area.height, layout.height));
    (ox, oy, area.width.saturating_sub(layout.width) / 2, area.height.saturating_sub(layout.height) / 2)
}

/// The box under a screen position, given the diagram is drawn in `area` with `selected` in view.
pub fn graph_hit(area: Rect, graph: &Graph, selected: usize, column: u16, row: u16, zoom: usize) -> Option<usize> {
    let d = dims(zoom);
    let layout = layout(graph, zoom);
    let (ox, oy, pad_x, pad_y) = viewport(area, &layout, selected, zoom);
    let x = column.checked_sub(area.x + pad_x)?.checked_add(ox)?;
    let y = row.checked_sub(area.y + pad_y)?.checked_add(oy)?;
    layout.pos.iter().position(|&(bx, by)| x >= bx && x < bx + d.w && y >= by && y < by + d.h)
}

/// Draws the diagram into `area`, panned to keep the selected box in view.
pub(super) fn draw_graph(frame: &mut Frame, area: Rect, graph: &Graph, selected: usize, zoom: usize) {
    let layout = layout(graph, zoom);
    let grid = canvas(graph, &layout, selected, zoom);
    let (view_w, view_h) = (area.width, area.height);
    let (ox, oy, pad_x, pad_y) = viewport(area, &layout, selected, zoom);
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
        GraphNode { kind: kind.into(), namespace: None, name: kind.to_lowercase(), detail: String::new(), layer }
    }

    fn chain() -> Graph {
        // Deployment -> ReplicaSet -> Pod -> ConfigMap, and a Service feeding the Pod.
        Graph { nodes: vec![node("Pod", 0), node("ReplicaSet", -1), node("Deployment", -2), node("Service", -1), node("ConfigMap", 1)], edges: vec![(1, 0), (2, 1), (3, 0), (0, 4)] }
    }

    #[test]
    fn columns_run_left_to_right_by_layer() {
        let g = chain();
        let l = layout(&g, DEFAULT_ZOOM);
        assert!(l.pos[2].0 < l.pos[1].0 && l.pos[1].0 < l.pos[0].0 && l.pos[0].0 < l.pos[4].0);
        assert_eq!(l.pos[1].0, l.pos[3].0, "same layer, same column");
        assert_ne!(l.pos[1].1, l.pos[3].1, "stacked, not overlapping");
    }

    #[test]
    fn the_arrows_reach_the_next_box() {
        let g = chain();
        let l = layout(&g, DEFAULT_ZOOM);
        let grid = canvas(&g, &l, 0, DEFAULT_ZOOM);
        let (x, y) = (usize::from(l.pos[0].0) - 1, usize::from(l.pos[0].1 + dims(DEFAULT_ZOOM).h / 2 - 1));
        assert_eq!(grid[y][x].0, '▶', "an arrow enters the Pod box");
    }

    #[test]
    fn moving_goes_to_the_nearest_box_in_that_direction() {
        let g = chain();
        let l = layout(&g, DEFAULT_ZOOM);
        assert!(matches!(neighbor(&g, &l, 0, Move::Left, DEFAULT_ZOOM), Some(1) | Some(3)));
        assert_eq!(neighbor(&g, &l, 0, Move::Right, DEFAULT_ZOOM), Some(4));
        assert_eq!(neighbor(&g, &l, 1, Move::Left, DEFAULT_ZOOM), Some(2));
        assert_eq!(neighbor(&g, &l, 1, Move::Down, DEFAULT_ZOOM), Some(3));
        assert_eq!(neighbor(&g, &l, 4, Move::Right, DEFAULT_ZOOM), None);
    }

    #[test]
    fn a_click_lands_on_the_box_under_it_and_not_on_the_gaps() {
        let g = chain();
        let l = layout(&g, DEFAULT_ZOOM);
        let area = Rect { x: 2, y: 3, width: 200, height: 60 };
        let (_, _, pad_x, pad_y) = viewport(area, &l, 0, DEFAULT_ZOOM);
        let (bx, by) = l.pos[4];
        assert_eq!(graph_hit(area, &g, 0, area.x + pad_x + bx + 1, area.y + pad_y + by + 1, DEFAULT_ZOOM), Some(4));
        assert_eq!(graph_hit(area, &g, 0, area.x + pad_x + bx + dims(DEFAULT_ZOOM).w + 1, area.y + pad_y + by, DEFAULT_ZOOM), None, "the gap after the box");
        assert_eq!(graph_hit(area, &g, 0, 0, 0, DEFAULT_ZOOM), None, "outside the diagram");
    }

    #[test]
    fn zoom_stays_within_the_defined_levels() {
        assert_eq!(zoom_out(0), 0, "already the most compact");
        assert_eq!(zoom_in(MAX_ZOOM), MAX_ZOOM, "already the largest");
        assert_eq!(zoom_in(zoom_out(DEFAULT_ZOOM)), DEFAULT_ZOOM, "one step each way is a no-op");
    }

    #[test]
    fn zoomed_out_boxes_show_one_line_with_the_kind_folded_into_the_name() {
        let g = chain();
        let zoom = zoom_out(DEFAULT_ZOOM);
        let l = layout(&g, zoom);
        let grid = canvas(&g, &l, 0, zoom);
        let (x0, y0) = (usize::from(l.pos[0].0), usize::from(l.pos[0].1));
        let row: String = grid[y0 + 1][x0 + 1..x0 + 1 + (usize::from(dims(zoom).w) - 2)].iter().map(|(c, _)| *c).collect();
        assert!(row.trim().starts_with("P pod"), "kind initial ahead of the name, got {row:?}");
    }
}
