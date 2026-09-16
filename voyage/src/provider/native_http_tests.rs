//! Native dispatch against scripted numeric-loopback HTTP, never external services.
use super::*;
use crate::model::{Message, ModelRequest, Role};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) async fn server(
    replies: Vec<(u16, String)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n =
                    tokio::time::timeout(std::time::Duration::from_secs(10), socket.read(&mut buf))
                        .await
                        .unwrap()
                        .unwrap();
                assert!(n > 0, "request ended before body");
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&bytes[..end]);
                    let length = head
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            let response = format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\nRetry-After: 2\r\nx-request-id: fixture-request\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    (url, task)
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "fixture-model".into(),
        messages: vec![
            Message::new(Role::System, "instructions"),
            Message::new(Role::User, "hello"),
        ],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: Some(32),
    }
}
fn provider(kind: usize, url: String) -> Box<dyn Provider> {
    match kind {
        0 => Box::new(
            openai::OpenAiProvider::new("synthetic-key".into(), Some(url))
                .with_max_tokens_parameter(true),
        ),
        1 => Box::new(anthropic::AnthropicProvider::new(
            "synthetic-key".into(),
            Some(url),
        )),
        _ => Box::new(openai_responses::OpenAiResponsesProvider::new(
            "synthetic-key".into(),
            Some(url),
        )),
    }
}
fn response(kind: usize) -> Value {
    match kind {
        0 => {
            json!({"choices":[{"finish_reason":"stop","message":{"content":"answer"}}],"usage":{"prompt_tokens":9,"completion_tokens":2}})
        }
        1 => {
            json!({"content":[{"type":"text","text":"answer"}],"stop_reason":"end_turn","usage":{"input_tokens":9,"output_tokens":2}})
        }
        _ => {
            json!({"id":"resp_fixture","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"answer"}]}],"usage":{"input_tokens":9,"output_tokens":2}})
        }
    }
}
fn sse(kind: usize) -> String {
    let events = match kind {
        0 => vec![
            json!({"choices":[{"delta":{"content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9,"completion_tokens":2}}),
        ],
        1 => vec![
            json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"answer"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
            json!({"type":"message_stop"}),
        ],
        _ => vec![
            json!({"type":"response.output_text.delta","delta":"answer"}),
            json!({"type":"response.completed","response":response(2)}),
        ],
    };
    let mut text = events
        .iter()
        .map(|e| format!("data: {e}\n\n"))
        .collect::<String>();
    if kind == 0 {
        text.push_str("data: [DONE]\n\n");
    }
    text
}
#[tokio::test]
async fn native_complete_stream_and_models_use_real_dispatch() {
    for kind in 0..3 {
        let (url, task) = server(vec![
            (200, response(kind).to_string()),
            (200, sse(kind)),
            (
                200,
                json!({"data":[{"id":"z"},{"id":"a"},{"id":"a"}],"has_more":false}).to_string(),
            ),
        ])
        .await;
        let p = provider(kind, format!("{url}/v1/"));
        let answer = p.complete(request()).await.unwrap();
        assert_eq!(answer.message.content, "answer");
        assert_eq!(answer.usage.input_tokens, 9);
        let events = p.stream(request()).await.unwrap().collect::<Vec<_>>().await;
        let completed = events
            .into_iter()
            .map(Result::unwrap)
            .find_map(|e| match e {
                ProviderStreamEvent::Completed(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(completed.message.content, "answer");
        assert_eq!(completed.usage.output_tokens, 2);
        let models = p.models().await.unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "z"]
        );
        let requests = task.await.unwrap();
        let endpoint = ["chat/completions", "messages", "responses"][kind];
        assert!(requests[0].starts_with(&format!("POST /v1/{endpoint} ")));
        for (index, raw) in requests[..2].iter().enumerate() {
            let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["model"], "fixture-model");
            assert_eq!(
                body.get("stream").and_then(Value::as_bool).unwrap_or(false),
                index == 1
            );
            assert!(raw.to_ascii_lowercase().contains(if kind == 1 {
                "x-api-key: synthetic-key"
            } else {
                "authorization: bearer synthetic-key"
            }));
        }
        assert!(requests[2].starts_with("GET /v1/models"));
    }
}
#[tokio::test]
async fn native_http_failures_and_invalid_success_bodies() {
    for kind in 0..3 {
        for (status, body) in [
            (401, "denied"),
            (403, "denied"),
            (429, "busy"),
            (408, "wait"),
            (409, "conflict"),
            (503, "down"),
            (400, "bad"),
            (302, "redirect"),
            (200, "not json"),
        ] {
            let (url, task) = server(vec![(status, body.into()), (status, body.into())]).await;
            let p = provider(kind, url);
            assert!(
                p.complete(request()).await.is_err(),
                "kind {kind}, status {status}"
            );
            match p.stream(request()).await {
                Err(_) => assert_ne!(status, 200),
                Ok(stream) => assert!(stream.collect::<Vec<_>>().await.iter().any(Result::is_err)),
            }
            assert_eq!(task.await.unwrap().len(), 2);
        }
    }
}
#[tokio::test]
async fn native_model_catalog_rejects_unavailable_and_malformed_lists() {
    for kind in 0..3 {
        for (status, body) in [
            (404, json!({})),
            (200, json!({})),
            (200, json!({"data":[{}]})),
            (200, json!({"data":[{"id":"synthetic-key"}]})),
            (200, json!({"data":[{"id":"bad\nmodel"}]})),
            (200, json!({"data":"wrong"})),
        ] {
            let (url, task) = server(vec![(status, body.to_string())]).await;
            assert!(provider(kind, url).models().await.is_err());
            task.await.unwrap();
        }
    }
}
#[tokio::test]
async fn anthropic_metadata_capacity_is_cached_and_pagination_is_followed() {
    let reply = response(1).to_string();
    let (url, task) = server(vec![
        (200, json!({"max_tokens":4096}).to_string()),
        (200, reply.clone()),
        (200, reply),
        (
            200,
            json!({"data":[{"id":"z","display_name":"Zed"}],"has_more":true,"last_id":"z"})
                .to_string(),
        ),
        (
            200,
            json!({"data":[{"id":"a"}],"has_more":false}).to_string(),
        ),
    ])
    .await;
    let p = provider(1, url);
    let mut req = request();
    req.max_tokens = None;
    p.complete(req.clone()).await.unwrap();
    p.complete(req).await.unwrap();
    assert_eq!(p.models().await.unwrap().len(), 2);
    let requests = task.await.unwrap();
    assert!(requests[0].starts_with("GET /models/fixture-model "));
    assert!(requests[4].contains("after_id=z"));
    for raw in &requests[1..3] {
        let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["max_tokens"], 4096);
    }
}
#[tokio::test]
async fn anthropic_capacity_and_pagination_errors_are_bounded() {
    for metadata in [
        json!({}),
        json!({"max_tokens":0}),
        json!({"max_tokens":-1}),
        json!({"max_tokens":4294967296_u64}),
    ] {
        let (url, task) = server(vec![(200, metadata.to_string())]).await;
        let mut req = request();
        req.max_tokens = None;
        assert!(provider(1, url).complete(req).await.is_err());
        task.await.unwrap();
    }
    for page in [
        json!({"data":[],"has_more":"yes"}),
        json!({"data":[],"has_more":true}),
        json!({"data":[],"has_more":true,"last_id":""}),
    ] {
        let (url, task) = server(vec![(200, page.to_string())]).await;
        assert!(provider(1, url).models().await.is_err());
        task.await.unwrap();
    }
    let page = json!({"data":[],"has_more":true,"last_id":"again"}).to_string();
    let (url, task) = server(vec![(200, page.clone()), (200, page)]).await;
    assert!(provider(1, url).models().await.is_err());
    task.await.unwrap();
}
