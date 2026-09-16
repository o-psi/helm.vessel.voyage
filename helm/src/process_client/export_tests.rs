use super::*;

#[test]
fn chunk_assembly_is_utf8_byte_exact_and_rejects_inconsistent_continuations() {
    let session = Uuid::new_v4();
    let first = json!({"session_id":session,"revision":4,"index":2,"offset":0,"encoding":"public_message_json_utf8","total_bytes":5,"data":"é","next_offset":2,"has_more":true});
    let mut text = String::new();
    assert!(append_response_chunk(&mut text, &first, session, 4, 2).unwrap());
    assert_eq!(text, "é");
    let last = json!({"session_id":session,"revision":4,"index":2,"offset":2,"encoding":"public_message_json_utf8","total_bytes":5,"data":"界","next_offset":5,"has_more":false});
    assert!(!append_response_chunk(&mut text, &last, session, 4, 2).unwrap());
    assert_eq!(text, "é界");
    for (field, value) in [
        ("session_id", json!(Uuid::new_v4())),
        ("revision", json!(5)),
        ("index", json!(3)),
        ("offset", json!(1)),
        ("encoding", json!("other")),
        ("total_bytes", json!(null)),
        ("total_bytes", json!(COPY_LIMIT + 1)),
        ("data", json!(null)),
        ("has_more", json!(null)),
        ("data", json!("")),
        ("next_offset", json!(3)),
        ("has_more", json!(false)),
        ("total_bytes", json!(1)),
    ] {
        let mut bad = first.clone();
        bad[field] = value;
        let mut text = String::new();
        assert!(
            append_response_chunk(&mut text, &bad, session, 4, 2).is_err(),
            "{field}"
        );
        assert!(text.is_empty());
    }
}

#[test]
fn copy_requires_complete_nonempty_canonical_assistant_text() {
    for value in [
        json!({}),
        json!({"role":"assistant"}),
        json!({"role":"assistant","content":17}),
        json!({"role":"assistant","content":""}),
        json!({"role":"assistant","content":"x".repeat(COPY_LIMIT + 1)}),
        json!({"role":"user","content":"x"}),
        json!({"role":"assistant","content":"x","projection_truncated":true}),
    ] {
        assert!(canonical_response(&value).is_err());
    }
    let text = "## Heading\n\ncanonical\x1b[2J界";
    assert_eq!(
        canonical_response(&json!({"role":"assistant","content":text})).unwrap(),
        text
    );
    assert!(response_clipboard_sequence("").is_err());
    assert!(response_clipboard_sequence(&"x".repeat(COPY_LIMIT + 1)).is_err());
    let encoded = response_clipboard_sequence(text).unwrap();
    use base64::Engine;
    let payload = encoded
        .strip_prefix("\x1b]52;c;")
        .unwrap()
        .strip_suffix('\x07')
        .unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap(),
        text.as_bytes()
    );
}
