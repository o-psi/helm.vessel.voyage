use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const API_VERSION: &str = "v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelmDescriptor {
    pub id: Uuid,
    pub name: String,
    pub endpoint: String,
    pub version: String,
    pub model: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredHelm {
    #[serde(flatten)]
    pub descriptor: HelmDescriptor,
    pub status: HelmStatus,
    pub registered_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HelmStatus {
    Online,
    Stale,
    Offline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistrationRequest {
    pub helm: HelmDescriptor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRequest {
    pub prompt: String,
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResponse {
    pub session_id: Uuid,
    pub answer: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
