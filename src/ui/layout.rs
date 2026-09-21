//! One place that decides how wide every table column is, so every view
//! spaces its columns the same way: each column is as wide as its
//! content (or its header, with room reserved for the sort number and
//! arrow so entering sort mode never shifts anything), separated by one
//! fixed gap, packed to the left — nothing stretches across the screen.
//!
//! Every column also has a minimum width (configurable, see
//! `config::TablesConfig`). Wide columns give up width down to their
//! minimum when space is short; if the minimums still don't all fit, the
//! table scrolls sideways one column at a time (`Window`).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

use super::*;

/// The gap between columns, everywhere.
pub(super) const COLUMN_GAP: u16 = 3;

/// `(n)` before a header plus ` ▲` after it.
const SORT_RESERVE: usize = 5;

/// Room the free-text (`flex`) column keeps at least, unless configured.
const FLEX_MIN: usize = 20;

struct ColumnMins {
    default: usize,
    per_column: HashMap<String, usize>,
}

static COLUMN_MINS: OnceLock<ColumnMins> = OnceLock::new();

/// Sets the column minimums from the config, once at startup. Unset (as in
/// tests), every column's minimum is 10.
pub fn configure_columns(default: usize, per_column: HashMap<String, usize>) {
    let per_column = per_column.into_iter().map(|(name, width)| (name.to_lowercase(), width)).collect();
    let _ = COLUMN_MINS.set(ColumnMins { default, per_column });
}

fn configured_min(header: &str) -> usize {
    match COLUMN_MINS.get() {
        Some(mins) => mins.per_column.get(&header.to_lowercase()).copied().unwrap_or(mins.default),
        None => 10,
    }
}

/// Every column's width, decided once for a table.
pub(super) struct Fitted {
    widths: Vec<usize>,
    /// The free-text column that takes all leftover room, if there is one
    /// and everything fits.
    flex: Option<usize>,
    /// Whether the columns fit the screen at their minimums.
    scrolls: bool,
}

/// The slice of columns currently on screen.
pub(super) struct Window {
    range: Range<usize>,
    /// The offset actually used (the requested one, clamped).
    pub(super) offset: usize,
    pub(super) constraints: Vec<Constraint>,
    pub(super) can_left: bool,
    pub(super) can_right: bool,
}

impl Window {
    /// The visible range of a full row of `len` cells.
    pub(super) fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Keeps only the visible columns of a full row.
    pub(super) fn slice<T>(&self, mut cells: Vec<T>) -> Vec<T> {
        cells.truncate(self.range.end);
        cells.drain(..self.range.start);
        cells
    }
}

impl Fitted {
    /// `rows` yields each row's text width per column. `flex` names the one
    /// column allowed to take the leftover room (an event message).
    pub(super) fn new(headers: &[&str], rows: impl Iterator<Item = Vec<usize>>, available: u16, flex: Option<usize>) -> Self {
        let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count() + SORT_RESERVE).collect();
        for row in rows {
            for (width, cell) in widths.iter_mut().zip(row) {
                *width = (*width).max(cell);
            }
        }
        // A column is never asked to be wider than it needs to be.
        let mins: Vec<usize> = headers.iter().zip(&widths).map(|(h, natural)| configured_min(h).min(*natural)).collect();
        if let Some(f) = flex {
            widths[f] = mins[f].max(FLEX_MIN.min(widths[f]));
        }

        let gaps = usize::from(COLUMN_GAP) * headers.len().saturating_sub(1);
        let available = usize::from(available);
        // Narrow the widest columns, one cell at a time, but not below
        // their minimums.
        while widths.iter().sum::<usize>() + gaps > available {
            let widest = (0..widths.len()).filter(|i| Some(*i) != flex && widths[*i] > mins[*i]).max_by_key(|i| widths[*i]);
            match widest {
                Some(i) => widths[i] -= 1,
                None => break,
            }
        }
        let scrolls = widths.iter().sum::<usize>() + gaps > available;
        Fitted { widths, flex: flex.filter(|_| !scrolls), scrolls }
    }

    /// The columns to show when scrolled `offset` columns to the right.
    pub(super) fn window(&self, offset: usize, available: u16) -> Window {
        let n = self.widths.len();
        let gap = usize::from(COLUMN_GAP);
        let available = usize::from(available);
        let span = |r: Range<usize>| -> usize { r.clone().map(|i| self.widths[i]).sum::<usize>() + gap * r.len().saturating_sub(1) };

        let (offset, end) = if !self.scrolls {
            (0, n)
        } else {
            // The furthest right you can scroll: the last screenful.
            let max_offset = (0..n).find(|&start| span(start..n) <= available).unwrap_or(n - 1);
            let offset = offset.min(max_offset);
            let mut end = offset + 1;
            while end < n && span(offset..end + 1) <= available {
                end += 1;
            }
            (offset, end)
        };
        let constraints = (offset..end)
            .map(|i| if Some(i) == self.flex { Constraint::Fill(1) } else { Constraint::Length(self.widths[i] as u16) })
            .collect();
        Window { range: offset..end, offset, constraints, can_left: offset > 0, can_right: end < n }
    }
}

