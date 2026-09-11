use super::CompatibleAuthentication;
use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    ModelInfo, Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent,
    checked_json, checked_stream_response,
};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: super::api_credential::ApiCredential,
    base_url: String,
    use_max_tokens: bool,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: super::native_http_client(),
            api_key: super::api_credential::ApiCredential::Legacy(api_key),
            use_max_tokens: false,
            base_url: base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .into(),
        }
    }
    pub(super) fn with_account(
        mut self,
        config: &crate::Config,
        redactor: Option<std::sync::Arc<crate::tools::Redactor>>,
    ) -> Self {
        if let Some(binding) = &config.account {
            self.api_key = super::api_credential::ApiCredential::Account {
                binding: binding.clone(),
                authority: config.provider_authority.clone(),
                redactor,
            };
        }
        self
    }

    pub fn with_max_tokens_parameter(mut self, enabled: bool) -> Self {
        self.use_max_tokens = enabled;
        self
    }
    fn body(&self, request: ModelRequest, streaming: bool) -> Result<Value, ProviderError> {
        super::inference::validate_request(&crate::config::ProviderKind::OpenaiChat, &request)?;
        let mut body = request_body(request, streaming)?;
        if self.use_max_tokens
            && let Some(value) = body
                .as_object_mut()
                .unwrap()
                .remove("max_completion_tokens")
        {
            body["max_tokens"] = value;
        }
        super::multimodal::check_body(&body)?;
        Ok(body)
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        super::discovery::models(&self.client, &self.base_url, &self.api_key.resolve()?)
            .await?
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "model-list unavailable; use a manual model ID".into(),
                )
            })
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        super::validate_native_endpoint(&self.base_url)?;
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(self, &crate::config::ProviderKind::OpenaiChat, &request)
            .await?;
        let result: Result<ModelResponse, ProviderError> =
            super::multimodal::guard(images, async {
                let body = self.body(request, false)?;

                let response = super::endpoint_http_client(&self.client, &self.base_url)
                    .post(format!("{}/chat/completions", self.base_url))
                    .apply_key(&self.api_key.resolve()?)
                    .json(&body)
                    .send()
                    .await
                    .map_err(map_transport)?;
                let value = checked_json(response).await?;
                decode_response(value)
            })
            .await;
        result
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        super::validate_native_endpoint(&self.base_url)?;
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(self, &crate::config::ProviderKind::OpenaiChat, &request)
            .await?;
        let result: Result<ProviderStream, ProviderError> =
            super::multimodal::guard(images, async {
                let response = super::endpoint_http_client(&self.client, &self.base_url)
                    .post(format!("{}/chat/completions", self.base_url))
                    .apply_key(&self.api_key.resolve()?)
                    .json(&self.body(request, true)?)
                    .send()
                    .await
                    .map_err(map_transport)?;
                let response = checked_stream_response(response).await?;
                let byte_stream = response.bytes_stream();
                Ok(Box::pin(openai_stream(byte_stream)) as ProviderStream)
            })
            .await;
        result.map(|stream| super::multimodal::guard_stream(images, stream))
    }
}

fn request_body(request: ModelRequest, streaming: bool) -> Result<Value, ProviderError> {
    let messages = encode_messages(&request.messages)?;
    let tools: Vec<Value> = request.tools.iter().map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}})).collect();
    let mut body = json!({"model":request.model,"messages":messages,"stream":streaming});
    if streaming {
        body["stream_options"] = json!({"include_usage":true});
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(value) = request.reasoning_effort {
        body["reasoning_effort"] = json!(value);
    }
    if let Some(value) = request.service_tier {
        body["service_tier"] = json!(value);
    }
    if let Some(value) = request.temperature {
        body["temperature"] = json!(value);
    }
    if let Some(value) = request.max_tokens {
        body["max_completion_tokens"] = json!(value);
    }
    Ok(body)
}

#[derive(Default)]
struct StreamAssembly {
    service_tier: Option<String>,
    content: String,
    calls: Vec<CallAssembly>,
    usage: Usage,
    saw_finish: bool,
    finish_reason: Option<String>,
}
#[derive(Default)]
struct CallAssembly {
    id: String,
    name: String,
    arguments: String,
}

