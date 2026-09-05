use super::*;
use crate::{
    model::ToolDefinition,
    session::{Session, SessionStore},
};
use futures_util::StreamExt;

fn request(messages: Vec<Message>) -> ModelRequest {
    ModelRequest {
        model: "compatible-fixture".into(),
        messages,
        tools: vec![ToolDefinition {
            name: "read_file".into(),
            description: "Read local evidence".into(),
            input_schema: json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        }],
        temperature: Some(0.25),
        max_tokens: Some(384),
    }
}

#[test]
fn leading_system_blocks_keep_exact_order_whitespace_and_unicode() {
    for parts in [
        vec!["base", "reconciliation"],
        vec!["", "", ""],
        vec![" base\n", "雪\\quoted\"", "\nlast "],
    ] {
        for streaming in [false, true] {
            let mut messages: Vec<_> = parts
                .iter()
                .map(|s| Message::new(Role::System, *s))
                .collect();
            messages.push(Message::new(Role::User, "human"));
            let original = serde_json::to_value(&messages).unwrap();
            let req = request(messages);
            let body = request_body(req.clone(), streaming);
            assert_eq!(
                body["messages"],
                json!([{"role":"system","content":parts.join("\n\n")},{"role":"user","content":"human"}])
            );
            assert_eq!(serde_json::to_value(&req.messages).unwrap(), original);
            assert_eq!(body["max_completion_tokens"], 384);
            assert_eq!(body["temperature"], 0.25);
            assert_eq!(body["tools"][0]["function"]["name"], "read_file");
            assert_eq!(body.get("stream_options").is_some(), streaming);
        }
    }
}

#[test]
fn nonleading_and_malformed_system_boundaries_are_not_promoted_or_discarded() {
    let mut malformed = Message::new(Role::System, "unusual metadata");
    malformed.tool_call_id = Some("preserve-invalid-id".into());
    let mut called = Message::new(Role::System, "unusual calls");
    called.tool_calls.push(ToolCall {
        id: "call".into(),
        name: "read_file".into(),
        arguments: json!({"path":"a"}),
    });
    for messages in [
        vec![],
        vec![Message::new(Role::System, "single")],
        vec![
            Message::new(Role::User, "user"),
            Message::new(Role::System, "late"),
            Message::new(Role::System, "still late"),
        ],
        vec![
            Message::new(Role::System, "base"),
            Message::new(Role::Assistant, "assistant"),
            Message::new(Role::System, "late"),
        ],
        vec![
            Message::new(Role::System, "base"),
            Message::tool("call", "untrusted"),
            Message::new(Role::System, "late"),
        ],
        vec![malformed, Message::new(Role::System, "next")],
        vec![
            Message::new(Role::System, "base"),
            called,
            Message::new(Role::System, "next"),
        ],
    ] {
        let expected: Vec<_> = messages.iter().map(encode_message).collect();
        assert_eq!(
            request_body(request(messages), false)["messages"],
            json!(expected)
        );
    }
}

#[tokio::test]
async fn native_http_accepts_coalesced_reconciliation_preserving_canonical_history() {
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::post};
    use std::sync::{Arc, Mutex};
    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = observed.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |Json(body): Json<Value>| {
        let captured = captured.clone();
        async move {
            captured.lock().unwrap().push(body.clone());
            let messages = body["messages"].as_array().unwrap();
            // Reproduce the real template: a second system role is rejected,
            // even when it is consecutive leading trusted runtime guidance.
            if messages.iter().skip(1).any(|m| m["role"] == "system") {
                return (StatusCode::BAD_REQUEST, "Only the initial system message is supported").into_response();
            }
            if body["stream"] == true {
                ([ ("content-type", "text/event-stream") ], concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"reviewed\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":13,\"completion_tokens\":2}}\n\n",
                    "data: [DONE]\n\n"
                )).into_response()
            } else {
                Json(json!({"choices":[{"message":{"content":"reviewed"}}],"usage":{"prompt_tokens":13,"completion_tokens":2}})).into_response()
            }
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let (shutdown, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path().join("sessions"));
    let mut session = Session::new(temp.path().to_owned(), "compatible-fixture".into());
    let mut call = Message::new(Role::Assistant, "");
    call.tool_calls.push(ToolCall {
        id: "read-evidence".into(),
        name: "read_file".into(),
        arguments: json!({"path":"actual.txt"}),
    });
    session.messages = vec![
        Message::new(Role::User, "verify 99 against 42"),
        call,
        Message::tool("read-evidence", "Measured records: 42"),
        Message::new(Role::Assistant, "provisional proposal"),
    ];
    let canonical = serde_json::to_value(&session.messages).unwrap();
    store.save(&mut session).await.unwrap();
    let provider = OpenAiProvider::new("synthetic-fixture".into(), Some(endpoint));
    let mut messages = vec![
        Message::new(Role::System, "Base guidance: preserve authority.\n"),
        Message::new(
            Role::System,
            "Reconciliation: account for unresolved todo; do not invent success. 雪",
        ),
    ];
    messages.extend(session.messages.clone());
    let mut req = request(messages);
    let original_request = serde_json::to_value(&req).unwrap();
    let report = crate::context::preflight(&mut req, 32768).unwrap();
    assert_eq!(report.omitted_messages, 0);
    let complete = provider.complete(req.clone()).await;
    let stream = provider.stream(req.clone()).await;
    let mut streamed = None;
    if let Ok(mut stream) = stream {
        while let Some(event) = stream.next().await {
            if let ProviderStreamEvent::Completed(response) = event.unwrap() {
                streamed = Some(response);
            }
        }
    }
    let _ = shutdown.send(());
    server.await.unwrap();
    assert_eq!(complete.unwrap().message.content, "reviewed");
    let streamed = streamed.expect("stream accepted by actual native HTTP template");
    assert_eq!(streamed.message.content, "reviewed");
    assert_eq!(streamed.usage.output_tokens, 2);
    assert_eq!(serde_json::to_value(&req).unwrap(), original_request);
    assert_eq!(
        serde_json::to_value(store.load(session.id).await.unwrap().messages).unwrap(),
        canonical
    );
    for body in observed.lock().unwrap().iter() {
        assert_eq!(
            body["messages"][0]["content"],
            "Base guidance: preserve authority.\n\n\nReconciliation: account for unresolved todo; do not invent success. 雪"
        );
        let tail: Vec<_> = session.messages.iter().map(encode_message).collect();
        assert_eq!(&body["messages"].as_array().unwrap()[1..], tail.as_slice());
        assert_eq!(body["max_completion_tokens"], 384);
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
    }
    assert_eq!(observed.lock().unwrap().len(), 2);
}
