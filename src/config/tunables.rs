//! The numeric knobs that code deep in the app reads (wheel speed, how fast
//! a double-click is, how often the API lists refresh, ...). Held in one
//! place that the config screen can change while knav runs.

use std::sync::RwLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tunables {
    /// Rows the selection moves per mouse-wheel notch.
    pub wheel_rows: usize,
    /// Two clicks on a row within this many milliseconds open it.
    pub double_click_ms: u64,
    /// How much of its square a suggestion's icon fills, in percent.
    pub suggestion_icon_percent: u8,
    /// Milliseconds between redraws while idle, and while a shell is open.
    pub idle_redraw_ms: u64,
    pub shell_redraw_ms: u64,
}

impl Default for Tunables {
    fn default() -> Self {
        Tunables { wheel_rows: 3, double_click_ms: 400, suggestion_icon_percent: 78, idle_redraw_ms: 200, shell_redraw_ms: 25 }
    }
}

static CURRENT: RwLock<Option<Tunables>> = RwLock::new(None);

pub fn tunables() -> Tunables {
    CURRENT.read().ok().and_then(|t| *t).unwrap_or_default()
}

pub fn set_tunables(tunables: Tunables) {
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(tunables);
    }
}
