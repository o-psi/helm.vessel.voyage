use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Provider, ProviderError, checked_json};
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
        let messages: Vec<Value> = request.messages.iter().map(encode_message).collect();
        let tools: Vec<Value> = request.tools.iter().map(|t| json!({
            "type": "function",
            "function": { "name": t.name, "description": t.description, "parameters": t.input_schema }
        })).collect();
        let mut body = json!({ "model": request.model, "messages": messages });
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(value) = request.temperature {
            body["temperature"] = json!(value);
        }
        if let Some(value) = request.max_tokens {
            body["max_completion_tokens"] = json!(value);
        }

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let value = checked_json(response).await?;
        decode_response(value)
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

    #[test]
    fn decodes_tool_call() {
        let response = decode_response(json!({
            "choices": [{"message": {"content": null, "tool_calls": [{"id":"x", "function":{"name":"read_file", "arguments":"{\"path\":\"a\"}"}}]}}],
            "usage": {"prompt_tokens": 2, "completion_tokens": 3}
        })).unwrap();
        assert_eq!(response.message.tool_calls[0].name, "read_file");
        assert_eq!(response.usage.output_tokens, 3);
    }
}
