//! Width and sanitization checks only; never constructs a live terminal backend.
use super::clipped;
use unicode_width::UnicodeWidthStr;

#[test]
fn ascii_clipping_includes_exact_boundary_and_empty_input() {
    for (columns, expected) in [(0, ""), (1, "a"), (3, "abc"), (4, "abcd"), (80, "abcd")] {
        assert_eq!(clipped("abcd", columns), expected);
    }
    assert_eq!(clipped("", u16::MAX), "");
}

#[test]
fn wide_graphemes_are_never_split_or_skipped_to_fit_later_text() {
    for (columns, expected) in [
        (0, ""),
        (1, ""),
        (2, "界"),
        (3, "界a"),
        (4, "界a"),
        (5, "界a界"),
    ] {
        assert_eq!(clipped("界a界", columns), expected);
    }
    assert_eq!(clipped("a界z", 2), "a");
}

#[test]
fn combining_sequences_stay_attached_at_clipping_boundary() {
    assert_eq!(clipped("e\u{301}x", 1), "e\u{301}");
    assert_eq!(clipped("e\u{301}x", 2), "e\u{301}x");
    assert_eq!(clipped("éx", 1), "é");
}

#[test]
fn emoji_sequences_are_kept_as_complete_graphemes() {
    for grapheme in ["👩‍💻", "🇬🇧", "👍🏽"] {
        let width = grapheme.width() as u16;
        assert!(width > 0);
        let text = format!("{grapheme}x");
        assert_eq!(clipped(&text, width - 1), "");
        assert_eq!(clipped(&text, width), grapheme);
        assert_eq!(clipped(&text, width + 1), text);
    }
}

#[test]
fn sanitization_happens_before_width_is_charged() {
    assert_eq!(clipped("a\x1b\0\r\x7fb", 2), "ab");
    assert_eq!(clipped("\u{202e}ab\u{2069}c", 3), "abc");
    assert_eq!(clipped("a\tb", 5), "a    ");
    assert_eq!(clipped("a\tb", 6), "a    b");
}

#[test]
fn every_direction_override_is_removed_even_at_narrow_widths() {
    for code in (0x202a..=0x202e).chain(0x2066..=0x2069) {
        let ch = char::from_u32(code).unwrap();
        let text = format!("{ch}x{ch}");
        assert_eq!(clipped(&text, 0), "");
        assert_eq!(clipped(&text, 1), "x");
    }
}

#[test]
fn width_bound_holds_for_mixed_unicode_and_sanitized_controls() {
    for text in [
        "plain title",
        "界界界",
        "e\u{301}界🦀",
        "a\tb",
        "\x1b[31mred",
        "👩‍💻 status",
    ] {
        for columns in 0..=32 {
            let output = clipped(text, columns);
            assert!(
                output.width() <= usize::from(columns),
                "{text:?}, {columns}: {output:?}"
            );
            assert!(!output.contains('\x1b'));
            assert!(!output.contains('\t'));
        }
    }
}
