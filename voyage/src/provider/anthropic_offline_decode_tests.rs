use super::*;

#[test]
fn decode_encode_tool_transcript_and_error_results() {
    let response = decode_response(json!({"stop_reason":"tool_use","content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"first"},{"type":"text","text":" second"},{"type":"tool_use","id":"c","name":"lookup","input":{"x":1}}],"usage":{"input_tokens":10,"output_tokens":3}})).unwrap();
    assert_eq!(response.message.content, "first\n second");
    assert_eq!(response.message.tool_calls[0].arguments, json!({"x":1}));
    assert_eq!(response.usage.input_tokens, 10);
    assert_eq!(response.usage.output_tokens, 3);
    let mut system = Message::new(Role::User, "system");
    system.role = Role::System;
    let mut tool = Message::new(Role::User, "failed");
    tool.role = Role::Tool;
    tool.tool_call_id = Some("c".into());
    tool.tool_success = Some(false);
    let messages = encode_messages(&[
        system,
        Message::new(Role::User, "question"),
        response.message,
        tool,
    ])
    .unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0], json!({"role":"user","content":"question"}));
    assert_eq!(messages[1]["content"][0]["text"], "first\n second");
    assert_eq!(messages[1]["content"][1]["input"], json!({"x":1}));
    assert_eq!(messages[2]["content"][0]["is_error"], true);
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "c");
}

#[test]
fn decoding_rejects_partial_or_unrecognized_stops() {
    for reason in [
        json!(null),
        json!("unknown"),
        json!("max_tokens"),
        json!("pause_turn"),
        json!("refusal"),
    ] {
        assert!(decode_response(json!({"stop_reason":reason,"content":[]})).is_err());
    }
    assert!(decode_response(json!({"stop_reason":"end_turn"})).is_err());
    let response = decode_response(
        json!({"stop_reason":"end_turn","content":[{"type":"text"},{"type":"tool_use"}]}),
    )
    .unwrap();
    assert!(response.message.content.is_empty());
    assert_eq!(response.message.tool_calls[0].arguments, json!({}));
    assert_eq!(response.usage.input_tokens, 0);
}

#[test]
fn streaming_ignores_sparse_empty_slots_but_not_partial_calls() {
    for call in [
        CallAssembly {
            started: true,
            id: "id".into(),
            ..Default::default()
        },
        CallAssembly {
            started: true,
            name: "name".into(),
            ..Default::default()
        },
        CallAssembly {
            started: true,
            id: "id".into(),
            name: "name".into(),
            arguments: "{".into(),
        },
    ] {
        assert!(
            finish_stream(StreamAssembly {
                calls: vec![call],
                stop_reason: Some("tool_use".into()),
                ..Default::default()
            })
            .is_err()
        );
    }
    let response = finish_stream(StreamAssembly {
        calls: vec![
            CallAssembly::default(),
            CallAssembly {
                started: true,
                id: "id".into(),
                name: "name".into(),
                ..Default::default()
            },
        ],
        stop_reason: Some("tool_use".into()),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].arguments, json!({}));
}

#[test]
fn counting_and_dispatch_share_instruction_schema_and_tool_encoding() {
    let mut assistant = Message::new(Role::Assistant, "inspect");
    assistant.tool_calls.push(ToolCall {
        id: "exact".into(),
        name: "read_file".into(),
        arguments: json!({"path":"é"}),
    });
    let request = ModelRequest {
        model: "fixture".into(),
        messages: vec![
            Message::new(Role::System, "rules"),
            Message::new(Role::User, "Unicode 🧭"),
            assistant,
            Message::tool("exact", "evidence"),
        ],
        tools: vec![crate::model::ToolDefinition {
            name: "read_file".into(),
            description: "read".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
            output_schema: None,
            annotations: None,
        }],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: Some(100),
    };
    let body = input_body(&request).unwrap();
    assert_eq!(body["system"], "rules");
    assert_eq!(
        body["tools"][0]["input_schema"],
        request.tools[0].input_schema
    );
    assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "exact");
    assert!(body.get("max_tokens").is_none());
    assert!(body.get("stream").is_none());
}

#[tokio::test]
async fn offline_count_endpoint_preserves_tokens_and_exact_model_capacity() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut counted = None;
        for body in [
            json!({"input_tokens":1234}),
            json!({"id":"fixture","max_tokens":100}),
            json!({"id":"fixture","context_window":5000}),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(socket.read_u8().await.unwrap());
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap();
            let length = header
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|n| n.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let mut input = vec![0; length];
            socket.read_exact(&mut input).await.unwrap();
            if header.starts_with("POST /messages/count_tokens ") {
                counted = Some(serde_json::from_slice::<Value>(&input).unwrap());
            }
            let text = body.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",text.len(),text).as_bytes()).await.unwrap();
        }
        counted.unwrap()
    });
    let provider = AnthropicProvider::new("offline-fixture-only".into(), Some(url));
    let request = ModelRequest {
        model: "fixture".into(),
        messages: vec![
            Message::new(Role::System, "rules"),
            Message::new(Role::User, "🧭"),
        ],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: None,
    };
    let pressure = provider.request_pressure(&request).await.unwrap();
    assert_eq!(pressure.input_tokens, Some(1234));
    assert_eq!(pressure.reserve_tokens, Some(100));
    assert_eq!(pressure.enabled_capacity, Some(5000));
    assert!(!pressure.should_prepare());
    assert_eq!(server.await.unwrap(), input_body(&request).unwrap());
}
