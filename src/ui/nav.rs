//! Keyboard movement on the Overview: which tile is selected and where an arrow goes.

use super::*;

/// The Overview selection: Resources, Events, a column header or a column's item.
/// Up into Events remembers the column, so Down returns to it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverviewSelection {
    Resources,
    Events(usize),
    Header(usize),
    Item(usize, usize),
}

pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Moves the Overview selection one step. Up and Down move within a column and on to
/// Events; Left and Right keep the item index, or land on a shorter column's header.
pub fn move_overview_selection(overview: &Overview, selection: OverviewSelection, dir: Direction) -> OverviewSelection {
    let total = overview.catalog.len();
    match selection {
        OverviewSelection::Resources => match dir {
            Direction::Down => OverviewSelection::Events(0),
            _ => selection,
        },
        OverviewSelection::Events(back_to) => match dir {
            Direction::Up => OverviewSelection::Resources,
            Direction::Down => {
                if total == 0 { selection } else { OverviewSelection::Header(back_to.min(total - 1)) }
            }
            _ => selection,
        },
        OverviewSelection::Header(col) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            match dir {
                Direction::Down => {
                    if column_len(overview, col) > 0 { OverviewSelection::Item(col, 0) } else { selection }
                }
                Direction::Up => OverviewSelection::Events(col),
                Direction::Left => if col > 0 { OverviewSelection::Header(col - 1) } else { selection },
                Direction::Right => if col + 1 < total { OverviewSelection::Header(col + 1) } else { selection },
            }
        }
        OverviewSelection::Item(col, item) => {
            if total == 0 {
                return selection;
            }
            let col = col.min(total - 1);
            let len = column_len(overview, col).max(1);
            let item = item.min(len - 1);
            match dir {
                Direction::Up => {
                    if item > 0 { OverviewSelection::Item(col, item - 1) } else { OverviewSelection::Header(col) }
                }
                Direction::Down => {
                    if item + 1 < len { OverviewSelection::Item(col, item + 1) } else { selection }
                }
                Direction::Left => {
                    if col == 0 {
                        selection
                    } else {
                        let target_len = column_len(overview, col - 1);
                        if target_len == 0 { OverviewSelection::Header(col - 1) } else { OverviewSelection::Item(col - 1, item.min(target_len - 1)) }
                    }
                }
                Direction::Right => {
                    if col + 1 >= total {
                        selection
                    } else {
                        let target_len = column_len(overview, col + 1);
                        if target_len == 0 { OverviewSelection::Header(col + 1) } else { OverviewSelection::Item(col + 1, item.min(target_len - 1)) }
                    }
                }
            }
        }
    }
}

/// The next section after `from` that has tiles, for the command menu's grid.
pub(super) fn next_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (from + 1..lens.len()).find(|&i| lens[i] > 0)
}

pub(super) fn prev_nonempty_section(lens: &[usize], from: usize) -> Option<usize> {
    (0..from).rev().find(|&i| lens[i] > 0)
}

pub(super) fn move_selection(section_lens: &[usize], cols: usize, current: (usize, usize), dir: Direction) -> (usize, usize) {
    let section_count = section_lens.len();
    if section_count == 0 {
        return current;
    }
    let section = current.0.min(section_count - 1);
    let len = section_lens[section].max(1);
    let tile = current.1.min(len - 1);
    let cols = cols.max(1);
    let row = tile / cols;
    let col = tile % cols;

    match dir {
        Direction::Left => {
            if col > 0 {
                (section, tile - 1)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => (s, section_lens[s].saturating_sub(1)),
                    None => (section, tile),
                }
            }
        }
        Direction::Right => {
            if tile + 1 < len {
                (section, tile + 1)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, 0),
                    None => (section, tile),
                }
            }
        }
        Direction::Up => {
            if row > 0 {
                (section, (row - 1) * cols + col)
            } else {
                match prev_nonempty_section(section_lens, section) {
                    Some(s) => {
                        let prev_len = section_lens[s];
                        let prev_rows = prev_len.div_ceil(cols).max(1);
                        let target = ((prev_rows - 1) * cols + col).min(prev_len.saturating_sub(1));
                        (s, target)
                    }
                    None => (section, tile),
                }
            }
        }
        Direction::Down => {
            let next = (row + 1) * cols + col;
            if next < len {
                (section, next)
            } else {
                match next_nonempty_section(section_lens, section) {
                    Some(s) => (s, col.min(section_lens[s].saturating_sub(1))),
                    None => (section, tile),
                }
            }
        }
    }
}
