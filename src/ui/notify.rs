//! The last notifications knav showed (toasts and notices), kept in full for the `N` page, so a
//! short message on screen never loses its detail.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many are kept.
const KEPT: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Level {
    Done,
    Info,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Local time, `14:03:27`.
    pub time: String,
    pub level: Level,
    pub text: String,
}

static LOG: Mutex<VecDeque<Entry>> = Mutex::new(VecDeque::new());

/// Remembers a notification. The same text twice in a row is kept once.
pub fn record(level: Level, text: &str) {
    let Ok(mut log) = LOG.lock() else { return };
    if log.back().is_some_and(|last| last.text == text) {
        return;
    }
    let time = k8s_openapi::jiff::Zoned::now().strftime("%H:%M:%S").to_string();
    log.push_back(Entry { time, level, text: text.to_string() });
    while log.len() > KEPT {
        log.pop_front();
    }
}

/// Everything kept, newest first.
pub fn all() -> Vec<Entry> {
    LOG.lock().map(|log| log.iter().rev().cloned().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_and_repeats_are_kept_once() {
        record(Level::Failed, "pods is forbidden: cannot list");
        record(Level::Failed, "pods is forbidden: cannot list");
        record(Level::Done, "Deleted pod a");
        let seen = all();
        assert_eq!(seen[0].text, "Deleted pod a");
        assert_eq!(seen.iter().filter(|e| e.text.starts_with("pods is forbidden")).count(), 1);
    }
}
