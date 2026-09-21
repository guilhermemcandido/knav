//! Key bindings: every action has a name, its screens and default keys, and the
//! config's `[keys]` table replaces them. Handlers match built-in keys, so the keymap
//! translates your key into the built-in one. Two actions on a screen can't share a key.

use std::collections::{BTreeMap, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::k8s::ResourceKind;
use crate::app::mode::Mode;

/// A screen that has its own keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Screen {
    List,
    Overview,
    Column,
    Events,
    Namespaces,
    Contexts,
    Containers,
    NodeDetail,
    Logs,
    Spec,
    Yaml,
    Settings,
    Themes,
    /// Read-only popups: they only have the shared keys.
    Other,
}

use Screen::*;

const ALL: &[Screen] = &[List, Overview, Column, Events, Namespaces, Contexts, Containers, NodeDetail, Logs, Spec, Yaml, Settings, Themes, Other];
const NAV: &[Screen] = &[List, Overview, Column, Events, Namespaces, Contexts, Containers, NodeDetail, Logs, Spec, Yaml, Settings, Themes];
const TABLES: &[Screen] = &[List, Events, Namespaces, Contexts, Containers, NodeDetail, Themes, Settings];
const SORTABLE: &[Screen] = &[List, Events, Namespaces, Contexts, Containers, NodeDetail];
const SEARCHABLE: &[Screen] = &[List, Events, Namespaces, Contexts, NodeDetail, Logs];

/// One bindable action.
pub struct Binding {
    pub id: &'static str,
    pub label: &'static str,
    pub screens: &'static [Screen],
    pub defaults: &'static [&'static str],
}

macro_rules! bindings {
    ($(($id:literal, $label:literal, $screens:expr, $defaults:expr)),+ $(,)?) => {
        pub const BINDINGS: &[Binding] = &[$(Binding { id: $id, label: $label, screens: $screens, defaults: $defaults }),+];
    };
}

bindings! {
    ("help", "Help", ALL, &["?"]),
    ("command", "Command line", ALL, &[":"]),
    ("quit", "Quit knav", ALL, &["Q"]),
    ("home", "Go to Home", ALL, &["H"]),
    ("contexts", "Contexts", ALL, &["C"]),
    ("back", "Back", ALL, &["q"]),
    ("cancel", "Cancel / clear marks", ALL, &["esc"]),
    ("move_down", "Move down", NAV, &["j", "down"]),
    ("move_up", "Move up", NAV, &["k", "up"]),
    ("move_left", "Move left", &[Overview, Column, Settings], &["h", "left"]),
    ("move_right", "Move right", &[Overview, Column, Settings], &["l", "right"]),
    ("top", "Go to top", &[List, Events, Namespaces, Contexts, Containers, NodeDetail, Themes, Settings, Yaml], &["g", "home"]),
    ("bottom", "Go to bottom", &[List, Events, Namespaces, Contexts, Containers, NodeDetail, Themes, Settings, Yaml], &["G", "end"]),
    ("page_down", "Page down", TABLES, &["ctrl-f", "pagedown"]),
    ("page_up", "Page up", TABLES, &["ctrl-b", "pageup"]),
    ("open", "Open / drill in", &[List, Overview, Events, NodeDetail, Column, Themes], &["enter"]),
    ("select", "Select", &[Namespaces, Contexts], &["enter"]),
    ("search", "Search / filter", SEARCHABLE, &["/", "f"]),
    ("sort", "Sort by column", SORTABLE, &["s"]),
    ("sort_age", "Sort by age", &[List], &["A"]),
    ("spec", "Show the spec", &[List, NodeDetail], &["d"]),
    ("edit", "Edit", &[List, NodeDetail], &["e"]),
    ("yaml", "YAML view", &[List], &["y"]),
    ("copy_name", "Copy the name", &[List], &["Y"]),
    ("delete", "Delete", &[List], &["D"]),
    ("scale_or_shell", "Scale / shell", &[List], &["S"]),
    ("restart", "Restart", &[List], &["r"]),
    ("cordon", "Cordon a node", &[List], &["c"]),
    ("trigger", "Trigger a CronJob", &[List], &["t"]),
    ("suspend", "Suspend a CronJob", &[List], &["u"]),
    ("forward", "Port-forward", &[List], &["F"]),
    ("decode", "Decode a secret", &[List], &["x"]),
    ("pod_logs", "Pod logs", &[List], &["l"]),
    ("previous_logs", "Previous logs", &[List, Containers], &["p"]),
    ("owner", "Jump to the owner", &[List], &["O"]),
    ("details", "Info about the object", &[List], &["i"]),
    ("related", "Related objects", &[List], &["R"]),
    ("mark", "Mark the row", &[List], &["space"]),
    ("open_browser", "Open in the browser", &[List], &["o"]),
    ("namespaces", "Namespaces", &[List, Overview], &["n"]),
    ("menu", "Show or hide the sidebar", &[List, Overview], &["b", "m"]),
    ("themes", "Themes", &[List, Overview], &["T"]),
    ("settings", "Settings", &[List, Overview], &[","]),
    ("history_back", "History back", &[List], &["["]),
    ("history_forward", "History forward", &[List], &["]"]),
    ("last_view", "Last view", &[List], &["-"]),
    ("faults", "Faults only", &[List], &["ctrl-z"]),
    ("wide", "Wide columns", &[List], &["ctrl-w"]),
}

