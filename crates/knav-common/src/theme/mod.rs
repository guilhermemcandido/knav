//! Every colour knav draws with, as named roles in one `Theme`. Screens ask
//! `theme()` for a role instead of hardcoding a colour, so a preset or the
//! config file (or the config screen, live) can change them all.

use std::collections::BTreeMap;
use std::sync::RwLock;

use ratatui::style::Color;

macro_rules! theme_roles {
    ($($field:ident: $label:literal, $default:expr;)+) => {
        /// One colour per role.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Theme {
            $(pub $field: Color,)+
        }

        impl Default for Theme {
            fn default() -> Self {
                Theme { $($field: $default,)+ }
            }
        }

        /// Every role, in the order the config screen lists them: its config
        /// key and a short description.
        #[allow(dead_code)]
        pub const ROLES: &[(&str, &str)] = &[$((stringify!($field), $label),)+];

        impl Theme {
            pub fn get(&self, role: &str) -> Option<Color> {
                match role {
                    $(stringify!($field) => Some(self.$field),)+
                    _ => None,
                }
            }

            pub fn set(&mut self, role: &str, color: Color) -> bool {
                match role {
                    $(stringify!($field) => { self.$field = color; true })+
                    _ => false,
                }
            }
        }
    };
}

theme_roles! {
    background: "Background (reset = your terminal's)", Color::Reset;
    foreground: "Default text (reset = your terminal's)", Color::Reset;
    row: "Row text", Color::Rgb(143, 191, 208);
    header: "Column headers", Color::Rgb(137, 180, 250);
    text_strong: "Emphasised text", Color::Rgb(226, 232, 240);
    text_soft: "Secondary text", Color::Rgb(200, 205, 218);
    desc: "Descriptions", Color::Rgb(170, 176, 192);
    muted: "Muted text, finished items", Color::Rgb(122, 128, 148);
    accent: "Titles and badges", Color::Rgb(120, 230, 230);
    warm: "Counts and numbers", Color::Rgb(240, 160, 110);
    key: "Key names and YAML keys", Color::Rgb(84, 148, 255);
    heading: "Help headings", Color::Rgb(96, 160, 72);
    label: "Form labels", Color::Rgb(214, 146, 120);
    info_label: "Header info labels", Color::Rgb(122, 140, 170);
    border: "Box lines", Color::Rgb(96, 125, 139);
    select_bg: "Selection bar", Color::Rgb(148, 191, 206);
    marked_bg: "Marked rows", Color::Rgb(84, 72, 24);
    pill_bg: "Title pill", Color::Rgb(50, 56, 72);
    panel_bg: "Panel fill", Color::Rgb(78, 88, 104);
    command: "Command line", Color::Rgb(122, 170, 214);
    ok: "Healthy", Color::Rgb(126, 201, 140);
    warn: "In progress, warnings", Color::Rgb(255, 167, 64);
    bad: "Broken, errors", Color::Rgb(217, 96, 106);
    highlight: "Search text and prompts", Color::Yellow;
    namespace: "Namespaces", Color::Cyan;
    container: "Containers", Color::Magenta;
    dim: "Text behind popups", Color::Rgb(40, 40, 40);
}

static THEME: RwLock<Option<Theme>> = RwLock::new(None);

pub fn theme() -> Theme {
    THEME.read().ok().and_then(|t| *t).unwrap_or_default()
}

/// Takes effect from the next frame.
pub fn set_theme(theme: Theme) {
    if let Ok(mut current) = THEME.write() {
        *current = Some(theme);
    }
}

/// A theme's colours in the terms themes are usually published in.
struct Palette {
    bg: &'static str,
    fg: &'static str,
    muted: &'static str,
    blue: &'static str,
    cyan: &'static str,
    green: &'static str,
    yellow: &'static str,
    orange: &'static str,
    red: &'static str,
    purple: &'static str,
    select: &'static str,
    border: &'static str,
}

fn hex(text: &str) -> Color {
    parse_color(text).expect("built-in palette colours are valid")
}

