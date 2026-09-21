//! The `?` help screen, laid out like k9s's: four columns (what you can do
//! to the selected thing, general keys, navigation, number hotkeys), keys
//! in blue as `<key>` and what they do beside them.

use super::*;

pub(super) struct Section {
    pub title: &'static str,
    pub entries: Vec<(String, String)>,
}

const NAVIGATION_KEYS: [&str; 5] = ["↑↓", "g/G", "hjkl", "←↑↓→", "jk"];
const GENERAL_KEYS: [&str; 10] = ["?", "n", "0-9", "s", "/", "m", "C", "q/esc", "esc", "space"];

/// Splits the screen's own `hints` (see `mode::hints_for`) into the help
/// columns and adds the keys that work everywhere.
pub(super) fn help_sections(hints: &[(&str, &str)], slots: &[Option<String>]) -> Vec<Section> {
    let entry = |key: &str, what: &str| (key.to_string(), what.to_string());
    let resource: Vec<(String, String)> = hints
        .iter()
        .filter(|(key, _)| !GENERAL_KEYS.contains(key) && !NAVIGATION_KEYS.iter().any(|n| key.contains(n)))
        .map(|(key, what)| entry(key, what))
        .collect();
    let mut hotkeys = vec![entry("0", "All namespaces")];
    hotkeys.extend(slots.iter().enumerate().filter_map(|(i, ns)| ns.as_ref().map(|ns| (format!("{}", i + 1), ns.clone()))));
    vec![
        Section { title: "RESOURCE", entries: resource },
        Section {
            title: "GENERAL",
            entries: vec![
                entry(":cmd", "Command mode"),
                entry("/term", "Filter mode"),
                entry("s", "Sort (then a column number)"),
                entry("n", "Namespaces"),
                entry("m", "Resources menu"),
                entry("C", "Contexts"),
                entry("space", "Mark"),
                entry("esc", "Back / clear marks"),
                entry("?", "Help"),
                entry(":q", "Quit"),
            ],
        },
        Section {
            title: "NAVIGATION",
            entries: vec![
                entry("j / ↓", "Down"),
                entry("k / ↑", "Up"),
                entry("g", "Go to top"),
                entry("G", "Go to bottom"),
                entry("ctrl-f", "Page down"),
                entry("ctrl-b", "Page up"),
                entry("← →", "Scroll columns"),
                entry("click", "Select row"),
                entry("dbl-click", "Open row"),
                entry("wheel", "Scroll"),
            ],
        },
        Section { title: "HOTKEYS", entries: hotkeys },
    ]
}

const HEADING_FG: Color = Color::Rgb(96, 160, 72);
const KEY_FG: Color = Color::Rgb(84, 148, 255);
const DESC_FG: Color = Color::Rgb(170, 176, 192);

/// Draws the help over the whole body of the screen.
pub(super) fn draw_help(frame: &mut Frame, hints: &[(&str, &str)], slots: &[Option<String>]) {
    let area = body_area(frame.area(), true);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme_border(false))
        .title(Line::styled(" Help ", Style::default().fg(Color::Rgb(120, 230, 230)).add_modifier(Modifier::BOLD)).centered());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let sections = help_sections(hints, slots);
    let columns = Layout::horizontal(vec![Constraint::Ratio(1, sections.len() as u32); sections.len()]).split(inner);
    for (section, column) in sections.iter().zip(columns.iter()) {
        let key_width = section.entries.iter().map(|(k, _)| k.chars().count() + 2).max().unwrap_or(0) + 2;
        let mut lines = vec![Line::styled(section.title, Style::default().fg(HEADING_FG).add_modifier(Modifier::BOLD))];
        lines.extend(section.entries.iter().map(|(key, what)| {
            let shown = format!("<{key}>");
            let pad = " ".repeat(key_width.saturating_sub(shown.chars().count()));
            Line::from(vec![
                Span::styled(shown, Style::default().fg(KEY_FG).add_modifier(Modifier::BOLD)),
                Span::raw(pad),
                Span::styled(what.clone(), Style::default().fg(DESC_FG)),
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
    fn hotkeys_list_all_and_the_reserved_namespaces() {
        let slots = [Some("kube-system".to_string()), None, Some("shop".to_string())];
        let hotkeys = &help_sections(&[], &slots)[3];
        assert_eq!(hotkeys.entries, [("0".to_string(), "All namespaces".to_string()), ("1".to_string(), "kube-system".to_string()), ("3".to_string(), "shop".to_string())]);
    }
}
