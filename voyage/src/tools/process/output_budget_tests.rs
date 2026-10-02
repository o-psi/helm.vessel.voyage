//! Rendered output/source-byte contract. Invalid terminal bytes and tiny budgets
//! must not lose unseen source or turn presentation expansion into extra budget.
use super::*;
fn capture(bytes: &[u8]) -> Capture {
    let mut capture = Capture::new(24, 80);
    capture.capture_output(bytes, 1024);
    capture
}
fn payload(text: &str) -> &str {
    text.split("\n\n[").next().unwrap()
}
#[test]
fn bounded_ascii_reads_deliver_all_source_once_with_exact_remaining_metadata() {
    let capture = capture(&[b'a'; 256]);
    let mut cursor = 0;
    let mut delivered = String::new();
    while cursor < capture.bytes.len() {
        let (text, next) = unread_chunk(&capture, cursor, 40).unwrap();
        assert!(text.len() <= 40 && next > cursor && next <= capture.bytes.len());
        assert_eq!(payload(&text).as_bytes(), &capture.bytes[cursor..next]);
        if next < capture.bytes.len() {
            assert!(text.ends_with(&format!(
                "[{} unread bytes remain]",
                capture.bytes.len() - next
            )));
        }
        delivered.push_str(payload(&text));
        cursor = next;
    }
    assert_eq!(delivered, "a".repeat(256));
}
#[test]
fn valid_unicode_budget_boundaries_never_create_replacement_characters() {
    for glyph in ["é", "界", "🛶"] {
        let capture = capture(glyph.repeat(32).as_bytes());
        for max in 32..=48 {
            let mut cursor = 0;
            let mut delivered = String::new();
            while cursor < capture.bytes.len() {
                let (text, next) = unread_chunk(&capture, cursor, max).unwrap();
                assert!(text.len() <= max && next > cursor && next <= capture.bytes.len());
                assert_eq!(next % glyph.len(), 0);
                assert!(!text.contains('\u{fffd}'));
                assert_eq!(
                    payload(&text),
                    std::str::from_utf8(&capture.bytes[cursor..next]).unwrap()
                );
                delivered.push_str(payload(&text));
                cursor = next;
            }
            assert_eq!(delivered, glyph.repeat(32));
        }
    }
}

#[test]
fn invalid_utf8_expansion_stays_bounded_and_consumes_exact_source_bytes() {
    let capture = capture(&[0xff; 128]);
    let mut cursor = 0;
    let mut replacements = 0;
    while cursor < capture.bytes.len() {
        let (text, next) = unread_chunk(&capture, cursor, 32).unwrap();
        assert!(text.len() <= 32 && next > cursor && next <= capture.bytes.len());
        assert_eq!(
            payload(&text),
            String::from_utf8_lossy(&capture.bytes[cursor..next])
        );
        assert_eq!(payload(&text).chars().count(), next - cursor);
        replacements += payload(&text).chars().count();
        cursor = next;
    }
    assert_eq!(replacements, 128);
}
#[test]
fn metadata_without_room_for_source_is_refused_without_consumption() {
    let ascii = capture(&[b'a'; 128]);
    for max in 0..=27 {
        assert!(unread_chunk(&ascii, 0, max).is_err());
    }
    let (empty, next) = unread_chunk(&ascii, ascii.bytes.len(), 0).unwrap();
    assert!(empty.is_empty());
    assert_eq!(next, ascii.bytes.len());
    let unicode = capture("界界".as_bytes());
    for max in 0..=2 {
        assert!(unread_chunk(&unicode, 0, max).is_err());
    }
}

#[test]
fn privacy_notice_requires_its_complete_budget_and_never_returns_capture() {
    let mut capture = capture(b"synthetic-private-output-canary");
    capture.make_private(0);
    let (notice, next) = unread_chunk(&capture, 0, 1024).unwrap();
    assert!(!notice.contains("synthetic-private-output-canary"));
    assert_eq!(next, capture.base);
    assert!(unread_chunk(&capture, 0, notice.len() - 1).is_err());
    assert_eq!(
        unread_chunk(&capture, 0, notice.len()).unwrap(),
        (notice, next)
    );
    assert!(capture.bytes.is_empty());
}
#[test]
fn already_dropped_utf8_fragment_keeps_exact_lossy_source_mapping() {
    let mut capture = Capture::new(24, 80);
    capture.capture_output("界".repeat(20).as_bytes(), 31);
    assert_eq!(capture.dropped, 29);
    assert_eq!(capture.base, 29);
    let (text, next) = unread_chunk(&capture, 0, 33).unwrap();
    assert_eq!(text, String::from_utf8_lossy(&capture.bytes));
    assert_eq!(next, 60);
    assert_eq!(text.len(), 33);
}
