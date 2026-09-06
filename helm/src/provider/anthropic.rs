use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    ModelInfo, Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent,
    checked_json, checked_stream_response, normalize_models,
};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};

pub struct AnthropicProvider {
    pub(super) client: reqwest::Client,
    api_key: String,
    base_url: String,
    output_capacities: tokio::sync::Mutex<std::collections::BTreeMap<String, u32>>,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: super::native_http_client(),
            api_key,
            output_capacities: Default::default(),
            base_url: base_url
                .unwrap_or_else(|| "https://api.anthropic.com/v1".into())
                .trim_end_matches('/')
                .into(),
        }
    }

    // Anthropic requires max_tokens. Resolve its supported maximum rather than
    // imposing a harness default: https://platform.claude.com/docs/en/api/http/models/retrieve
    async fn output_tokens(&self, request: &ModelRequest) -> Result<u32, ProviderError> {
        if let Some(value) = request.max_tokens.filter(|value| *value > 0) {
            return Ok(value);
        }
        let mut capacities = self.output_capacities.lock().await;
        if let Some(value) = capacities.get(&request.model) {
            return Ok(*value);
        }
        let mut url = reqwest::Url::parse(&format!("{}/models/", self.base_url))
            .map_err(|_| ProviderError::Request("invalid Anthropic model metadata URL".into()))?;
        url.path_segments_mut()
            .map_err(|_| ProviderError::Request("invalid Anthropic model metadata URL".into()))?
            .pop_if_empty()
            .push(&request.model);
        let response = self
            .client
            .get(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .send()
            .await
            .map_err(super::catalog::transport)?;
        if matches!(response.status().as_u16(), 404 | 405 | 501) {
            return Err(missing_output_capacity());
        }
        let mut remaining = super::catalog::MAX_BYTES;
        let metadata = super::catalog::json(response, &mut remaining).await?;
        let maximum = metadata
            .get("max_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(missing_output_capacity)?;
        capacities.insert(request.model.clone(), maximum);
        Ok(maximum)
    }
}

fn missing_output_capacity() -> ProviderError {
    ProviderError::Request("Anthropic requires max_tokens, but this endpoint did not provide a positive model output capacity; configure an explicit max_tokens supported by the endpoint".into())
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = Vec::new();
        let mut after_id: Option<String> = None;
        let mut remaining = super::catalog::MAX_BYTES;
        let mut cursors = std::collections::BTreeSet::new();
        loop {
            let mut request = self
                .client
                .get(format!("{}/models", self.base_url))
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .query(&[("limit", "1000")]);
            if let Some(cursor) = &after_id {
                request = request.query(&[("after_id", cursor)]);
            }
            let value = super::catalog::json(
                request.send().await.map_err(super::catalog::transport)?,
                &mut remaining,
            )
            .await?;
            let data = value.get("data").and_then(Value::as_array).ok_or_else(|| {
                ProviderError::InvalidResponse("Anthropic models response omitted data".into())
            })?;
            if models.len().saturating_add(data.len()) > super::catalog::MAX_MODELS {
                return Err(ProviderError::InvalidResponse(
                    "model list exceeds 1024 entries".into(),
                ));
            }
            for item in data {
                let id = item.get("id").and_then(Value::as_str).ok_or_else(|| {
                    ProviderError::InvalidResponse("Anthropic model omitted id".into())
                })?;
                let mut model = ModelInfo::minimal(id);
                model.display_name =
                    super::catalog::optional_text(item, "display_name", id)?.to_owned();
                super::validate_model(&model, &[&self.api_key])?;
                models.push(model);
            }
            match value.get("has_more") {
                None | Some(Value::Bool(false)) => break,
                Some(Value::Bool(true)) => {}
                _ => {
                    return Err(ProviderError::InvalidResponse(
                        "invalid model pagination flag".into(),
                    ));
                }
            }
            let cursor = value
                .get("last_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("model pagination omitted cursor".into())
                })?;
            super::catalog::validate_text(cursor, 512, true, &[&self.api_key])?;
            if !cursors.insert(cursor.to_owned()) || cursors.len() >= super::catalog::MAX_PAGES {
                return Err(ProviderError::InvalidResponse(
                    "model pagination repeated or exceeded 16 pages".into(),
                ));
            }
            after_id = Some(cursor.to_owned());
        }
        super::validate_models(&models, &[&self.api_key])?;
        normalize_models(&mut models);
        Ok(models)
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let system = request
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages = encode_messages(&request.messages);
        let tools: Vec<Value> = request.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.input_schema })).collect();
        let mut body = json!({ "model": request.model, "max_tokens": self.output_tokens(&request).await?, "system": system, "messages": messages });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(value) = request.temperature {
            body["temperature"] = json!(value);
        }
        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(map_transport)?;
        decode_response(checked_json(response).await?)
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let system = request
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let messages = encode_messages(&request.messages);
        let tools: Vec<Value> = request.tools.iter().map(|t| json!({"name":t.name,"description":t.description,"input_schema":t.input_schema})).collect();
        let mut body = json!({"model":request.model,"max_tokens":self.output_tokens(&request).await?,"system":system,"messages":messages,"stream":true});
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(value) = request.temperature {
            body["temperature"] = json!(value);
        }
        let response = self
            .client
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(map_transport)?;
        Ok(Box::pin(anthropic_stream(
            checked_stream_response(response).await?.bytes_stream(),
        )))
    }
}

