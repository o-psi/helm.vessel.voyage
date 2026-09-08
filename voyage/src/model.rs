use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SteeringStatus {
    Queued,
    Applied,
    NotApplied,
    UnknownAfterRestart,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SteeringReceipt {
    pub id: uuid::Uuid,
    pub status: SteeringStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    /// Trusted local operator attribution; never inferred from authored text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_name: Option<String>,
    /// Durable local message time; absent for legacy history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_success: Option<bool>,
    /// Provider-owned replay metadata persisted with sessions for native continuation.
    /// Providers must use a typed, versioned, bounded envelope; model switches clear it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state: Option<Value>,
    /// Local delivery receipt; provider adapters never send this metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steering: Option<SteeringReceipt>,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Vec::new(),
            tool_success: None,
            provider_state: None,
            steering: None,
        }
    }

    pub fn steering(content: impl Into<String>) -> Self {
        let mut message = Self::new(Role::User, content);
        message.steering = Some(SteeringReceipt {
            id: uuid::Uuid::new_v4(),
            status: SteeringStatus::Queued,
        });
        message
    }

    pub fn tool(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::tool_result(call_id, content, true)
    }
    pub fn tool_result(
        call_id: impl Into<String>,
        content: impl Into<String>,
        success: bool,
    ) -> Self {
        Self {
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(call_id.into()),
            tool_calls: Vec::new(),
            tool_success: Some(success),
            provider_state: None,
            steering: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct ModelResponse {
    pub service_tier: Option<String>,
    pub message: Message,
    pub usage: Usage,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
