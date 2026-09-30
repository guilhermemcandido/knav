//! The `config.toml` file and its defaults.

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

/// Which way the log view reads: `oldest_first` (the default) has new lines at the
/// bottom, `newest_first` at the top.
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
    pub sections: Vec<String>,
    pub items: BTreeMap<String, Vec<String>>,
    /// Hidden categories (`Config`) and kinds (`Config/Secrets`).
    pub hidden: Vec<String>,
}

/// Names a saved layout may still use from before a rename. An unknown name is
/// unlisted, which moves its category after every listed one.
const LEGACY_NAMES: &[(&str, &str)] = &[("Custom Resources", "CustomResources"), ("Helm Releases", "HelmReleases")];

fn current_name(name: &str) -> String {
    LEGACY_NAMES.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new).to_string()
}

impl OverviewConfig {
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

/// Column sizing: as wide as the content, never below the minimum. When the minimums
/// don't fit, the table scrolls sideways.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct TablesConfig {
    pub min_column_width: usize,
    /// Per-column minimums by lowercase header, like `name = 24`.
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct PortForwardConfig {
    /// Open `http://localhost:<port>` once it runs; ask first when false.
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

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    /// The line style of every box: `rounded` (default), `thick` or `double`.
    pub border: String,
    /// How much of its square a command suggestion's icon fills, in percent.
    pub suggestion_icon_percent: u8,
    /// Kind icons on the Overview cards and in the command line.
    pub icons: bool,
    pub idle_redraw_ms: u64,
    pub shell_redraw_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { border: "rounded".into(), suggestion_icon_percent: 78, icons: true, idle_redraw_ms: 200, shell_redraw_ms: 25 }
    }
}

/// Blocks every change knav can make: delete, edit, scale, restart, cordon and shells.
#[derive(Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct ReadOnlyConfig {
    /// On for every context.
    pub enabled: bool,
    /// On for contexts matching one of these, where `*` matches anything (`prod*`).
    pub contexts: Vec<String>,
}

impl ReadOnlyConfig {
    pub fn applies_to(&self, context: &str) -> bool {
        self.enabled || self.contexts.iter().any(|pattern| wildcard_match(pattern, context))
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, rest)) => text.strip_prefix(head).is_some_and(|tail| (0..=tail.len()).filter(|&i| tail.is_char_boundary(i)).any(|i| wildcard_match(rest, &tail[i..]))),
    }
}

/// The ids of the extensions turned on.
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
    pub read_only: ReadOnlyConfig,
    /// Key bindings by action id, each one key or a list.
    #[serde(deserialize_with = "one_or_many")]
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Config {
    /// Reads `config.toml` from `config_dir()`. A missing file or field means the
    /// default; a malformed file gives all defaults.
    pub fn load() -> Self {
        Self::load_reporting().0
    }

    /// Like `load`, plus what went wrong, to show once the screen is up.
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

    #[test]
    fn read_only_contexts_match_with_wildcards() {
        let config: ReadOnlyConfig = toml::from_str(r#"contexts = ["prod*", "*-live", "staging", "*payments*"]"#).unwrap();
        assert!(config.applies_to("eu-payments-2"), "a * on both sides matches inside");
        assert!(config.applies_to("payments"));
        assert!(config.applies_to("prod-eu"));
        assert!(config.applies_to("shop-live"));
        assert!(config.applies_to("staging"));
        assert!(!config.applies_to("staging-2"));
        assert!(!config.applies_to("dev"));
        assert!(ReadOnlyConfig { enabled: true, contexts: vec![] }.applies_to("dev"));
    }
}