#[derive(Default)]
struct StreamAssembly {
    content: String,
    calls: Vec<CallAssembly>,
    usage: Usage,
}
#[derive(Default)]
struct CallAssembly {
    id: String,
    name: String,
    arguments: String,
}

fn anthropic_stream<S>(
    mut source: S,
) -> impl futures_util::Stream<Item = Result<ProviderStreamEvent, ProviderError>> + Send
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    use futures_util::StreamExt;
    async_stream::try_stream! {
        let mut pending=Vec::new();
        let mut assembly=StreamAssembly::default();
        while let Some(chunk)=source.next().await {
            pending.extend_from_slice(&chunk.map_err(map_transport)?);
            if pending.len() > 4 * 1024 * 1024 {
                Err(ProviderError::InvalidResponse("Anthropic stream event exceeded 4 MiB".into()))?;
            }
            while let Some(frame)=super::openai::take_sse_frame(&mut pending) {
                let data=super::openai::sse_data(&frame);
                if data.is_empty(){continue;}
                let value:Value=serde_json::from_slice(data).map_err(|e|ProviderError::InvalidResponse(format!("invalid Anthropic stream event: {e}")))?;
                if let Some(usage) = value.pointer("/message/usage").or_else(|| value.get("usage")) {
                    yield ProviderStreamEvent::UsageReported(super::reported_usage(usage, "input_tokens", "output_tokens")?);
                }
                if value.get("type").and_then(Value::as_str)==Some("message_stop") {
                    yield ProviderStreamEvent::Completed(finish_stream(assembly)?);
                    return;
                }
                for event in apply_stream_event(&value,&mut assembly){yield ProviderStreamEvent::Delta(event);}
            }
        }
        Err(ProviderError::InvalidResponse("Anthropic stream ended before message_stop".into()))?;
    }
}

