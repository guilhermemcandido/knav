//! Every setting the config screen edits: what it is called, what values it
//! takes, how it is read from and written to `config.toml` (keeping the
//! file's comments), and how a change takes effect while knav runs.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::config::Config;
use crate::theme::{self, Theme};
use crate::tunables::{Tunables, set_tunables};

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Choice(Vec<String>),
    Number { min: i64, max: i64 },
    Bool,
    Color,
}

#[derive(Clone, Debug)]
pub struct Setting {
    /// Dotted path in `config.toml`, e.g. `ui.border`.
    pub path: String,
    pub section: &'static str,
    pub label: String,
    pub kind: Kind,
    /// Only takes effect the next time knav starts.
    pub restart: bool,
}

fn choice(options: &[&str]) -> Kind {
    Kind::Choice(options.iter().map(|o| o.to_string()).collect())
}

/// Every setting, grouped by section in the order the screen shows them.
pub fn registry() -> Vec<Setting> {
    let mut settings = Vec::new();
    let mut add = |path: &str, section: &'static str, label: &str, kind: Kind, restart: bool| {
        settings.push(Setting { path: path.to_string(), section, label: label.to_string(), kind, restart });
    };
    add("theme.preset", "Theme", "Colour preset", choice(theme::PRESETS), false);
    add("ui.border", "Appearance", "Box lines", choice(&["heavy-rounded", "thick", "rounded", "double", "block", "arcs"]), false);
    add("ui.suggestion_icon_percent", "Appearance", "Command icon size (%)", Kind::Number { min: 30, max: 100 }, false);
    add("ui.idle_redraw_ms", "Appearance", "Idle redraw (ms)", Kind::Number { min: 50, max: 1000 }, false);
    add("ui.shell_redraw_ms", "Appearance", "Shell redraw (ms)", Kind::Number { min: 10, max: 200 }, false);
    add("tables.min_column_width", "Tables", "Minimum column width", Kind::Number { min: 4, max: 60 }, false);
    add("tables.wide_by_default", "Tables", "Wide columns on start", Kind::Bool, true);
    add("tables.faults_by_default", "Tables", "Faults only on start", Kind::Bool, true);
    add("logs.order", "Logs", "Log order", choice(&["oldest_first", "newest_first"]), false);
    add("logs.timestamp_format", "Logs", "Log timestamps", choice(&["short", "full"]), false);
    add("mouse.wheel_rows", "Mouse", "Wheel rows per notch", Kind::Number { min: 1, max: 20 }, false);
    add("mouse.double_click_ms", "Mouse", "Double-click time (ms)", Kind::Number { min: 100, max: 1000 }, false);
    add("startup.mode", "Behaviour", "Start with", choice(&["direct", "menu"]), true);
    add("portforward.open_browser", "Behaviour", "Open browser on port-forward", Kind::Bool, false);
    add("api.refresh_seconds", "Behaviour", "API list refresh (s)", Kind::Number { min: 1, max: 60 }, false);
    for (role, label) in theme::ROLES {
        add(&format!("theme.colors.{role}"), "Colours", label, Kind::Color, false);
    }
    settings
}

/// Makes the config take effect: colours, box lines, column widths and the
/// numeric knobs. Returns what was ignored (bad colours and the like).
pub fn apply(config: &Config) -> Vec<String> {
    let (theme, ignored) = theme::build(&config.theme.preset, &config.theme.colors);
    theme::set_theme(theme);
    crate::ui::configure_border(&config.ui.border);
    crate::ui::configure_columns(config.tables.min_column_width, config.tables.min_widths.clone());
    set_tunables(Tunables {
        wheel_rows: config.mouse.wheel_rows.max(1),
        double_click_ms: config.mouse.double_click_ms,
        api_refresh_seconds: config.api.refresh_seconds.max(1),
        suggestion_icon_percent: config.ui.suggestion_icon_percent.clamp(30, 100),
        idle_redraw_ms: config.ui.idle_redraw_ms.max(10),
        shell_redraw_ms: config.ui.shell_redraw_ms.max(5),
    });
    ignored
}

fn lookup<'a>(value: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    path.split('.').try_fold(value, |v, key| v.get(key))
}

