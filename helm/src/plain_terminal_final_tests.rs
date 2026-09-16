//! Pure console regression checks: no terminal handles, environment, or global ownership.
use super::*;
use crate::terminal::{TerminalCell, TerminalColor, TerminalError, TerminalModes};

fn summary(number: u128, title: &str, state: TerminalState) -> TerminalSummary {
    TerminalSummary {
        id: TerminalId(uuid::Uuid::from_u128(number)),
        title: title.into(),
        state,
    }
}

fn snapshot(cells: Vec<Vec<TerminalCell>>) -> TerminalSnapshot {
    TerminalSnapshot {
        id: TerminalId(uuid::Uuid::from_u128(1)),
        title: "editor".into(),
        state: TerminalState::Running,
        revision: 7,
        cells,
        cursor: None,
        modes: TerminalModes::default(),
        dropped_unread_bytes: 0,
        privacy: None,
    }
}

fn cell(text: &str) -> TerminalCell {
    TerminalCell {
        text: text.into(),
        ..TerminalCell::default()
    }
}

fn rendered(snapshot: &TerminalSnapshot, columns: u16, rows: u16) -> String {
    String::from_utf8(frame(snapshot, columns, rows)).unwrap()
}

#[test]
fn selection_requires_an_exact_unique_reference() {
    let items = vec![
        summary(1, "shell", TerminalState::Running),
        summary(2, "shell-two", TerminalState::Running),
    ];
    assert_eq!(select(&items, "shell").unwrap(), items[0].id);
    assert_eq!(
        select(&items, &items[1].id.to_string()).unwrap(),
        items[1].id
    );
    for reference in ["", "shel", "Shell", " shell", "shell ", "unknown"] {
        assert!(
            select(&items, reference)
                .unwrap_err()
                .to_string()
                .contains("missing or ambiguous")
        );
    }
    assert!(select(&[], "shell").is_err());
}

#[test]
fn duplicate_titles_and_id_title_collisions_are_ambiguous() {
    let first = summary(1, "same", TerminalState::Running);
    let second = summary(2, "same", TerminalState::Running);
    assert!(select(&[first.clone(), second], "same").is_err());
    let collision = summary(3, &first.id.to_string(), TerminalState::Running);
    assert!(select(&[first.clone(), collision], &first.id.to_string()).is_err());
    // A single item matching both predicates is still one match.
    let self_named = summary(1, &first.id.to_string(), TerminalState::Running);
    assert_eq!(
        select(&[self_named], &first.id.to_string()).unwrap(),
        first.id
    );
}

#[test]
fn saved_metadata_cannot_reattach_any_nonrunning_state() {
    for state in [
        TerminalState::Disconnected,
        TerminalState::Exited { code: None },
        TerminalState::Exited { code: Some(0) },
        TerminalState::Exited { code: Some(9) },
    ] {
        let item = summary(4, "old", state);
        for reference in ["old".to_owned(), item.id.to_string()] {
            assert!(
                select(std::slice::from_ref(&item), &reference)
                    .unwrap_err()
                    .to_string()
                    .contains("no longer running")
            );
        }
    }
    let items = [
        summary(1, "same", TerminalState::Running),
        summary(2, "same", TerminalState::Disconnected),
    ];
    assert!(
        select(&items, "same")
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
}

#[test]
fn pending_lines_preserve_suffix_and_consume_crlf_as_one_separator() {
    let mut pending = PendingInput::default();
    assert!(pending.is_empty());
    pending.append(b"one\r\ntwo\nthree\rfour").unwrap();
    for expected in ["one", "two", "three"] {
        assert_eq!(pending.take_line().unwrap().as_deref(), Some(expected));
    }
    assert_eq!(pending.take_line().unwrap(), None);
    assert_eq!(pending.bytes, b"four");
    pending.append(b"\n").unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("four"));
    assert!(pending.is_empty());
}

