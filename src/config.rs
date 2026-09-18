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

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub logs: LogsConfig,
    pub keybindings: Keybindings,
    pub startup: StartupConfig,
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
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
        base.join("knav").join("config.toml")
    }
}
