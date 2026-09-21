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

/// How knav starts up, k9s/Lens-style: `direct` connects straight to
/// whatever context `kube` would infer (in-cluster, or the kubeconfig's
/// `current-context`) — no extra screen, matching k9s's default. `menu`
/// always shows the freelens-style cluster picker first, even if there's
/// only one context. `--context` on the command line bypasses this
/// entirely regardless of which mode is configured.
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
    /// The line style of every box: `heavy-rounded` (default), `thick`,
    /// `rounded`, `double`, `block` or `arcs`.
    pub border: String,
    /// How much of its square a command suggestion's icon fills, in percent.
    pub suggestion_icon_percent: u8,
    pub idle_redraw_ms: u64,
    pub shell_redraw_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig { border: "heavy-rounded".into(), suggestion_icon_percent: 78, idle_redraw_ms: 200, shell_redraw_ms: 25 }
    }
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
}

impl Config {
    /// Reads `$XDG_CONFIG_HOME/knav/config.toml` (falling back to
    /// `~/.config/knav/config.toml`) — the same convention as this
    /// machine's nvim/tmux/herdr configs, not the platform-specific
    /// location a crate like `dirs` would pick on macOS
    /// (`~/Library/Application Support`). No file, or a field left out,
    /// just means "use the default" — a config file is never required.
    /// A malformed file is reported and defaults are used rather than
    /// refusing to start, since printing a real error after the TUI
    /// takes over the screen isn't possible.
    pub fn load() -> Self {
        let path = Self::path();
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Config::default();
        };
        match toml::from_str(&contents) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("warning: failed to parse {}: {e}\nusing defaults", path.display());
                Config::default()
            }
        }
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.toml")
    }

    /// `$XDG_CONFIG_HOME/knav` (or `~/.config/knav`) — the config file
    /// and knav's small saved state live here.
    pub fn dir() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
        base.join("knav")
    }
}