#[test]
fn pending_empty_lines_and_split_utf8_are_not_lost() {
    let mut pending = PendingInput::default();
    pending.append(b"\n\r\n\r").unwrap();
    for _ in 0..3 {
        assert_eq!(pending.take_line().unwrap(), Some(String::new()));
    }
    pending.append(&[0xe7, 0x95]).unwrap();
    assert_eq!(pending.take_line().unwrap(), None);
    pending.append(&[0x8c, b'\n']).unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("界"));
    assert!(pending.is_empty());
}

#[test]
fn invalid_pending_line_is_retained_until_explicit_discard() {
    let mut pending = PendingInput::default();
    pending.append(b"ok\n\xff\nnext\n").unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("ok"));
    let original = pending.bytes.clone();
    for _ in 0..2 {
        assert!(
            pending
                .take_line()
                .unwrap_err()
                .to_string()
                .contains("not UTF-8")
        );
        assert_eq!(pending.bytes, original);
    }
    pending.discard();
    assert!(pending.is_empty());
    pending.append(b"recovered\n").unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("recovered"));
}

#[test]
fn pending_capacity_refusal_is_atomic_and_capacity_is_reusable() {
    let mut pending = PendingInput::default();
    pending.append(&vec![b'x'; MAX_PENDING - 1]).unwrap();
    assert!(pending.append(b"yz").is_err());
    assert_eq!(pending.bytes.len(), MAX_PENDING - 1);
    assert!(pending.bytes.iter().all(|byte| *byte == b'x'));
    pending.append(b"\n").unwrap();
    pending.append(b"").unwrap();
    assert!(pending.append(b"z").is_err());
    assert_eq!(pending.take_line().unwrap().unwrap().len(), MAX_PENDING - 1);
    pending.append(b"again\n").unwrap();
    assert_eq!(pending.take_line().unwrap().as_deref(), Some("again"));
}

#[test]
fn backspace_removes_one_scalar_not_one_byte_or_grapheme() {
    let mut pending = PendingInput::default();
    pending.append("aé界🦀e\u{301}".as_bytes()).unwrap();
    for expected in ["aé界🦀e", "aé界🦀", "aé界", "aé", "a", "", ""] {
        pending.backspace();
        assert_eq!(pending.bytes, expected.as_bytes());
    }
}

#[test]
fn malformed_suffixes_remain_editable_one_byte_at_a_time() {
    for bytes in [
        vec![0xff],
        vec![b'a', 0x80],
        vec![0xe7, 0x95],
        vec![0x80; 8],
        vec![0xc0, 0x80],
    ] {
        assert_eq!(previous_scalar_start(&bytes, bytes.len()), bytes.len() - 1);
        let mut pending = PendingInput::default();
        pending.append(&bytes).unwrap();
        pending.backspace();
        assert_eq!(pending.bytes, bytes[..bytes.len() - 1]);
    }
    assert_eq!(previous_scalar_start(b"", 0), 0);
    assert_eq!(previous_scalar_start("a界z".as_bytes(), 4), 1);
}

#[test]
fn visibility_escapes_controls_and_direction_overrides() {
    assert_eq!(
        visible("a\n\r\t\0\x1b\x7f"),
        "a\\n\\r\\t\\u{0}\\u{1b}\\u{7f}"
    );
    for code in (0x202a..=0x202e).chain(0x2066..=0x2069) {
        let ch = char::from_u32(code).unwrap();
        assert_eq!(visible(&ch.to_string()), ch.escape_default().to_string());
    }
    assert_eq!(visible("é界e\u{301}🦀"), "é界e\u{301}🦀");
}

#[test]
fn prompt_prefixes_follow_width_and_blocked_thresholds() {
    for columns in 0..=8 {
        assert_eq!(prompt_view(b"", 0, columns, true), (">".into(), 0));
    }
    for columns in [9, 18] {
        assert_eq!(prompt_view(b"", 0, columns, true), ("helm> ".into(), 0));
    }
    assert_eq!(prompt_view(b"", 0, 19, true), ("limit; Ctrl+U> ".into(), 0));
    assert_eq!(prompt_view(b"", 0, 19, false), ("helm> ".into(), 0));
}

