use async_trait::async_trait;
use serde_json::{Value, json};

use super::{
    Provider, ProviderDelta, ProviderError, ProviderStream, ProviderStreamEvent, checked_json,
    checked_stream_response,
};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into())
                .trim_end_matches('/')
                .into(),
        }
    }
}

#[async_trait]
impl Provider for OpenAiProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let body = request_body(request, false);

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(map_transport)?;
        let value = checked_json(response).await?;
        decode_response(value)
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&request_body(request, true))
            .send()
            .await
            .map_err(map_transport)?;
        let response = checked_stream_response(response).await?;
        let byte_stream = response.bytes_stream();
        Ok(Box::pin(openai_stream(byte_stream)))
    }
}

fn request_body(request: ModelRequest, streaming: bool) -> Value {
    let messages: Vec<Value> = request.messages.iter().map(encode_message).collect();
    let tools: Vec<Value> = request.tools.iter().map(|t| json!({"type":"function","function":{"name":t.name,"description":t.description,"parameters":t.input_schema}})).collect();
    let mut body = json!({"model":request.model,"messages":messages,"stream":streaming});
    if streaming {
        body["stream_options"] = json!({"include_usage":true});
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(value) = request.temperature {
        body["temperature"] = json!(value);
    }
    if let Some(value) = request.max_tokens {
        body["max_completion_tokens"] = json!(value);
    }
    body
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
            while let Some(frame) = take_sse_frame(&mut pending) {
                let data = sse_data(&frame);
                if data.is_empty() {
                    continue;
                } else if data == b"[DONE]" {
                    yield ProviderStreamEvent::Completed(finish_stream(assembly)?);
                    return;
                }
                let value: Value = serde_json::from_slice(data).map_err(|e| ProviderError::InvalidResponse(format!("invalid OpenAI stream event: {e}")))?;
                for event in apply_stream_chunk(&value, &mut assembly)? {
                    yield ProviderStreamEvent::Delta(event);
                }
            }
        }
        Err(ProviderError::InvalidResponse("OpenAI stream ended before [DONE]".into()))?;
    }
}

fn apply_stream_chunk(
    value: &Value,
    assembly: &mut StreamAssembly,
) -> Result<Vec<ProviderDelta>, ProviderError> {
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
    let calls = assembly
        .calls
        .into_iter()
        .map(|c| {
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
        },
        usage: assembly.usage,
    })
}
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
    if error.is_timeout() {
        ProviderError::Timeout(error.to_string())
    } else if error.is_connect() {
        ProviderError::Unavailable(error.to_string())
    } else {
        ProviderError::Request(error.to_string())
    }
}

fn encode_message(message: &Message) -> Value {
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut value = json!({ "role": role, "content": message.content });
    if let Some(id) = &message.tool_call_id {
        value["tool_call_id"] = json!(id);
    }
    if !message.tool_calls.is_empty() {
        value["tool_calls"] = json!(message.tool_calls.iter().map(|c| json!({
            "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments.to_string() }
        })).collect::<Vec<_>>());
    }
    value
}

fn decode_response(value: Value) -> Result<ModelResponse, ProviderError> {
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
        message: Message {
            role: Role::Assistant,
            content,
            tool_call_id: None,
            tool_calls,
        },
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[test]
    fn decodes_tool_call() {
        let response = decode_response(json!({
            "choices": [{"message": {"content": null, "tool_calls": [{"id":"x", "function":{"name":"read_file", "arguments":"{\"path\":\"a\"}"}}]}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 3}
        })).unwrap();
        assert_eq!(response.message.tool_calls[0].name, "read_file");
        assert_eq!(response.usage.output_tokens, 3);
    }

    #[tokio::test]
    async fn decodes_fragmented_native_stream() {
        let fixture = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a\\\"}\"}}]}}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n"
        ).as_bytes();
        let chunks = fixture
            .chunks(17)
            .map(|c| Ok::<_, reqwest::Error>(bytes::Bytes::copy_from_slice(c)))
            .collect::<Vec<_>>();
        let events = openai_stream(futures_util::stream::iter(chunks))
            .collect::<Vec<_>>()
            .await;
        let completed = events
            .into_iter()
            .find_map(|e| match e.unwrap() {
                ProviderStreamEvent::Completed(r) => Some(r),
                _ => None,
            })
            .unwrap();
        assert_eq!(completed.message.content, "hello");
        assert_eq!(
            completed.message.tool_calls[0].arguments,
            json!({"path":"a"})
        );
        assert_eq!(completed.usage.output_tokens, 3);
    }
}
