//! The freelens/Lens-style cluster picker: a full-screen list of every
//! kubeconfig context, live-filtered by fuzzy match as you type, shown
//! before knav connects to anything. Only reached when the config opts
//! into it (`startup.mode = "menu"`) — the default is to connect directly
//! to the current context, k9s-style (see `Config::startup`).

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::fuzzy;
use crate::k8s::ContextInfo;

/// Runs the picker to completion and restores the terminal before
/// returning — self-contained, since it happens before the rest of knav's
/// state (watches, the main `run` loop) exists at all. `Ok(None)` means
/// the user cancelled (Esc/Ctrl-C/q on an empty filter); the caller should
/// exit cleanly rather than falling back to some default context, since
/// showing this screen was already an explicit choice to let them decide.
pub fn run(contexts: &[ContextInfo]) -> Result<Option<String>> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, contexts);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, contexts: &[ContextInfo]) -> Result<Option<String>> {
    let mut filter = String::new();
    let mut state = ListState::default();
    state.select(Some(0));

    loop {
        let matches = filtered(contexts, &filter);
        let selected = state.selected().unwrap_or(0).min(matches.len().saturating_sub(1));
        state.select((!matches.is_empty()).then_some(selected));

        terminal.draw(|frame| draw(frame, &matches, &filter, &mut state))?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        match key.code {
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
            KeyCode::Enter => {
                if let Some(ctx) = state.selected().and_then(|i| matches.get(i)) {
                    return Ok(Some(ctx.name.clone()));
                }
            }
            KeyCode::Up => select_prev(&mut state, matches.len()),
            KeyCode::Down => select_next(&mut state, matches.len()),
            KeyCode::Backspace => {
                filter.pop();
                state.select(Some(0));
            }
            KeyCode::Char(c) => {
                filter.push(c);
                state.select(Some(0));
            }
            _ => {}
        }
    }
}

/// Every context whose name fuzzy-matches `filter`, best match first —
/// same ranking `--context` uses to resolve non-interactively, so typing
/// the exact string you'd pass on the command line picks the same context
/// here too.
fn filtered<'a>(contexts: &'a [ContextInfo], filter: &str) -> Vec<&'a ContextInfo> {
    let mut scored: Vec<(i64, &ContextInfo)> =
        contexts.iter().filter_map(|c| fuzzy::score(filter, &c.name).map(|s| (s, c))).collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, c)| c).collect()
}

fn select_next(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let next = state.selected().map(|i| (i + 1).min(len - 1)).unwrap_or(0);
    state.select(Some(next));
}

fn select_prev(state: &mut ListState, len: usize) {
    if len == 0 {
        return;
    }
    let prev = state.selected().map(|i| i.saturating_sub(1)).unwrap_or(0);
    state.select(Some(prev));
}

fn draw(frame: &mut ratatui::Frame, matches: &[&ContextInfo], filter: &str, state: &mut ListState) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

    let filter_block = Block::default().borders(Borders::ALL).border_set(crate::ui::BORDER_SET).title("Select a cluster");
    let filter_line = Line::from(vec![
        Span::styled("🔍 ", Style::default()),
        Span::styled(filter, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled("▏", Style::default().add_modifier(Modifier::RAPID_BLINK)),
    ]);
    frame.render_widget(Paragraph::new(filter_line).block(filter_block), chunks[0]);

    let items: Vec<ListItem> = matches
        .iter()
        .map(|c| {
            let marker = if c.is_current { " (current)" } else { "" };
            let line = Line::from(vec![
                Span::styled(format!("{:<40}", c.name), Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(c.cluster.clone(), Style::default().fg(Color::DarkGray)),
                Span::styled(marker, Style::default().fg(Color::Cyan)),
            ]);
            ListItem::new(line)
        })
        .collect();

    if matches.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(crate::ui::BORDER_SET)
            .title("No matches");
        let empty = Paragraph::new("No matching context").alignment(Alignment::Center).block(block);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let title = format!("Contexts ({})", matches.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_set(crate::ui::BORDER_SET).title(title))
        .highlight_style(Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD))
        .highlight_symbol("➤ ");
    frame.render_stateful_widget(list, chunks[1], state);
}