fn openai_stream<S>(
    mut source: S,
) -> impl futures_util::Stream<Item = Result<ProviderStreamEvent, ProviderError>> + Send
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
{
    use futures_util::StreamExt;
    async_stream::try_stream! {
        let mut pending = Vec::new();
        let mut assembly = StreamAssembly::default();
        while let Some(chunk) = source.next().await {
            pending.extend_from_slice(&chunk.map_err(map_transport)?);
            if pending.len() > MAX_SSE_BUFFER_BYTES {
                Err(ProviderError::InvalidResponse("OpenAI stream event exceeded 4 MiB".into()))?;
            }
            while let Some(frame) = take_sse_frame(&mut pending) {
                let data = sse_data(&frame);
                if data.is_empty() {
                    continue;
                } else if data == b"[DONE]" {
                    yield ProviderStreamEvent::Completed(finish_stream(assembly)?);
                    return;
                }
                let value: Value = serde_json::from_slice(data).map_err(|e| ProviderError::InvalidResponse(format!("invalid OpenAI stream event: {e}")))?;
                if let Some(tier) = super::reported_service_tier(value.get("service_tier")) { assembly.service_tier = Some(tier); }
                if let Some(usage) = value.get("usage") {
                    yield ProviderStreamEvent::UsageReported(super::reported_usage(usage, "prompt_tokens", "completion_tokens")?);
                }
                for event in apply_stream_chunk(&value, &mut assembly)? {
                    yield ProviderStreamEvent::Delta(event);
                }
            }
        }
        if assembly.saw_finish {
            yield ProviderStreamEvent::Completed(finish_stream(assembly)?);
            return;
        }
        Err(ProviderError::InvalidResponse("OpenAI stream ended before [DONE]".into()))?;
    }
}

