//! Attributed subordinate execution. Child identities are distinct from the canonical parent.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisclosedMessage {
    pub role: String,
    pub content: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantBinding {
    pub binding_id: Uuid,
    pub revision: u64,
    pub parent_vessel_id: Uuid,
    pub parent_session_id: Uuid,
    pub principal_id: Uuid,
    pub workspace: PathBuf,
    pub config_path: Option<PathBuf>,
    pub max_context_bytes: u32,
    pub max_assignments: u16,
    pub expires_at_ms: u64,
    pub revoked: bool,
    pub cancel_existing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentRequest {
    pub assignment_id: Uuid,
    pub binding_id: Uuid,
    pub binding_revision: u64,
    pub parent_vessel_id: Uuid,
    pub parent_session_id: Uuid,
    pub parent_run_id: Uuid,
    pub expires_at_ms: u64,
    pub task: String,
    pub context: Vec<DisclosedMessage>,
    pub policy: ParticipantPolicy,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssignmentObservation {
    pub assignment_id: Uuid,
    pub participant_vessel_id: Uuid,
    pub parent_session_id: Uuid,
    pub parent_run_id: Uuid,
    pub child_session_id: Uuid,
    pub child_incarnation: Option<Uuid>,
    pub run_id: Option<Uuid>,
    pub state: String,
    pub cleanup_observed: bool,
    pub result: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantGrantBinding {
    pub binding_id: Uuid,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantEndpoint {
    pub name: String,
    pub credential_file: PathBuf,
    pub participant_vessel_id: Uuid,
    pub binding_id: Uuid,
    pub binding_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantPolicy {
    pub access: String,
    /// Retired command deny-list. Retained only for saved-document compatibility; never enforced.
    #[doc(hidden)]
    #[serde(default, rename = "deny_commands")]
    pub legacy_deny_commands: Vec<String>,
    pub inherit_env: Vec<String>,
    pub github_enabled: bool,
    pub timeout_secs: u64,
    pub max_output_bytes: usize,
    pub max_subagents: usize,
}
