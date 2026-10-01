//! `P`: choose this cluster's permission mode, which contexts are always read-only,
//! and which get a red header.

use crate::ops::NoticeTone;
use super::super::*;
use super::Cx;

/// The tabs: this cluster, the read-only contexts, the highlighted contexts.
pub(crate) const THIS_CLUSTER: usize = 0;
pub(crate) const CONTEXTS: usize = 1;
pub(crate) const HIGHLIGHT: usize = 2;
const TABS: usize = 3;

pub(super) fn open(st: &mut State) {
    let back = std::mem::replace(&mut st.mode, Mode::List);
    let cursor = usize::from(st.read_only());
    st.mode = Mode::Permissions { tab: THIS_CLUSTER, cursor, input: None, error: None, back: Box::new(back) };
}

/// The config list a tab edits, and the rows before its patterns ("All contexts").
fn list_of(tab: usize) -> (&'static str, usize) {
    if tab == HIGHLIGHT { ("highlight.contexts", 0) } else { ("read_only.contexts", 1) }
}

fn patterns(config: &Config, tab: usize) -> Vec<String> {
    if tab == HIGHLIGHT { config.highlight.contexts.clone() } else { config.read_only.contexts.clone() }
}

/// Saves a list of context patterns, leaving it out of the file when empty.
fn save_list(st: &mut State, path: &str, contexts: &[String]) -> Result<()> {
    let mut array = toml_edit::Array::new();
    for c in contexts {
        array.push(c.as_str());
    }
    let value = (!contexts.is_empty()).then_some(toml_edit::Value::Array(array));
    let config = crate::config::edit::save(&Config::path(), path, value)?;
    st.reload(config);
    Ok(())
}

fn save_contexts(st: &mut State, contexts: &[String]) -> Result<()> {
    save_list(st, "read_only.contexts", contexts)
}

/// Makes this context read-only or not, saved so it holds next time. When something
/// else still decides (a pattern, `--read-only`), the choice holds until knav restarts
/// and the returned note says why.
fn set_read_only(st: &mut State, want: bool) -> Result<Option<String>> {
    let context = st.context.clone();
    let mut contexts = st.config.read_only.contexts.clone();
    if want && !st.config.read_only.applies_to(&context) {
        contexts.push(context.clone());
    } else if !want {
        contexts.retain(|c| *c != context);
    }
    if contexts != st.config.read_only.contexts {
        save_contexts(st, &contexts)?;
    }
    st.read_only_override = None;
    if st.read_only() == want {
        return Ok(None);
    }
    st.read_only_override = Some(want);
    let why = if st.read_only_flag {
        "--read-only"
    } else if st.config.read_only.enabled {
        "\"All contexts\""
    } else {
        "a pattern in Read-only contexts"
    };
    Ok(Some(format!("Using your role until knav restarts, then {why} makes {context} read-only again")))
}

pub(super) fn handle(event: Event, st: &mut State, _cx: &mut Cx) -> Result<Option<SessionEnd>> {
    let Event::Key(key) = event else { return Ok(None) };
    let Mode::Permissions { tab, .. } = &st.mode else { return Ok(None) };
    let (path, first) = list_of(*tab);
    let patterns = patterns(&st.config, *tab);
    // This cluster has two choices; a list tab its leading rows, each pattern, then "Add".
    let rows = if *tab == THIS_CLUSTER { 2 } else { first + patterns.len() + 1 };
    let Mode::Permissions { tab, cursor, input, error, back } = &mut st.mode else { return Ok(None) };

    // Typing a pattern: Enter keeps it, an empty one removes it, Esc drops the change.
    if let Some((replaces, text)) = input {
        match key.code {
            KeyCode::Esc => *input = None,
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char(c) => text.push(c),
            KeyCode::Enter => {
                let (replaces, text) = (*replaces, text.trim().to_string());
                let mut next = patterns;
                match replaces {
                    Some(i) if text.is_empty() => {
                        next.remove(i);
                    }
                    Some(i) => next[i] = text,
                    None if !text.is_empty() && !next.contains(&text) => next.push(text),
                    None => {}
                }
                *input = None;
                if let Err(e) = save_list(st, path, &next) {
                    set_error(st, e);
                }
            }
            _ => {}
        }
        return Ok(None);
    }

    *error = None;
    let pattern_at = |cursor: usize| cursor.checked_sub(first).filter(|i| *i < patterns.len());
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => st.mode = std::mem::replace(&mut **back, Mode::List),
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            *tab = (*tab + 1) % TABS;
            *cursor = 0;
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            *tab = (*tab + TABS - 1) % TABS;
            *cursor = 0;
        }
        KeyCode::Down | KeyCode::Char('j') => *cursor = (*cursor + 1).min(rows - 1),
        KeyCode::Up | KeyCode::Char('k') => *cursor = cursor.saturating_sub(1),
        KeyCode::Enter | KeyCode::Char(' ') if *tab == THIS_CLUSTER => {
            let want = *cursor == 1;
            let back = std::mem::replace(back, Box::new(Mode::List));
            match set_read_only(st, want) {
                Ok(None) => st.mode = *back,
                Ok(Some(note)) => st.mode = Mode::Notice { text: note, tone: NoticeTone::Info, back },
                Err(e) => st.mode = Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back },
            }
        }
        // The read-only tab starts with "All contexts".
        KeyCode::Enter | KeyCode::Char(' ') if *tab == CONTEXTS && *cursor == 0 => {
            let everywhere = !st.config.read_only.enabled;
            match crate::config::edit::save(&Config::path(), "read_only.enabled", everywhere.then_some(toml_edit::Value::from(true))) {
                Ok(config) => st.reload(config),
                Err(e) => set_error(st, e),
            }
        }
        KeyCode::Enter => match pattern_at(*cursor) {
            Some(i) => *input = Some((Some(i), patterns[i].clone())),
            None => *input = Some((None, String::new())),
        },
        KeyCode::Char('a') if *tab != THIS_CLUSTER => {
            *cursor = rows - 1;
            *input = Some((None, String::new()));
        }
        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace if *tab != THIS_CLUSTER => {
            if let Some(i) = pattern_at(*cursor) {
                let mut next = patterns;
                next.remove(i);
                if let Err(e) = save_list(st, path, &next) {
                    set_error(st, e);
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

fn set_error(st: &mut State, e: anyhow::Error) {
    if let Mode::Permissions { error, .. } = &mut st.mode {
        *error = Some(format!("{e:#}"));
    }
}