/// A key as the keymap compares them: the code and Ctrl/Alt. Shift is folded
/// into the character (`D` is shift-d), and is kept only for non-characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeySpec {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeySpec {
    pub fn of(event: &KeyEvent) -> KeySpec {
        let mut modifiers = event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        if matches!(event.code, KeyCode::Char(_)) {
            modifiers -= KeyModifiers::SHIFT;
        }
        // Terminals send Shift-Tab as BackTab already.
        KeySpec { code: event.code, modifiers }
    }

    pub fn to_event(self) -> KeyEvent {
        KeyEvent::new(self.code, self.modifiers)
    }
}

const NAMED_KEYS: &[(&str, KeyCode)] = &[
    ("enter", KeyCode::Enter),
    ("esc", KeyCode::Esc),
    ("tab", KeyCode::Tab),
    ("backtab", KeyCode::BackTab),
    ("space", KeyCode::Char(' ')),
    ("backspace", KeyCode::Backspace),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
];

/// `j`, `D`, `ctrl-z`, `alt-x`, `enter`, `f5`, `space`, ... Digits are
/// reserved for the namespace and sort shortcuts.
pub fn parse_key(text: &str) -> Result<KeySpec, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("empty key".into());
    }
    let mut modifiers = KeyModifiers::NONE;
    let mut rest = text;
    loop {
        let lower = rest.to_lowercase();
        let (flag, len) = if lower.starts_with("ctrl-") {
            (KeyModifiers::CONTROL, 5)
        } else if lower.starts_with("alt-") {
            (KeyModifiers::ALT, 4)
        } else if lower.starts_with("shift-") && rest.len() > 6 {
            (KeyModifiers::SHIFT, 6)
        } else {
            break;
        };
        modifiers |= flag;
        rest = &rest[len..];
    }
    let lower = rest.to_lowercase();
    let code = if let Some((_, code)) = NAMED_KEYS.iter().find(|(name, _)| *name == lower) {
        *code
    } else if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
        KeyCode::F(n)
    } else {
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                if c.is_ascii_digit() {
                    return Err(format!("'{c}' is reserved for the number shortcuts"));
                }
                // Ctrl-Z and ctrl-z are the same key.
                let c = if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && c.is_ascii_alphabetic() && !modifiers.contains(KeyModifiers::SHIFT) { c.to_ascii_lowercase() } else { c };
                // shift-x is the capital letter.
                if modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_alphabetic() {
                    modifiers -= KeyModifiers::SHIFT;
                    KeyCode::Char(c.to_ascii_uppercase())
                } else {
                    KeyCode::Char(c)
                }
            }
            _ => return Err(format!("'{text}' is not a key (try j, D, ctrl-z, enter, f5, space)")),
        }
    };
    if matches!(code, KeyCode::Char(_)) {
        modifiers -= KeyModifiers::SHIFT;
    }
    Ok(KeySpec { code, modifiers })
}

