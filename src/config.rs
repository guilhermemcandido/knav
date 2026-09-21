use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize)]
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

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LogsConfig {
    pub timestamp_format: TimestampFormat,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct LogsKeybindings {
    pub toggle_timestamp: char,
}

impl Default for LogsKeybindings {
    fn default() -> Self {
        LogsKeybindings { toggle_timestamp: 't' }
    }
}

#[derive(Deserialize, Default)]
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
#[derive(Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartupMode {
    #[default]
    Direct,
    Menu,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct StartupConfig {
    pub mode: StartupMode,
}

/// Table column sizing. A column is as wide as its content, but never
/// squeezed below its minimum; when the columns' minimums don't all fit the
/// screen, the table scrolls sideways (←/→) instead.
#[derive(Deserialize)]
#[serde(default)]
pub struct TablesConfig {
    /// The minimum width of every column unless overridden below.
    pub min_column_width: usize,
    /// Per-column minimums, keyed by the lowercase header name
    /// (`name = 24`, `namespace = 14`, `"up-to-date" = 12`).
    pub min_widths: std::collections::HashMap<String, usize>,
}

impl Default for TablesConfig {
    fn default() -> Self {
        TablesConfig { min_column_width: 10, min_widths: std::collections::HashMap::new() }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub logs: LogsConfig,
    pub keybindings: Keybindings,
    pub startup: StartupConfig,
    pub tables: TablesConfig,
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

    fn path() -> PathBuf {
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
