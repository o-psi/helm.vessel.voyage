//! Shared test-only native transport fixture; no runtime schema validation hook.
use super::{
    AnthropicProvider, OpenAiProvider, OpenAiResponsesProvider, Provider, ProviderStreamEvent,
};
use crate::model::{Message, ModelRequest, ToolDefinition};
use futures_util::StreamExt;
use serde_json::{Value, json};

pub(crate) fn validator(schema: &Value) -> jsonschema::Validator {
    jsonschema::draft202012::meta::validate(schema).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(schema)
        .unwrap()
}

pub(crate) async fn assert_native_schema(
    tool: ToolDefinition,
    corpus: Vec<(Value, bool)>,
    branches: usize,
) {
    use axum::{
        Json, Router,
        extract::{OriginalUri, State},
        routing::post,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    async fn endpoint(
        State(tx): State<tokio::sync::mpsc::UnboundedSender<Value>>,
        OriginalUri(uri): OriginalUri,
        Json(body): Json<Value>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        let stream = body["stream"] == true;
        let path = uri.path();
        tx.send(body).unwrap();
        let message = json!({"role":"assistant","content":"schema-ok"});
        let response = if path.ends_with("/chat/completions") {
            if stream {
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"delta":{"content":"schema-ok"}}]})
                )
            } else {
                json!({"choices":[{"message":message}]}).to_string()
            }
        } else if path.ends_with("/responses") {
            let value = json!({"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"schema-ok"}]}]});
            if stream {
                format!(
                    "data: {}\n\n",
                    json!({"type":"response.completed","response":value})
                )
            } else {
                value.to_string()
            }
        } else {
            assert!(path.ends_with("/messages"));
            if stream {
                format!(
                    "data: {}\n\ndata: {}\n\n",
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"schema-ok"}}),
                    json!({"type":"message_stop"})
                )
            } else {
                json!({"content":[{"type":"text","text":"schema-ok"}]}).to_string()
            }
        };
        (
            [(
                "content-type",
                if stream {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )],
            response,
        )
            .into_response()
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let router = Router::new()
        .route("/v1/chat/completions", post(endpoint))
        .route("/v1/responses", post(endpoint))
        .route("/v1/messages", post(endpoint))
        .with_state(tx);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    struct Stop(tokio::task::JoinHandle<()>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let server = Stop(task);
    let providers: Vec<Box<dyn Provider>> = vec![
        Box::new(OpenAiProvider::new("fixture".into(), Some(base.clone()))),
        Box::new(OpenAiResponsesProvider::new(
            "fixture".into(),
            Some(base.clone()),
        )),
        Box::new(AnthropicProvider::new("fixture".into(), Some(base))),
    ];
    for (index, provider) in providers.into_iter().enumerate() {
        for stream in [false, true] {
            let request = ModelRequest {
                model: "fixture".into(),
                messages: vec![Message::new(crate::model::Role::User, "schema fixture")],
                tools: vec![tool.clone()],
                temperature: None,
                max_tokens: Some(128),
            };
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                if stream {
                    let mut events = provider.stream(request).await.unwrap();
                    let mut completed = false;
                    while let Some(event) = events.next().await {
                        if let ProviderStreamEvent::Completed(response) = event.unwrap() {
                            assert_eq!(response.message.content, "schema-ok");
                            completed = true;
                        }
                    }
                    assert!(completed);
                } else {
                    assert_eq!(
                        provider.complete(request).await.unwrap().message.content,
                        "schema-ok"
                    );
                }
            })
            .await
            .unwrap();
            let body = rx.recv().await.unwrap();
            let raw = &body["tools"][0];
            let schema = match index {
                0 => &raw["function"]["parameters"],
                1 => &raw["parameters"],
                _ => &raw["input_schema"],
            };
            assert_eq!(schema, &tool.input_schema);
            assert_eq!(schema["oneOf"].as_array().unwrap().len(), branches);
            if index == 1 {
                assert_eq!(raw["strict"], false);
            } else if index == 0 {
                assert!(raw["function"].get("strict").is_none());
            } else {
                assert!(raw.get("strict").is_none());
            }
            let validator = validator(schema);
            for (args, expected) in &corpus {
                assert_eq!(
                    validator.is_valid(args),
                    *expected,
                    "provider {index}: {args}"
                );
            }
            assert_eq!(
                body["max_output_tokens"]
                    .as_u64()
                    .or_else(|| body["max_completion_tokens"].as_u64())
                    .or_else(|| body["max_tokens"].as_u64()),
                Some(128)
            );
        }
    }
    server.0.abort();
}
