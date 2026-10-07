//! The kinds pinned to Home, and whether Home shows only those, saved in
//! `<config dir>/pinned-kinds` as `kind<TAB>Label` lines and one `view<TAB>all` line.

use crate::config::Config;

use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PinnedKinds {
    /// Tile labels, in the order they were pinned.
    pub(crate) labels: Vec<String>,
    /// Home shows every category, not just the pinned kinds.
    pub(crate) show_all: bool,
}

impl PinnedKinds {
    pub(crate) fn load() -> Self {
        Self::parse(&std::fs::read_to_string(Self::path()).unwrap_or_default())
    }

    fn path() -> PathBuf {
        Config::dir().join("pinned-kinds")
    }

    fn parse(text: &str) -> Self {
        let mut pinned = PinnedKinds::default();
        let mut view = None;
        for line in text.lines() {
            match line.split_once('\t') {
                Some(("kind", label)) if !pinned.labels.iter().any(|l| l == label) => pinned.labels.push(label.to_string()),
                Some(("view", which)) => view = Some(which == "all"),
                _ => {}
            }
        }
        // With no choice saved, Home shows every kind until something is pinned.
        pinned.show_all = view.unwrap_or(pinned.labels.is_empty());
        pinned
    }

    /// The labels Home shows instead of the categories, if it is showing favorites.
    pub(crate) fn home(&self) -> Option<&[String]> {
        (!self.show_all).then_some(self.labels.as_slice())
    }

    /// Moves a pinned kind one place earlier (`back`) or later. True when it moved.
    pub(crate) fn nudge(&mut self, label: &str, back: bool) -> bool {
        let Some(at) = self.labels.iter().position(|l| l == label) else { return false };
        let to = if back { at.checked_sub(1) } else { Some(at + 1).filter(|i| *i < self.labels.len()) };
        let Some(to) = to else { return false };
        self.labels.swap(at, to);
        self.save();
        true
    }

    pub(crate) fn contains(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }

    /// Pins `label`, or unpins it if it was. True when it is pinned now.
    pub(crate) fn toggle(&mut self, label: &str) -> bool {
        let pinned = !self.contains(label);
        if pinned {
            self.labels.push(label.to_string());
        } else {
            self.labels.retain(|l| l != label);
        }
        self.save();
        pinned
    }

    pub(crate) fn toggle_view(&mut self) {
        self.show_all = !self.show_all;
        self.save();
    }

    /// Best effort, like the namespace keys.
    fn save(&self) {
        let path = Self::path();
        let mut out: Vec<String> = self.labels.iter().map(|l| format!("kind\t{l}")).collect();
        out.push(format!("view\t{}", if self.show_all { "all" } else { "favorites" }));
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, out.join("\n") + "\n").is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_shows_favorites_only_when_there_are_some() {
        let mut pinned = PinnedKinds::default();
        assert_eq!(pinned.home(), Some(&[][..]));
        pinned.labels.push("Pods".into());
        assert_eq!(pinned.home(), Some(&["Pods".to_string()][..]));
        pinned.show_all = true;
        assert_eq!(pinned.home(), None);
    }

    #[test]
    fn parses_what_it_saves() {
        let pinned = PinnedKinds::parse("kind\tPods\nkind\tPods\nkind\tNodes\nview\tall\n");
        assert_eq!(pinned.labels, ["Pods", "Nodes"]);
        assert!(pinned.show_all);
        assert!(PinnedKinds::parse("").show_all, "nothing pinned yet: every kind");
        assert!(!PinnedKinds::parse("kind\tPods\n").show_all);
    }

    #[test]
    fn nudging_swaps_neighbours_and_stops_at_the_ends() {
        let mut pinned = PinnedKinds { labels: vec!["A".into(), "B".into(), "C".into()], show_all: false };
        assert!(!pinned.nudge("A", true));
        assert!(pinned.nudge("A", false));
        assert_eq!(pinned.labels, ["B", "A", "C"]);
        assert!(!pinned.nudge("C", false));
    }
}
