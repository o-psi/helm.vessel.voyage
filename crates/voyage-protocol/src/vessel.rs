//! Public Vessel service API. Versioning is independent of private runtime IPC.
//! No runtime command, runtime token or runtime response is part of this contract.
pub use crate::process::{
    AccessCredential, ApprovedWorkspace, ConnectionGrant, LocalAccessCredential, ProcessInfo,
    ProcessRight, ProcessState, WorkspaceCredential,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub const VESSEL_API_VERSION: u32 = 1;
pub const MAX_VESSEL_BODY: usize = 4 * 1024 * 1024;
pub const COMMAND_PATH: &str = "/v1/vessel/command";
pub const PAIR_PATH: &str = "/v1/vessel/pair";
pub const PAIR_CAPABILITIES_PATH: &str = "/v1/vessel/pair/capabilities";
pub const EVENTS_PATH: &str = "/v1/vessel/events";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselEventSubscription {
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub after: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselEventRequest {
    pub protocol: u32,
    pub subscriptions: Vec<VesselEventSubscription>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselEvent {
    pub protocol: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub result: Value,
    pub error: Option<String>,
    pub outcome_unknown: bool,
}

/// A session operation. Vessel locates its current owner. Only operations on live
/// resources require the exact observed incarnation; session reads/turns do not.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "WireVoyageRequest")]
pub struct VoyageRequest {
    pub session_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incarnation: Option<Uuid>,
    #[serde(flatten)]
    pub command: VoyageCommand,
}

// Serde flatten + deny_unknown_fields rejects valid operations, while a
// flattened unit variant silently consumes unknown fields. Parse the operation
// independently and explicitly reject keys outside its serialized field set.
#[derive(Deserialize)]
struct WireVoyageRequest {
    session_id: Uuid,
    #[serde(default)]
    incarnation: Option<Uuid>,
    #[serde(flatten)]
    fields: serde_json::Map<String, Value>,
}

impl TryFrom<WireVoyageRequest> for VoyageRequest {
    type Error = String;
    fn try_from(wire: WireVoyageRequest) -> Result<Self, Self::Error> {
        let command: VoyageCommand = serde_json::from_value(Value::Object(wire.fields.clone()))
            .map_err(|_| "invalid public voyage operation".to_owned())?;
        let encoded = serde_json::to_value(&command)
            .map_err(|_| "invalid public voyage operation".to_owned())?;
        let allowed = encoded
            .as_object()
            .ok_or("invalid public voyage operation")?;
        if wire.fields.keys().any(|key| !allowed.contains_key(key)) {
            return Err("unknown public voyage operation field".to_owned());
        }
        Ok(Self {
            session_id: wire.session_id,
            incarnation: wire.incarnation,
            command,
        })
    }
}

/// Public session result, without an embedded private runtime protocol envelope.
/// Errors and delivery uncertainty belong to the outer VesselResponse.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoyageReply {
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub result: Value,
}

/// Deliberate public operation whitelist. Extending RuntimeCommand does not expose
/// a new client operation. Preserve serialized mutation payloads for saved receipts.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoyageCommand {
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
        operation: TerminalAction,
    },
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
    /// Read immutable session-owned artifact bytes; requires history authority.
    ReadArtifact {
        artifact_id: Uuid,
        offset: u64,
        limit: u32,
    },
    /// Bounded immutable upload, independently deduplicated by upload_id. No execution.
    UploadImage {
        upload_id: Uuid,
        name: String,
        data_base64: String,
    },
    /// Ordered content references. Image bytes never enter command reservations.
    SubmitContent {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        content: Vec<crate::content::ContentPart>,
    },
    Submit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coordination: Option<crate::coordination::CoordinationSource>,
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        prompt: String,
    },
    Receipt {
        command_id: Uuid,
    },
    Resolve {
        command_id: Uuid,
        #[serde(default)]
        original: Option<Box<VoyageCommand>>,
    },
    Cancel {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        run_id: Uuid,
    },
    Steer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        coordination: Option<crate::coordination::CoordinationSource>,
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
    /// Atomically replace next-turn inference overrides. Null means provider default.
    SetInference {
        command_id: Uuid,
        expected_revision: u64,
        expires_at_ms: u64,
        model: String,
        reasoning_effort: Option<String>,
        service_tier: Option<String>,
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
}