/// Width of a cell's text, in terminal cells.
pub(super) fn cell_width(text: &str) -> usize {
    text.chars().count()
}

/// The one call every table makes: fit the columns to the content, then
/// pick the visible window (updating `hscroll` to what's actually usable).
pub(super) fn layout_table(
    headers: &[&str],
    rows: impl Iterator<Item = Vec<usize>>,
    available: u16,
    flex: Option<usize>,
    hscroll: &mut usize,
) -> Window {
    let window = Fitted::new(headers, rows, available, flex).window(*hscroll, available);
    *hscroll = window.offset;
    window
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lengths(w: &Window) -> Vec<u16> {
        w.constraints.iter().map(|c| if let Constraint::Length(n) = c { *n } else { 0 }).collect()
    }

    #[test]
    fn columns_are_as_wide_as_their_widest_cell_or_header() {
        // Header "NAME" reserves 4 + 5 = 9; the widest name is 12.
        let fit = Fitted::new(&["NAME", "AGE"], vec![vec![12, 2], vec![7, 2]].into_iter(), 200, None);
        assert_eq!(lengths(&fit.window(0, 200)), [12, 8], "AGE reserves 3 + 5 = 8 even though cells are 2");
    }

    #[test]
    fn nothing_stretches_to_fill_the_width() {
        let fit = Fitted::new(&["A", "B"], std::iter::empty(), 500, None);
        assert_eq!(lengths(&fit.window(0, 500)), [6, 6]);
    }

    #[test]
    fn the_flex_column_takes_the_rest_when_everything_fits() {
        let fit = Fitted::new(&["TYPE", "MESSAGE"], vec![vec![7, 300]].into_iter(), 100, Some(1));
        let w = fit.window(0, 100);
        assert!(matches!(w.constraints[1], Constraint::Fill(1)));
        assert!(!w.can_right);
    }

    #[test]
    fn the_widest_columns_shrink_toward_their_minimums_before_scrolling() {
        // 3 columns, 2 gaps of 3: 30 + 10 + 10 + 6 = 56 wanted, 40 available. The
        // 30-wide column narrows (minimum 10 here); no scrolling needed.
        let fit = Fitted::new(&["A", "B", "C"], vec![vec![30, 10, 10]].into_iter(), 40, None);
        let w = fit.window(0, 40);
        assert!(!w.can_right && !w.can_left);
        let l = lengths(&w);
        assert!(l.iter().map(|x| *x as usize).sum::<usize>() + 6 <= 40, "{l:?}");
        assert!(l[0] >= 10);
    }

    #[test]
    fn when_minimums_do_not_fit_the_table_scrolls_a_column_at_a_time() {
        // Four columns of 10 (their minimum) + 3 gaps of 3 = 49; only 30 available.
        let fit = Fitted::new(&["A", "B", "C", "D"], vec![vec![10; 4]].into_iter(), 30, None);
        let first = fit.window(0, 30);
        assert_eq!(first.range(), 0..2);
        assert!(first.can_right && !first.can_left);
        let last = fit.window(99, 30);
        assert!(last.can_left && !last.can_right, "scrolling past the end clamps to the last screenful");
        assert_eq!(last.offset, last.range().start);
        assert_eq!(last.range().end, 4);
    }

    #[test]
    fn slice_keeps_only_the_visible_cells() {
        let fit = Fitted::new(&["A", "B", "C", "D"], vec![vec![10; 4]].into_iter(), 30, None);
        let w = fit.window(1, 30);
        let cells = vec!["a", "b", "c", "d"];
        assert_eq!(w.slice(cells), ["b", "c"]);
    }
}
