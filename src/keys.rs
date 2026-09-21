//! Turning a key press into the bytes a terminal program expects, for
//! typing into an embedded shell.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// The bytes for `key`. `app_cursor` is the program's "application cursor
/// keys" mode (vim and friends), which changes what the arrows send.
pub fn encode(key: &KeyEvent, app_cursor: bool) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    // xterm's modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);
    let csi = |final_byte: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{final_byte}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{final_byte}").into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |number: u8| -> Vec<u8> {
        if modifier > 1 { format!("\x1b[{number};{modifier}~").into_bytes() } else { format!("\x1b[{number}~").into_bytes() }
    };
    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl => match c {
            'a'..='z' | 'A'..='Z' => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![27],
            '\\' | '4' => vec![28],
            ']' | '5' => vec![29],
            '^' | '6' => vec![30],
            '_' | '7' | '/' => vec![31],
            '?' | '8' => vec![127],
            other => other.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => csi('A'),
        KeyCode::Down => csi('B'),
        KeyCode::Right => csi('C'),
        KeyCode::Left => csi('D'),
        KeyCode::Home => csi('H'),
        KeyCode::End => csi('F'),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::Delete => tilde(3),
        KeyCode::Insert => tilde(2),
        KeyCode::F(n @ 1..=4) => format!("\x1bO{}", (b'P' + n - 1) as char).into_bytes(),
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n) - 5]),
        _ => return Vec::new(),
    };
    // Alt sends the key preceded by Escape (arrows already carry it in their parameter).
    if alt && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace) {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Vec<u8> {
        encode(&KeyEvent::new(code, modifiers), false)
    }

    #[test]
    fn plain_keys_send_themselves() {
        assert_eq!(press(KeyCode::Char('a'), KeyModifiers::NONE), b"a");
        assert_eq!(press(KeyCode::Char('é'), KeyModifiers::NONE), "é".as_bytes());
        assert_eq!(press(KeyCode::Enter, KeyModifiers::NONE), b"\r");
        assert_eq!(press(KeyCode::Backspace, KeyModifiers::NONE), [0x7f]);
        assert_eq!(press(KeyCode::Tab, KeyModifiers::NONE), b"\t");
    }

    #[test]
    fn ctrl_letters_are_control_codes() {
        assert_eq!(press(KeyCode::Char('c'), KeyModifiers::CONTROL), [3]);
        assert_eq!(press(KeyCode::Char('d'), KeyModifiers::CONTROL), [4]);
        assert_eq!(press(KeyCode::Char('l'), KeyModifiers::CONTROL), [12]);
        assert_eq!(press(KeyCode::Char('['), KeyModifiers::CONTROL), [27]);
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(press(KeyCode::Char('b'), KeyModifiers::ALT), b"\x1bb");
    }

    #[test]
    fn arrows_follow_the_cursor_mode_and_modifiers() {
        assert_eq!(press(KeyCode::Up, KeyModifiers::NONE), b"\x1b[A");
        assert_eq!(encode(&KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true), b"\x1bOA");
        assert_eq!(press(KeyCode::Left, KeyModifiers::CONTROL), b"\x1b[1;5D");
        assert_eq!(press(KeyCode::Right, KeyModifiers::ALT), b"\x1b[1;3C");
    }

    #[test]
    fn navigation_and_function_keys() {
        assert_eq!(press(KeyCode::Delete, KeyModifiers::NONE), b"\x1b[3~");
        assert_eq!(press(KeyCode::PageUp, KeyModifiers::NONE), b"\x1b[5~");
        assert_eq!(press(KeyCode::F(1), KeyModifiers::NONE), b"\x1bOP");
        assert_eq!(press(KeyCode::F(5), KeyModifiers::NONE), b"\x1b[15~");
        assert_eq!(press(KeyCode::F(12), KeyModifiers::NONE), b"\x1b[24~");
    }
}
