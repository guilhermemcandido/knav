//! The `config.toml` file, its defaults, and the settings built on it.

pub mod favorites;
pub mod edit;
pub mod tunables;

use std::path::PathBuf;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimestampFormat {
    #[default]
    Short,
    Full,
}

impl TimestampFormat {
    pub fn toggled(self) -> Self {
        match self {
            TimestampFormat::Short => TimestampFormat::Full,
            TimestampFormat::Full => TimestampFormat::Short,
        }
    }
}

/// Which way the log view reads: `oldest_first` is a normal top-down
/// reading order with new lines arriving at the bottom (the default);
/// `newest_first` puts the latest line at the top.
#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOrder {
    #[default]
    OldestFirst,
    NewestFirst,
}

impl LogOrder {
    pub fn toggled(self) -> Self {
        match self {
            LogOrder::OldestFirst => LogOrder::NewestFirst,
            LogOrder::NewestFirst => LogOrder::OldestFirst,
        }
    }
}

/// The order and visibility of the Overview's categories and kinds. Anything
/// not listed keeps its default place after the listed ones.
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct OverviewConfig {
    /// Category names, in the order to show them.
    pub sections: Vec<String>,
    /// Kind names per category, in the order to show them.
    pub items: BTreeMap<String, Vec<String>>,
    /// Hidden categories (`Config`) and kinds (`Config/Secrets`).
    pub hidden: Vec<String>,
}

/// Names a saved layout may still use from before they were renamed. An
/// unrecognized name is silently unlisted, which drops it after every listed
/// category (an old "Custom Resources" put CustomResources after Helm).
const LEGACY_NAMES: &[(&str, &str)] = &[("Custom Resources", "CustomResources"), ("Helm Releases", "HelmReleases")];

fn current_name(name: &str) -> String {
    LEGACY_NAMES.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new).to_string()
}

impl OverviewConfig {
    /// Rewrites legacy category and kind names to their current spelling.
    pub fn migrate_legacy_names(&mut self) {
        for name in &mut self.sections {
            *name = current_name(name);
        }
        self.items = std::mem::take(&mut self.items).into_iter().map(|(section, items)| (current_name(&section), items.iter().map(|i| current_name(i)).collect())).collect();
        for entry in &mut self.hidden {
            *entry = entry.split('/').map(current_name).collect::<Vec<_>>().join("/");
        }
    }
}

#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct LogsConfig {
    pub timestamp_format: TimestampFormat,
    pub order: LogOrder,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct LogsKeybindings {
    pub toggle_timestamp: char,
    pub toggle_order: char,
}

impl Default for LogsKeybindings {
    fn default() -> Self {
        LogsKeybindings { toggle_timestamp: 't', toggle_order: 'o' }
    }
}

#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Keybindings {
    pub logs: LogsKeybindings,
}

/// How knav starts: `direct` connects to the context `kube` infers, `menu` shows the
/// cluster picker first. `--context` on the command line skips both.
#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupMode {
    #[default]
    Direct,
    Menu,
}

#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct StartupConfig {
    pub mode: StartupMode,
}

/// Table column sizing. A column is as wide as its content, but never
/// squeezed below its minimum; when the columns' minimums don't all fit the
/// screen, the table scrolls sideways (←/→) instead.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TablesConfig {
    /// The minimum width of every column unless overridden below.
    pub min_column_width: usize,
    /// Per-column minimums, keyed by the lowercase header name
    /// (`name = 24`, `namespace = 14`, `"up-to-date" = 12`).
    pub min_widths: std::collections::HashMap<String, usize>,
    /// Start every list with the wide columns / only the faulty rows.
    pub wide_by_default: bool,
    pub faults_by_default: bool,
}

impl Default for TablesConfig {
    fn default() -> Self {
        TablesConfig { min_column_width: 10, min_widths: std::collections::HashMap::new(), wide_by_default: false, faults_by_default: false }
    }
}

/// What starting a port-forward does besides forwarding.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PortForwardConfig {
    /// Open `http://localhost:<port>` in the browser once it is running;
    /// when false, ask first.
    pub open_browser: bool,
}

