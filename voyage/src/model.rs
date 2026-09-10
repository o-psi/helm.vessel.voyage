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

#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination: Option<voyage_protocol::coordination::CoordinationSource>,
    /// Trusted local operator attribution; never inferred from authored text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_name: Option<String>,
    /// Durable local message time; absent for legacy history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<Box<voyage_protocol::tool_result::ToolOutput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_outcome: Option<voyage_protocol::tool_result::ToolOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<voyage_protocol::content::ContentPart>,
    #[serde(skip)]
    pub image_data: std::collections::BTreeMap<uuid::Uuid, Vec<u8>>,
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
            coordination: None,
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role,
            content: content.into(),
            tool_output: None,
            tool_outcome: None,
            parts: Vec::new(),
            image_data: Default::default(),
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
            coordination: None,
            operator_name: None,
            created_at: Some(chrono::Utc::now()),
            role: Role::Tool,
            content: content.into(),
            tool_output: None,
            tool_outcome: None,
            parts: Vec::new(),
            image_data: Default::default(),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// Untrusted descriptive hints, never execution-policy grants.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world_hint: Option<bool>,
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

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Message")
            .field("role", &self.role)
            .field("content", &self.content)
            .field("parts", &self.parts)
            .finish_non_exhaustive()
    }
}
