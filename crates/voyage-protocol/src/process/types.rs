//! Private process transport. Filesystem/peer authentication is required separately.
use super::{EnrollmentIdentity, GrantBinding, ProcessRight};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub const PROCESS_PROTOCOL: u32 = 1;
pub const MAX_PROCESS_FRAME: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeCommand {
    Clear {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        confirm_session_id: Uuid,
    },
    Compact {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        retain: u32,
    },
    AssignmentObserve {
        run_id: Uuid,
        assignment_id: Uuid,
        participant: String,
        cancel: bool,
    },
    OperatorTool {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        name: String,
        arguments: Value,
    },
    Github {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        words: Vec<String>,
    },
    SetAccess {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        access: String,
    },
    Configure {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        config_path: PathBuf,
    },
    Relinquish {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        transfer_id: Uuid,
        destination_vessel_id: Uuid,
        prepare_digest: String,
    },
    WorkflowInputs {
        input_id: Uuid,
        values: Vec<(String, String)>,
    },
    WorkflowPreview {
        id: String,
        scope: Option<String>,
        user_directory: Option<PathBuf>,
        inputs: Vec<(String, String)>,
        trust_digest: Option<String>,
    },
    WorkflowSubmit {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        id: String,
        scope: Option<String>,
        user_directory: Option<PathBuf>,
        inputs: Vec<(String, String)>,
        trust_digest: Option<String>,
        private_inputs_id: Option<Uuid>,
    },
    Controls {
        run_id: Option<Uuid>,
        section: String,
    },
    ExecuteTool {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        run_id: Uuid,
        name: String,
        arguments: Value,
    },
    Terminal {
        run_id: Uuid,
        terminal_id: Uuid,
        operation: TerminalOperation,
    },
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
    Archive {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        archived: bool,
    },
    Delete {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        confirm_session_id: Uuid,
    },
    Branch {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        branch_id: Uuid,
        name: Option<String>,
    },
    Events {
        after: u64,
        limit: u32,
        wait_ms: u32,
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

impl RuntimeCommand {
    /// Pure session observations served without waking a cleanly suspended runtime.
    pub fn observes_suspended(&self) -> bool {
        matches!(
            self,
            Self::Health
                | Self::Stop
                | Self::Snapshot
                | Self::History { .. }
                | Self::MessageChunk { .. }
                | Self::RunOutput { .. }
                | Self::Receipt { .. }
                | Self::Events { .. }
                | Self::Decisions
                | Self::Controls { .. }
        )
    }
}

/// Private human terminal traffic is never journalled or replayed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalOperation {
    Attach,
    Snapshot,
    Write { bytes: Vec<u8> },
    Resize { columns: u16, rows: u16 },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRequest {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub token: String,
    #[serde(default)]
    pub authorization: Option<GrantBinding>,
    pub command: RuntimeCommand,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeResponse {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    /// An automatic clean resume may replace the requested runtime incarnation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<Uuid>,
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
    Suspended,
    Unavailable,
    Stopped,
    CleanupUnconfirmed,
    Relinquished,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessRegistration {
    #[serde(default)]
    pub executable: Option<PathBuf>,
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub command_id: Uuid,
    #[serde(default)]
    pub restart_from: Option<Uuid>,
    #[serde(default)]
    pub initialize: Option<super::RuntimeInitialization>,
    #[serde(default)]
    pub config_path: Option<PathBuf>,
    pub token: String,
    pub workspace: PathBuf,
    pub state: ProcessState,
}

/// Public summary emitted only alongside positively observed runtime cleanup.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivedVoyage {
    pub name: Option<String>,
    pub revision: u64,
    pub receipt: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchivedVoyage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletion: Option<Value>,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub workspace: PathBuf,
    pub state: ProcessState,
}
impl From<&ProcessRegistration> for ProcessInfo {
    fn from(value: &ProcessRegistration) -> Self {
        Self {
            archive: None,
            deletion: None,
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
    /// Local transport relays may wake only a positively suspended owner.
    Wake {
        session_id: Uuid,
    },
    Recover {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
        acknowledge_cleanup: Option<Uuid>,
        reconcile_tools: Option<Uuid>,
        expected_revision: Option<u64>,
        #[serde(default)]
        acknowledge_resources: Vec<Uuid>,
    },
    AcceptParticipant {
        command_id: Uuid,
        binding: super::ParticipantBinding,
    },
    RemoveParticipant {
        command_id: Uuid,
        binding_id: Uuid,
        expected_revision: u64,
        cancel: bool,
    },
    Assign {
        request: super::AssignmentRequest,
    },
    FenceAssignment {
        request: super::AssignmentRequest,
    },
    ObserveAssignment {
        assignment_id: Uuid,
    },
    CancelAssignment {
        assignment_id: Uuid,
    },
    Identity,
    TrustVessel {
        identity: super::VesselIdentity,
    },
    PrepareTransfer {
        command_id: Uuid,
        transfer_id: Uuid,
        source_vessel_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
        expires_at_ms: u64,
    },
    ExportTransfer {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        preparation: super::SignedArtifact<super::TransferPreparation>,
    },
    AcceptTransfer {
        manifest: super::SignedArtifact<super::TransferManifest>,
    },
    TransferChunk {
        transfer_id: Uuid,
        offset: u64,
        limit: u32,
    },
    UploadTransferChunk {
        transfer_id: Uuid,
        offset: u64,
        data: String,
    },
    ActivateTransfer {
        command_id: Uuid,
        transfer_id: Uuid,
    },
    Grant {
        command_id: Uuid,
        grant_id: Uuid,
        principal_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        rights: Vec<ProcessRight>,
        expires_at_ms: u64,
        enrollment: Option<EnrollmentIdentity>,
        endpoint: String,
    },
    RevokeGrant {
        command_id: Uuid,
        grant_id: Uuid,
        expected_revision: u64,
    },
    Granted {
        grant_id: Uuid,
        token: String,
        command: Box<VesselCommand>,
    },
    Capabilities,
    Catalogue,
    Start {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
    },
    StartConfigured {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
    },
    StartOutbound {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
        enrollment_directory: PathBuf,
        origin: String,
        allow_insecure_loopback: bool,
        source_directory: Option<PathBuf>,
        expected_revision: Option<u64>,
    },
    ManagedImport {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        source_directory: PathBuf,
        expected_revision: u64,
        config_path: Option<PathBuf>,
    },
    Import {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        source_directory: PathBuf,
        expected_revision: u64,
        source_sha256: String,
        config_path: Option<PathBuf>,
    },
    Branch {
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        branch_id: Uuid,
        name: Option<String>,
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
