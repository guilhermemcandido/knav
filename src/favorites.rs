//! Namespaces reserved to the number keys 1-9 (0 is always "all"), saved
//! per kubeconfig context so they survive restarts and context switches.
//! Stored one per line as `context<TAB>slot<TAB>namespace` in
//! `<config dir>/namespaces`.

use std::path::PathBuf;

use super::*;

pub(crate) const SLOTS: usize = 9;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Toggled {
    Pinned(usize),
    Unpinned(usize),
    Full,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Favorites {
    /// Index 0 is key `1`.
    pub(crate) slots: Vec<Option<String>>,
}

impl Favorites {
    pub(crate) fn empty() -> Self {
        Favorites { slots: vec![None; SLOTS] }
    }

    /// The namespace on number key `key` (1-9).
    pub(crate) fn get(&self, key: usize) -> Option<&str> {
        self.slots.get(key.checked_sub(1)?)?.as_deref()
    }

    /// Pins `namespace` to the first free slot, or unpins it if it's
    /// already pinned.
    pub(crate) fn toggle(&mut self, namespace: &str) -> Toggled {
        if let Some(i) = self.slots.iter().position(|s| s.as_deref() == Some(namespace)) {
            self.slots[i] = None;
            return Toggled::Unpinned(i + 1);
        }
        match self.slots.iter().position(Option::is_none) {
            Some(i) => {
                self.slots[i] = Some(namespace.to_string());
                Toggled::Pinned(i + 1)
            }
            None => Toggled::Full,
        }
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

    /// Rewrites this context's lines, leaving every other context's alone.
    /// Best-effort: failing to save a convenience isn't worth an error.
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
        let _ = std::fs::write(path, out.join("\n") + "\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_pins_into_the_first_free_slot_and_unpins() {
        let mut f = Favorites::empty();
        assert_eq!(f.toggle("default"), Toggled::Pinned(1));
        assert_eq!(f.toggle("kube-system"), Toggled::Pinned(2));
        assert_eq!(f.get(2), Some("kube-system"));
        assert_eq!(f.toggle("default"), Toggled::Unpinned(1));
        assert_eq!(f.get(1), None);
        // The freed slot is reused.
        assert_eq!(f.toggle("prod"), Toggled::Pinned(1));
    }

    #[test]
    fn full_when_all_nine_are_taken() {
        let mut f = Favorites::empty();
        for i in 0..SLOTS {
            f.toggle(&format!("ns{i}"));
        }
        assert_eq!(f.toggle("one-more"), Toggled::Full);
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
