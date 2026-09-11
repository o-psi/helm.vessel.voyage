//! Private process transport. HTTP adapter authentication is required separately.
use super::GrantBinding;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

pub const PROCESS_PROTOCOL: u32 = 1;
pub const MAX_PROCESS_FRAME: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeCommand {
    /// Follow/resume the ordinary owner for an explicitly authorized local share.
    /// No browser effect, sharing authority, or agent turn is created.
    PrepareBrowser,
    Browser {
        operation: crate::browser::BrowserOperation,
    },
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
        /// Optional declared secret names to render as references, never values.
        /// Required secret names are always included. None preserves legacy preview.
        /// Clients selecting optional names must check response `secret_names` parity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        optional_secret_names: Option<Vec<String>>,
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
    /// Resolve delivery without executing the original request. A missing command
    /// is durably fenced against later admission before returning not_admitted.
    Resolve {
        command_id: Uuid,
        #[serde(default)]
        original: Option<Box<RuntimeCommand>>,
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
    /// Browser executors can receive private agent arguments as well as effects.
    /// Execute alone is insufficient; scoped admission also needs History.
    pub fn requires_browser_history(&self) -> bool {
        match self {
            Self::PrepareBrowser | Self::Browser { .. } => true,
            Self::Resolve {
                original: Some(original),
                ..
            } => original.requires_browser_history(),
            _ => false,
        }
    }
    /// Immutable identity of a journalled mutation, excluding resolution itself.
    pub fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Browser { operation } => operation.mutation_id(),
            Self::Clear { command_id, .. }
            | Self::Compact { command_id, .. }
            | Self::OperatorTool { command_id, .. }
            | Self::Github { command_id, .. }
            | Self::SetAccess { command_id, .. }
            | Self::Configure { command_id, .. }
            | Self::Relinquish { command_id, .. }
            | Self::WorkflowSubmit { command_id, .. }
            | Self::SubmitContent { command_id, .. }
            | Self::Submit { command_id, .. }
            | Self::Cancel { command_id, .. }
            | Self::Steer { command_id, .. }
            | Self::Rename { command_id, .. }
            | Self::SetModel { command_id, .. }
            | Self::SetInference { command_id, .. }
            | Self::Respond { command_id, .. }
            | Self::Archive { command_id, .. }
            | Self::Delete { command_id, .. }
            | Self::Branch { command_id, .. }
            | Self::ExecuteTool { command_id, .. } => Some(*command_id),
            _ => None,
        }
    }

    /// Commands served under the session fence without starting execution.
    /// Resolve may record non-admission; it never dispatches the original work.
    pub fn observes_suspended(&self) -> bool {
        matches!(
            self,
            Self::Health
                | Self::Stop
                | Self::Snapshot
                | Self::History { .. }
                | Self::MessageChunk { .. }
                | Self::RunOutput { .. }
                | Self::ReadArtifact { .. }
                | Self::Receipt { .. }
                | Self::Resolve { .. }
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
    /// Last bounded canonical name observed from this owner. This is catalogue
    /// metadata only; canonical conversation state remains in the voyage journal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
            name: value.name.clone(),
        }
    }
}

fn unknown_outcome() -> bool {
    true
}

// Upload bytes must never enter diagnostics via enclosing request Debug derives.
impl std::fmt::Debug for RuntimeCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeCommand")
            .field("command_id", &self.mutation_id())
            .finish_non_exhaustive()
    }
}
