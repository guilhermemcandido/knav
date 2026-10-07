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
        for line in text.lines() {
            match line.split_once('\t') {
                Some(("kind", label)) if !pinned.labels.iter().any(|l| l == label) => pinned.labels.push(label.to_string()),
                Some(("view", "all")) => pinned.show_all = true,
                _ => {}
            }
        }
        pinned
    }

    /// The labels Home shows instead of the categories, if it is showing favorites.
    pub(crate) fn home(&self) -> Option<&[String]> {
        (!self.show_all && !self.labels.is_empty()).then_some(self.labels.as_slice())
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
        if self.show_all {
            out.push("view\tall".into());
        }
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
        assert_eq!(pinned.home(), None);
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
    }
}
