use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Provider, ProviderError, checked_json};
use crate::model::{Message, ModelRequest, ModelResponse, Role, ToolCall, Usage};

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: base_url
                .unwrap_or_else(|| "https://api.anthropic.com/v1".into())
                .trim_end_matches('/')
                .into(),
        }
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
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
        let mut body = json!({ "model": request.model, "max_tokens": request.max_tokens.unwrap_or(8192), "system": system, "messages": messages });
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
