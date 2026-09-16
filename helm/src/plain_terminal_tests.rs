use super::*;

#[test]
fn pending_lines_are_transactional_bounded_and_utf8_checked() {
    let mut pending = PendingInput::default();
    assert!(pending.is_empty());
    pending.append("é\r\nsecond\nlast".as_bytes()).unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("é"));
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("second"));
    assert_eq!(pending.take_line().unwrap(), None);
    pending.backspace();
    assert_eq!(pending.bytes, b"las");
    pending.discard();
    pending.append(&[0xff, b'\n']).unwrap();
    assert!(pending.take_line().is_err());
    assert_eq!(pending.bytes, [0xff, b'\n']);
    pending.discard();
    pending.append(&vec![b'a'; MAX_PENDING]).unwrap();
    assert!(pending.append(b"x").is_err());
    assert_eq!(pending.bytes.len(), MAX_PENDING);
}

#[test]
fn backspace_deletes_one_scalar_or_one_malformed_byte() {
    for (input, remaining) in [
        ("a界".as_bytes(), b"a".as_slice()),
        ("é".as_bytes(), b"".as_slice()),
        (&[0xff, 0x80][..], &[0xff][..]),
        (b"", b""),
    ] {
        let mut pending = PendingInput::default();
        pending.append(input).unwrap();
        pending.backspace();
        assert_eq!(pending.bytes, remaining);
    }
}

#[test]
fn prompt_view_scrolls_and_escapes_untrusted_input() {
    assert_eq!(prompt_view(b"abc", 1, 80, false), ("helm> abc".into(), 2));
    assert_eq!(prompt_view(b"abcdef", 6, 4, false), (">ef".into(), 0));
    assert!(
        prompt_view(b"abc", 3, 80, true)
            .0
            .starts_with("limit; Ctrl+U> ")
    );
    assert_eq!(visible("\x1b\n\u{202e}界"), "\\u{1b}\\n\\u{202e}界");
    for width in [0, 1, 8, 18, 40] {
        let (text, back) = prompt_view("界abc\x1b".as_bytes(), 3, width, true);
        assert!(!text.contains('\x1b'));
        assert!(back <= unicode_width::UnicodeWidthStr::width(text.as_str()));
    }
}

#[test]
fn cleanup_failures_are_typed_for_either_stage() {
    assert!(cleanup_result(Ok(()), Ok(())).is_ok());
    for (discard, restore) in [(true, false), (false, true), (true, true)] {
        let failure = || Err(io::Error::other("fixture"));
        let error = cleanup_result(
            if discard { failure() } else { Ok(()) },
            if restore { failure() } else { Ok(()) },
        )
        .unwrap_err();
        assert!(error.downcast_ref::<CleanupFailure>().is_some());
    }
}

#[cfg(unix)]
#[test]
fn input_encoding_caps_repeat_and_rejects_nonbytes() {
    let mut encoding = InputEncoding::default();
    let mut bytes = Vec::new();
    encoding.push(65, 100, &mut bytes).unwrap();
    assert_eq!(bytes, vec![b'A'; 64]);
    encoding.push(66, 0, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 64);
    assert!(encoding.push(256, 1, &mut bytes).is_err());
}

#[test]
fn selection_requires_unique_running_identity() {
    let id = TerminalId(uuid::Uuid::new_v4());
    let other = TerminalId(uuid::Uuid::new_v4());
    let mut items = vec![TerminalSummary {
        id,
        title: "fixture".into(),
        state: TerminalState::Running,
    }];
    assert_eq!(select(&items, "fixture").unwrap(), id);
    assert_eq!(select(&items, &id.to_string()).unwrap(), id);
    assert!(select(&items, "missing").is_err());
    items.push(TerminalSummary {
        id: other,
        title: "fixture".into(),
        state: TerminalState::Running,
    });
    assert!(select(&items, "fixture").is_err());
    assert_eq!(select(&items, &other.to_string()).unwrap(), other);
    for state in [
        TerminalState::Disconnected,
        TerminalState::Exited { code: Some(0) },
        TerminalState::Exited { code: None },
    ] {
        items[0].state = state;
        assert!(select(&items, &id.to_string()).is_err());
    }
}

#[test]
fn frame_formats_emulated_cells_without_forwarding_cell_controls() {
    use crate::terminal::{TerminalCell, TerminalColor};
    let mut snapshot = TerminalSnapshot {
        id: TerminalId(uuid::Uuid::new_v4()),
        title: "fixture\x1b".into(),
        state: TerminalState::Running,
        revision: 1,
        cells: vec![
            vec![
                TerminalCell {
                    text: "界".into(),
                    width: 2,
                    foreground: TerminalColor::Indexed(3),
                    background: TerminalColor::Rgb(1, 2, 3),
                    bold: true,
                    dim: true,
                    italic: true,
                    underlined: true,
                    reversed: true,
                },
                TerminalCell {
                    text: "MUST_SKIP".into(),
                    width: 0,
                    ..Default::default()
                },
                TerminalCell {
                    text: "\x1b".into(),
                    ..Default::default()
                },
                TerminalCell {
                    text: "".into(),
                    ..Default::default()
                },
            ],
            vec![TerminalCell {
                text: "second row".into(),
                ..Default::default()
            }],
        ],
        cursor: Some((1, 0)),
        modes: Default::default(),
        dropped_unread_bytes: 0,
        privacy: None,
    };
    let output = String::from_utf8(frame(&snapshot, 80, 4)).unwrap();
    assert!(output.contains("fixture\\u{1b}"));
    assert!(output.contains("\x1b[38;5;3m"));
    assert!(output.contains("\x1b[48;2;1;2;3m"));
    for code in [1, 2, 3, 4, 7] {
        assert!(output.contains(&format!("\x1b[{code}m")));
    }
    assert!(output.contains("界"));
    assert!(!output.contains("MUST_SKIP"));
    assert!(output.contains("\\u{1b}"));
    assert!(output.ends_with("\x1b[2;2H\x1b[?25h"));
    snapshot.cursor = Some((80, 3));
    assert!(
        !String::from_utf8(frame(&snapshot, 80, 4))
            .unwrap()
            .contains("\x1b[?25h")
    );
    for (columns, rows) in [(0, 0), (1, 1), (2, 2)] {
        let output = String::from_utf8(frame(&snapshot, columns, rows)).unwrap();
        assert!(!output.contains("second row"));
    }
}