fn apply_stream_chunk(
    value: &Value,
    assembly: &mut StreamAssembly,
) -> Result<Vec<ProviderDelta>, ProviderError> {
    if value.get("error").is_some() {
        return Err(super::rejection::classify(value, None).unwrap_or_else(|| {
            ProviderError::Request("OpenAI stream reported a provider error".into())
        }));
    }
    let mut events = Vec::new();
    if let Some(usage) = value.get("usage") {
        assembly.usage.input_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(assembly.usage.input_tokens);
        assembly.usage.output_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(assembly.usage.output_tokens);
    }
    let Some(delta) = value.pointer("/choices/0/delta") else {
        return Ok(events);
    };
    if value
        .pointer("/choices/0/finish_reason")
        .is_some_and(|value| !value.is_null())
    {
        assembly.saw_finish = true;
        assembly.finish_reason = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
    if let Some(text) = delta.get("content").and_then(Value::as_str) {
        assembly.content.push_str(text);
        events.push(ProviderDelta::Text(text.into()));
    }
    for raw in delta
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let index = raw.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        while assembly.calls.len() <= index {
            assembly.calls.push(CallAssembly::default());
        }
        let call = &mut assembly.calls[index];
        let id = raw.get("id").and_then(Value::as_str).map(str::to_owned);
        if let Some(value) = &id {
            call.id.push_str(value);
        }
        let name = raw
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(value) = &name {
            call.name.push_str(value);
        }
        let arguments = raw
            .pointer("/function/arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        call.arguments.push_str(&arguments);
        events.push(ProviderDelta::ToolCall {
            index,
            id,
            name,
            arguments,
        });
    }
    Ok(events)
}
fn finish_stream(assembly: StreamAssembly) -> Result<ModelResponse, ProviderError> {
    validate_finish_reason(assembly.finish_reason.as_deref())?;
    let calls = assembly
        .calls
        .into_iter()
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
        service_tier: assembly.service_tier,
        message: Message {
            coordination: None,
            tool_outcome: None,
            tool_output: None,
            parts: Vec::new(),
            image_data: Default::default(),
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
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
const MAX_SSE_BUFFER_BYTES: usize = 4 * 1024 * 1024;
pub(crate) fn take_sse_frame(pending: &mut Vec<u8>) -> Option<Vec<u8>> {
    let (at, width) = pending
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|p| (p, 2))
        .or_else(|| {
            pending
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .map(|p| (p, 4))
        })?;
    let frame = pending[..at].to_vec();
    pending.drain(..at + width);
    Some(frame)
}
pub(crate) fn sse_data(frame: &[u8]) -> &[u8] {
    frame
        .split(|b| *b == b'\n')
        .find_map(|line| {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            line.strip_prefix(b"data: ")
                .or_else(|| line.strip_prefix(b"data:"))
        })
        .unwrap_or_default()
}

fn map_transport(error: reqwest::Error) -> ProviderError {
    super::map_transport(error)
}

/// Compatible templates commonly accept one initial system block. Preserve the
/// exact ordered text at the same authority level without changing canonical
/// messages. Stop at any other role or unusual tool metadata; never promote or
/// silently discard a malformed/late message to make a template accept it.
fn encode_messages(messages: &[Message]) -> Result<Vec<Value>, ProviderError> {
    let leading = messages
        .iter()
        .take_while(|message| {
            message.role == Role::System
                && message.parts.is_empty()
                && message.image_data.is_empty()
                && message.tool_call_id.is_none()
                && message.tool_calls.is_empty()
        })
        .count();
    if leading < 2 {
        return messages.iter().map(encode_message).collect();
    }
    let content = messages[..leading]
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    std::iter::once(Ok(json!({"role": "system", "content": content})))
        .chain(messages[leading..].iter().map(encode_message))
        .collect()
}

fn encode_message(message: &Message) -> Result<Value, ProviderError> {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let content = super::multimodal::content(message, super::multimodal::Wire::Chat)?
        .unwrap_or_else(|| json!(message.content));
    let mut value = json!({ "role": role, "content": content });
    if let Some(id) = &message.tool_call_id {
        value["tool_call_id"] = json!(id);
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] = json!(message.tool_calls.iter().map(|c| json!({
            "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments.to_string() }
        })).collect::<Vec<_>>());
    }
    Ok(value)
}

fn decode_response(value: Value) -> Result<ModelResponse, ProviderError> {
    validate_finish_reason(
        value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str),
    )?;
    let raw = value
        .pointer("/choices/0/message")
        .ok_or_else(|| ProviderError::InvalidResponse("missing choices[0].message".into()))?;
    let content = raw
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let tool_calls = raw
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|call| {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let args = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments = serde_json::from_str(args)
                .map_err(|e| ProviderError::InvalidResponse(format!("bad tool arguments: {e}")))?;
            Ok(ToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect::<Result<Vec<_>, ProviderError>>()?;
    let usage = Usage {
        input_tokens: value
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: value
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    Ok(ModelResponse {
        service_tier: super::reported_service_tier(value.get("service_tier")),
        message: Message {
            coordination: None,
            tool_outcome: None,
            tool_output: None,
            parts: Vec::new(),
            image_data: Default::default(),
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role: Role::Assistant,
            content,
            tool_call_id: None,
            tool_calls,
            tool_success: None,
            provider_state: None,
            steering: None,
        },
        usage,
    })
}

fn validate_finish_reason(reason: Option<&str>) -> Result<(), ProviderError> {
    match reason {
        Some("stop" | "tool_calls") => Ok(()),
        Some("length" | "content_filter") => Err(ProviderError::Incomplete),
        _ => Err(ProviderError::InvalidResponse(
            "missing or unsupported OpenAI finish reason".into(),
        )),
    }
}

#[cfg(test)]
mod context_rejection_tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn sse_body_transport_failure_is_distinct_from_clean_eof() {
        let response = super::super::failure_tests::http_response(200, "0", "data: {", 100).await;
        let stream = openai_stream(response.bytes_stream());
        futures_util::pin_mut!(stream);
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.category(), "transport");
        assert_eq!(error.http_status(), None);
        assert!(!error.is_retryable());
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn truncated_sse_is_not_context_rejection_or_retryable() {
        let source = futures_util::stream::iter(vec![Ok(bytes::Bytes::from_static(b"data: {"))]);
        let stream = openai_stream(source);
        futures_util::pin_mut!(stream);
        let error = stream.next().await.unwrap().unwrap_err();
        assert_eq!(error.category(), "invalid_response");
        assert_eq!(error.http_status(), None);
        assert!(!error.is_context_length());
        assert!(!error.is_retryable());
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn chat_sse_classifies_explicit_context_and_account_rejections() {
        for (code, context) in [
            ("context_length_exceeded", true),
            ("insufficient_quota", false),
        ] {
            let frame = format!(
                "data: {}\n\n",
                json!({"error":{"code":code,"message":"PRIVATE"}})
            );
            let source = futures_util::stream::iter(vec![Ok(bytes::Bytes::from(frame))]);
            let stream = openai_stream(source);
            futures_util::pin_mut!(stream);
            let error = stream.next().await.unwrap().unwrap_err();
            if context {
                assert!(matches!(error, ProviderError::ContextLength));
            } else {
                assert!(matches!(error, ProviderError::UsageLimit));
            }
            assert!(!error.is_retryable());
            assert!(!error.to_string().contains("PRIVATE"));
            assert!(stream.next().await.is_none());
        }
    }
}