fn channels(color: Color) -> (f32, f32, f32) {
    match color {
        Color::Rgb(r, g, b) => (r as f32, g as f32, b as f32),
        Color::Black => (0.0, 0.0, 0.0),
        Color::Red => (205.0, 49.0, 49.0),
        Color::Green => (13.0, 188.0, 121.0),
        Color::Yellow => (229.0, 229.0, 16.0),
        Color::Blue => (36.0, 114.0, 200.0),
        Color::Magenta => (188.0, 63.0, 188.0),
        Color::Cyan => (17.0, 168.0, 205.0),
        Color::Gray => (229.0, 229.0, 229.0),
        Color::DarkGray => (102.0, 102.0, 102.0),
        Color::White | Color::LightRed | Color::LightGreen | Color::LightYellow | Color::LightBlue | Color::LightMagenta | Color::LightCyan => (240.0, 240.0, 240.0),
        _ => (0.0, 0.0, 0.0),
    }
}

/// Brightness from 0 (black) to 1 (white).
fn luminance(color: Color) -> f32 {
    let (r, g, b) = channels(color);
    (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
}

/// Readable text for something drawn on `bg`: black on light colours, white on dark.
pub fn on(bg: Color) -> Color {
    if luminance(bg) > 0.5 { Color::Black } else { Color::White }
}

/// `from` moved `amount` (0-1) of the way to `to`.
fn blend(from: Color, to: Color, amount: f32) -> Color {
    let (a, b) = (channels(from), channels(to));
    let mix = |x: f32, y: f32| (x + (y - x) * amount).round().clamp(0.0, 255.0) as u8;
    Color::Rgb(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

fn from_palette(p: &Palette) -> Theme {
    let (bg, fg, muted) = (hex(p.bg), hex(p.fg), hex(p.muted));
    Theme {
        background: bg,
        foreground: fg,
        row: fg,
        header: hex(p.blue),
        text_strong: blend(fg, if luminance(bg) > 0.5 { Color::Rgb(0, 0, 0) } else { Color::Rgb(255, 255, 255) }, 0.35),
        text_soft: fg,
        desc: blend(fg, muted, 0.45),
        muted,
        accent: hex(p.cyan),
        warm: hex(p.orange),
        key: hex(p.blue),
        heading: hex(p.green),
        label: hex(p.orange),
        info_label: muted,
        border: hex(p.border),
        select_bg: hex(p.select),
        marked_bg: blend(bg, hex(p.yellow), 0.28),
        pill_bg: blend(bg, fg, 0.14),
        panel_bg: blend(bg, fg, 0.22),
        command: hex(p.yellow),
        ok: hex(p.green),
        warn: hex(p.orange),
        bad: hex(p.red),
        highlight: hex(p.yellow),
        namespace: hex(p.cyan),
        container: hex(p.purple),
        dim: blend(bg, fg, 0.16),
    }
}

/// The published palettes knav ships as themes.
const PALETTES: &[(&str, Palette)] = &[
    ("dracula", Palette { bg: "#282a36", fg: "#f8f8f2", muted: "#6272a4", blue: "#bd93f9", cyan: "#8be9fd", green: "#50fa7b", yellow: "#f1fa8c", orange: "#ffb86c", red: "#ff5555", purple: "#ff79c6", select: "#44475a", border: "#6272a4" }),
    ("nord", Palette { bg: "#2e3440", fg: "#d8dee9", muted: "#616e88", blue: "#81a1c1", cyan: "#88c0d0", green: "#a3be8c", yellow: "#ebcb8b", orange: "#d08770", red: "#bf616a", purple: "#b48ead", select: "#434c5e", border: "#4c566a" }),
    ("gruvbox-dark", Palette { bg: "#282828", fg: "#ebdbb2", muted: "#928374", blue: "#83a598", cyan: "#8ec07c", green: "#b8bb26", yellow: "#fabd2f", orange: "#fe8019", red: "#fb4934", purple: "#d3869b", select: "#504945", border: "#665c54" }),
    ("gruvbox-light", Palette { bg: "#fbf1c7", fg: "#3c3836", muted: "#928374", blue: "#076678", cyan: "#427b58", green: "#79740e", yellow: "#b57614", orange: "#af3a03", red: "#9d0006", purple: "#8f3f71", select: "#d5c4a1", border: "#bdae93" }),
    ("catppuccin-mocha", Palette { bg: "#1e1e2e", fg: "#cdd6f4", muted: "#6c7086", blue: "#89b4fa", cyan: "#89dceb", green: "#a6e3a1", yellow: "#f9e2af", orange: "#fab387", red: "#f38ba8", purple: "#cba6f7", select: "#45475a", border: "#585b70" }),
    ("catppuccin-latte", Palette { bg: "#eff1f5", fg: "#4c4f69", muted: "#9ca0b0", blue: "#1e66f5", cyan: "#04a5e5", green: "#40a02b", yellow: "#df8e1d", orange: "#fe640b", red: "#d20f39", purple: "#8839ef", select: "#ccd0da", border: "#acb0be" }),
    ("tokyo-night", Palette { bg: "#1a1b26", fg: "#c0caf5", muted: "#565f89", blue: "#7aa2f7", cyan: "#7dcfff", green: "#9ece6a", yellow: "#e0af68", orange: "#ff9e64", red: "#f7768e", purple: "#bb9af7", select: "#33467c", border: "#3b4261" }),
    ("one-dark", Palette { bg: "#282c34", fg: "#abb2bf", muted: "#5c6370", blue: "#61afef", cyan: "#56b6c2", green: "#98c379", yellow: "#e5c07b", orange: "#d19a66", red: "#e06c75", purple: "#c678dd", select: "#3e4451", border: "#4b5263" }),
    ("monokai", Palette { bg: "#272822", fg: "#f8f8f2", muted: "#75715e", blue: "#66d9ef", cyan: "#66d9ef", green: "#a6e22e", yellow: "#e6db74", orange: "#fd971f", red: "#f92672", purple: "#ae81ff", select: "#49483e", border: "#75715e" }),
    ("solarized-dark", Palette { bg: "#002b36", fg: "#93a1a1", muted: "#586e75", blue: "#268bd2", cyan: "#2aa198", green: "#859900", yellow: "#b58900", orange: "#cb4b16", red: "#dc322f", purple: "#6c71c4", select: "#073642", border: "#586e75" }),
    ("solarized-light", Palette { bg: "#fdf6e3", fg: "#586e75", muted: "#93a1a1", blue: "#268bd2", cyan: "#2aa198", green: "#859900", yellow: "#b58900", orange: "#cb4b16", red: "#dc322f", purple: "#6c71c4", select: "#eee8d5", border: "#93a1a1" }),
    ("rose-pine", Palette { bg: "#191724", fg: "#e0def4", muted: "#6e6a86", blue: "#31748f", cyan: "#9ccfd8", green: "#9ccfd8", yellow: "#f6c177", orange: "#ebbcba", red: "#eb6f92", purple: "#c4a7e7", select: "#26233a", border: "#403d52" }),
    ("everforest", Palette { bg: "#2d353b", fg: "#d3c6aa", muted: "#859289", blue: "#7fbbb3", cyan: "#83c092", green: "#a7c080", yellow: "#dbbc7f", orange: "#e69875", red: "#e67e80", purple: "#d699b6", select: "#475258", border: "#56635f" }),
    ("github-dark", Palette { bg: "#0d1117", fg: "#c9d1d9", muted: "#8b949e", blue: "#58a6ff", cyan: "#79c0ff", green: "#3fb950", yellow: "#d29922", orange: "#db6d28", red: "#f85149", purple: "#bc8cff", select: "#21262d", border: "#30363d" }),
];

/// Themes that don't paint a background: they use your terminal's own.
fn transparent(name: &str) -> Option<Theme> {
    let base = Theme::default();
    Some(match name {
        "knav" => base,
        "steel" => Theme {
            row: Color::Rgb(112, 184, 214),
            header: Color::Rgb(112, 200, 255),
            select_bg: Color::Rgb(122, 196, 214),
            border: Color::Rgb(70, 130, 180),
            key: Color::Rgb(70, 130, 255),
            heading: Color::Rgb(0, 170, 0),
            ok: Color::Rgb(0, 200, 120),
            warn: Color::Rgb(255, 165, 0),
            bad: Color::Rgb(220, 90, 90),
            muted: Color::Rgb(150, 150, 170),
            accent: Color::Rgb(0, 255, 255),
            command: Color::Rgb(255, 215, 0),
            ..base
        },
        "high-contrast" => Theme {
            row: Color::White,
            header: Color::Rgb(255, 255, 0),
            text_strong: Color::White,
            text_soft: Color::White,
            desc: Color::Rgb(230, 230, 230),
            muted: Color::Rgb(180, 180, 180),
            accent: Color::Rgb(0, 255, 255),
            warm: Color::Rgb(255, 200, 0),
            key: Color::Rgb(120, 180, 255),
            border: Color::White,
            select_bg: Color::White,
            marked_bg: Color::Rgb(90, 90, 0),
            command: Color::Rgb(255, 255, 0),
            ok: Color::Rgb(0, 255, 0),
            warn: Color::Rgb(255, 200, 0),
            bad: Color::Rgb(255, 60, 60),
            dim: Color::Rgb(90, 90, 90),
            ..base
        },
        "mono" => Theme {
            row: Color::Gray,
            header: Color::White,
            text_strong: Color::White,
            text_soft: Color::Gray,
            desc: Color::Gray,
            muted: Color::DarkGray,
            accent: Color::White,
            warm: Color::Gray,
            key: Color::White,
            heading: Color::White,
            label: Color::Gray,
            info_label: Color::DarkGray,
            border: Color::DarkGray,
            select_bg: Color::White,
            marked_bg: Color::DarkGray,
            pill_bg: Color::DarkGray,
            panel_bg: Color::DarkGray,
            command: Color::White,
            ok: Color::White,
            warn: Color::Gray,
            bad: Color::White,
            highlight: Color::White,
            namespace: Color::Gray,
            container: Color::Gray,
            ..base
        },
        _ => return None,
    })
}

pub fn lookup_theme(name: &str) -> Option<Theme> {
    builtin(name).or_else(|| user_theme(name))
}

/// The built-in themes, in the order the picker lists them.
pub fn builtin_names() -> Vec<&'static str> {
    let mut names = vec!["knav", "steel", "high-contrast", "mono"];
    names.extend(PALETTES.iter().map(|(name, _)| *name));
    names
}

pub fn builtin(name: &str) -> Option<Theme> {
    // Names presets had before they were renamed.
    let name = match name {
        "solarized" => "solarized-dark",
        other => other,
    };
    transparent(name).or_else(|| PALETTES.iter().find(|(n, _)| *n == name).map(|(_, palette)| from_palette(palette)))
}

/// The folder for the user's own themes (`<name>.toml`).
pub fn themes_dir() -> std::path::PathBuf {
    crate::util::config_dir().join("themes")
}

/// A theme file: an optional `base` (a built-in or user theme to start
/// from), then colour roles either at the top or under `[colors]`.
pub fn theme_from_toml(text: &str, dir: &std::path::Path) -> Option<Theme> {
    let table: toml::Table = text.parse().ok()?;
    let mut theme = match table.get("base").and_then(|b| b.as_str()) {
        Some(base) => builtin(base).or_else(|| user_theme_in(dir, base))?,
        None => Theme::default(),
    };
    let colors = table.get("colors").and_then(|c| c.as_table()).unwrap_or(&table);
    for (role, value) in colors {
        if let Some(color) = value.as_str().and_then(parse_color) {
            theme.set(role, color);
        }
    }
    Some(theme)
}

pub fn user_theme_in(dir: &std::path::Path, name: &str) -> Option<Theme> {
    // A name is a file name, nothing else.
    if name.is_empty() || name.contains(['/', '\\', '.']) {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(format!("{name}.toml"))).ok()?;
    theme_from_toml(&text, dir)
}

pub fn user_theme(name: &str) -> Option<Theme> {
    user_theme_in(&themes_dir(), name)
}

/// The names of the user's theme files, sorted.
pub fn user_theme_names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "toml").then(|| path.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    names.sort();
    names
}

/// Every theme by name: the built-in ones, then the user's own.
pub fn all_names() -> Vec<String> {
    let mut names: Vec<String> = builtin_names().into_iter().map(str::to_string).collect();
    let builtins = names.clone();
    names.extend(user_theme_names_in(&themes_dir()).into_iter().filter(|n| !builtins.contains(n)));
    names
}

const NAMED: &[(&str, Color)] = &[
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("gray", Color::Gray),
    ("darkgray", Color::DarkGray),
    ("lightred", Color::LightRed),
    ("lightgreen", Color::LightGreen),
    ("lightyellow", Color::LightYellow),
    ("lightblue", Color::LightBlue),
    ("lightmagenta", Color::LightMagenta),
    ("lightcyan", Color::LightCyan),
    ("white", Color::White),
    ("reset", Color::Reset),
];

/// `#rrggbb`, `#rgb`, a terminal colour name (`red`, `darkgray`, ...) or
/// `indexed:N` (the 256-colour palette).
pub fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim().to_lowercase();
    if let Some(hex) = text.strip_prefix('#') {
        let expanded: String = if hex.len() == 3 { hex.chars().flat_map(|c| [c, c]).collect() } else { hex.to_string() };
        if expanded.len() != 6 || !expanded.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&expanded[i..i + 2], 16).ok();
        return Some(Color::Rgb(byte(0)?, byte(2)?, byte(4)?));
    }
    if let Some(index) = text.strip_prefix("indexed:") {
        return index.trim().parse::<u8>().ok().map(Color::Indexed);
    }
    NAMED.iter().find(|(name, _)| *name == text.replace([' ', '-', '_'], "")).map(|(_, color)| *color)
}

