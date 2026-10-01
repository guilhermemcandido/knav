//! The `?` help: columns for this screen's keys, general keys, navigation and namespace
//! hotkeys, each column as wide as what it holds.

use super::*;

pub(super) struct Section {
    pub title: &'static str,
    pub entries: Vec<(String, String)>,
}

const NAVIGATION_KEYS: [&str; 5] = ["↑↓", "g/G", "hjkl", "←↑↓→", "jk"];
const GENERAL_KEYS: [&str; 15] = ["?", "n", "0-9", "s", "A", "/", "m", "b/m", "C", "E", "T", ",", "q/esc", "esc", "space"];

/// Screen hints are lowercase ("containers"); help reads as a list of labels.
fn capitalized(what: &str) -> String {
    let mut chars = what.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Splits the screen's `hints` into help columns and adds the keys that work everywhere.
/// The Overview is the one screen with no hints.
pub(super) fn help_sections(hints: &[(&str, &str)], slots: &[Option<String>]) -> Vec<Section> {
    let entry = |key: &str, what: &str| (key.to_string(), what.to_string());
    // An action's keys as they are now: `default` when untouched.
    let shown = |default: &str, id: &str, what: &str| {
        let keys = crate::input::keymap::keys_now(id);
        let untouched = crate::input::keymap::BINDINGS.iter().find(|b| b.id == id).is_some_and(|b| keys.iter().map(String::as_str).eq(b.defaults.iter().copied()));
        (if untouched { default.to_string() } else { keys.iter().map(|k| crate::input::keymap::glyph(k)).collect::<Vec<_>>().join(" / ") }, what.to_string())
    };
    // Two actions' keys side by side.
    let pair = |default: &str, first: &str, second: &str, what: &str| {
        let a = crate::input::keymap::keys_now(first);
        let b = if second.is_empty() { Vec::new() } else { crate::input::keymap::keys_now(second) };
        let untouched = |id: &str, keys: &[String]| crate::input::keymap::BINDINGS.iter().find(|x| x.id == id).is_none_or(|x| keys.iter().map(String::as_str).eq(x.defaults.iter().copied()));
        let both_default = untouched(first, &a) && (second.is_empty() || untouched(second, &b));
        let text = if both_default { default.to_string() } else { a.iter().chain(b.iter()).map(|k| crate::input::keymap::glyph(k)).collect::<Vec<_>>().join(" ") };
        (text, what.to_string())
    };
    let mut namespaces = vec![entry("0", "All namespaces")];
    namespaces.extend(slots.iter().enumerate().filter_map(|(i, ns)| ns.as_ref().map(|ns| (format!("{}", i + 1), ns.clone()))));
    let namespaces = Section { title: "NAMESPACES", entries: namespaces };

    // The Overview has no rows to filter, sort or mark, so moving between tiles is
    // its whole navigation, said once.
    if hints.is_empty() {
        return vec![
            Section {
                title: "OVERVIEW",
                entries: vec![entry("hjkl / arrows", "Move between tiles"), entry("enter", "Open the selected tile"), entry("click", "Select a tile"), entry("double-click", "Open a tile"), entry("wheel", "Scroll")],
            },
            Section {
                title: "GENERAL",
                entries: vec![
                    shown(":", "command", "Command mode"),
                    shown("n", "namespaces", "Pick a namespace"),
                    shown("C", "contexts", "Switch context"),
                    shown("E", "extensions", "Extensions"),
                    shown("T", "themes", "Themes"),
                    shown(",", "settings", "Settings"),
                    shown("P", "permissions", "Permissions"),
                    shown("!", "problems", "Problems"),
                    shown("b / m", "menu", "Toggle the sidebar"),
                    shown("?", "help", "This help"),
                    shown("Q", "quit", "Quit knav"),
                ],
            },
            namespaces,
        ];
    }
    let resource: Vec<(String, String)> = hints
        .iter()
        .filter(|(key, _)| !GENERAL_KEYS.contains(key) && !NAVIGATION_KEYS.iter().any(|n| key.contains(n)))
        .map(|(key, what)| entry(key, &capitalized(what)))
        .collect();
    let mut sections = vec![
        Section { title: "THIS SCREEN", entries: resource },
        Section {
            title: "GENERAL",
            entries: vec![
                shown(":", "command", "Command mode"),
                shown("/", "search", "Filter"),
                shown("s", "sort", "Sort by column"),
                shown("A", "sort_age", "Sort by age"),
                shown("space", "mark", "Mark a row"),
                shown("ctrl-z", "faults", "Faults only"),
                shown("ctrl-w", "wide", "Wide columns"),
                shown("n", "namespaces", "Pick a namespace"),
                shown("C", "contexts", "Switch context"),
                shown("E", "extensions", "Extensions"),
                shown("T", "themes", "Themes"),
                shown(",", "settings", "Settings"),
                shown("P", "permissions", "Permissions"),
                shown("!", "problems", "Problems"),
                shown("b / m", "menu", "Toggle the sidebar"),
                shown("?", "help", "This help"),
                shown("Q", "quit", "Quit knav"),
            ],
        },
        Section {
            title: "NAVIGATION",
            entries: vec![
                pair("j / ↓", "move_down", "", "Down"),
                pair("k / ↑", "move_up", "", "Up"),
                shown("g", "top", "Top"),
                shown("G", "bottom", "Bottom"),
                shown("ctrl-f", "page_down", "Page down"),
                shown("ctrl-b", "page_up", "Page up"),
                entry("← →", "Scroll columns"),
                entry("enter", "Open"),
                shown("esc", "cancel", "Back / clear marks"),
                pair("[ ]", "history_back", "history_forward", "History back / forward"),
                shown("minus", "last_view", "Last view"),
                shown("H", "home", "Home"),
                entry("click", "Select a row"),
                entry("double-click", "Open a row"),
                entry("wheel", "Scroll"),
            ],
        },
        namespaces,
    ];
    // A dashboard has nothing of its own beyond scrolling.
    sections.retain(|s| !s.entries.is_empty());
    sections
}

const KEY_GAP: usize = 2;
const COLUMN_GAP: u16 = 4;

fn key_width(section: &Section) -> usize {
    section.entries.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0)
}