/// The text `parse_key` reads back.
pub fn format_key(key: KeySpec) -> String {
    let mut out = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        out.push_str("ctrl-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        out.push_str("alt-");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        out.push_str("shift-");
    }
    match key.code {
        KeyCode::F(n) => out.push_str(&format!("f{n}")),
        KeyCode::Char(' ') => out.push_str("space"),
        KeyCode::Char(c) => out.push(c),
        code => out.push_str(NAMED_KEYS.iter().find(|(_, c)| *c == code).map(|(name, _)| *name).unwrap_or("?")),
    }
    out
}

fn defaults_of(binding: &Binding) -> Vec<KeySpec> {
    binding.defaults.iter().map(|k| parse_key(k).expect("built-in keys are valid")).collect()
}

/// The screen a mode's keys belong to, or `None` where keys are text being
/// typed (or fixed prompts), which are never remapped.
pub fn screen_of(mode: &Mode, kind: ResourceKind) -> Option<Screen> {
    Some(match mode {
        Mode::List if kind == ResourceKind::Overview => Overview,
        Mode::List => List,
        Mode::ColumnDetail { .. } => Column,
        Mode::Events { editing: false, .. } => Events,
        Mode::NamespacePick { editing: false, .. } => Namespaces,
        Mode::Context { editing: false, .. } => Contexts,
        Mode::Containers { .. } => Containers,
        Mode::NodeDetail { editing: false, .. } => NodeDetail,
        Mode::Logs { filter_editing: false, .. } => Logs,
        Mode::Spec { viewing: None, .. } => Spec,
        Mode::Yaml { .. } => Yaml,
        Mode::Settings { editing: None, capture: None, .. } => Settings,
        Mode::ThemePicker { .. } => Themes,
        Mode::EventDetail { .. } | Mode::ResourcesDetail | Mode::Relations { .. } | Mode::Details { .. } => Other,
        _ => return None,
    })
}

fn overlaps(a: &Binding, b: &Binding) -> bool {
    a.screens.iter().any(|s| b.screens.contains(s))
}

/// The keys in effect: the user's, else the defaults, per action id.
pub type Overrides = BTreeMap<String, Vec<KeySpec>>;

/// Who already uses `key` on a screen `binding` shares, other than `binding`.
pub fn conflict<'a>(overrides: &Overrides, binding: &Binding, key: KeySpec) -> Option<&'a Binding> {
    BINDINGS.iter().filter(|other| other.id != binding.id && overlaps(binding, other)).find(|other| {
        let effective = overrides.get(other.id).cloned().unwrap_or_else(|| defaults_of(other));
        effective.contains(&key)
    })
}

