//! Namespaces on number keys 1-9 (0 is always all), saved per context in `config.toml`
//! as `[namespaces]` entries like `"my-context" = { 1 = "default" }`.

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

    /// The file the keys used to live in, before they moved into `config.toml`.
    fn legacy_path() -> PathBuf {
        Config::dir().join("namespaces")
    }

    pub(crate) fn load(context: &str) -> Self {
        Self::import_legacy();
        let text = std::fs::read_to_string(Config::path()).unwrap_or_default();
        Self::from_toml(&text, context)
    }

    /// This context's keys from `[namespaces."<context>"]`, like `1 = "default"`.
    fn from_toml(text: &str, context: &str) -> Self {
        let mut favorites = Self::empty();
        let table = text.parse::<toml::Table>().ok().and_then(|t| t.get("namespaces")?.get(context)?.as_table().cloned());
        for (slot, namespace) in table.into_iter().flatten() {
            if let (Ok(slot), Some(namespace)) = (slot.parse::<usize>(), namespace.as_str())
                && (1..=SLOTS).contains(&slot)
            {
                favorites.slots[slot - 1] = Some(namespace.to_string());
            }
        }
        favorites
    }

    /// Moves the old `namespaces` file into `config.toml` once, keeping what is already
    /// there, and leaves the file as `namespaces.old`.
    fn import_legacy() {
        let legacy = Self::legacy_path();
        let Ok(text) = std::fs::read_to_string(&legacy) else { return };
        let mut by_context: std::collections::BTreeMap<String, Self> = std::collections::BTreeMap::new();
        for line in text.lines() {
            let mut parts = line.splitn(3, '\t');
            if let (Some(context), Some(slot), Some(namespace)) = (parts.next(), parts.next(), parts.next())
                && let Ok(slot) = slot.parse::<usize>()
                && (1..=SLOTS).contains(&slot)
            {
                by_context.entry(context.to_string()).or_insert_with(Self::empty).slots[slot - 1] = Some(namespace.to_string());
            }
        }
        let config = std::fs::read_to_string(Config::path()).unwrap_or_default();
        for (context, favorites) in by_context {
            if Self::from_toml(&config, &context) == Self::empty() {
                favorites.save(&context);
            }
        }
        let _ = std::fs::rename(&legacy, legacy.with_extension("old"));
    }

    /// Writes this context's keys to `config.toml`, keeping the rest of the file. Best
    /// effort: a failed save isn't worth an error.
    pub(crate) fn save(&self, context: &str) {
        let mut table = toml_edit::InlineTable::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if let Some(namespace) = slot {
                table.insert((i + 1).to_string(), toml_edit::Value::from(namespace.as_str()));
            }
        }
        let value = (!table.is_empty()).then(|| toml_edit::Value::InlineTable(table));
        let _ = crate::config::edit::save_keys(&Config::path(), &["namespaces", context], value);
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
    fn reads_only_the_matching_context() {
        let text = "[namespaces]\n\"a.b\" = { 1 = \"default\", 3 = \"kube-system\" }\nb = { 1 = \"other\" }\n";
        let f = Favorites::from_toml(text, "a.b");
        assert_eq!(f.get(1), Some("default"));
        assert_eq!(f.get(2), None);
        assert_eq!(f.get(3), Some("kube-system"));
    }
}
