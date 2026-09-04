use super::*;

#[test]
fn prompt_history_preserves_draft_and_bounds() {
    let mut history = PromptHistory::default();
    let mut composer = Composer::default();
    composer.insert_str("draft λ");
    composer.cursor = 5;
    history.navigate(&mut composer, true);
    assert_eq!(composer.text, "draft λ");
    history.record("first");
    history.record("second\n界");
    history.navigate(&mut composer, false);
    assert_eq!(composer.cursor, 5);
    history.navigate(&mut composer, true);
    assert_eq!(composer.text, "second\n界");
    assert_eq!(composer.cursor, composer.text.len());
    history.navigate(&mut composer, true);
    history.navigate(&mut composer, true);
    assert_eq!(composer.text, "first");
    composer.insert('!');
    history.navigate(&mut composer, false);
    assert_eq!(composer.text, "second\n界");
    history.navigate(&mut composer, false);
    assert_eq!(composer.text, "draft λ");
    assert_eq!(composer.cursor, 5);
    assert_eq!(history.entries[0], "first");
    history.navigate(&mut composer, true);
    composer.insert('!');
    history.record(&composer.take());
    history.navigate(&mut composer, true);
    assert_eq!(composer.text, "second\n界!");
    history.navigate(&mut composer, false);
    assert!(composer.text.is_empty());
}

#[test]
fn prompt_history_seeds_only_user_messages_and_resets_for_new_session() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    for (role, text) in [
        (Role::System, "instructions"),
        (Role::User, "first"),
        (Role::Assistant, "reply"),
        (Role::Tool, "result"),
        (Role::User, "  "),
        (Role::User, "last"),
    ] {
        session.messages.push(crate::Message::new(role, text));
    }
    let mut app = App::new(session, Vec::new());
    assert_eq!(app.prompt_history.entries, ["first", "last"]);
    app.session.messages.clear();
    app.prompt_history.navigate(&mut app.composer, true);
    assert_eq!(app.composer.text, "last");
    start_new_session(&mut app, None);
    assert!(app.prompt_history.entries.is_empty());
    assert!(app.prompt_history.position.is_none());
}

#[test]
fn composer_edits_utf8_safely() {
    let mut composer = Composer::default();
    composer.insert('λ');
    composer.insert('x');
    composer.backspace();
    assert_eq!(composer.text, "λ");
    composer.backspace();
    assert!(composer.text.is_empty());
    composer.insert_str("one\nλtwo");
    composer.line_start();
    assert_eq!(&composer.text[composer.cursor..], "λtwo");
    composer.line_end();
    composer.delete();
    assert_eq!(composer.cursor, composer.text.len());
}

#[test]
fn truncation_uses_character_boundaries() {
    assert_eq!(one_line("αβγδε", 3), "αβγ…");
    assert_eq!(
        compact_line(
            "HTTP 400: {\n  \"error\": {\n    \"message\": \"No tool call found\"\n  }\n}",
            200,
        ),
        "HTTP 400: { \"error\": { \"message\": \"No tool call found\" } }"
    );
}

#[test]
fn cursor_tracks_wrapping_and_wide_characters() {
    assert_eq!(cursor_position("abcd", 4), (1, 0));
    assert_eq!(cursor_position("ab\n界", 4), (1, 2));
    assert_eq!(cursor_position("abcde", 4), (1, 1));
}

#[test]
fn display_text_cannot_emit_terminal_control_sequences() {
    assert_eq!(display_safe("before\u{1b}[2Jafter\r"), "before�[2Jafter�");
    assert_eq!(
        display_safe("line one\nline two\tvalue"),
        "line one\nline two\tvalue"
    );
}
