//! Private process transport. Filesystem/peer authentication is required separately.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub const PROCESS_PROTOCOL: u32 = 1;
pub const MAX_PROCESS_FRAME: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeCommand {
    Health,
    Snapshot,
    History {
        offset: u64,
        limit: u32,
        expected_revision: Option<u64>,
    },
    MessageChunk {
        index: u64,
        offset: u64,
        limit: u32,
        expected_revision: u64,
    },
    RunOutput {
        run_id: Uuid,
        offset: u64,
        limit: u32,
    },
    Submit {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        prompt: String,
    },
    Receipt {
        command_id: Uuid,
    },
    Cancel {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        run_id: Uuid,
    },
    Steer {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        run_id: Uuid,
        prompt: String,
    },
    Rename {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        name: String,
    },
    SetModel {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        model: String,
    },
    Decisions,
    Respond {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        run_id: Uuid,
        decision_id: Uuid,
        response: Value,
    },
    Stop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequest {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub token: String,
    pub command: RuntimeCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeResponse {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub result: Value,
    pub error: Option<String>,
    /// True means a mutation may have been admitted; retrieve its durable receipt.
    #[serde(default = "unknown_outcome")]
    pub outcome_unknown: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Starting,
    Live,
    Unavailable,
    Stopped,
    CleanupUnconfirmed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessRegistration {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub command_id: Uuid,
    #[serde(default)]
    pub restart_from: Option<Uuid>,
    pub token: String,
    pub workspace: PathBuf,
    pub state: ProcessState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub workspace: PathBuf,
    pub state: ProcessState,
}
impl From<&ProcessRegistration> for ProcessInfo {
    fn from(value: &ProcessRegistration) -> Self {
        Self {
            session_id: value.session_id,
            incarnation: value.incarnation,
            workspace: value.workspace.clone(),
            state: value.state.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum VesselCommand {
    Capabilities,
    Catalogue,
    Start {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
    },
    Restart {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    },
    Inspect {
        session_id: Uuid,
    },
    Forward {
        session_id: Uuid,
        incarnation: Uuid,
        command: RuntimeCommand,
    },
    Stop {
        session_id: Uuid,
        incarnation: Uuid,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselRequest {
    pub protocol: u32,
    pub command: VesselCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VesselResponse {
    pub protocol: u32,
    pub result: Value,
    pub error: Option<String>,
    /// True means a mutation may have been admitted; retrieve its durable receipt.
    #[serde(default = "unknown_outcome")]
    pub outcome_unknown: bool,
}

fn unknown_outcome() -> bool {
    true
}