fn column_width(section: &Section) -> u16 {
    let what = section.entries.iter().map(|(_, w)| w.chars().count()).max().unwrap_or(0);
    (key_width(section) + KEY_GAP + what).max(section.title.chars().count()) as u16
}

/// Draws the help centred, each column as wide as its content. On a terminal too
/// narrow for that, the columns share the room in proportion.
pub(super) fn draw_help(frame: &mut Frame, hints: &[(&str, &str)], slots: &[Option<String>], shortcuts_line: bool) {
    // The same body area the page uses.
    let bounds = body_area(frame.area(), shortcuts_line);
    let sections = help_sections(hints, slots);
    let widths: Vec<u16> = sections.iter().map(column_width).collect();
    let content_width = widths.iter().sum::<u16>() + COLUMN_GAP * (sections.len() as u16).saturating_sub(1);
    let content_height = sections.iter().map(|s| s.entries.len()).max().unwrap_or(0) as u16;
    let height = (content_height + 5 /* heading, blank line, borders, bottom padding */).min(bounds.height);
    let width = (content_width + 6 /* borders and two columns of padding a side */).min(bounds.width);
    let area = Rect { x: bounds.x + bounds.width.saturating_sub(width) / 2, y: bounds.y + bounds.height.saturating_sub(height) / 2, width, height };
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title_centered("Help", false))
        .title_bottom(hint_strip(&[("esc", "close")]).right_aligned())
        .padding(Padding::new(2, 2, 1, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let fits = content_width <= inner.width;
    let constraints: Vec<Constraint> = widths.iter().map(|w| if fits { Constraint::Length(*w) } else { Constraint::Fill(*w) }).collect();
    let columns = Layout::horizontal(constraints).spacing(if fits { COLUMN_GAP } else { 2 }).split(inner);
    for (section, column) in sections.iter().zip(columns.iter()) {
        let key_width = key_width(section) + KEY_GAP;
        let mut lines = vec![Line::styled(section.title, Style::default().fg(theme().heading).add_modifier(Modifier::BOLD)), Line::default()];
        lines.extend(section.entries.iter().map(|(key, what)| {
            let pad = " ".repeat(key_width.saturating_sub(key.chars().count()));
            Line::from(vec![
                Span::styled(key.clone(), Style::default().fg(theme().key).add_modifier(Modifier::BOLD)),
                Span::raw(pad),
                Span::styled(what.clone(), Style::default().fg(theme().desc)),
            ])
        }));
        frame.render_widget(Paragraph::new(lines), *column);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_specific_keys_go_under_resource_and_common_ones_do_not_repeat() {
        let hints = [("↑↓/jk", "move"), ("enter", "containers"), ("d", "spec"), ("D", "delete"), ("n", "namespaces"), ("C", "contexts"), ("E", "extensions"), ("T", "themes"), (",", "settings"), ("q/esc", "back")];
        let sections = help_sections(&hints, &[]);
        let resource: Vec<&str> = sections[0].entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(resource, ["enter", "d", "D"]);
    }

    #[test]
    fn the_overview_says_how_to_move_between_tiles_once() {
        let sections = help_sections(&[], &[]);
        assert_eq!(sections.iter().map(|s| s.title).collect::<Vec<_>>(), ["OVERVIEW", "GENERAL", "NAMESPACES"]);
        assert!(sections[0].entries.iter().any(|(k, _)| k == "enter"));
    }

    #[test]
    fn a_screen_with_only_navigation_has_no_empty_column() {
        let sections = help_sections(&[("↑↓/jk", "scroll"), ("g/G", "top/bottom"), ("q/esc", "back")], &[]);
        assert!(sections.iter().all(|s| !s.entries.is_empty()));
        assert!(sections.iter().all(|s| s.title != "OVERVIEW"));
    }

    #[test]
    fn every_entry_names_a_key() {
        // A wrong action id in `shown` gives no keys.
        for hints in [&[][..], &[("enter", "open"), ("d", "spec")][..]] {
            for (key, what) in help_sections(hints, &[]).iter().flat_map(|s| &s.entries) {
                assert!(!key.trim().is_empty(), "\"{what}\" has no key");
            }
        }
    }

    #[test]
    fn no_key_is_drawn_as_an_arrow_ligature() {
        // Fonts fuse `<->` into an arrow, so the dash key goes by name.
        let sections = help_sections(&[], &[]);
        assert!(sections.iter().flat_map(|s| &s.entries).all(|(k, _)| k != "-"));
    }

    #[test]
    fn hotkeys_list_all_and_the_reserved_namespaces() {
        let slots = [Some("kube-system".to_string()), None, Some("shop".to_string())];
        let sections = help_sections(&[], &slots);
        let hotkeys = sections.iter().find(|s| s.title == "NAMESPACES").unwrap();
        assert_eq!(hotkeys.entries, [("0".to_string(), "All namespaces".to_string()), ("1".to_string(), "kube-system".to_string()), ("3".to_string(), "shop".to_string())]);
    }

    #[test]
    fn the_sidebar_key_is_listed_with_both_of_its_keys() {
        for hints in [&[][..], &[("enter", "open")][..]] {
            let sections = help_sections(hints, &[]);
            assert!(sections[1].entries.iter().any(|(k, what)| k == "b / m" && what.contains("sidebar")));
        }
    }
}