/// The user's keys from the config, checked: an action whose keys don't
/// parse, or collide with another action on a shared screen, keeps its
/// defaults. Returns what was ignored and why.
pub fn overrides_from_config(keys: &BTreeMap<String, Vec<String>>) -> (Overrides, Vec<String>) {
    let mut candidates = Overrides::new();
    let mut problems = Vec::new();
    for binding in BINDINGS {
        let Some(texts) = keys.get(binding.id) else { continue };
        let parsed: Result<Vec<KeySpec>, String> = texts.iter().map(|t| parse_key(t)).collect();
        match parsed {
            Ok(list) if list.is_empty() => problems.push(format!("keys.{}: no keys given", binding.id)),
            Ok(list) => {
                if let Some(twice) = list.iter().enumerate().find(|(i, k)| list[..*i].contains(k)).map(|(_, k)| *k) {
                    problems.push(format!("keys.{}: '{}' is listed twice", binding.id, format_key(twice)));
                } else {
                    candidates.insert(binding.id.to_string(), list);
                }
            }
            Err(e) => problems.push(format!("keys.{}: {e}", binding.id)),
        }
    }
    // Checked against the final set, so two actions can swap keys; when two
    // really collide, the one later in the list loses and keeps its default.
    for binding in BINDINGS {
        let Some(list) = candidates.get(binding.id).cloned() else { continue };
        let clash = list.iter().find_map(|key| conflict(&candidates, binding, *key).map(|other| (*key, other.label)));
        if let Some((key, label)) = clash {
            problems.push(format!("keys.{}: '{}' is already '{}'", binding.id, format_key(key), label));
            candidates.remove(binding.id);
        }
    }
    for id in keys.keys().filter(|id| !BINDINGS.iter().any(|b| b.id == id.as_str())) {
        problems.push(format!("keys.{id}: no such action"));
    }
    (candidates, problems)
}

use std::sync::RwLock;

static CURRENT: RwLock<Option<Keymap>> = RwLock::new(None);

/// Makes `keymap` the one the UI reads when it names keys (help, hints).
pub fn set_current(keymap: &Keymap) {
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(keymap.clone());
    }
}

/// The keys in effect for an action, for the UI to name.
pub fn keys_now(id: &str) -> Vec<String> {
    match CURRENT.read().ok().as_deref() {
        Some(Some(keymap)) => keymap.keys_of(id),
        _ => BINDINGS.iter().find(|b| b.id == id).map(|b| b.defaults.iter().map(|k| k.to_string()).collect()).unwrap_or_default(),
    }
}

/// A key named for people: arrows as arrows.
pub fn glyph(key: &str) -> String {
    match key {
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        other => other.to_string(),
    }
}

/// What one screen does with each key it is sent.
#[derive(Clone, Debug, Default)]
pub struct Keymap {
    /// The key you pressed -> the built-in key to hand the handler, or `None`
    /// when that key belongs to an action you moved elsewhere.
    tables: HashMap<Screen, HashMap<KeySpec, Option<KeySpec>>>,
    overrides: Overrides,
}

impl Keymap {
    pub fn new(overrides: Overrides) -> Keymap {
        let mut tables: HashMap<Screen, HashMap<KeySpec, Option<KeySpec>>> = HashMap::new();
        for screen in ALL {
            let table = tables.entry(*screen).or_default();
            let here: Vec<&Binding> = BINDINGS.iter().filter(|b| b.screens.contains(screen)).collect();
            // First what the user's keys turn into...
            for binding in &here {
                let defaults = defaults_of(binding);
                let effective = overrides.get(binding.id).cloned().unwrap_or_else(|| defaults.clone());
                for (i, key) in effective.iter().enumerate() {
                    table.insert(*key, Some(defaults[i.min(defaults.len() - 1)]));
                }
            }
            // ...then the built-in keys nobody claimed any more.
            for binding in &here {
                let effective = overrides.get(binding.id).cloned().unwrap_or_else(|| defaults_of(binding));
                for default in defaults_of(binding) {
                    if !effective.contains(&default) {
                        table.entry(default).or_insert(None);
                    }
                }
            }
        }
        Keymap { tables, overrides }
    }

    pub fn from_config(keys: &BTreeMap<String, Vec<String>>) -> (Keymap, Vec<String>) {
        let (overrides, problems) = overrides_from_config(keys);
        (Keymap::new(overrides), problems)
    }

    /// From the whole config (its `[keys]` table).
    pub fn from_app_config(config: &crate::config::Config) -> (Keymap, Vec<String>) {
        Keymap::from_config(&config.keys)
    }

