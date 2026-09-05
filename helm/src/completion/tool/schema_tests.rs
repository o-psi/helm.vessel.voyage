use super::*;
use crate::{
    model::{Message, ModelRequest},
    provider::{
        AnthropicProvider, OpenAiProvider, OpenAiResponsesProvider, Provider, ProviderStreamEvent,
    },
    todo::TodoScope,
};
use futures_util::StreamExt;

const ID: &str = "00112233-4455-4677-8899-aabbccddeeff";

fn definition(root: &std::path::Path) -> ToolDefinition {
    CompletionTool::new(
        Arc::new(TodoStore::new(
            root.join("todos.json"),
            TodoScope::workspace(root.to_owned()),
        )),
        AgentTreeStore::new(root.join("agents.json")),
    )
    .definition()
}

fn validator(schema: &Value) -> jsonschema::Validator {
    jsonschema::draft202012::meta::validate(schema).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .should_validate_formats(true)
        .build(schema)
        .unwrap()
}

// Expected results are independent of the advertised schema. This compares
// canonical JSON UUID strings/integer values, not every lenient serde spelling.
fn corpus() -> Vec<(Value, bool)> {
    let mut cases = vec![(json!({"action":"snapshot"}), true)];
    for kind in ["todo", "agent"] {
        cases.push((json!({"action":"read","kind":kind,"id":ID}), true));
        cases.push((
            json!({"action":"adopt","kind":kind,"id":ID,"revision":0}),
            true,
        ));
        for disposition in [
            "completed_with_evidence",
            "cancelled_with_reason",
            "blocked_with_impact",
            "deferred_with_impact",
            "incorporated",
            "failure_with_impact",
            "not_needed_with_reason",
        ] {
            cases.push((json!({"action":"account","kind":kind,"id":ID,"revision":1,"fingerprint":"snapshot-fingerprint","disposition":disposition,"reason":"Verified evidence 雪\nwith a concrete impact"}),true));
        }
    }
    cases.push((
        json!({"action":"adopt","kind":"todo","id":ID,"revision":u64::MAX}),
        true,
    ));
    // Empty review strings are syntactically valid. Runtime semantics still
    // reject inadequate evidence/reasons/fingerprints; schemas cannot grant them.
    cases.push((json!({"action":"account","kind":"todo","id":ID,"revision":0,"fingerprint":"","disposition":"completed_with_evidence","reason":""}),true));
    let valid = cases.clone();
    for (value, _) in valid {
        for key in value.as_object().unwrap().keys() {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(key);
            cases.push((missing, false));
            let mut null = value.clone();
            null[key] = Value::Null;
            cases.push((null, false));
        }
        for (key, replacement) in [
            ("kind", json!("todo")),
            ("id", json!(ID)),
            ("revision", json!(1)),
            ("fingerprint", json!("fingerprint")),
            ("disposition", json!("deferred_with_impact")),
            ("reason", json!("reason")),
            ("unexpected", json!(true)),
        ] {
            if value.get(key).is_none() {
                let mut extra = value.clone();
                extra[key] = replacement;
                cases.push((extra, false));
            }
        }
    }
    cases.extend([
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":(u64::MAX as f64)*2.0}),false),
        (json!({"action":"read","kind":"todo","reason":"","revision":1}),false),
        (json!({"action":"read","id":ID}),false),
        (json!({"action":"read","kind":"unknown","id":ID}),false),
        (json!({"action":"read","kind":"todo","id":"not-a-uuid"}),false),
        (json!({"action":"read","kind":"todo","id":false}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":-1}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":1.5}),false),
        (json!({"action":"adopt","kind":"todo","id":ID,"revision":"1"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":1,"disposition":"incorporated","reason":"why"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":"f","disposition":"done","reason":"why"}),false),
        (json!({"action":"account","kind":"todo","id":ID,"revision":1,"fingerprint":"f","disposition":"incorporated","reason":[]}),false),
        (json!({"action":"unknown"}),false),(json!({}),false),(json!([]),false),(json!(null),false),
    ]);
    cases
}

#[test]
fn advertised_completion_schema_matches_strict_action_corpus() {
    let root = tempfile::tempdir().unwrap();
    let schema = definition(root.path()).input_schema;
    let validator = validator(&schema);
    for (args, expected) in corpus() {
        assert_eq!(
            serde_json::from_value::<Args>(args.clone()).is_ok(),
            expected,
            "decoder: {args}"
        );
        assert_eq!(validator.is_valid(&args), expected, "schema: {args}");
    }
}

#[test]
fn completion_action_examples_are_valid_and_describe_fresh_review() {
    let root = tempfile::tempdir().unwrap();
    let definition = definition(root.path());
    let schema = &definition.input_schema;
    let validator = validator(schema);
    let examples = schema["examples"]
        .as_array()
        .expect("valid per-action examples");
    assert_eq!(examples.len(), 4);
    for example in examples {
        assert!(validator.is_valid(example), "{example}");
        assert!(serde_json::from_value::<Args>(example.clone()).is_ok());
    }
    assert!(definition.description.contains("revision and fingerprint"));
    assert!(
        definition
            .description
            .contains("read accepts only action, kind and id")
    );
    for (_, property) in schema["properties"].as_object().unwrap() {
        assert!(
            property["description"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty())
        );
    }
}

#[tokio::test]
async fn native_http_providers_preserve_completion_action_contract() {
    use axum::{
        Json, Router,
        extract::{OriginalUri, State},
        routing::post,
    };
    let root = tempfile::tempdir().unwrap();
    let tool = definition(root.path());
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
            assert_eq!(schema["oneOf"].as_array().unwrap().len(), 4);
            if index == 1 {
                assert_eq!(raw["strict"], false);
            } else if index == 0 {
                assert!(raw["function"].get("strict").is_none());
            } else {
                assert!(raw.get("strict").is_none());
            }
            let validator = validator(schema);
            for (args, expected) in corpus() {
                assert_eq!(
                    validator.is_valid(&args),
                    expected,
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