fn show(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The value in effect for `setting`, as text.
pub fn current(config: &Config, theme: &Theme, setting: &Setting) -> String {
    if let Some(role) = setting.path.strip_prefix("theme.colors.") {
        return theme.get(role).map(theme::format_color).unwrap_or_default();
    }
    toml::Value::try_from(config).ok().and_then(|v| lookup(&v, &setting.path).map(show)).unwrap_or_default()
}

/// What `setting` is when the config says nothing about it.
pub fn default_of(setting: &Setting) -> String {
    let base = Config::default();
    let (theme, _) = theme::build(&base.theme.preset, &base.theme.colors);
    // A colour's default depends on the preset chosen; the caller passes that
    // through `preset_default` when it matters.
    current(&base, &theme, setting)
}

/// Whether the config file sets `setting` itself.
pub fn is_customised(config: &Config, setting: &Setting) -> bool {
    if let Some(role) = setting.path.strip_prefix("theme.colors.") {
        return config.theme.colors.contains_key(role);
    }
    let defaults = toml::Value::try_from(Config::default()).ok();
    let now = toml::Value::try_from(config).ok();
    match (defaults, now) {
        (Some(d), Some(n)) => lookup(&d, &setting.path) != lookup(&n, &setting.path),
        _ => false,
    }
}

/// A typed value for the file, checked against the setting's kind.
pub fn typed_value(setting: &Setting, text: &str) -> Result<toml_edit::Value> {
    let text = text.trim();
    Ok(match &setting.kind {
        Kind::Choice(options) => {
            if !options.iter().any(|o| o == text) {
                bail!("{text} is not one of {}", options.join(", "));
            }
            toml_edit::Value::from(text)
        }
        Kind::Number { min, max } => {
            let number: i64 = text.parse().with_context(|| format!("'{text}' is not a number"))?;
            if number < *min || number > *max {
                bail!("{number} is out of range ({min} to {max})");
            }
            toml_edit::Value::from(number)
        }
        Kind::Bool => match text {
            "true" | "on" | "yes" => toml_edit::Value::from(true),
            "false" | "off" | "no" => toml_edit::Value::from(false),
            _ => bail!("{text} is not on or off"),
        },
        Kind::Color => {
            if theme::parse_color(text).is_none() {
                bail!("'{text}' is not a colour (use #rrggbb, a name like red, or indexed:N)");
            }
            toml_edit::Value::from(text)
        }
    })
}

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
        }
    }
    Ok(doc.to_string())
}

/// Writes `value` (or removes the key) in the config file and returns the
/// config as it now reads. The file is only replaced if the result parses.
pub fn save(file: &Path, path: &str, value: Option<toml_edit::Value>) -> Result<Config> {
    let existing = std::fs::read_to_string(file).unwrap_or_default();
    let updated = edit_document(&existing, path, value)?;
    let config: Config = toml::from_str(&updated).context("that would make config.toml invalid")?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(file, updated).with_context(|| format!("writing {}", file.display()))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(path: &str) -> Setting {
        registry().into_iter().find(|s| s.path == path).unwrap_or_else(|| panic!("{path}"))
    }

    #[test]
    fn every_path_is_unique_and_every_colour_role_has_a_setting() {
        let settings = registry();
        let mut paths: Vec<&str> = settings.iter().map(|s| s.path.as_str()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), settings.len());
        for (role, _) in theme::ROLES {
            assert!(settings.iter().any(|s| s.path == format!("theme.colors.{role}")), "{role}");
        }
    }

    #[test]
    fn every_setting_reads_back_a_value_and_its_default_is_valid() {
        let config = Config::default();
        let (theme, _) = theme::build(&config.theme.preset, &config.theme.colors);
        for setting in registry() {
            let now = current(&config, &theme, &setting);
            assert!(!now.is_empty(), "{} has no current value", setting.path);
            assert!(typed_value(&setting, &now).is_ok(), "{}: default '{now}' is rejected", setting.path);
        }
    }

    #[test]
    fn values_are_checked_against_the_kind() {
        assert!(typed_value(&find("ui.border"), "thick").is_ok());
        assert!(typed_value(&find("ui.border"), "wavy").is_err());
        assert!(typed_value(&find("mouse.wheel_rows"), "5").is_ok());
        assert!(typed_value(&find("mouse.wheel_rows"), "0").is_err());
        assert!(typed_value(&find("mouse.wheel_rows"), "many").is_err());
        assert!(typed_value(&find("portforward.open_browser"), "off").is_ok());
        assert!(typed_value(&find("portforward.open_browser"), "maybe").is_err());
        assert!(typed_value(&find("theme.colors.ok"), "#00ff00").is_ok());
        assert!(typed_value(&find("theme.colors.ok"), "lime").is_err());
    }

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

    #[test]
    fn customised_means_the_file_sets_it_to_something_else() {
        let mut config = Config::default();
        assert!(!is_customised(&config, &find("ui.border")));
        config.ui.border = "thick".into();
        assert!(is_customised(&config, &find("ui.border")));
        config.theme.colors.insert("ok".into(), "#00ff00".into());
        assert!(is_customised(&config, &find("theme.colors.ok")));
    }

    #[test]
    fn applying_the_config_sets_the_theme() {
        let mut config = Config::default();
        config.theme.colors.insert("ok".into(), "#010203".into());
        let ignored = apply(&config);
        assert!(ignored.is_empty());
        assert_eq!(theme::theme().ok, ratatui::style::Color::Rgb(1, 2, 3));
        apply(&Config::default());
        assert_eq!(theme::theme().ok, Theme::default().ok);
    }
}