    /// The key event to hand the handler for `event` on `screen`; `None` to drop it.
    pub fn translate(&self, screen: Screen, event: &KeyEvent) -> Option<KeyEvent> {
        match self.tables.get(&screen).and_then(|t| t.get(&KeySpec::of(event))) {
            Some(Some(built_in)) => Some(built_in.to_event()),
            Some(None) => None,
            None => Some(*event),
        }
    }

    /// The keys in effect for an action, as config text.
    pub fn keys_of(&self, id: &str) -> Vec<String> {
        match self.overrides.get(id) {
            Some(keys) => keys.iter().map(|k| format_key(*k)).collect(),
            None => BINDINGS.iter().find(|b| b.id == id).map(|b| b.defaults.iter().map(|k| k.to_string()).collect()).unwrap_or_default(),
        }
    }

    /// What a built-in key (as the hints and help name it) is called now on
    /// `screen`: the keys of the action it belongs to, or itself if unbound.
    pub fn display(&self, screen: Screen, built_in: &str) -> String {
        let Ok(key) = parse_key(built_in) else { return built_in.to_string() };
        for binding in BINDINGS.iter().filter(|b| b.screens.contains(&screen)) {
            let defaults = defaults_of(binding);
            if let Some(at) = defaults.iter().position(|d| *d == key) {
                let effective = self.keys_of(binding.id);
                return match self.overrides.get(binding.id) {
                    // Position by position: the n-th default is now the n-th key.
                    Some(_) => effective.get(at.min(effective.len() - 1)).cloned().unwrap_or_else(|| built_in.to_string()),
                    None => built_in.to_string(),
                };
            }
        }
        built_in.to_string()
    }

    /// Every hint key, split on `/`, shown with the keys now in effect.
    pub fn display_hint(&self, screen: Screen, hint: &str) -> String {
        if hint == "/" || hint.is_empty() {
            return self.display(screen, hint);
        }
        hint.split('/').map(|token| self.display(screen, token)).collect::<Vec<_>>().join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn keys(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs.iter().map(|(id, list)| (id.to_string(), list.iter().map(|k| k.to_string()).collect())).collect()
    }

    #[test]
    fn the_built_in_keys_never_collide_on_a_screen() {
        for (i, a) in BINDINGS.iter().enumerate() {
            for b in &BINDINGS[i + 1..] {
                if !overlaps(a, b) {
                    continue;
                }
                for key in defaults_of(a) {
                    assert!(!defaults_of(b).contains(&key), "{} and {} both use {}", a.id, b.id, format_key(key));
                }
            }
        }
    }

    #[test]
    fn action_ids_are_unique_and_have_keys() {
        let mut ids: Vec<&str> = BINDINGS.iter().map(|b| b.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BINDINGS.len());
        assert!(BINDINGS.iter().all(|b| !b.defaults.is_empty() && !b.screens.is_empty()));
    }

    #[test]
    fn keys_parse_and_format_round_trip() {
        for text in ["j", "D", "ctrl-z", "alt-x", "enter", "esc", "space", "f5", "pagedown", "ctrl-f", "backtab", "[", ",", "-", ":", "/"] {
            let key = parse_key(text).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert_eq!(parse_key(&format_key(key)).unwrap(), key, "{text}");
        }
        assert_eq!(parse_key("shift-d").unwrap(), parse_key("D").unwrap());
        assert_eq!(parse_key("Ctrl-Z").unwrap(), parse_key("ctrl-z").unwrap());
    }

    #[test]
    fn bad_keys_are_rejected_and_digits_are_reserved() {
        for bad in ["", "ctrl-", "nothing", "f13", "5", "ctrl-5"] {
            assert!(parse_key(bad).is_err(), "{bad}");
        }
        assert!(parse_key("5").unwrap_err().contains("reserved"));
    }

    #[test]
    fn shift_is_folded_into_the_letter_when_a_key_event_arrives() {
        let event = KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT);
        assert_eq!(KeySpec::of(&event), parse_key("D").unwrap());
    }

