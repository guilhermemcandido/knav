//! Copying text to the system clipboard: the platform's own tool when there
//! is one, else the terminal's OSC 52 escape (which many terminals honour,
//! including over ssh).

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

/// Copies `text`; returns how it was done, for the notice.
pub fn copy(text: &str) -> Result<&'static str> {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"]), ("xsel", &["--clipboard", "--input"])]
    };
    for (program, args) in candidates {
        if pipe_into(program, args, text).is_ok() {
            return Ok(program);
        }
    }
    osc52(text)?;
    Ok("the terminal")
}

/// A note for the copy's outcome: nothing for the platform's own tool (always the
/// same one per machine, so naming it says nothing); a clarifier for the OSC 52
/// fallback, since not every terminal honours it and a failed paste needs an explanation.
pub fn how_note(how: &str) -> &'static str {
    if how == "the terminal" { " (via the terminal's OSC 52 escape, not always supported)" } else { "" }
}

fn pipe_into(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    child.stdin.take().map(|mut stdin| stdin.write_all(text.as_bytes())).transpose()?;
    // A clipboard tool that hangs (no display to talk to) must not hang knav with it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match child.try_wait()? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => bail!("{program} failed"),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                bail!("{program} did not answer");
            }
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}

fn osc52(text: &str) -> Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", STANDARD.encode(text))?;
    out.flush()?;
    Ok(())
}
