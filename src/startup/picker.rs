//! The context picker shown before connecting when asked for. It draws the same
//! dialog as `C` inside the app.

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::widgets::TableState;

use crate::k8s::ContextInfo;
use crate::ui;

/// Runs the picker and restores the terminal. `Ok(None)` means the user cancelled.
/// `failed` is a context that couldn't be opened and why: it starts selected, so Enter
/// tries it again once the login is fixed.
pub fn run(contexts: &[ContextInfo], failed: Option<&crate::k8s::Unreachable>) -> Result<Option<String>> {
    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, contexts, failed);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, contexts: &[ContextInfo], failed: Option<&crate::k8s::Unreachable>) -> Result<Option<String>> {
    let mut filter = String::new();
    let start = failed.and_then(|f| ui::context_matches(contexts, "").iter().position(|c| c.name == f.context)).unwrap_or(0);
    let mut state = TableState::default().with_selected(start);
    let error = failed.map(|f| format!("{}: {}\n{}", f.context, f.reason, f.fix));

    loop {
        let matches = ui::context_matches(contexts, &filter);
        terminal.draw(|frame| {
            let view = ui::ContextView { items: &matches, total: contexts.len(), filter: &filter, state: &mut state, error: error.as_deref(), leave: "quit" };
            ui::draw_context_picker(frame, view);
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else { continue };
        let last = matches.len().saturating_sub(1);
        let selected = state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Esc if !filter.is_empty() => filter.clear(),
            KeyCode::Esc => return Ok(None),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(None),
            KeyCode::Enter => {
                if let Some(ctx) = matches.get(selected) {
                    return Ok(Some(ctx.name.clone()));
                }
            }
            KeyCode::Up => state.select(Some(selected.saturating_sub(1))),
            KeyCode::Down => state.select(Some((selected + 1).min(last))),
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
