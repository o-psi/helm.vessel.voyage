//! Actual editing-key journeys; no terminal, clipboard, provider or send effect.
use super::*;
use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers as M};

fn draft(text: &str) -> Composer {
    let mut composer = Composer::default();
    composer.set_text(text.into());
    composer
}
fn key(composer: &mut Composer, code: K, modifiers: M) {
    assert!(composer.edit_key(KeyEvent::new(code, modifiers)));
    validate_markers(&composer.text, &composer.markers).unwrap();
    assert!(composer.text.is_char_boundary(composer.cursor));
    assert!(
        !composer
            .markers
            .iter()
            .any(|m| m.start < composer.cursor && composer.cursor < m.end)
    );
}

#[test]
fn word_navigation_keeps_unicode_text_and_crosses_whitespace_without_inserting_it() {
    let original = "αβ  gamma\n界界";
    let mut c = draft(original);
    key(&mut c, K::Home, M::CONTROL);
    key(&mut c, K::Right, M::CONTROL);
    assert_eq!(c.cursor, "αβ".len());
    key(&mut c, K::Right, M::ALT);
    assert_eq!(c.cursor, "αβ  gamma".len());
    key(&mut c, K::Right, M::CONTROL);
    assert_eq!(c.cursor, original.len());
    key(&mut c, K::Right, M::CONTROL);
    assert_eq!(c.cursor, original.len());
    key(&mut c, K::Left, M::ALT);
    assert_eq!(c.cursor, "αβ  gamma\n".len());
    key(&mut c, K::Left, M::CONTROL);
    assert_eq!(c.cursor, "αβ  ".len());
    key(&mut c, K::Left, M::CONTROL);
    key(&mut c, K::Left, M::CONTROL);
    assert_eq!(c.cursor, 0);
    assert_eq!(c.text, original);
}

#[test]
fn word_selection_replaces_exact_authored_range_and_undo_restores_it() {
    let mut c = draft("keep αβ tail");
    c.cursor = "keep ".len();
    key(&mut c, K::Right, M::CONTROL | M::SHIFT);
    assert_eq!(c.selection(), Some((5, "keep αβ".len())));
    assert_eq!(c.insertion_len("Δ".len()), "keep Δ tail".len());
    c.insert_str("Δ");
    assert_eq!(c.text, "keep Δ tail");
    assert_eq!(c.cursor, "keep Δ".len());
    assert_eq!(c.selection(), None);
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "keep αβ tail");
    key(&mut c, K::Char('y'), M::CONTROL);
    assert_eq!(c.text, "keep Δ tail");
}

#[test]
fn backward_and_forward_word_delete_have_distinct_exact_ranges() {
    let mut backward = draft("first  αβ third");
    backward.cursor = "first  αβ".len();
    key(&mut backward, K::Backspace, M::CONTROL);
    assert_eq!(backward.text, "first   third");
    assert_eq!(backward.cursor, "first  ".len());
    let mut forward = draft("first  αβ third");
    forward.cursor = "first".len();
    key(&mut forward, K::Delete, M::ALT);
    assert_eq!(forward.text, "first third");
    assert_eq!(forward.cursor, "first".len());
    key(&mut forward, K::Char('z'), M::CONTROL);
    assert_eq!(forward.text, "first  αβ third");
}

#[test]
fn word_movement_and_deletion_cannot_split_owned_image_labels() {
    let mut c = draft("left ");
    let image = Uuid::from_u128(353);
    c.insert_image(image);
    c.cursor = c.text.len();
    c.insert_str(" right");
    c.cursor = "left ".len();
    key(&mut c, K::Right, M::CONTROL);
    assert_eq!(c.cursor, c.markers[0].end);
    key(&mut c, K::Left, M::CONTROL);
    assert_eq!(c.cursor, c.markers[0].start);
    key(&mut c, K::Delete, M::CONTROL);
    assert_eq!(c.text, "left  right");
    assert!(c.markers.is_empty());
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(
        c.text, "left  right",
        "deleted image ownership cannot resurrect"
    );
}

#[test]
fn vertical_navigation_uses_display_cells_and_short_line_end() {
    let mut c = draft("ab界\nx\npqrs");
    c.viewport_width.set(8);
    c.cursor = "ab".len();
    key(&mut c, K::Down, M::NONE);
    assert_eq!(c.cursor, "ab界\nx".len());
    key(&mut c, K::Down, M::NONE);
    assert_eq!(c.cursor, "ab界\nx\np".len());
    key(&mut c, K::Up, M::NONE);
    assert_eq!(c.cursor, "ab界\nx".len());
    key(&mut c, K::Home, M::CONTROL);
    key(&mut c, K::Up, M::NONE);
    assert_eq!(c.cursor, 0);
    key(&mut c, K::End, M::CONTROL);
    key(&mut c, K::Down, M::NONE);
    assert_eq!(c.cursor, c.text.len());
    assert_eq!(c.text, "ab界\nx\npqrs");
}

