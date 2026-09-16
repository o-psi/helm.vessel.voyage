use super::*;

#[test]
fn keyboard_translation_matrix_never_touches_terminal() {
    let modes = TerminalModes::default();
    for (code, end) in [
        (KeyCode::Up, 'A'),
        (KeyCode::Down, 'B'),
        (KeyCode::Right, 'C'),
        (KeyCode::Left, 'D'),
        (KeyCode::Home, 'H'),
        (KeyCode::End, 'F'),
    ] {
        assert_eq!(
            key_bytes(code, KeyModifiers::NONE, &modes).unwrap(),
            format!("\x1b[{end}").as_bytes()
        );
        assert_eq!(
            key_bytes(
                code,
                KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL,
                &modes
            )
            .unwrap(),
            format!("\x1b[1;8{end}").as_bytes()
        );
        let mut application = modes;
        application.app_cursor = true;
        assert_eq!(
            key_bytes(code, KeyModifiers::NONE, &application).unwrap(),
            format!("\x1bO{end}").as_bytes()
        );
    }
    for n in 1..=4 {
        let end = char::from(b'P' + n - 1);
        assert_eq!(
            key_bytes(KeyCode::F(n), KeyModifiers::NONE, &modes).unwrap(),
            format!("\x1bO{end}").as_bytes()
        );
        assert_eq!(
            key_bytes(KeyCode::F(n), KeyModifiers::CONTROL, &modes).unwrap(),
            format!("\x1b[1;5{end}").as_bytes()
        );
    }
    for (code, n) in [
        (KeyCode::Insert, 2),
        (KeyCode::Delete, 3),
        (KeyCode::PageUp, 5),
        (KeyCode::PageDown, 6),
        (KeyCode::F(5), 15),
        (KeyCode::F(6), 17),
        (KeyCode::F(7), 18),
        (KeyCode::F(8), 19),
        (KeyCode::F(9), 20),
        (KeyCode::F(10), 21),
        (KeyCode::F(11), 23),
        (KeyCode::F(12), 24),
    ] {
        assert_eq!(
            key_bytes(code, KeyModifiers::NONE, &modes).unwrap(),
            format!("\x1b[{n}~").as_bytes()
        );
        assert_eq!(
            key_bytes(code, KeyModifiers::ALT, &modes).unwrap(),
            format!("\x1b[{n};3~").as_bytes()
        );
    }
    for (ch, byte) in [
        ('4', 28),
        ('5', 29),
        ('6', 30),
        ('7', 31),
        (' ', 0),
        ('@', 0),
        ('2', 0),
        ('?', 127),
        ('8', 127),
        ('a', 1),
    ] {
        assert_eq!(
            key_bytes(KeyCode::Char(ch), KeyModifiers::CONTROL, &modes),
            Some(vec![byte])
        );
    }
    for (code, bytes) in [
        (KeyCode::Enter, b"\r".to_vec()),
        (KeyCode::Backspace, vec![127]),
        (KeyCode::Tab, b"\t".to_vec()),
        (KeyCode::Esc, vec![27]),
        (KeyCode::BackTab, b"\x1b[Z".to_vec()),
        (KeyCode::Char('λ'), "λ".as_bytes().to_vec()),
    ] {
        assert_eq!(
            key_bytes(code, KeyModifiers::NONE, &modes),
            Some(bytes.clone())
        );
        if code != KeyCode::BackTab {
            let mut alt = vec![27];
            alt.extend(bytes);
            assert_eq!(key_bytes(code, KeyModifiers::ALT, &modes), Some(alt));
        }
    }
    assert!(key_bytes(KeyCode::F(13), KeyModifiers::NONE, &modes).is_none());
}

#[test]
fn keypad_and_paste_respect_explicit_modes() {
    let mut modes = TerminalModes {
        app_keypad: true,
        ..Default::default()
    };
    for (code, end) in [
        ('0', 'p'),
        ('9', 'y'),
        ('.', 'n'),
        (',', 'l'),
        ('+', 'k'),
        ('-', 'm'),
        ('*', 'j'),
        ('/', 'o'),
        ('=', 'X'),
    ] {
        assert_eq!(
            keypad(
                KeyCode::Char(code),
                KeyEventState::KEYPAD,
                KeyModifiers::NONE,
                &modes
            )
            .unwrap(),
            format!("\x1bO{end}").as_bytes()
        );
    }
    assert_eq!(
        keypad(
            KeyCode::Enter,
            KeyEventState::KEYPAD,
            KeyModifiers::NONE,
            &modes
        )
        .unwrap(),
        b"\x1bOM"
    );
    assert!(
        keypad(
            KeyCode::Char('a'),
            KeyEventState::KEYPAD,
            KeyModifiers::NONE,
            &modes
        )
        .is_none()
    );
    assert!(
        keypad(
            KeyCode::Enter,
            KeyEventState::NONE,
            KeyModifiers::NONE,
            &modes
        )
        .is_none()
    );
    assert!(
        keypad(
            KeyCode::Enter,
            KeyEventState::KEYPAD,
            KeyModifiers::ALT,
            &modes
        )
        .is_none()
    );
    assert_eq!(paste("plain\ntext".into(), &modes).unwrap(), b"plain\ntext");
    modes.bracketed_paste = true;
    assert_eq!(
        paste("plain\ntext".into(), &modes).unwrap(),
        b"\x1b[200~plain\ntext\x1b[201~"
    );
    assert!(paste("\x1b[201~".into(), &modes).is_err());
}
