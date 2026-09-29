//! Namespaces on number keys 1-9 (0 is always all), saved per context in
//! `<config dir>/namespaces` as `context<TAB>slot<TAB>namespace` lines.

use crate::config::Config;

use std::path::PathBuf;


pub(crate) const SLOTS: usize = 9;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Favorites {
    /// Index 0 is key `1`.
    pub(crate) slots: Vec<Option<String>>,
}

impl Favorites {
    pub(crate) fn empty() -> Self {
        Favorites { slots: vec![None; SLOTS] }
    }

    pub(crate) fn get(&self, key: usize) -> Option<&str> {
        self.slots.get(key.checked_sub(1)?)?.as_deref()
    }

    /// Puts `namespace` on key `key` (1-9), moving it off any key it had. Returns
    /// the namespace that was on `key`.
    pub(crate) fn assign(&mut self, key: usize, namespace: &str) -> Option<String> {
        let index = key.checked_sub(1).filter(|i| *i < SLOTS)?;
        for slot in &mut self.slots {
            if slot.as_deref() == Some(namespace) {
                *slot = None;
            }
        }
        self.slots[index].replace(namespace.to_string())
    }

    pub(crate) fn clear(&mut self, key: usize) {
        if let Some(slot) = key.checked_sub(1).and_then(|i| self.slots.get_mut(i)) {
            *slot = None;
        }
    }

    /// The key (1-9) a namespace is on, if any.
    pub(crate) fn key_of(&self, namespace: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.as_deref() == Some(namespace)).map(|i| i + 1)
    }

    fn path() -> PathBuf {
        Config::dir().join("namespaces")
    }

    pub(crate) fn load(context: &str) -> Self {
        Self::parse(&std::fs::read_to_string(Self::path()).unwrap_or_default(), context)
    }

    fn parse(text: &str, context: &str) -> Self {
        let mut favorites = Self::empty();
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            if let (Some(ctx), Some(slot), Some(ns)) = (parts.next(), parts.next(), parts.next())
                && ctx == context
                && let Ok(slot) = slot.parse::<usize>()
                && (1..=SLOTS).contains(&slot)
            {
                favorites.slots[slot - 1] = Some(ns.to_string());
            }
        }
        favorites
    }

    /// Rewrites this context's lines, keeping every other context's. Best effort:
    /// a failed save isn't worth an error.
    pub(crate) fn save(&self, context: &str) {
        let path = Self::path();
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let mut out: Vec<String> = existing.lines().filter(|l| l.split('\t').next() != Some(context)).map(String::from).collect();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(ns) = slot {
                out.push(format!("{context}\t{}\t{ns}", i + 1));
            }
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Written beside the file, then renamed over it, so a crash never leaves half a file.
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
    fn assign_puts_a_namespace_on_the_chosen_key() {
        let mut f = Favorites::empty();
        assert_eq!(f.assign(3, "default"), None);
        assert_eq!(f.get(3), Some("default"));
        assert_eq!(f.key_of("default"), Some(3));
    }

    #[test]
    fn assigning_replaces_the_occupant_and_moves_the_namespace() {
        let mut f = Favorites::empty();
        f.assign(1, "default");
        f.assign(2, "kube-system");
        // kube-system moves from 2 to 1, bumping default off.
        assert_eq!(f.assign(1, "kube-system"), Some("default".to_string()));
        assert_eq!(f.get(1), Some("kube-system"));
        assert_eq!(f.get(2), None);
        assert_eq!(f.key_of("default"), None);
    }

    #[test]
    fn clear_frees_a_key_and_out_of_range_keys_do_nothing() {
        let mut f = Favorites::empty();
        f.assign(4, "prod");
        f.clear(4);
        assert_eq!(f.get(4), None);
        assert_eq!(f.assign(0, "x"), None);
        assert_eq!(f.assign(10, "x"), None);
        assert_eq!(f.key_of("x"), None);
        f.clear(0);
        f.clear(10);
    }

    #[test]
    fn get_ignores_out_of_range_keys() {
        let f = Favorites::empty();
        assert_eq!(f.get(0), None);
        assert_eq!(f.get(10), None);
    }

    #[test]
    fn parse_reads_only_the_matching_context() {
        let text = "a\t1\tdefault\nb\t1\tother\na\t3\tkube-system\n";
        let f = Favorites::parse(text, "a");
        assert_eq!(f.get(1), Some("default"));
        assert_eq!(f.get(2), None);
        assert_eq!(f.get(3), Some("kube-system"));
    }
}
