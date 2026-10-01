//! Your own commands (`[[commands]]` in the config), run on the selected object from
//! their key on a list or by name on the `:` line.

use crate::config::{CommandOutput, CustomCommand};
use crate::input::keymap::{self, Keymap, KeySpec, Screen};
use crate::ops::{NoticeTone, actions::Target, custom};
use super::super::*;
use super::Cx;

/// What is wrong with the commands in the config. A command with a bad key keeps
/// working from the `:` line.
pub(crate) fn problems(commands: &[CustomCommand], keymap: &Keymap) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen: Vec<KeySpec> = Vec::new();
    for c in commands {
        if c.name.trim().is_empty() || c.run.trim().is_empty() {
            problems.push("commands: every command needs a name and a run line".into());
            continue;
        }
        let Some(text) = &c.key else { continue };
        match key_of(c) {
            None => problems.push(format!("commands.{}: '{text}' is not a key", c.name)),
            Some(key) if keymap.uses(Screen::List, key) || matches!(key.code, KeyCode::Char('0'..='9')) => {
                problems.push(format!("commands.{}: '{text}' already does something on lists", c.name));
            }
            Some(key) if seen.contains(&key) => problems.push(format!("commands.{}: '{text}' is used by another command", c.name)),
            Some(key) => seen.push(key),
        }
    }
    problems
}

fn key_of(command: &CustomCommand) -> Option<KeySpec> {
    keymap::parse_key(command.key.as_deref()?).ok()
}

/// The command `key` runs on lists, if any. Keys that clash are left out, so a bad
/// config never takes over a built-in key.
pub(super) fn for_key(st: &State, key: &crossterm::event::KeyEvent) -> Option<usize> {
    let pressed = KeySpec::of(key);
    if st.keymap.uses(Screen::List, pressed) {
        return None;
    }
    st.config.commands.iter().position(|c| key_of(c) == Some(pressed))
}

/// The help's entries for the commands with a key: the key and the command's name.
pub(crate) fn hints(commands: &[CustomCommand]) -> Vec<(String, String)> {
    commands.iter().filter_map(|c| Some((c.key.clone()?, c.name.clone()))).collect()
}

/// Runs command `index` on `target`, unless read-only mode or its kinds say no.
pub(super) fn run(st: &mut State, cx: &mut Cx, index: usize, target: &Target) {
    let Some(command) = st.config.commands.get(index).cloned() else { return };
    if !command.applies_to(&target.kind) {
        let back = std::mem::replace(&mut st.mode, Mode::List);
        st.mode = Mode::Notice { text: format!("{} is for {}, not {}", command.name, command.kinds.join(", "), target.kind), tone: NoticeTone::Info, back: Box::new(back) };
        return;
    }
    if !command.read_only && st.refuse_if_read_only() {
        return;
    }
    let subject = custom::Subject { kind: &target.kind, name: &target.name, namespace: target.namespace.as_deref(), context: cx.active_context };
    let line = custom::command_line(&command.run, &subject);
    let title = format!("{} {}", command.name, target.label());
    match command.output {
        CommandOutput::View => crate::app::jobs::run_command(st, title, line, true),
        CommandOutput::Background => crate::app::jobs::run_command(st, title, line, false),
        CommandOutput::Terminal => {
            let back = std::mem::replace(&mut st.mode, Mode::List);
            st.mode = match custom::in_terminal(cx.terminal, &line) {
                Ok(Some(0)) => back,
                Ok(code) => Mode::Notice { text: format!("{title} {}", code.map_or("was stopped".to_string(), |c| format!("exited with code {c}"))), tone: NoticeTone::Failed, back: Box::new(back) },
                Err(e) => Mode::Notice { text: format!("{e:#}"), tone: NoticeTone::Failed, back: Box::new(back) },
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, key: Option<&str>) -> CustomCommand {
        CustomCommand { name: name.into(), key: key.map(str::to_string), kinds: Vec::new(), run: "true".into(), output: CommandOutput::View, read_only: false }
    }

    #[test]
    fn keys_that_already_do_something_are_reported_not_taken() {
        let keymap = Keymap::from_config(&Default::default()).0;
        let problems = problems(&[command("ok", Some("ctrl-k")), command("delete-ish", Some("D")), command("digit", Some("3")), command("twice", Some("ctrl-k"))], &keymap);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("delete-ish")) && problems.iter().any(|p| p.contains("digit")) && problems.iter().any(|p| p.contains("twice")));
    }

    #[test]
    fn kinds_limit_where_a_command_runs() {
        let mut c = command("x", None);
        assert!(c.applies_to("Pod"), "every kind when none are given");
        c.kinds = vec!["Deployment".into()];
        assert!(c.applies_to("deployment") && !c.applies_to("Pod"));
    }
}
