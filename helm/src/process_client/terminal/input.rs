//! Encode supported xterm keys using the child's last observed modes, never the outer TTY's.
use anyhow::{Result, ensure};
use crossterm::event::{KeyCode, KeyEventState, KeyModifiers};
use voyage_protocol::terminal::TerminalModes;

pub(super) fn detached(code: KeyCode, modifiers: KeyModifiers) -> bool {
    matches!(code, KeyCode::Char('\u{1d}'))
        || (modifiers.contains(KeyModifiers::CONTROL) && matches!(code, KeyCode::Char(']' | '5')))
}

pub(super) fn paste(text: String, modes: &TerminalModes) -> Result<Vec<u8>> {
    let overhead = if modes.bracketed_paste { 12 } else { 0 };
    ensure!(
        text.len() <= 65536 - overhead,
        "private paste exceeds input frame limit"
    );
    if modes.bracketed_paste {
        // A pasted terminator could turn the remaining paste into commands.
        ensure!(
            !text.contains('\x1b'),
            "private bracketed paste contains an escape character"
        );
        Ok(format!("\x1b[200~{text}\x1b[201~").into_bytes())
    } else {
        Ok(text.into_bytes())
    }
}

pub(super) fn keypad(
    code: KeyCode,
    state: KeyEventState,
    modifiers: KeyModifiers,
    modes: &TerminalModes,
) -> Option<Vec<u8>> {
    if !modes.app_keypad || !state.contains(KeyEventState::KEYPAD) || !modifiers.is_empty() {
        return None;
    }
    let end = match code {
        KeyCode::Char(ch @ '0'..='9') => char::from(b'p' + ch as u8 - b'0'),
        KeyCode::Char('.') => 'n',
        KeyCode::Char(',') => 'l',
        KeyCode::Char('+') => 'k',
        KeyCode::Char('-') => 'm',
        KeyCode::Char('*') => 'j',
        KeyCode::Char('/') => 'o',
        KeyCode::Char('=') => 'X',
        KeyCode::Enter => 'M',
        _ => return None,
    };
    Some(format!("\x1bO{end}").into_bytes())
}

pub(super) fn key_bytes(
    code: KeyCode,
    modifiers: KeyModifiers,
    modes: &TerminalModes,
) -> Option<Vec<u8>> {
    let parameter = 1
        + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let final_byte = match code {
        KeyCode::Up => Some('A'),
        KeyCode::Down => Some('B'),
        KeyCode::Right => Some('C'),
        KeyCode::Left => Some('D'),
        KeyCode::Home => Some('H'),
        KeyCode::End => Some('F'),
        _ => None,
    };
    if let Some(end) = final_byte {
        return Some(
            if parameter > 1 {
                format!("\x1b[1;{parameter}{end}")
            } else if modes.app_cursor {
                format!("\x1bO{end}")
            } else {
                format!("\x1b[{end}")
            }
            .into_bytes(),
        );
    }
    if let KeyCode::F(number @ 1..=4) = code {
        let end = char::from(b'P' + number - 1);
        return Some(
            if parameter > 1 {
                format!("\x1b[1;{parameter}{end}")
            } else {
                format!("\x1bO{end}")
            }
            .into_bytes(),
        );
    }
    let tilde = match code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        KeyCode::F(5) => Some(15),
        KeyCode::F(6) => Some(17),
        KeyCode::F(7) => Some(18),
        KeyCode::F(8) => Some(19),
        KeyCode::F(9) => Some(20),
        KeyCode::F(10) => Some(21),
        KeyCode::F(11) => Some(23),
        KeyCode::F(12) => Some(24),
        _ => None,
    };
    if let Some(number) = tilde {
        return Some(
            if parameter > 1 {
                format!("\x1b[{number};{parameter}~")
            } else {
                format!("\x1b[{number}~")
            }
            .into_bytes(),
        );
    }
    let mut bytes = match code {
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Char(ch @ '4'..='7') if modifiers.contains(KeyModifiers::CONTROL) => {
            vec![ch as u8 - b'4' + 0x1c]
        }
        KeyCode::Char(' ' | '@' | '2') if modifiers.contains(KeyModifiers::CONTROL) => vec![0],
        KeyCode::Char('?' | '8') if modifiers.contains(KeyModifiers::CONTROL) => vec![127],
        KeyCode::Char(ch) if modifiers.contains(KeyModifiers::CONTROL) && ch.is_ascii() => {
            vec![(ch.to_ascii_uppercase() as u8) & 0x1f]
        }
        KeyCode::Char(ch) => ch.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![27],
        _ => return None,
    };
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 27);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_navigation_modifiers_and_paste() {
        let mut modes = TerminalModes::default();
        assert_eq!(
            key_bytes(KeyCode::Up, KeyModifiers::NONE, &modes).unwrap(),
            b"\x1b[A"
        );
        modes.app_cursor = true;
        assert_eq!(
            key_bytes(KeyCode::Up, KeyModifiers::NONE, &modes).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(
            key_bytes(
                KeyCode::Left,
                KeyModifiers::CONTROL | KeyModifiers::ALT,
                &modes
            )
            .unwrap(),
            b"\x1b[1;7D"
        );
        assert_eq!(
            key_bytes(KeyCode::F(12), KeyModifiers::SHIFT, &modes).unwrap(),
            b"\x1b[24;2~"
        );
        assert_eq!(
            key_bytes(KeyCode::Char('界'), KeyModifiers::NONE, &modes).unwrap(),
            "界".as_bytes()
        );
        modes.app_keypad = true;
        assert_eq!(
            keypad(
                KeyCode::Char('1'),
                KeyEventState::KEYPAD,
                KeyModifiers::NONE,
                &modes
            )
            .unwrap(),
            b"\x1bOq"
        );
        assert!(
            keypad(
                KeyCode::Char('1'),
                KeyEventState::empty(),
                KeyModifiers::NONE,
                &modes
            )
            .is_none()
        );
        modes.bracketed_paste = true;
        assert_eq!(
            paste("a\n界".into(), &modes).unwrap(),
            "\x1b[200~a\n界\x1b[201~".as_bytes()
        );
        assert!(paste("x".repeat(65525), &modes).is_err());
        assert!(paste("\x1b[201~oops".into(), &modes).is_err());
        assert!(detached(KeyCode::Char('5'), KeyModifiers::CONTROL));
        assert!(detached(KeyCode::Char('\u{1d}'), KeyModifiers::NONE));
        assert!(!detached(KeyCode::Char('t'), KeyModifiers::CONTROL));
    }
}