    #[test]
    fn with_no_config_every_key_passes_through() {
        let map = Keymap::new(Overrides::new());
        for code in [KeyCode::Char('j'), KeyCode::Char('D'), KeyCode::Enter, KeyCode::Esc, KeyCode::Char('7')] {
            assert_eq!(map.translate(List, &press(code)), Some(press(code)));
        }
    }

    #[test]
    fn a_rebound_action_answers_to_its_new_key_and_the_old_one_goes_quiet() {
        let (map, problems) = Keymap::from_config(&keys(&[("delete", &["X"])]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.translate(List, &press(KeyCode::Char('X'))), Some(press(KeyCode::Char('D'))));
        assert_eq!(map.translate(List, &press(KeyCode::Char('D'))), None, "D no longer deletes");
        // Other screens are untouched.
        assert_eq!(map.translate(Events, &press(KeyCode::Char('D'))), Some(press(KeyCode::Char('D'))));
    }

    #[test]
    fn several_keys_map_position_by_position() {
        let (map, problems) = Keymap::from_config(&keys(&[("move_down", &["z", "down"])]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.translate(List, &press(KeyCode::Char('z'))), Some(press(KeyCode::Char('j'))));
        assert_eq!(map.translate(List, &press(KeyCode::Down)), Some(press(KeyCode::Down)));
        assert_eq!(map.translate(List, &press(KeyCode::Char('j'))), None);
    }

    #[test]
    fn swapping_two_actions_keys_works() {
        let (map, problems) = Keymap::from_config(&keys(&[("delete", &["r"]), ("restart", &["D"])]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.translate(List, &press(KeyCode::Char('r'))), Some(press(KeyCode::Char('D'))));
        assert_eq!(map.translate(List, &press(KeyCode::Char('D'))), Some(press(KeyCode::Char('r'))));
    }

    #[test]
    fn a_key_another_action_uses_on_the_same_screen_is_refused() {
        let (map, problems) = Keymap::from_config(&keys(&[("delete", &["r"])]));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("already 'Restart'"), "{problems:?}");
        assert_eq!(map.translate(List, &press(KeyCode::Char('D'))), Some(press(KeyCode::Char('D'))), "the default stays");
    }

    #[test]
    fn popup_letters_are_not_bindable() {
        // The events popup's `w` / `n` / `a` and the like are fixed.
        let (_, problems) = Keymap::from_config(&keys(&[("filter_warnings", &["m"])]));
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn a_global_key_may_not_take_a_key_used_anywhere() {
        let (_, problems) = Keymap::from_config(&keys(&[("help", &["D"])]));
        assert_eq!(problems.len(), 1, "help is on every screen, D deletes on the list");
    }

    #[test]
    fn bad_config_entries_are_reported_and_ignored() {
        let (map, problems) = Keymap::from_config(&keys(&[("delete", &["nonsense"]), ("nope", &["x"]), ("yaml", &[]), ("copy_name", &["Z", "Z"])]));
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert_eq!(map.keys_of("delete"), ["D"]);
    }

    #[test]
    fn hints_show_the_keys_in_effect() {
        let (map, _) = Keymap::from_config(&keys(&[("delete", &["X"]), ("move_down", &["z", "down"])]));
        assert_eq!(map.display(List, "D"), "X");
        assert_eq!(map.display(List, "s"), "s", "untouched");
        assert_eq!(map.display_hint(List, "j/down"), "z/down");
        assert_eq!(map.display(List, "zzz"), "zzz");
    }

    #[test]
    fn screens_that_take_text_are_not_remapped() {
        assert_eq!(screen_of(&Mode::Search, ResourceKind::Pods), None);
        assert_eq!(screen_of(&Mode::List, ResourceKind::Overview), Some(Overview));
        assert_eq!(screen_of(&Mode::List, ResourceKind::Pods), Some(List));
    }
}
