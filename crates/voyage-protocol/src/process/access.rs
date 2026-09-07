//! Explicit session grants. Enrollment membership alone is never an execution grant.
use super::RuntimeCommand;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessRight {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentIdentity {
    pub database_path: PathBuf,
    pub machine_id: Uuid,
    pub epoch: u64,
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
    pub enrollment: Option<EnrollmentIdentity>,
    pub token_hash: String,
    #[serde(default)]
    pub parent_grant: Option<GrantBinding>,
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
        | RuntimeCommand::Receipt { .. } => Some(ProcessRight::History),
        RuntimeCommand::Decisions => Some(ProcessRight::Decide),
        RuntimeCommand::Submit { .. }
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
        RuntimeCommand::Terminal { .. } => Some(ProcessRight::Terminal),
        RuntimeCommand::Configure { .. }
        | RuntimeCommand::SetAccess { .. }
        | RuntimeCommand::SetInference { .. } => None,
        _ => None,
    }
}