#[test]
fn vertical_navigation_preserves_combining_graphemes_and_wrapped_columns() {
    let mut c = draft("e\u{301}a界bc");
    c.viewport_width.set(4);
    c.cursor = "e\u{301}".len();
    key(&mut c, K::Down, M::NONE);
    assert_eq!(c.cursor, "e\u{301}a界b".len());
    key(&mut c, K::Up, M::NONE);
    assert_eq!(c.cursor, "e\u{301}".len());
    assert_eq!(c.authored_text(), "e\u{301}a界bc");
}

#[test]
fn vertical_shift_selection_spans_newlines_then_plain_navigation_clears_it() {
    let mut c = draft("abc\ndef\nghi");
    c.viewport_width.set(8);
    c.cursor = 1;
    key(&mut c, K::Down, M::SHIFT);
    assert_eq!(c.selection(), Some((1, 5)));
    key(&mut c, K::Down, M::SHIFT);
    assert_eq!(c.selection(), Some((1, 9)));
    assert_eq!(&c.text[1..9], "bc\ndef\ng");
    key(&mut c, K::Left, M::NONE);
    assert_eq!(c.selection(), None);
    assert_eq!(c.cursor, 8);
    assert_eq!(c.text, "abc\ndef\nghi");
}

#[test]
fn home_end_edit_current_line_and_control_versions_address_the_whole_draft() {
    let mut c = draft("αβ\nsecond\nlast");
    c.cursor = "αβ\nsec".len();
    key(&mut c, K::Home, M::NONE);
    assert_eq!(c.cursor, "αβ\n".len());
    key(&mut c, K::End, M::NONE);
    assert_eq!(c.cursor, "αβ\nsecond".len());
    key(&mut c, K::Home, M::CONTROL | M::SHIFT);
    assert_eq!(c.selection(), Some((0, "αβ\nsecond".len())));
    key(&mut c, K::End, M::CONTROL);
    assert_eq!(c.selection(), None);
    assert_eq!(c.cursor, c.text.len());
}

#[test]
fn select_all_cut_yanks_only_authored_text_and_cannot_restore_deleted_image() {
    let mut c = draft("before ");
    c.insert_image(Uuid::from_u128(354));
    c.cursor = c.text.len();
    c.insert_str(" after [Image 7]");
    key(&mut c, K::Char('a'), M::CONTROL);
    assert_eq!(c.selection(), Some((0, c.text.len())));
    key(&mut c, K::Char('x'), M::CONTROL);
    assert_eq!(c.text, "");
    assert!(c.markers.is_empty());
    key(&mut c, K::Char('y'), M::ALT);
    assert_eq!(c.text, "before  after [Image 7]");
    assert!(c.markers.is_empty(), "literal labels remain text");
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "");
    key(&mut c, K::Char('Z'), M::CONTROL);
    assert_eq!(c.text, "before  after [Image 7]");
    assert!(c.markers.is_empty());
}

#[test]
fn cut_without_selection_does_not_change_text_or_previous_text_yank() {
    let mut c = draft("αβ");
    key(&mut c, K::Char('a'), M::CONTROL);
    key(&mut c, K::Char('x'), M::CONTROL);
    c.insert_str("kept ");
    key(&mut c, K::Char('x'), M::CONTROL);
    assert_eq!(c.text, "kept ");
    key(&mut c, K::Char('y'), M::ALT);
    assert_eq!(c.text, "kept αβ");
}

#[test]
fn text_yank_obeys_insertion_bound_but_can_replace_an_equal_sized_selection() {
    let mut c = draft(&"x".repeat(65536));
    key(&mut c, K::Char('a'), M::CONTROL);
    key(&mut c, K::Char('x'), M::CONTROL);
    c.insert_str("y");
    key(&mut c, K::Char('y'), M::ALT);
    assert_eq!(c.text, "y", "yank cannot exceed the composer byte bound");
    key(&mut c, K::Char('a'), M::CONTROL);
    key(&mut c, K::Char('y'), M::ALT);
    assert_eq!(c.text, "x".repeat(65536));
    assert_eq!(c.selection(), None);
}

#[test]
fn undo_and_redo_restore_exact_unicode_caret_but_fence_async_paste() {
    let mut c = draft("αβ");
    c.cursor = "α".len();
    c.insert_str("界");
    c.set_paste_anchor();
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "αβ");
    assert_eq!(c.cursor, "α".len());
    assert_eq!(c.take_paste_range(), None);
    c.set_paste_anchor();
    key(&mut c, K::Char('z'), M::CONTROL | M::SHIFT);
    assert_eq!(c.text, "α界β");
    assert_eq!(c.cursor, "α界".len());
    assert_eq!(c.take_paste_range(), None);
}

#[test]
fn new_edit_after_undo_discards_redo_without_replaying_stale_text() {
    let mut c = draft("base");
    c.insert_str(" first");
    key(&mut c, K::Char('z'), M::CONTROL);
    c.insert_str(" replacement");
    key(&mut c, K::Char('y'), M::CONTROL);
    assert_eq!(c.text, "base replacement");
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "base");
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "base");
}

