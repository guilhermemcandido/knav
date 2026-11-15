//! The terminal's window or tab title: `knav · <context> · <namespace>`, put back on exit.

use std::io::{Write, stdout};

use crossterm::{execute, terminal::SetTitle};

/// Saves the current title on the terminal's title stack (xterm, iTerm2, kitty, tmux).
pub(crate) fn push() {
    let _ = stdout().write_all(b"\x1b[22;2t");
    let _ = stdout().flush();
}

/// Brings back the title saved by `push`.
pub(crate) fn pop() {
    let _ = stdout().write_all(b"\x1b[23;2t");
    let _ = stdout().flush();
}

pub(crate) fn text(context: &str, namespace: Option<&str>) -> String {
    format!("knav · {context} · {}", namespace.unwrap_or("all namespaces"))
}

/// Sets the title when it differs from `last`.
pub(crate) fn update(last: &mut String, title: String) {
    if *last != title {
        let _ = execute!(stdout(), SetTitle(&title));
        *last = title;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_context_and_namespace() {
        assert_eq!(text("prod", Some("web")), "knav · prod · web");
        assert_eq!(text("prod", None), "knav · prod · all namespaces");
    }
}
