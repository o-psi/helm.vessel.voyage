use super::*;

#[test]
fn repeated_calls_deduplicate_by_semantic_arguments_but_conflicts_fail() {
    let first = json!({"type":"function_call","call_id":"c","name":"n","arguments":"{\"x\":1}"});
    let mut same = first.clone();
    same["arguments"] = json!("{ \"x\": 1 }");
    let message = json!({"type":"message","role":"assistant","content":[]});
    assert_eq!(
        unique_replay_calls(vec![first.clone(), message.clone(), same]).unwrap(),
        vec![first.clone(), message]
    );
    for (key, value) in [
        ("name", json!("other")),
        ("arguments", json!("{\"x\":2}")),
        ("arguments", json!("{")),
    ] {
        let mut conflict = first.clone();
        conflict[key] = value;
        assert!(unique_replay_calls(vec![first.clone(), conflict]).is_err());
    }
}

#[test]
fn empty_replay_recovers_neutral_text_and_calls_before_tool_results() {
    let mut message = Message::new(Role::Assistant, "answer");
    message.tool_calls.push(ToolCall {
        id: "call".into(),
        name: "lookup".into(),
        arguments: json!({"q":1}),
    });
    message.provider_state = Some(make_replay(&[]).unwrap());
    let encoded = encode_message(&message).unwrap();
    assert_eq!(encoded.len(), 2);
    assert_eq!(encoded[0]["type"], "message");
    assert_eq!(encoded[0]["content"][0]["text"], "answer");
    assert_eq!(encoded[1]["call_id"], "call");
    let result = encode_message(&Message::tool("call", "result")).unwrap();
    assert_eq!(
        result,
        vec![json!({"type":"function_call_output","call_id":"call","output":"result"})]
    );
    let mut unmatched = Message::new(Role::Tool, "orphan");
    assert!(encode_message(&unmatched).unwrap().is_empty());
    unmatched.role = Role::System;
    assert_eq!(encode_message(&unmatched).unwrap()[0]["role"], "system");
}
