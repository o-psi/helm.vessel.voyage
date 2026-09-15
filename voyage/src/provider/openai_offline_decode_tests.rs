use super::*;

#[test]
fn decode_and_encode_multi_call_transcript() {
    let response = decode_response(json!({"service_tier":"default","choices":[{"finish_reason":"tool_calls","message":{"content":"answer","tool_calls":[{"id":"one","function":{"name":"lookup","arguments":"{\"q\":1}"}},{"id":"two","function":{"name":"ping"}}]}}],"usage":{"prompt_tokens":42,"completion_tokens":5}})).unwrap();
    assert_eq!(response.usage.input_tokens, 42);
    assert_eq!(response.usage.output_tokens, 5);
    assert_eq!(response.message.tool_calls[1].arguments, json!({}));
    let encoded = encode_message(&response.message).unwrap();
    assert_eq!(encoded["role"], "assistant");
    assert_eq!(
        encoded["tool_calls"][0]["function"]["arguments"],
        "{\"q\":1}"
    );
    let mut tool = Message::new(Role::User, "tool output");
    tool.role = Role::Tool;
    tool.tool_call_id = Some("one".into());
    let mut system = Message::new(Role::User, "instructions");
    system.role = Role::System;
    let messages = encode_messages(&[
        system,
        Message::new(Role::User, "question"),
        response.message,
        tool,
    ])
    .unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[3]["tool_call_id"], "one");
    assert_eq!(messages[3]["content"], "tool output");
}

#[test]
fn decode_requires_terminal_reason_and_valid_argument_json() {
    for reason in [
        json!(null),
        json!("unknown"),
        json!("length"),
        json!("content_filter"),
    ] {
        assert!(
            decode_response(
                json!({"choices":[{"finish_reason":reason,"message":{"content":"partial"}}]})
            )
            .is_err()
        );
    }
    assert!(decode_response(json!({"choices":[{"finish_reason":"stop"}]})).is_err());
    assert!(decode_response(json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"function":{"arguments":"{"}}]}}]})).is_err());
    let response =
        decode_response(json!({"choices":[{"finish_reason":"stop","message":{"content":null}}]}))
            .unwrap();
    assert!(response.message.content.is_empty());
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(response.usage.input_tokens, 0);
}

#[test]
fn streamed_call_identity_and_arguments_are_validated_at_completion() {
    for call in [
        CallAssembly::default(),
        CallAssembly {
            id: "id".into(),
            ..Default::default()
        },
        CallAssembly {
            name: "name".into(),
            ..Default::default()
        },
        CallAssembly {
            id: "id".into(),
            name: "name".into(),
            arguments: "{".into(),
        },
    ] {
        let assembly = StreamAssembly {
            calls: vec![call],
            finish_reason: Some("tool_calls".into()),
            ..Default::default()
        };
        assert!(finish_stream(assembly).is_err());
    }
    let mut assembly = StreamAssembly::default();
    let delta = apply_stream_chunk(&json!({"usage":{"prompt_tokens":8}}), &mut assembly).unwrap();
    assert!(delta.is_empty());
    apply_stream_chunk(&json!({"usage":{"completion_tokens":3}}), &mut assembly).unwrap();
    assert_eq!(assembly.usage.input_tokens, 8);
    assert_eq!(assembly.usage.output_tokens, 3);
    assert!(
        apply_stream_chunk(&json!({"error":{"message":"PRIVATE"}}), &mut assembly)
            .unwrap_err()
            .to_string()
            .find("PRIVATE")
            .is_none()
    );
}

#[test]
fn sse_frames_preserve_pending_tail_and_ignore_comments() {
    let mut pending = b": ping\r\n\r\ndata: {\"ok\":true}\r\n\r\ntail".to_vec();
    let comment = take_sse_frame(&mut pending).unwrap();
    assert!(sse_data(&comment).is_empty());
    let event = take_sse_frame(&mut pending).unwrap();
    assert_eq!(sse_data(&event), br#"{"ok":true}"#);
    assert_eq!(pending, b"tail");
    assert!(take_sse_frame(&mut pending).is_none());
    assert_eq!(pending, b"tail");
}
