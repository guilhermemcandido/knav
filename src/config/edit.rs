//! Edits `config.toml` in place, keeping its comments and layout.

use std::path::Path;

use anyhow::{Context as _, Result};

use super::Config;

/// `document` with `path` set to `value` (or removed, for `None`), comments
/// and everything else untouched.
pub fn edit_document(document: &str, path: &str, value: Option<toml_edit::Value>) -> Result<String> {
    let mut doc: toml_edit::DocumentMut = document.parse().context("config.toml is not valid TOML")?;
    let keys: Vec<&str> = path.split('.').collect();
    let (last, parents) = keys.split_last().context("empty setting path")?;
    match value {
        Some(value) => {
            let mut table = doc.as_table_mut();
            for key in parents {
                let item = table.entry(key).or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
                table = item.as_table_mut().with_context(|| format!("'{key}' in config.toml is not a table"))?;
            }
            table.insert(last, toml_edit::Item::Value(value));
        }
        None => {
            let mut table = Some(doc.as_table_mut());
            for key in parents {
                table = table.and_then(|t| t.get_mut(key)).and_then(|i| i.as_table_mut());
            }
            if let Some(table) = table {
                table.remove(last);
            }
            // Don't leave empty `[section]` headers behind.
            for depth in (1..=parents.len()).rev() {
                let mut table = Some(doc.as_table_mut());
                for key in &parents[..depth - 1] {
                    table = table.and_then(|t| t.get_mut(key)).and_then(|i| i.as_table_mut());
                }
                if let Some(table) = table
                    && table.get(parents[depth - 1]).and_then(|i| i.as_table()).is_some_and(|t| t.is_empty())
                {
                    table.remove(parents[depth - 1]);
                }
            }
        }
    }
    Ok(doc.to_string())
}

/// Writes `value` (or removes the key) in the config file and returns the
/// config as it now reads. The file is only replaced if the result parses.
pub fn save(file: &Path, path: &str, value: Option<toml_edit::Value>) -> Result<Config> {
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    let updated = edit_document(&existing, path, value)?;
    let mut config: Config = toml::from_str(&updated).context("that would make config.toml invalid")?;
    config.overview.migrate_legacy_names();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(file, updated).with_context(|| format!("writing {}", file.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_keeps_comments_and_other_settings() {
        let original = "# my config\n[logs]\n# how logs read\norder = \"newest_first\"\n";
        let updated = edit_document(original, "ui.border", Some(toml_edit::Value::from("double"))).unwrap();
        assert!(updated.contains("# my config") && updated.contains("# how logs read"));
        assert!(updated.contains("order = \"newest_first\""));
        let config: Config = toml::from_str(&updated).unwrap();
        assert_eq!(config.ui.border, "double");
    }

    #[test]
    fn nested_tables_are_created_and_removals_are_clean() {
        let updated = edit_document("", "theme.colors.ok", Some(toml_edit::Value::from("#00ff00"))).unwrap();
        let config: Config = toml::from_str(&updated).unwrap();
        assert_eq!(config.theme.colors.get("ok").map(String::as_str), Some("#00ff00"));
        let removed = edit_document(&updated, "theme.colors.ok", None).unwrap();
        assert!(toml::from_str::<Config>(&removed).unwrap().theme.colors.is_empty());
        assert!(!removed.contains("theme"), "the emptied sections are gone: {removed:?}");
        // Removing what is not there is harmless.
        assert!(edit_document("", "ui.border", None).is_ok());
    }

    #[test]
    fn a_file_that_is_not_toml_is_refused() {
        assert!(edit_document("[[[", "ui.border", Some(toml_edit::Value::from("thick"))).is_err());
    }

    #[test]
    fn saving_writes_the_file_and_returns_the_new_config() {
        let dir = std::env::temp_dir().join(format!("knav-settings-{}", std::process::id()));
        let file = dir.join("config.toml");
        let config = save(&file, "mouse.wheel_rows", Some(toml_edit::Value::from(7))).unwrap();
        assert_eq!(config.mouse.wheel_rows, 7);
        assert!(std::fs::read_to_string(&file).unwrap().contains("wheel_rows = 7"));
        let config = save(&file, "mouse.wheel_rows", None).unwrap();
        assert_eq!(config.mouse.wheel_rows, 3, "removed, so the default is back");
        std::fs::remove_dir_all(dir).ok();
    }
}