impl VoyageCommand {
    pub fn requires_incarnation(&self) -> bool {
        matches!(
            self,
            Self::ExecuteTool { .. }
                | Self::Terminal { .. }
                | Self::Cancel { .. }
                | Self::Steer { .. }
                | Self::Respond { .. }
        )
    }
    pub fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Clear { command_id, .. }
            | Self::Compact { command_id, .. }
            | Self::OperatorTool { command_id, .. }
            | Self::Github { command_id, .. }
            | Self::SetAccess { command_id, .. }
            | Self::Configure { command_id, .. }
            | Self::WorkflowSubmit { command_id, .. }
            | Self::ExecuteTool { command_id, .. }
            | Self::SubmitContent { command_id, .. }
            | Self::Submit { command_id, .. }
            | Self::Receipt { command_id, .. }
            | Self::Cancel { command_id, .. }
            | Self::Steer { command_id, .. }
            | Self::Rename { command_id, .. }
            | Self::SetModel { command_id, .. }
            | Self::SetInference { command_id, .. }
            | Self::Archive { command_id, .. }
            | Self::Delete { command_id, .. }
            | Self::Respond { command_id, .. } => Some(*command_id),
            _ => None,
        }
    }
}

/// Human terminal traffic is private, never persisted or replayed.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalAction {
    Attach,
    Snapshot,
    Write { bytes: Vec<u8> },
    Resize { columns: u16, rows: u16 },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum VesselCommand {
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
        binding: crate::process::ParticipantBinding,
    },
    RemoveParticipant {
        command_id: Uuid,
        binding_id: Uuid,
        expected_revision: u64,
        cancel: bool,
    },
    Assign {
        request: crate::process::AssignmentRequest,
    },
    FenceAssignment {
        request: crate::process::AssignmentRequest,
    },
    ObserveAssignment {
        assignment_id: Uuid,
    },
    CancelAssignment {
        assignment_id: Uuid,
    },
    Identity,
    TrustVessel {
        identity: crate::process::VesselIdentity,
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
        preparation: crate::process::SignedArtifact<crate::process::TransferPreparation>,
    },
    AcceptTransfer {
        manifest: crate::process::SignedArtifact<crate::process::TransferManifest>,
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
        endpoint: String,
    },
    RevokeGrant {
        command_id: Uuid,
        grant_id: Uuid,
        expected_revision: u64,
    },
    Granted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_vessel_id: Option<Uuid>,
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
    /// Resolve the exact original creation identity without launching a process.
    ResolveStart {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
    },
    StartConfigured {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
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
    Stop {
        session_id: Uuid,
        incarnation: Uuid,
    },
    /// Explicit voyage operations serialize directly as top-level op fields.
    #[serde(untagged)]
    Voyage(VoyageRequest),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VesselRequest {
    pub protocol: u32,
    pub command: VesselCommand,
}

#[derive(Clone, Serialize, Deserialize)]
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

pub use crate::process::{
    AssignmentRequest, ParticipantBinding, ParticipantGrantBinding, SignedArtifact,
    TransferManifest, TransferPreparation, VesselIdentity, read_frame, write_frame,
};

// Upload bytes must never enter diagnostics via enclosing request Debug derives.
impl std::fmt::Debug for VoyageCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoyageCommand")
            .field("command_id", &self.mutation_id())
            .finish_non_exhaustive()
    }
}

// Granted envelopes and grant/pairing results can contain credentials. Never let
// enclosing request/response diagnostics expose bearer tokens or private input.
impl std::fmt::Debug for VesselCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VesselCommand")
            .field("variant", &std::mem::discriminant(self))
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for VesselResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VesselResponse")
            .field("protocol", &self.protocol)
            .field("has_error", &self.error.is_some())
            .field("outcome_unknown", &self.outcome_unknown)
            .finish_non_exhaustive()
    }
}