impl Default for PortForwardConfig {
    fn default() -> Self {
        PortForwardConfig { open_browser: true }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct MouseConfig {
    pub wheel_rows: usize,
    pub double_click_ms: u64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        MouseConfig { wheel_rows: 3, double_click_ms: 400 }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ApiConfig {
    /// Seconds between refreshes of `:api` and custom-resource lists.
    pub refresh_seconds: u64,
}

impl Default for ApiConfig {
    fn default() -> Self {
        ApiConfig { refresh_seconds: 2 }
    }
}

/// A key binding written as `"x"` or `["x", "ctrl-d"]`.
fn one_or_many<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<String, Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Keys {
        One(String),
        Many(Vec<String>),
    }
    let raw = BTreeMap::<String, Keys>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(|(id, keys)| (id, match keys { Keys::One(k) => vec![k], Keys::Many(list) => list })).collect())
}

/// Colours: a preset, and per-role overrides on top of it (`#rrggbb`, a
/// terminal colour name, or `indexed:N`).
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub preset: String,
    pub colors: BTreeMap<String, String>,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig { preset: "knav".into(), colors: BTreeMap::new() }
    }
}

/// Look-and-feel options.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// The line style of every box: `rounded` (default), `thick` or `double`.
    pub border: String,
    /// How much of its square a command suggestion's icon fills, in percent.
    pub suggestion_icon_percent: u8,
    pub idle_redraw_ms: u64,
    pub shell_redraw_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { border: "rounded".into(), suggestion_icon_percent: 78, idle_redraw_ms: 200, shell_redraw_ms: 25 }
    }
}

/// Which extensions (see `crate::extensions`) are turned on, by id. An
/// extension is inert data until its id is here.
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ExtensionsConfig {
    pub enabled: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub portforward: PortForwardConfig,
    pub logs: LogsConfig,
    pub keybindings: Keybindings,
    pub startup: StartupConfig,
    pub tables: TablesConfig,
    pub ui: UiConfig,
    pub theme: ThemeConfig,
    pub mouse: MouseConfig,
    pub api: ApiConfig,
    pub overview: OverviewConfig,
    pub extensions: ExtensionsConfig,
    /// Key bindings by action id; each is one key or a list (see `keymap`).
    #[serde(deserialize_with = "one_or_many")]
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Config {
    /// Reads `$XDG_CONFIG_HOME/knav/config.toml`, falling back to `~/.config/knav/config.toml`.
    /// A missing file or field means the default; a malformed file gives the defaults.
    pub fn load() -> Self {
        Self::load_reporting().0
    }

    /// Like `load`, with what went wrong in words, for showing once the screen is up.
    pub fn load_reporting() -> (Self, Vec<String>) {
        let path = Self::path();
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), Vec::new()),
            Err(e) => return (Config::default(), vec![format!("could not read {}: {e}", path.display())]),
        };
        match toml::from_str::<Config>(&contents) {
            Ok(mut config) => {
                config.overview.migrate_legacy_names();
                (config, Vec::new())
            }
            Err(e) => (Config::default(), vec![format!("{} could not be read, so defaults are in use:\n{e}", path.display())]),
        }
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// `$XDG_CONFIG_HOME/knav` (or `~/.config/knav`), the config file
    /// and knav's small saved state live here.
    pub fn dir() -> PathBuf {
        crate::util::config_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_saved_before_the_renames_uses_the_current_names() {
        let mut overview: OverviewConfig = toml::from_str(
            r#"
            sections = ["Access Control", "Custom Resources", "Helm"]
            items = { Helm = ["Helm Releases", "HelmChart"] }
            hidden = ["Custom Resources", "Helm/Helm Releases"]
            "#,
        )
        .unwrap();
        overview.migrate_legacy_names();
        assert_eq!(overview.sections, ["Access Control", "CustomResources", "Helm"]);
        assert_eq!(overview.items["Helm"], ["HelmReleases", "HelmChart"]);
        assert_eq!(overview.hidden, ["CustomResources", "Helm/HelmReleases"]);
    }
}