fn apply_stream_event(value: &Value, assembly: &mut StreamAssembly) -> Vec<ProviderDelta> {
    let mut events = Vec::new();
    match value.get("type").and_then(Value::as_str) {
        Some("message_start") => {
            assembly.usage.input_tokens = value
                .pointer("/message/usage/input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0)
        }
        Some("message_delta") => {
            assembly.usage.output_tokens = value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(assembly.usage.output_tokens)
        }
        Some("content_block_start")
            if value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use") =>
        {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            while assembly.calls.len() <= index {
                assembly.calls.push(CallAssembly::default());
            }
            let call = &mut assembly.calls[index];
            call.id = value
                .pointer("/content_block/id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into();
            call.name = value
                .pointer("/content_block/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into();
            events.push(ProviderDelta::ToolCall {
                index,
                id: Some(call.id.clone()),
                name: Some(call.name.clone()),
                arguments: String::new(),
            });
        }
        Some("content_block_delta") => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            match value.pointer("/delta/type").and_then(Value::as_str) {
                Some("text_delta") => {
                    if let Some(text) = value.pointer("/delta/text").and_then(Value::as_str) {
                        assembly.content.push_str(text);
                        events.push(ProviderDelta::Text(text.into()));
                    }
                }
                Some("input_json_delta") => {
                    while assembly.calls.len() <= index {
                        assembly.calls.push(CallAssembly::default());
                    }
                    let partial = value
                        .pointer("/delta/partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    assembly.calls[index].arguments.push_str(partial);
                    events.push(ProviderDelta::ToolCall {
                        index,
                        id: None,
                        name: None,
                        arguments: partial.into(),
                    });
                }
                _ => {}
            }
        }
        _ => {}
    }
    events
}
fn finish_stream(assembly: StreamAssembly) -> Result<ModelResponse, ProviderError> {
    let calls = assembly
        .calls
        .into_iter()
        .filter(|call| !call.name.is_empty())
        .map(|c| {
            if c.id.is_empty() || c.name.is_empty() {
                return Err(ProviderError::InvalidResponse(
                    "streamed tool call omitted id or name".into(),
                ));
            }
            Ok(ToolCall {
                id: c.id,
                name: c.name,
                arguments: serde_json::from_str(if c.arguments.is_empty() {
                    "{}"
                } else {
                    &c.arguments
                })
                .map_err(|e| {
                    ProviderError::InvalidResponse(format!("bad streamed tool arguments: {e}"))
                })?,
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    Ok(ModelResponse {
        message: Message {
            role: Role::Assistant,
            content: assembly.content,
            tool_call_id: None,
            tool_calls: calls,
            tool_success: None,
            provider_state: None,
            steering: None,
        },
        usage: assembly.usage,
    })
}

fn map_transport(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout(error.to_string())
    } else if error.is_connect() {
        ProviderError::Unavailable(error.to_string())
    } else {
        ProviderError::Request(error.to_string())
    }
}

fn encode_messages(messages: &[Message]) -> Vec<Value> {
    let mut result = Vec::new();
    for message in messages.iter().filter(|m| m.role != Role::System) {
        match message.role {
            Role::User => result.push(json!({"role":"user", "content": message.content})),
            Role::Assistant => {
                let mut blocks = Vec::new();
                if !message.content.is_empty() { blocks.push(json!({"type":"text", "text":message.content})); }
                blocks.extend(message.tool_calls.iter().map(|c| json!({"type":"tool_use", "id":c.id, "name":c.name, "input":c.arguments})));
                result.push(json!({"role":"assistant", "content":blocks}));
            }
            Role::Tool => result.push(json!({"role":"user", "content":[{"type":"tool_result", "tool_use_id":message.tool_call_id, "content":message.content}]})),
            Role::System => {}
        }
    }
    result
}

fn decode_response(value: Value) -> Result<ModelResponse, ProviderError> {
    let blocks = value
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::InvalidResponse("missing content".into()))?;
    let content = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let tool_calls = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|b| ToolCall {
            id: b
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            name: b
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            arguments: b.get("input").cloned().unwrap_or_else(|| json!({})),
        })
        .collect();
    Ok(ModelResponse {
        message: Message {
            role: Role::Assistant,
            content,
            tool_call_id: None,
            tool_calls,
            tool_success: None,
            provider_state: None,
            steering: None,
        },
        usage: Usage {
            input_tokens: value
                .pointer("/usage/input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output_tokens: value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
    })
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use futures_util::StreamExt;
    #[tokio::test]
    async fn decodes_anthropic_text_and_tool_stream() {
        let fixture=concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":4}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"shell\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\\\"pwd\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n").as_bytes();
        let chunks = fixture
            .chunks(23)
            .map(|c| Ok::<_, reqwest::Error>(bytes::Bytes::copy_from_slice(c)))
            .collect::<Vec<_>>();
        let events = anthropic_stream(futures_util::stream::iter(chunks))
            .collect::<Vec<_>>()
            .await;
        let completed = events
            .into_iter()
            .find_map(|e| match e.unwrap() {
                ProviderStreamEvent::Completed(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(completed.message.content, "hi");
        assert_eq!(completed.message.tool_calls[0].name, "shell");
        assert_eq!(completed.usage.input_tokens, 4);
        assert_eq!(completed.usage.output_tokens, 7);
    }
}

#[cfg(test)]
mod output_limit_tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{OriginalUri, State},
        routing::{get, post},
    };
    use futures_util::StreamExt;

    fn request(model: &str, max_tokens: Option<u32>) -> ModelRequest {
        ModelRequest {
            model: model.into(),
            messages: vec![Message::new(Role::User, "hello")],
            tools: vec![],
            max_tokens,
            temperature: None,
        }
    }

    #[tokio::test]
    async fn discovers_model_capacity_once_for_complete_and_stream_and_preserves_explicit_limits() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        async fn metadata(
            State(tx): State<tokio::sync::mpsc::UnboundedSender<Value>>,
            OriginalUri(uri): OriginalUri,
        ) -> Json<Value> {
            tx.send(json!({"metadata":uri.path()})).unwrap();
            Json(json!({"id":"fixture", "max_tokens":131072}))
        }
        async fn message(
            State(tx): State<tokio::sync::mpsc::UnboundedSender<Value>>,
            Json(body): Json<Value>,
        ) -> String {
            let streaming = body["stream"] == true;
            tx.send(body).unwrap();
            if streaming {
                "data: {\"type\":\"message_stop\"}\n\n".into()
            } else {
                json!({"content":[]}).to_string()
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/v1/models/{model}", get(metadata))
            .route("/v1/messages", post(message))
            .with_state(tx);
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let provider = AnthropicProvider::new("fixture-secret".into(), Some(base));
        provider
            .complete(request("model/with?delimiters", None))
            .await
            .unwrap();
        assert_eq!(
            rx.recv().await.unwrap()["metadata"],
            "/v1/models/model%2Fwith%3Fdelimiters"
        );
        assert_eq!(rx.recv().await.unwrap()["max_tokens"], 131072);
        let events = provider
            .stream(request("model/with?delimiters", None))
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(matches!(
            events.last().unwrap(),
            Ok(ProviderStreamEvent::Completed(_))
        ));
        assert_eq!(rx.recv().await.unwrap()["max_tokens"], 131072);
        provider
            .complete(request("custom-without-metadata", Some(123)))
            .await
            .unwrap();
        assert_eq!(rx.recv().await.unwrap()["max_tokens"], 123);
        assert!(rx.try_recv().is_err());
        server.abort();
    }

    #[tokio::test]
    async fn metadata_failure_is_sanitized_and_explicit_limits_work_without_discovery() {
        for status in [401, 404, 405, 500, 501] {
            let router = Router::new().route(
                "/v1/models/fixture",
                get(move || async move {
                    (
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        "fixture-secret private diagnostics",
                    )
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let provider = AnthropicProvider::new(
                "fixture-secret".into(),
                Some(format!("http://{}/v1", listener.local_addr().unwrap())),
            );
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let error = provider
                .output_tokens(&request("fixture", None))
                .await
                .unwrap_err();
            if status == 401 {
                assert!(matches!(error, ProviderError::Authentication(_)));
            }
            assert!(!error.to_string().contains("fixture-secret"));
            assert!(!error.to_string().contains("private diagnostics"));
            assert!(provider.output_capacities.lock().await.is_empty());
            assert_eq!(
                provider
                    .output_tokens(&request("fixture", Some(77)))
                    .await
                    .unwrap(),
                77
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn invalid_or_missing_capacity_is_not_cached_or_replaced_with_a_guessed_limit() {
        for metadata in [
            json!({}),
            json!({"max_tokens":null}),
            json!({"max_tokens":0}),
            json!({"max_tokens":-1}),
            json!({"max_tokens":"8192"}),
            json!({"max_tokens":4294967296u64}),
        ] {
            let router = Router::new().route(
                "/v1/models/fixture",
                get(move || {
                    let metadata = metadata.clone();
                    async move { Json(metadata) }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let provider = AnthropicProvider::new(
                "fixture-secret".into(),
                Some(format!("http://{}/v1", listener.local_addr().unwrap())),
            );
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let error = provider
                .output_tokens(&request("fixture", None))
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("configure an explicit max_tokens"),
                "{error}"
            );
            assert!(!error.contains("fixture-secret"));
            assert!(provider.output_capacities.lock().await.is_empty());
            assert_eq!(
                provider
                    .output_tokens(&request("fixture", Some(32)))
                    .await
                    .unwrap(),
                32
            );
            server.abort();
        }
    }
}