/// The text `parse_color` reads back: a name for terminal colours, else hex.
pub fn format_color(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => format!("indexed:{i}"),
        other => NAMED.iter().find(|(_, c)| *c == other).map(|(name, _)| (*name).to_string()).unwrap_or_else(|| "reset".into()),
    }
}

/// A preset with the config's per-role overrides on top. Unknown presets and
/// unparsable colours are skipped; the returned list says what was ignored.
pub fn build(preset_name: &str, overrides: &BTreeMap<String, String>) -> (Theme, Vec<String>) {
    let mut ignored = Vec::new();
    let mut theme = lookup_theme(preset_name).unwrap_or_else(|| {
        ignored.push(format!("unknown theme preset '{preset_name}'"));
        Theme::default()
    });
    for (role, value) in overrides {
        match parse_color(value) {
            Some(color) if theme.set(role, color) => {}
            Some(_) => ignored.push(format!("unknown colour role '{role}'")),
            None => ignored.push(format!("'{value}' is not a colour (role '{role}')")),
        }
    }
    (theme, ignored)
}

/// One theme in the picker, with its swatch colours.
pub struct ThemeEntry {
    pub name: String,
    pub swatch: Vec<ratatui::style::Color>,
}

/// The picker's rows, and where the current theme is among them.
pub fn theme_entries(current: &str) -> (Vec<ThemeEntry>, usize) {
    let entries: Vec<ThemeEntry> = all_names()
        .into_iter()
        .map(|name| {
            let t = crate::theme::lookup_theme(&name).unwrap_or_default();
            ThemeEntry { swatch: vec![t.background, t.foreground, t.header, t.ok, t.warn, t.bad, t.accent, t.container, t.select_bg], name }
        })
        .collect();
    let at = entries.iter().position(|e| e.name == current).unwrap_or(0);
    (entries, at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse_from_hex_names_and_palette_indexes() {
        assert_eq!(parse_color("#94bfce"), Some(Color::Rgb(0x94, 0xbf, 0xce)));
        assert_eq!(parse_color("#fa0"), Some(Color::Rgb(0xff, 0xaa, 0x00)));
        assert_eq!(parse_color("Dark Gray"), Some(Color::DarkGray));
        assert_eq!(parse_color("light-blue"), Some(Color::LightBlue));
        assert_eq!(parse_color("indexed:208"), Some(Color::Indexed(208)));
    }

    #[test]
    fn bad_colours_are_rejected() {
        for bad in ["", "#12", "#gggggg", "#1234567", "indexed:300", "chartreuse"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
    }

    #[test]
    fn formatting_round_trips() {
        for color in [Color::Rgb(1, 2, 3), Color::Indexed(9), Color::Red, Color::DarkGray] {
            assert_eq!(parse_color(&format_color(color)), Some(color));
        }
    }

    #[test]
    fn every_role_can_be_read_and_set_by_name() {
        let mut theme = Theme::default();
        for (role, _) in ROLES {
            assert!(theme.get(role).is_some(), "{role}");
            assert!(theme.set(role, Color::Rgb(9, 9, 9)), "{role}");
            assert_eq!(theme.get(role), Some(Color::Rgb(9, 9, 9)));
        }
        assert!(!theme.set("nonsense", Color::Red));
    }

    #[test]
    fn every_builtin_theme_exists_and_differs_from_the_default_except_the_default() {
        let names = builtin_names();
        assert!(names.len() >= 18, "{}", names.len());
        for name in names {
            let theme = builtin(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(theme == Theme::default(), name == "knav", "{name}");
        }
        assert!(builtin("nope").is_none());
        assert_eq!(builtin("solarized"), builtin("solarized-dark"), "the old name still works");
    }

    #[test]
    fn palette_themes_paint_their_background_and_keep_text_readable() {
        for name in builtin_names().into_iter().filter(|n| PALETTES.iter().any(|(p, _)| p == n)) {
            let theme = builtin(name).unwrap();
            assert_ne!(theme.background, Color::Reset, "{name}");
            // Text must stand clear of the background.
            let gap = (luminance(theme.foreground) - luminance(theme.background)).abs();
            assert!(gap > 0.3, "{name}: text vs background {gap}");
        }
        assert!(luminance(builtin("gruvbox-light").unwrap().background) > 0.5);
        assert!(luminance(builtin("dracula").unwrap().background) < 0.2);
    }

    #[test]
    fn text_on_a_colour_is_black_or_white_by_brightness() {
        assert_eq!(on(Color::Rgb(250, 240, 200)), Color::Black);
        assert_eq!(on(Color::Rgb(40, 42, 54)), Color::White);
        assert_eq!(on(Color::Yellow), Color::Black);
        assert_eq!(on(Color::Blue), Color::White);
    }

    #[test]
    fn a_user_theme_file_can_start_from_a_builtin_and_override_roles() {
        let dir = std::env::temp_dir().join(format!("knav-themes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.toml"), "base = \"dracula\"\n[colors]\nok = \"#010203\"\nnonsense = \"red\"\nbad = \"not-a-colour\"\n").unwrap();
        std::fs::write(dir.join("flat.toml"), "background = \"#101010\"\n").unwrap();
        let mine = user_theme_in(&dir, "mine").unwrap();
        assert_eq!(mine.ok, Color::Rgb(1, 2, 3));
        assert_eq!(mine.background, builtin("dracula").unwrap().background, "inherited");
        assert_eq!(mine.bad, builtin("dracula").unwrap().bad, "a bad colour is ignored");
        assert_eq!(user_theme_in(&dir, "flat").unwrap().background, Color::Rgb(16, 16, 16));
        assert_eq!(user_theme_names_in(&dir), ["flat", "mine"]);
        assert!(user_theme_in(&dir, "../etc").is_none(), "a name is not a path");
        assert!(user_theme_in(&dir, "missing").is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn overrides_sit_on_top_of_the_preset_and_bad_ones_are_reported() {
        let overrides: BTreeMap<String, String> = [("ok", "#00ff00"), ("bad", "chartreuse"), ("wat", "red")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let (theme, ignored) = build("steel", &overrides);
        assert_eq!(theme.ok, Color::Rgb(0, 255, 0));
        assert_eq!(theme.heading, builtin("steel").unwrap().heading);
        assert_eq!(ignored.len(), 2);
        let (fallback, ignored) = build("nope", &BTreeMap::new());
        assert_eq!(fallback, Theme::default());
        assert_eq!(ignored.len(), 1);
    }
}
