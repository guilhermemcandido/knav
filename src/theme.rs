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
    on_select: "Text on the selection bar", Color::Black;
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

/// The theme in use.
pub fn theme() -> Theme {
    THEME.read().ok().and_then(|t| *t).unwrap_or_default()
}

/// Makes `theme` the one in use, from the next frame on.
pub fn set_theme(theme: Theme) {
    if let Ok(mut current) = THEME.write() {
        *current = Some(theme);
    }
}

/// The built-in themes.
pub const PRESETS: &[&str] = &["knav", "k9s", "high-contrast", "solarized", "mono"];

pub fn preset(name: &str) -> Option<Theme> {
    let base = Theme::default();
    Some(match name {
        "knav" => base,
        "k9s" => Theme {
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
            on_select: Color::Black,
            marked_bg: Color::Rgb(90, 90, 0),
            command: Color::Rgb(255, 255, 0),
            ok: Color::Rgb(0, 255, 0),
            warn: Color::Rgb(255, 200, 0),
            bad: Color::Rgb(255, 60, 60),
            dim: Color::Rgb(90, 90, 90),
            ..base
        },
        "solarized" => Theme {
            row: Color::Rgb(147, 161, 161),
            header: Color::Rgb(38, 139, 210),
            text_strong: Color::Rgb(238, 232, 213),
            text_soft: Color::Rgb(147, 161, 161),
            desc: Color::Rgb(131, 148, 150),
            muted: Color::Rgb(101, 123, 131),
            accent: Color::Rgb(42, 161, 152),
            warm: Color::Rgb(203, 75, 22),
            key: Color::Rgb(38, 139, 210),
            heading: Color::Rgb(133, 153, 0),
            label: Color::Rgb(203, 75, 22),
            info_label: Color::Rgb(101, 123, 131),
            border: Color::Rgb(88, 110, 117),
            select_bg: Color::Rgb(42, 161, 152),
            on_select: Color::Rgb(0, 43, 54),
            marked_bg: Color::Rgb(88, 110, 0),
            pill_bg: Color::Rgb(7, 54, 66),
            command: Color::Rgb(181, 137, 0),
            ok: Color::Rgb(133, 153, 0),
            warn: Color::Rgb(181, 137, 0),
            bad: Color::Rgb(220, 50, 47),
            highlight: Color::Rgb(181, 137, 0),
            namespace: Color::Rgb(42, 161, 152),
            container: Color::Rgb(211, 54, 130),
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
            on_select: Color::Black,
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
    let mut theme = preset(preset_name).unwrap_or_else(|| {
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
    fn every_preset_exists_and_differs_from_the_default_except_the_default() {
        for name in PRESETS {
            let theme = preset(name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(theme == Theme::default(), *name == "knav", "{name}");
        }
        assert!(preset("nope").is_none());
    }

    #[test]
    fn overrides_sit_on_top_of_the_preset_and_bad_ones_are_reported() {
        let overrides: BTreeMap<String, String> = [("ok", "#00ff00"), ("bad", "chartreuse"), ("wat", "red")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let (theme, ignored) = build("k9s", &overrides);
        assert_eq!(theme.ok, Color::Rgb(0, 255, 0));
        assert_eq!(theme.heading, preset("k9s").unwrap().heading);
        assert_eq!(ignored.len(), 2);
        let (fallback, ignored) = build("nope", &BTreeMap::new());
        assert_eq!(fallback, Theme::default());
        assert_eq!(ignored.len(), 1);
    }
}