#[test]
fn prompt_scrolls_left_and_counts_suffix_in_display_columns() {
    assert_eq!(
        prompt_view(b"abcdefgh", 8, 11, false),
        ("helm> efgh".into(), 0)
    );
    assert_eq!(
        prompt_view(b"abcdefgh", 2, 11, false),
        ("helm> abcd".into(), 2)
    );
    assert_eq!(
        prompt_view("a界z".as_bytes(), 1, 11, false),
        ("helm> a界z".into(), 3)
    );
    assert_eq!(
        prompt_view(b"abc", usize::MAX, 20, false),
        ("helm> abc".into(), 0)
    );
    assert_eq!(prompt_view(b"abcdef", 0, 4, false), (">ab".into(), 2));
}

#[test]
fn prompt_sanitizes_invalid_utf8_and_preserves_combining_width() {
    assert_eq!(prompt_view(b"a\xff", 2, 20, false), ("helm> a�".into(), 0));
    assert_eq!(
        prompt_view("e\u{301}界".as_bytes(), 0, 10, false),
        ("helm> e\u{301}界".into(), 3)
    );
    let (text, back) = prompt_view(b"\x1b\n", 0, 80, false);
    assert_eq!(text, "helm> \\u{1b}\\n");
    assert_eq!(back, 8);
}

#[test]
fn frame_reserves_header_and_obeys_row_and_column_limits() {
    let screen = snapshot(vec![vec![cell("A"), cell("B"), cell("C")], vec![cell("D")]]);
    let text = rendered(&screen, 2, 2);
    assert!(text.starts_with("\x1b[?25l\x1b[0m\x1b[H\x1b[2KC"));
    assert!(text.contains("\x1b[2;1H\x1b[0m\x1b[2K\x1b[0mA\x1b[0mB"));
    assert!(!text.contains("\x1b[0mC"));
    assert!(!text.contains("\x1b[3;1H"));
    for rows in [0, 1] {
        assert!(!rendered(&screen, 80, rows).contains("\x1b[2;1H"));
    }
    assert_eq!(
        rendered(&snapshot(vec![]), 0, 0),
        "\x1b[?25l\x1b[0m\x1b[H\x1b[2K"
    );
}

#[test]
fn frame_emits_both_color_models_and_all_attributes() {
    let mut decorated = cell("X");
    decorated.foreground = TerminalColor::Indexed(17);
    decorated.background = TerminalColor::Rgb(1, 2, 3);
    decorated.bold = true;
    decorated.dim = true;
    decorated.italic = true;
    decorated.underlined = true;
    decorated.reversed = true;
    let mut other = cell("Y");
    other.foreground = TerminalColor::Rgb(4, 5, 6);
    other.background = TerminalColor::Indexed(255);
    let text = rendered(&snapshot(vec![vec![decorated, other, cell("Z")]]), 80, 3);
    assert!(
        text.contains("\x1b[0m\x1b[38;5;17m\x1b[48;2;1;2;3m\x1b[1m\x1b[2m\x1b[3m\x1b[4m\x1b[7mX")
    );
    assert!(text.contains("\x1b[0m\x1b[38;2;4;5;6m\x1b[48;5;255mY\x1b[0mZ"));
}

#[test]
fn frame_skips_wide_continuations_and_renders_empty_cells_as_spaces() {
    let screen = snapshot(vec![vec![
        cell("界"),
        cell("SHOULD_NOT_APPEAR"),
        cell(""),
        cell("Z"),
    ]]);
    let text = rendered(&screen, 80, 3);
    assert!(text.ends_with("\x1b[0m界\x1b[0m \x1b[0mZ"));
    assert!(!text.contains("SHOULD_NOT_APPEAR"));
}

#[test]
fn frame_sanitizes_title_and_cell_payload_without_touching_metadata() {
    let mut screen = snapshot(vec![vec![
        cell("\x1b"),
        cell("x"),
        cell("x"),
        cell("x"),
        cell("x"),
        cell("x"),
        cell("x"),
    ]]);
    screen.title = "name\n\u{202e}".into();
    let saved = screen.clone();
    let text = rendered(&screen, 100, 3);
    assert!(text.contains("name\\n\\u{202e}"));
    assert!(text.contains("\\u{1b}"));
    assert!(!text.contains('\u{202e}'));
    assert_eq!(screen, saved);
}

