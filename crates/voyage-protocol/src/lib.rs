use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const API_VERSION: &str = "v1";
pub const PROTOCOL_VERSION: u16 = 1;
pub const PAIRING_PREFIX: &str = "voyage:v1:";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelmDescriptor {
    pub id: Uuid,
    pub name: String,
    pub version: String,
    pub model: String,
    pub capabilities: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredHelm {
    #[serde(flatten)]
    pub descriptor: HelmDescriptor,
    pub status: HelmStatus,
    pub paired_at: DateTime<Utc>,
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
pub struct PairingStartRequest {
    pub helm: HelmDescriptor,
    pub protocol_version: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingStartResponse {
    pub code: String,
    pub worker_token: String,
    pub expires_at: DateTime<Utc>,
    pub protocol_version: u16,
}

impl PairingStartResponse {
    pub fn connection_string(&self) -> String {
        format!("{PAIRING_PREFIX}{}", self.code)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingClaimRequest {
    pub connection_string: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairingStatus {
    pub code: String,
    pub claimed: bool,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub status: HelmStatus,
    pub protocol_version: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRequest {
    pub prompt: String,
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEnvelope {
    pub id: Uuid,
    pub request: TaskRequest,
    pub created_at: DateTime<Utc>,
    pub attempt: u32,
    pub lease_id: Uuid,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub session_id: Uuid,
    pub answer: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: Uuid,
    pub helm_id: Uuid,
    pub request: TaskRequest,
    pub state: TaskState,
    pub result: Option<TaskResult>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub attempt: u32,
    pub max_attempts: u32,
    pub lease_id: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskFailure {
    pub task_id: Uuid,
    pub lease_id: Uuid,
    pub error: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskCompletion {
    pub lease_id: Uuid,
    pub result: TaskResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskCancellation {
    pub task_id: Uuid,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetSummary {
    pub total_helms: usize,
    pub online_helms: usize,
    pub queued_tasks: usize,
    pub running_tasks: usize,
    pub failed_tasks: usize,
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
