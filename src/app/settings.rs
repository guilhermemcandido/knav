//! Every setting the config screen edits: its name, the values it takes, and
//! how a change takes effect while knav runs.

use anyhow::{Context as _, Result, bail};

use crate::config::Config;
use crate::theme::{self, Theme};
use crate::config::tunables::{Tunables, set_tunables};

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Choice(Vec<String>),
    Number { min: i64, max: i64 },
    Bool,
    #[allow(dead_code)]
    Color,
    /// Key bindings for an action, typed as `x, ctrl-d`.
    Keys,
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
    add("theme.preset", "Theme", "Theme", Kind::Choice(theme::all_names()), false);
    add("ui.border", "Appearance", "Box lines", choice(&["rounded", "thick", "double"]), false);
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
    for binding in crate::input::keymap::BINDINGS {
        add(&format!("keys.{}", binding.id), "Keys", binding.label, Kind::Keys, false);
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
    crate::k8s::set_refresh_seconds(config.api.refresh_seconds);
    set_tunables(Tunables {
        wheel_rows: config.mouse.wheel_rows.max(1),
        double_click_ms: config.mouse.double_click_ms,
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

/// What a setting does, in a line or two.
pub fn describe(setting: &Setting) -> &'static str {
    if setting.path.starts_with("keys.") {
        return "Press enter, then the key you want. You can use only that key or add it to the current ones. Keys another action already uses on the same screen are refused. r resets it.";
    }
    match setting.path.as_str() {
        "theme.preset" => "The colour theme. T opens the picker, which previews each theme as you move.",
        "ui.border" => "The lines around every box: rounded corners, thick lines or double lines.",
        "ui.suggestion_icon_percent" => "How large the icons in the command line suggestions are, as a percent of their row.",
        "ui.idle_redraw_ms" => "How often the screen refreshes while nothing is happening. Lower is smoother, higher uses less CPU.",
        "ui.shell_redraw_ms" => "How often the screen refreshes while a shell is open. Lower feels snappier.",
        "tables.min_column_width" => "The narrowest a column can get when space is short. Past that the table scrolls sideways.",
        "tables.wide_by_default" => "Start with the extra columns shown, like kubectl -o wide. Ctrl-w toggles them at any time.",
        "tables.faults_by_default" => "Start with lists showing only rows that need attention. Ctrl-z toggles it at any time.",
        "logs.order" => "oldest_first reads like a file with new lines at the bottom. newest_first puts new lines on top.",
        "logs.timestamp_format" => "short shows the time only, full shows the whole timestamp. t toggles it in the log view.",
        "mouse.wheel_rows" => "How many rows one notch of the mouse wheel moves.",
        "mouse.double_click_ms" => "Two clicks on the same row or tile within this time count as a double-click and open it.",
        "startup.mode" => "direct connects to your current kubeconfig context and opens Home, like k9s. menu shows a cluster picker first, even with a single context. --context skips both.",
        "portforward.open_browser" => "Open the browser as soon as a port-forward starts. When off, knav asks first.",
        "api.refresh_seconds" => "How often the API resources list refreshes in the background.",
        _ => "",
    }
}

/// The value in effect for `setting`, as text.
pub fn current(config: &Config, theme: &Theme, setting: &Setting) -> String {
    if let Some(id) = setting.path.strip_prefix("keys.") {
        // The comma key is written `comma` here, since commas separate the keys.
        return crate::input::keymap::keys_now(id).iter().map(|k| if k == "," { "comma" } else { k.as_str() }).collect::<Vec<_>>().join(", ");
    }
    if let Some(role) = setting.path.strip_prefix("theme.colors.") {
        return theme.get(role).map(theme::format_color).unwrap_or_default();
    }
    toml::Value::try_from(config).ok().and_then(|v| lookup(&v, &setting.path).map(show)).unwrap_or_default()
}

/// Whether the config file sets `setting` itself.
pub fn is_customised(config: &Config, setting: &Setting) -> bool {
    if let Some(id) = setting.path.strip_prefix("keys.") {
        return config.keys.contains_key(id);
    }
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
pub fn typed_value(config: &Config, setting: &Setting, text: &str) -> Result<toml_edit::Value> {
    let text = text.trim();
    Ok(match &setting.kind {
        Kind::Keys => {
            let id = setting.path.strip_prefix("keys.").context("a key setting")?;
            let list: Vec<String> = text.split(',').map(|k| k.trim()).filter(|k| !k.is_empty()).map(|k| if k.eq_ignore_ascii_case("comma") { ",".to_string() } else { k.to_string() }).collect();
            if list.is_empty() {
                bail!("give at least one key (for example x, ctrl-d)");
            }
            // The whole set of bindings, checked as the config loader would.
            let mut keys = config.keys.clone();
            keys.insert(id.to_string(), list.clone());
            let (_, problems) = crate::input::keymap::overrides_from_config(&keys);
            let prefix = format!("keys.{id}: ");
            if let Some(problem) = problems.iter().find_map(|p| p.strip_prefix(&prefix)) {
                bail!("{problem}");
            }
            let mut array = toml_edit::Array::new();
            for key in list {
                array.push(key);
            }
            toml_edit::Value::Array(array)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Colours are edited in theme files, not on the screen, but the value
    /// handling stays for `[theme.colors]` in the config.
    fn colour_setting(path: &str) -> Setting {
        Setting { path: path.to_string(), section: "Colours", label: path.to_string(), kind: Kind::Color, restart: false }
    }

    fn find(path: &str) -> Setting {
        registry().into_iter().find(|s| s.path == path).unwrap_or_else(|| colour_setting(path))
    }

    #[test]
    fn every_path_is_unique() {
        let settings = registry();
        let mut paths: Vec<&str> = settings.iter().map(|s| s.path.as_str()).collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), settings.len());
    }

    #[test]
    fn every_setting_reads_back_a_value_and_its_default_is_valid() {
        let config = Config::default();
        let (theme, _) = theme::build(&config.theme.preset, &config.theme.colors);
        for setting in registry() {
            let now = current(&config, &theme, &setting);
            assert!(!now.is_empty(), "{} has no current value", setting.path);
            assert!(typed_value(&config, &setting, &now).is_ok(), "{}: default '{now}' is rejected", setting.path);
        }
    }

    #[test]
    fn values_are_checked_against_the_kind() {
        assert!(typed_value(&Config::default(), &find("ui.border"), "thick").is_ok());
        assert!(typed_value(&Config::default(), &find("ui.border"), "wavy").is_err());
        assert!(typed_value(&Config::default(), &find("mouse.wheel_rows"), "5").is_ok());
        assert!(typed_value(&Config::default(), &find("mouse.wheel_rows"), "0").is_err());
        assert!(typed_value(&Config::default(), &find("mouse.wheel_rows"), "many").is_err());
        assert!(typed_value(&Config::default(), &find("portforward.open_browser"), "off").is_ok());
        assert!(typed_value(&Config::default(), &find("portforward.open_browser"), "maybe").is_err());
        assert!(typed_value(&Config::default(), &find("theme.colors.ok"), "#00ff00").is_ok());
        assert!(typed_value(&Config::default(), &find("theme.colors.ok"), "lime").is_err());
    }

    #[test]
    fn key_settings_are_lists_checked_for_conflicts() {
        let config = Config::default();
        let delete = find("keys.delete");
        let ok = typed_value(&config, &delete, "X, ctrl-d").unwrap();
        assert_eq!(ok.to_string().replace(' ', ""), "[\"X\",\"ctrl-d\"]");
        assert!(typed_value(&config, &delete, "").is_err());
        assert!(typed_value(&config, &delete, "nonsense").is_err());
        let clash = typed_value(&config, &delete, "r").unwrap_err().to_string();
        assert!(clash.contains("already 'Restart'"), "{clash}");
        assert!(typed_value(&config, &delete, "D").is_ok(), "its own current key is fine");
    }

    #[test]
    fn every_action_has_a_key_setting_that_reads_back_its_defaults() {
        let config = Config::default();
        let (theme, _) = theme::build("knav", &Default::default());
        for binding in crate::input::keymap::BINDINGS {
            let setting = find(&format!("keys.{}", binding.id));
            assert_eq!(current(&config, &theme, &setting), binding.defaults.iter().map(|k| if *k == "," { "comma" } else { k }).collect::<Vec<_>>().join(", "), "{}", binding.id);
        }
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
