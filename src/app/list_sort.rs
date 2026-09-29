//! Sort mode for a popup table: `s`, then a column number.

use crossterm::event::KeyCode;

use crate::k8s::sort::SortSpec;
use crate::ui;

/// Sort state for a popup table: the column and direction, and whether sort mode is on.
/// The main lists keep theirs in `State`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ListSort {
    pub(crate) spec: Option<SortSpec>,
    pub(crate) choosing: bool,
}

impl ListSort {
    pub(crate) fn view(self) -> ui::SortState {
        ui::SortState { column: self.spec.map(|s| s.column), descending: self.spec.is_some_and(|s| s.descending), choosing: self.choosing, cursor: None }
    }

    /// Feeds it a key; `true` if it was a sort key (`s` to enter, digits, then `s`, `q`
    /// or Esc to leave). `typing` means a text field has focus.
    pub(crate) fn handle(&mut self, code: KeyCode, columns: usize, typing: bool) -> bool {
        if typing || columns == 0 {
            return false;
        }
        if !self.choosing {
            if code == KeyCode::Char('s') {
                self.choosing = true;
                return true;
            }
            return false;
        }
        match code {
            KeyCode::Char(c @ '0'..='9') => {
                // The digits are columns 0-9.
                let column = c as usize - '0' as usize;
                if column < columns {
                    self.spec = Some(SortSpec::pressed(self.spec, column));
                }
                true
            }
            KeyCode::Char('s' | 'q') | KeyCode::Esc => {
                self.choosing = false;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_sort_mode_enters_with_s_cycles_digits_and_leaves() {
        let mut sort = ListSort::default();
        assert!(!sort.handle(KeyCode::Char('j'), 4, false), "other keys are not ours outside the mode");
        assert!(sort.handle(KeyCode::Char('s'), 4, false));
        assert!(sort.choosing);
        assert!(sort.handle(KeyCode::Char('2'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: false }));
        assert!(sort.handle(KeyCode::Char('2'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }));
        // Past the last column: consumed, but nothing changes.
        assert!(sort.handle(KeyCode::Char('9'), 4, false));
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }));
        assert!(sort.choosing);
        assert!(sort.handle(KeyCode::Esc, 4, false));
        assert!(!sort.choosing);
        assert_eq!(sort.spec, Some(SortSpec { column: 2, descending: true }), "leaving keeps the sort");
    }

    #[test]
    fn popup_sort_ignores_keys_while_typing() {
        let mut sort = ListSort::default();
        assert!(!sort.handle(KeyCode::Char('s'), 4, true));
        assert!(!sort.choosing);
    }
}
