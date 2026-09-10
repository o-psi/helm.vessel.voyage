//! Explicit session grants, distinct from workspace connection authority.
use super::RuntimeCommand;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessRight {
    /// Workspace catalogue access; distinct from per-session observation.
    Catalogue,
    /// Creation inside an explicitly approved workspace.
    Create,
    Observe,
    History,
    Execute,
    Steer,
    Decide,
    Cancel,
    Lifecycle,
    Terminal,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantBinding {
    pub grant_id: Uuid,
    pub revision: u64,
    pub principal_id: Uuid,
}

/// Host-private authority record; never include this record in public snapshots.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessGrant {
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub session_id: Uuid,
    pub workspace: PathBuf,
    pub revision: u64,
    pub rights: Vec<ProcessRight>,
    pub expires_at_ms: u64,
    pub revoked: bool,
    pub token_hash: String,
    #[serde(default)]
    pub parent_grant: Option<GrantBinding>,
    /// Separate workspace authority; never substitutes for a participant parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_binding: Option<GrantBinding>,
    #[serde(default)]
    pub participant_binding: Option<super::ParticipantGrantBinding>,
}

/// A private client credential file, distinct from provider credentials.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessCredential {
    pub endpoint: String,
    pub grant_id: Uuid,
    pub session_id: Uuid,
    pub token: String,
}

/// Private discovery record for the account-local Vessel HTTP listener.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalAccessCredential {
    pub endpoint: String,
    pub token: String,
}

#[allow(unreachable_patterns)]
pub fn required_process_right(command: &RuntimeCommand) -> Option<ProcessRight> {
    match command {
        RuntimeCommand::Resolve {
            command_id,
            original: Some(original),
        } if original.mutation_id() == Some(*command_id) => required_process_right(original),
        // Legacy payload-free resolution can reserve an unknown ID and therefore
        // requires the host's local authority, not a read-only scoped grant.
        RuntimeCommand::Resolve { .. } => None,
        RuntimeCommand::Health | RuntimeCommand::Events { .. } => Some(ProcessRight::Observe),
        RuntimeCommand::Controls { section, .. } if section == "host_resources" => None,
        RuntimeCommand::Controls { section, .. }
            if matches!(section.as_str(), "tools" | "policy") =>
        {
            Some(ProcessRight::Observe)
        }
        RuntimeCommand::Controls { .. } | RuntimeCommand::WorkflowPreview { .. } => {
            Some(ProcessRight::History)
        }
        RuntimeCommand::AssignmentObserve { .. } => Some(ProcessRight::History),
        RuntimeCommand::Snapshot
        | RuntimeCommand::History { .. }
        | RuntimeCommand::MessageChunk { .. }
        | RuntimeCommand::RunOutput { .. }
        | RuntimeCommand::ReadArtifact { .. }
        | RuntimeCommand::Receipt { .. } => Some(ProcessRight::History),
        RuntimeCommand::Decisions => Some(ProcessRight::Decide),
        RuntimeCommand::UploadImage { .. }
        | RuntimeCommand::SubmitContent { .. }
        | RuntimeCommand::Submit { .. }
        | RuntimeCommand::ExecuteTool { .. }
        | RuntimeCommand::OperatorTool { .. }
        | RuntimeCommand::WorkflowInputs { .. }
        | RuntimeCommand::WorkflowSubmit { .. } => Some(ProcessRight::Execute),
        RuntimeCommand::Steer { .. } => Some(ProcessRight::Steer),
        RuntimeCommand::Respond { .. } => Some(ProcessRight::Decide),
        RuntimeCommand::Cancel { .. } => Some(ProcessRight::Cancel),
        RuntimeCommand::Clear { .. }
        | RuntimeCommand::Compact { .. }
        | RuntimeCommand::Rename { .. }
        | RuntimeCommand::SetModel { .. }
        | RuntimeCommand::Archive { .. }
        | RuntimeCommand::Delete { .. }
        | RuntimeCommand::Stop => Some(ProcessRight::Lifecycle),
        RuntimeCommand::PrepareBrowser | RuntimeCommand::Browser { .. } => {
            Some(ProcessRight::Execute)
        }
        RuntimeCommand::Terminal { .. } => Some(ProcessRight::Terminal),
        RuntimeCommand::Configure { .. }
        | RuntimeCommand::SetAccess { .. }
        | RuntimeCommand::SetInference { .. } => None,
        _ => None,
    }
}

/// Explicit v1 workspace credential. Legacy session credentials remain unchanged.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCredential {
    pub schema_version: u32,
    pub kind: String,
    pub endpoint: String,
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub vessel_id: Uuid,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedWorkspace {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub provider_ready: Option<bool>,
}

/// Host-private workspace authority. A missing session ID never confers this scope.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionGrant {
    pub schema_version: u32,
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub vessel_id: Uuid,
    pub revision: u64,
    pub rights: Vec<ProcessRight>,
    pub expires_at_ms: u64,
    pub revoked: bool,
    pub token_hash: String,
    pub workspaces: Vec<ApprovedWorkspace>,
}
