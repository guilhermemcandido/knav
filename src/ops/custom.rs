//! Your own commands: the selected object filled into a shell command line, run with
//! its output captured, in the terminal, or in the background.

use std::io::{Write, stdout};
use std::process::Command;

use anyhow::{Context as _, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;

/// What a command runs on.
pub struct Subject<'a> {
    pub kind: &'a str,
    pub name: &'a str,
    pub namespace: Option<&'a str>,
    pub context: &'a str,
}

/// `template` with the placeholders filled, each value quoted for the shell so an
/// odd name can't change the command.
pub fn command_line(template: &str, s: &Subject) -> String {
    let quote = |v: &str| format!("'{}'", v.replace('\'', r"'\''"));
    template
        .replace("{kind}", &quote(&s.kind.to_lowercase()))
        .replace("{name}", &quote(s.name))
        .replace("{namespace}", &quote(s.namespace.unwrap_or("")))
        .replace("{context}", &quote(s.context))
}

/// Runs `line` and returns what it printed. A failure carries the output too, since
/// that is usually where the reason is.
pub async fn capture(line: String) -> Result<String> {
    let output = tokio::process::Command::new("sh").arg("-c").arg(&line).output().await.context("couldn't start the shell")?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(text)
    } else {
        let code = output.status.code().map_or("a signal".to_string(), |c| format!("code {c}"));
        anyhow::bail!("exited with {code}\n{}", last_lines(&text, 5))
    }
}

/// The last `n` non-empty lines, for a notice.
pub fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// Hands the terminal to `line` (for interactive tools), then waits for Enter so its
/// last output can be read. Returns the exit code.
pub fn in_terminal(terminal: &mut ratatui::DefaultTerminal, line: &str) -> Result<Option<i32>> {
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();
    let status = Command::new("sh").arg("-c").arg(line).status();
    let code = status.as_ref().ok().and_then(|s| s.code());
    print!("\n[{}] press Enter to go back to knav", code.map_or("stopped".to_string(), |c| format!("exited with {c}")));
    let _ = stdout().flush();
    let _ = std::io::stdin().read_line(&mut String::new());
    *terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    status.context("couldn't start the shell")?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_and_quoted() {
        let s = Subject { kind: "Pod", name: "web-1", namespace: Some("shop"), context: "prod" };
        assert_eq!(command_line("kubectl describe {kind} {name} -n {namespace} --context {context}", &s), "kubectl describe 'pod' 'web-1' -n 'shop' --context 'prod'");
        let odd = Subject { kind: "Pod", name: "a'b; rm -rf /", namespace: None, context: "c" };
        assert_eq!(command_line("echo {name}", &odd), r"echo 'a'\''b; rm -rf /'");
    }

    #[test]
    fn last_lines_skip_blank_ones() {
        assert_eq!(last_lines("a\n\nb\nc\n\n", 2), "b\nc");
    }
}
