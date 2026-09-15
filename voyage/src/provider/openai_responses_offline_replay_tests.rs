use super::*;

fn output() -> Vec<Value> {
    vec![
        json!({"type":"reasoning","id":"r","encrypted_content":"opaque","summary":[{"type":"summary_text","text":"why"}],"status":"completed"}),
        json!({"type":"message","role":"assistant","id":"m","content":[{"type":"output_text","text":"answer"}],"status":"completed"}),
        json!({"type":"function_call","id":"f","call_id":"c","name":"lookup","arguments":"{\"q\":1}","status":"completed"}),
    ]
}

#[test]
fn replay_round_trip_and_response_message_encoding() {
    let output = output();
    let replay = make_replay(&output).unwrap();
    assert_eq!(decode_replay(&replay).unwrap(), output);
    let response = decode_response(
        json!({"status":"completed","output":output,"usage":{"input_tokens":3,"output_tokens":2}}),
    )
    .unwrap();
    assert_eq!(response.message.content, "answer");
    assert_eq!(response.message.tool_calls[0].id, "c");
    assert_eq!(response.message.tool_calls[0].arguments, json!({"q":1}));
    let encoded = encode_message(&response.message).unwrap();
    assert_eq!(encoded, output);
    assert_eq!(response.message.provider_state, Some(replay));
}

#[test]
fn replay_rejects_untrusted_schema_versions_and_fields() {
    let good = make_replay(&output()).unwrap();
    for (key, value) in [
        ("kind", json!("other")),
        ("version", json!(255)),
        ("items", json!({})),
        ("unexpected", json!(true)),
    ] {
        let mut bad = good.clone();
        bad[key] = value;
        assert!(decode_replay(&bad).is_err(), "accepted {key}");
    }
    for item in [
        json!({}),
        json!({"type":"web_search_call"}),
        json!({"type":"reasoning","id":"r"}),
        json!({"type":"reasoning","encrypted_content":"x"}),
        json!({"type":"function_call","name":"lookup"}),
        json!({"type":"function_call","call_id":"c"}),
        json!({"type":"message","role":"user","content":[]}),
    ] {
        assert!(make_replay(&[item]).is_err());
    }
    let mut bad = good;
    bad["items"][0]["private_extra"] = json!(true);
    assert!(decode_replay(&bad).is_err());
}

#[test]
fn replay_limits_and_lossy_provider_fields_are_explicit() {
    let item = json!({"type":"message","role":"assistant","content":[]});
    assert!(make_replay(&vec![item; MAX_REPLAY_ITEMS + 1]).is_err());
    let huge =
        json!({"type":"reasoning","id":"r","encrypted_content":"x".repeat(MAX_REPLAY_BYTES + 1)});
    assert!(make_replay(&[huge]).is_err());
    assert!(decode_replay(&json!({"padding":"x".repeat(MAX_REPLAY_BYTES + 1)})).is_err());
    let replay = make_replay(&[
        json!({"type":"reasoning","id":"r","encrypted_content":"e","summary":[{"type":"unknown","text":"discard"},{"type":"summary_text"}]}),
        json!({"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"discard"},{"type":"output_text"}]}),
        json!({"type":"function_call","call_id":"c","name":"n"}),
    ]).unwrap();
    let items = decode_replay(&replay).unwrap();
    assert_eq!(
        items[0]["summary"],
        json!([{"type":"summary_text","text":""}])
    );
    assert_eq!(
        items[1]["content"],
        json!([{"type":"output_text","text":""}])
    );
    assert_eq!(items[2]["arguments"], "{}");
}

#[test]
fn final_calls_validate_identity_and_json_and_default_empty_arguments() {
    for call in [
        json!({"type":"function_call","call_id":"c"}),
        json!({"type":"function_call","name":"n"}),
        json!({"type":"function_call","call_id":"c","name":"n","arguments":"{"}),
    ] {
        assert!(decode_response(json!({"status":"completed","output":[call]})).is_err());
    }
    let response = decode_response(
        json!({"status":"completed","output":[{"type":"function_call","call_id":"c","name":"n"}]}),
    )
    .unwrap();
    assert_eq!(response.message.tool_calls[0].arguments, json!({}));
    let mut assembly = Assembly::default();
    merge_final(
        &json!({"output":output(),"usage":{"input_tokens":4}}),
        &mut assembly,
    )
    .unwrap();
    assert_eq!(assembly.content, "answer");
    assert_eq!(assembly.usage.input_tokens, 4);
    assert_eq!(finish(assembly).unwrap().message.tool_calls.len(), 1);
}
