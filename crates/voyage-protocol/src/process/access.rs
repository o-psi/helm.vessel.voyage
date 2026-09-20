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
    AccountUse,
    AccountEnroll,
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
    /// Explicit owner connection authority; valid only with a current owner connection binding.
    #[serde(default)]
    pub full_access: bool,
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub session_id: Uuid,
    pub workspace: PathBuf,
    pub revision: u64,
    pub rights: Vec<ProcessRight>,
    #[serde(default)]
    pub accounts: Vec<Uuid>,
    #[serde(default)]
    pub enrollment_connections: Vec<Uuid>,
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
        RuntimeCommand::Resolve { .. } | RuntimeCommand::NotificationEvents { .. } => None,
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
        | RuntimeCommand::ProviderAttempts { .. }
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
        RuntimeCommand::Respond { response, .. } if response.get("root_grant").is_some() => None,
        RuntimeCommand::Respond { .. } => Some(ProcessRight::Decide),
        RuntimeCommand::Cancel { .. } => Some(ProcessRight::Cancel),
        RuntimeCommand::Clear { .. }
        | RuntimeCommand::Compact { .. }
        | RuntimeCommand::Rename { .. }
        | RuntimeCommand::SetModel { .. }
        | RuntimeCommand::Archive { .. }
        | RuntimeCommand::Delete { .. }
        | RuntimeCommand::Stop => Some(ProcessRight::Lifecycle),
        RuntimeCommand::HostBrowser { operation, .. } => Some(operation.required_right()),
        RuntimeCommand::PrepareBrowser | RuntimeCommand::Browser { .. } => {
            Some(ProcessRight::Execute)
        }
        RuntimeCommand::Terminal { .. } => Some(ProcessRight::Terminal),
        RuntimeCommand::SetAccountInference { .. } => Some(ProcessRight::AccountUse),
        RuntimeCommand::Respond { response, .. } if response.get("root_grant").is_some() => {
            Some(ProcessRight::Execute)
        }
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
    /// Missing in legacy grants means scoped, never owner.
    #[serde(default)]
    pub full_access: bool,
    pub schema_version: u32,
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub vessel_id: Uuid,
    pub revision: u64,
    pub rights: Vec<ProcessRight>,
    #[serde(default)]
    pub accounts: Vec<Uuid>,
    #[serde(default)]
    pub enrollment_connections: Vec<Uuid>,
    pub expires_at_ms: u64,
    pub revoked: bool,
    pub token_hash: String,
    pub workspaces: Vec<ApprovedWorkspace>,
}

impl ProcessRight {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Catalogue,
            Self::AccountUse,
            Self::AccountEnroll,
            Self::Create,
            Self::Observe,
            Self::History,
            Self::Execute,
            Self::Steer,
            Self::Decide,
            Self::Cancel,
            Self::Lifecycle,
            Self::Terminal,
        ]
    }
}

/// Additional human owner operations; unknown and internal commands remain denied.
pub fn owner_connection_right(command: &RuntimeCommand) -> Option<ProcessRight> {
    match command {
        RuntimeCommand::Resolve {
            command_id,
            original: Some(original),
        } if original.mutation_id() == Some(*command_id) => owner_connection_right(original),
        RuntimeCommand::Respond { response, .. } if response.get("root_grant").is_some() => {
            Some(ProcessRight::Execute)
        }
        RuntimeCommand::Configure { .. }
        | RuntimeCommand::SetAccess { .. }
        | RuntimeCommand::SetInference { .. } => Some(ProcessRight::Execute),
        RuntimeCommand::Controls { section, .. } if section == "host_resources" => {
            Some(ProcessRight::History)
        }
        _ => required_process_right(command),
    }
}

impl ConnectionGrant {
    /// Owner and scoped allowlists cannot be combined. Empty legacy scopes do not confer ownership.
    pub fn valid_owner_scope(&self) -> bool {
        !self.full_access
            || (self.workspaces.is_empty()
                && self.accounts.is_empty()
                && self.enrollment_connections.is_empty()
                && self.rights == ProcessRight::all())
    }
}

#[cfg(test)]
mod root_grant_tests {
    use super::*;
    #[test]
    fn generic_decide_cannot_manage_filesystem_authority() {
        let mut command = RuntimeCommand::Respond {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 10,
            run_id: Uuid::new_v4(),
            decision_id: Uuid::new_v4(),
            response: serde_json::json!({"root_grant":"approved"}),
        };
        assert_eq!(required_process_right(&command), None);
        assert_eq!(
            owner_connection_right(&command),
            Some(ProcessRight::Execute)
        );
        let wrapped = RuntimeCommand::Resolve {
            command_id: command.mutation_id().unwrap(),
            original: Some(Box::new(command.clone())),
        };
        assert_eq!(required_process_right(&wrapped), None);
        assert_eq!(
            owner_connection_right(&wrapped),
            Some(ProcessRight::Execute)
        );
        if let RuntimeCommand::Respond { response, .. } = &mut command {
            *response = serde_json::json!("approved");
        }
        assert_eq!(required_process_right(&command), Some(ProcessRight::Decide));
    }
}