#[test]
fn undo_count_and_total_history_bytes_are_bounded_without_losing_the_latest_edit() {
    let mut c = draft("");
    for _ in 0..100 {
        c.insert('x');
    }
    for _ in 0..80 {
        key(&mut c, K::Char('z'), M::CONTROL);
    }
    assert_eq!(c.text, "x".repeat(36));
    for _ in 0..80 {
        key(&mut c, K::Char('y'), M::CONTROL);
    }
    assert_eq!(c.text, "x".repeat(100));
    c.set_text("α".repeat(32768));
    for _ in 0..20 {
        c.insert('x');
    }
    assert!(c.undo.len() + c.redo.len() <= 64);
    assert!(
        c.undo
            .iter()
            .chain(&c.redo)
            .map(|s| s.text.len())
            .sum::<usize>()
            <= 1024 * 1024
    );
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, format!("{}{}", "α".repeat(32768), "x".repeat(19)));
}

#[test]
fn take_clears_text_image_yank_paste_selection_and_history() {
    let mut c = draft("text");
    key(&mut c, K::Char('a'), M::CONTROL);
    key(&mut c, K::Char('x'), M::CONTROL);
    c.insert_image(Uuid::from_u128(355));
    c.set_paste_anchor();
    let taken = c.take();
    assert_eq!(taken, "[Image 1]");
    key(&mut c, K::Char('y'), M::ALT);
    key(&mut c, K::Char('z'), M::CONTROL);
    assert_eq!(c.text, "");
    assert_eq!(c.cursor, 0);
    assert_eq!(c.selection(), None);
    assert!(c.markers.is_empty());
    assert_eq!(c.take_paste_range(), None);
}

#[test]
fn nonediting_keys_leave_send_history_and_application_authority_with_the_caller() {
    let mut c = draft("unsent Δ");
    c.set_paste_anchor();
    for event in [
        KeyEvent::new(K::Enter, M::NONE),
        KeyEvent::new(K::Enter, M::CONTROL),
        KeyEvent::new(K::Esc, M::NONE),
        KeyEvent::new(K::Tab, M::NONE),
        KeyEvent::new(K::Char('q'), M::NONE),
    ] {
        assert!(!c.edit_key(event));
        assert_eq!(c.text, "unsent Δ");
        assert_eq!(c.cursor, c.text.len());
    }
    assert_eq!(c.take_paste_range(), Some((c.text.len(), c.text.len())));
}

#[test]
fn prompt_history_bounds_entries_and_bytes_and_rejects_empty_oversized_duplicates() {
    let mut h = PromptHistory::default();
    h.record("first");
    h.record("first");
    h.record(" \n\t");
    h.record(&"x".repeat(65537));
    assert_eq!(h.entries, ["first"]);
    for i in 0..300 {
        h.record(&format!("prompt {i}"));
    }
    assert_eq!(h.entries.len(), 256);
    assert_eq!(h.entries.first().unwrap(), "prompt 44");
    assert_eq!(h.entries.last().unwrap(), "prompt 299");
    let mut large = PromptHistory::default();
    for i in 0..20 {
        large.record(&format!("{i:02}{}", "x".repeat(65534)));
    }
    assert_eq!(large.entries.len(), 16);
    assert!(large.entries[0].starts_with("04"));
    assert_eq!(
        large.entries.iter().map(String::len).sum::<usize>(),
        1024 * 1024
    );
}

#[test]
fn recalled_prompts_do_not_overwrite_unsent_unicode_draft_or_resurrect_async_anchor() {
    let mut h = PromptHistory::default();
    h.record("older");
    h.record("newer");
    let mut c = draft("unsent αβ");
    c.cursor = "unsent α".len();
    c.set_paste_anchor();
    h.navigate(&mut c, false);
    assert_eq!(c.text, "unsent αβ");
    h.navigate(&mut c, true);
    assert_eq!(c.text, "newer");
    assert_eq!(c.take_paste_range(), None);
    h.navigate(&mut c, true);
    h.navigate(&mut c, true);
    assert_eq!(c.text, "older");
    h.navigate(&mut c, false);
    assert_eq!(c.text, "newer");
    h.navigate(&mut c, false);
    assert_eq!(c.text, "unsent αβ");
    assert_eq!(c.cursor, "unsent α".len());
    assert_eq!(h.position, None);
    assert_eq!(c.take_paste_range(), None);
}

#[test]
fn history_refuses_to_replace_owned_images_and_record_resets_navigation() {
    let mut h = PromptHistory::default();
    let mut c = draft("draft");
    h.navigate(&mut c, true);
    assert_eq!(c.text, "draft");
    h.record("retained");
    c.insert_image(Uuid::from_u128(356));
    let text = c.text.clone();
    let markers = c.markers.clone();
    h.navigate(&mut c, true);
    assert_eq!(c.text, text);
    assert_eq!(c.markers, markers);
    assert_eq!(h.position, None);
    c.set_text("unsent".into());
    h.navigate(&mut c, true);
    assert_eq!(h.position, Some(0));
    h.record("sent once");
    assert_eq!(h.position, None);
    assert!(h.draft.text.is_empty());
    assert_eq!(h.entries, ["retained", "sent once"]);
}
