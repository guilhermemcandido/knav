//! The `?` help screen, laid out like k9s's: four columns (what you can do
//! to the selected thing, general keys, navigation, number hotkeys), keys
//! in blue as `<key>` and what they do beside them.

use super::*;

pub(super) struct Section {
    pub title: &'static str,
    pub entries: Vec<(String, String)>,
}

const NAVIGATION_KEYS: [&str; 5] = ["↑↓", "g/G", "hjkl", "←↑↓→", "jk"];
const GENERAL_KEYS: [&str; 12] = ["?", "n", "0-9", "s", "A", "/", "m", "b/m", "C", "q/esc", "esc", "space"];

/// Splits the screen's own `hints` (see `mode::hints_for`) into the help
/// columns and adds the keys that work everywhere.
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
    let resource: Vec<(String, String)> = hints
        .iter()
        .filter(|(key, _)| !GENERAL_KEYS.contains(key) && !NAVIGATION_KEYS.iter().any(|n| key.contains(n)))
        .map(|(key, what)| entry(key, what))
        .collect();
    // The Overview has no list of its own, so say what its tiles do.
    let on_overview = resource.is_empty();
    let resource = if on_overview {
        vec![entry("enter", "Open tile"), entry("hjkl", "Move tiles"), entry("click", "Select tile"), entry("dbl-click", "Open tile")]
    } else {
        resource
    };
    let mut hotkeys = vec![entry("0", "All namespaces")];
    hotkeys.extend(slots.iter().enumerate().filter_map(|(i, ns)| ns.as_ref().map(|ns| (format!("{}", i + 1), ns.clone()))));
    // The Overview has no rows to filter, sort, mark or scroll sideways.
    if on_overview {
        return vec![
            Section { title: "RESOURCE", entries: resource },
            Section {
                title: "GENERAL",
                entries: vec![
                    shown(":cmd", "command", "Command mode"),
                    shown("n", "namespaces", "Namespaces"),
                    shown("b / m", "menu", "Show or hide the sidebar (Shift-← focuses it)"),
                    shown("C", "contexts", "Contexts"),
                    shown("T", "themes", "Themes"),
                    shown(",", "settings", "Settings"),
                    shown("?", "help", "Help"),
                    shown("Q", "quit", "Quit"),
                ],
            },
            Section {
                title: "NAVIGATION",
                entries: vec![pair("j / ↓", "move_down", "", "Down"), pair("k / ↑", "move_up", "", "Up"), entry("← →", "Previous / next column"), entry("wheel", "Scroll")],
            },
            Section { title: "HOTKEYS", entries: hotkeys },
        ];
    }
    vec![
        Section { title: "RESOURCE", entries: resource },
        Section {
            title: "GENERAL",
            entries: vec![
                shown(":cmd", "command", "Command mode"),
                shown("/term", "search", "Filter mode"),
                shown("s", "sort", "Sort by column"),
                shown("A", "age", "Sort by age"),
                shown("n", "namespaces", "Namespaces"),
                shown("b / m", "menu", "Show or hide the sidebar (Shift-← focuses it)"),
                shown("C", "contexts", "Contexts"),
                shown("T", "themes", "Themes"),
                shown(",", "settings", "Settings"),
                shown("space", "mark", "Mark"),
                shown("ctrl-z", "faults", "Faults only"),
                shown("ctrl-w", "wide", "Wide columns"),
                pair("[ ]", "history_back", "history_forward", "History back / forward"),
                shown("minus", "last_view", "Last view"),
                shown("esc", "cancel", "Back / clear marks"),
                shown("?", "help", "Help"),
                shown("Q", "quit", "Quit"),
            ],
        },
        Section {
            title: "NAVIGATION",
            entries: vec![
                pair("j / ↓", "move_down", "", "Down"),
                pair("k / ↑", "move_up", "", "Up"),
                shown("g", "top", "Go to top"),
                shown("G", "bottom", "Go to bottom"),
                shown("ctrl-f", "page_down", "Page down"),
                shown("ctrl-b", "page_up", "Page up"),
                entry("← →", "Scroll columns"),
                entry("click", "Select row"),
                entry("dbl-click", "Open row"),
                entry("wheel", "Scroll"),
            ],
        },
        Section { title: "HOTKEYS", entries: hotkeys },
    ]
}


/// Draws the help over the whole body of the screen.
pub(super) fn draw_help(frame: &mut Frame, hints: &[(&str, &str)], slots: &[Option<String>], shortcuts_line: bool) {
    // The same body the page itself uses (the Overview has one header line, lists two).
    let area = body_area(frame.area(), shortcuts_line);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(border_set())
        .border_style(theme_border(false))
        .title(pill_title_centered("Help", false));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = help_sections(hints, slots);
    let columns = Layout::horizontal(vec![Constraint::Ratio(1, sections.len() as u32); sections.len()]).split(inner);
    for (section, column) in sections.iter().zip(columns.iter()) {
        let key_width = section.entries.iter().map(|(k, _)| k.chars().count() + 2).max().unwrap_or(0) + 2;
        let mut lines = vec![Line::styled(section.title, Style::default().fg(theme().heading).add_modifier(Modifier::BOLD))];
        lines.extend(section.entries.iter().map(|(key, what)| {
            let shown = format!("<{key}>");
            let pad = " ".repeat(key_width.saturating_sub(shown.chars().count()));
            Line::from(vec![
                Span::styled(shown, Style::default().fg(theme().key).add_modifier(Modifier::BOLD)),
                Span::raw(pad),
                Span::styled(what.clone(), Style::default().fg(theme().desc)),
            ])
        }));
        let padded = Rect { x: column.x + 1, width: column.width.saturating_sub(1), ..*column };
        frame.render_widget(Paragraph::new(lines), padded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_specific_keys_go_under_resource_and_common_ones_do_not_repeat() {
        let hints = [("↑↓/jk", "move"), ("enter", "containers"), ("d", "spec"), ("D", "delete"), ("n", "namespaces"), ("q/esc", "back")];
        let sections = help_sections(&hints, &[]);
        let resource: Vec<&str> = sections[0].entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(resource, ["enter", "d", "D"]);
    }

    #[test]
    fn the_overview_still_gets_a_resource_column() {
        let sections = help_sections(&[], &[]);
        assert!(sections[0].entries.iter().any(|(k, _)| k == "enter"));
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
        let hotkeys = &help_sections(&[], &slots)[3];
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
