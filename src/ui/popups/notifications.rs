//! The notifications page: what knav told you lately, newest first, with the selected one in full.

use super::*;
use crate::ui::notify::Level;

pub(in crate::ui) fn draw_notifications(frame: &mut Frame, view: NotificationsView) {
    let NotificationsView { entries, state } = view;
    let area = centered_rect(94, 88, frame.area());
    frame.render_widget(Clear, area);
    let muted = Style::default().fg(theme().muted);
    let title = pill_title(&format!("Notifications ({})", entries.len()), false, theme_border(false));
    let block = Block::default().borders(Borders::ALL).border_set(border_set()).border_style(theme_border(false)).title(title).title_bottom(hint_strip(&[("↑↓", "move"), ("g/G", "first/last"), ("esc", "back")]).right_aligned());
    if entries.is_empty() {
        frame.render_widget(Paragraph::new(Line::styled("Nothing yet. Errors, refusals and the results of actions are kept here.", muted)).alignment(Alignment::Center).block(block), area);
        return;
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // The list on top, the selected one wrapped in full beneath it.
    let detail_height = (inner.height / 3).clamp(4, 10).min(inner.height.saturating_sub(2));
    let [list, detail] = Layout::vertical([Constraint::Min(1), Constraint::Length(detail_height)]).areas(inner);
    let rows = entries.iter().map(|e| {
        let (mark, color) = match e.level {
            Level::Done => ("✓", theme().ok),
            Level::Info => ("•", theme().accent),
            Level::Failed => ("✗", theme().bad),
        };
        let first = e.text.lines().next().unwrap_or("");
        Row::new([Cell::from(e.time.clone()).style(muted), Cell::from(mark).style(Style::default().fg(color)), Cell::from(first.to_string()).style(theme_row(false))])
    });
    let table = Table::new(rows, [Constraint::Length(8), Constraint::Length(1), Constraint::Fill(1)])
        .column_spacing(2)
        .highlight_symbol("")
        .row_highlight_style(selection_style(crate::k8s::describe::Tone::Plain, false));
    let selected = state.selected().unwrap_or(0).min(entries.len() - 1);
    state.select(Some(selected));
    frame.render_stateful_widget(table, list, state);
    let rule = Block::default().borders(Borders::TOP).border_set(border_set()).border_style(theme_border(false));
    let text = rule.inner(detail);
    frame.render_widget(rule, detail);
    frame.render_widget(Paragraph::new(entries[selected].text.clone()).style(theme_row(false)).wrap(Wrap { trim: false }), text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::notify::Entry;

    #[test]
    fn the_selected_notification_shows_in_full_beneath_the_list() {
        let long = "pod: access denied (forbidden)\npods is forbidden: User \"ana\" cannot list resource \"pods\" in API group \"\" in the namespace \"kube-system\"";
        let entries = vec![Entry { time: "10:00:00".into(), level: Level::Failed, text: long.into() }, Entry { time: "09:59:00".into(), level: Level::Done, text: "Deleted pod a".into() }];
        let mut state = TableState::default().with_selected(0);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| draw_notifications(frame, NotificationsView { entries: &entries, state: &mut state })).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..30).map(|y| (0..100).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect();
        assert!(text.contains("Notifications (2)") && text.contains("Deleted pod a"), "{text}");
        assert!(text.contains("cannot list resource") && text.contains("kube-system"), "the detail line is shown: {text}");
    }
}