#[test]
fn frame_only_shows_cursor_inside_content_rectangle() {
    let mut screen = snapshot(vec![]);
    for (cursor, expected) in [
        (Some((0, 0)), Some("\x1b[2;1H\x1b[?25h")),
        (Some((4, 2)), Some("\x1b[4;5H\x1b[?25h")),
        (Some((5, 0)), None),
        (Some((0, 3)), None),
        (Some((u16::MAX, u16::MAX)), None),
        (None, None),
    ] {
        screen.cursor = cursor;
        let text = rendered(&screen, 5, 4);
        if let Some(suffix) = expected {
            assert!(text.ends_with(suffix));
        } else {
            assert!(!text.contains("\x1b[?25h"));
        }
    }
    screen.cursor = Some((0, 0));
    for (columns, rows) in [(0, 4), (5, 0), (5, 1)] {
        assert!(!rendered(&screen, columns, rows).contains("\x1b[?25h"));
    }
}

#[test]
fn cleanup_requires_both_isolation_and_restoration_to_succeed() {
    for discard_failed in [false, true] {
        for restore_failed in [false, true] {
            let result = cleanup_result(
                if discard_failed {
                    Err(io::Error::other("discard"))
                } else {
                    Ok(())
                },
                if restore_failed {
                    Err(io::Error::other("restore"))
                } else {
                    Ok(())
                },
            );
            if discard_failed || restore_failed {
                let error = result.unwrap_err();
                assert!(error.downcast_ref::<CleanupFailure>().is_some());
                assert!(error.to_string().contains("chat must stop"));
            } else {
                result.unwrap();
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn byte_encoding_bounds_repeat_and_rejects_nonbytes_atomically() {
    let mut encoding = InputEncoding::default();
    let mut bytes = vec![b'x'];
    encoding.push(0xff, 100, &mut bytes).unwrap();
    assert_eq!(bytes.len(), 65);
    assert!(bytes[1..].iter().all(|byte| *byte == 0xff));
    let saved = bytes.clone();
    encoding.push(0x100, 0, &mut bytes).unwrap();
    assert!(encoding.push(0x100, 1, &mut bytes).is_err());
    assert_eq!(bytes, saved);
    encoding.push(b'a'.into(), 2, &mut bytes).unwrap();
    assert!(bytes.ends_with(b"aa"));
}

#[tokio::test]
async fn terminal_operation_preserves_success_and_typed_failure() {
    let cancel = CancellationToken::new();
    assert_eq!(
        terminal_operation(async { Ok(42) }, &cancel).await.unwrap(),
        42
    );
    let error = terminal_operation::<()>(async { Err(TerminalError::Closed) }, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<TerminalError>(),
        Some(TerminalError::Closed)
    ));
}

#[tokio::test]
async fn cancellation_wins_without_polling_ready_operation() {
    let cancel = CancellationToken::new();
    cancel.cancel();
    let polled = std::cell::Cell::new(false);
    let operation = async {
        polled.set(true);
        Ok(())
    };
    let error = terminal_operation(operation, &cancel).await.unwrap_err();
    assert!(error.to_string().contains("interrupted"));
    assert!(!polled.get());
}

#[tokio::test]
async fn pending_operation_can_be_cancelled_without_terminal_io() {
    let cancel = CancellationToken::new();
    let operation = terminal_operation::<()>(std::future::pending(), &cancel);
    let cancellation = async {
        tokio::task::yield_now().await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(operation, cancellation);
    assert!(result.unwrap_err().to_string().contains("interrupted"));
}

#[tokio::test]
async fn stalled_operation_times_out_without_terminal_io() {
    let cancel = CancellationToken::new();
    let error = terminal_operation::<()>(std::future::pending(), &cancel)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "terminal operation timed out; detached");
    assert!(!cancel.is_cancelled());
}
