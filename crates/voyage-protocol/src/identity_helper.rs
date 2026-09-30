//! Private, bounded helper messages. The supervisor selects and drops the OS
//! identity before exec; these messages never grant execution authority.
use crate::accounts::{AccountBinding, EnrollmentActor, EnrollmentRequest, Transport};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub const IDENTITY_HELPER_SCHEMA: u32 = 1;
pub const IDENTITY_HELPER_BYTES: usize = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityHelperRequest {
    pub schema: u32,
    pub workspace: PathBuf,
    pub operation: IdentityHelperOperation,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdentityHelperOperation {
    /// Offline journal reconciliation under its exact current execution identity.
    RecoverBound { request: BoundRecoveryRequest },
    /// Source identity validates an owned portable checkpoint; only metadata returns.
    ObserveTransferArtifact { artifact_path: PathBuf },
    /// Reads an exact private frozen launch file; no login defaults or network.
    ReviewConfig { config_path: PathBuf },
    Accounts {
        scope: IdentityAccountScope,
        transport: Option<Transport>,
    },
    Defaults {
        scope: IdentityAccountScope,
        profile: Option<crate::execution_profiles::ExecutionProfile>,
        profiles_revision: u64,
    },
    Models {
        scope: IdentityAccountScope,
        account: AccountBinding,
    },
    ValidateAccount {
        scope: IdentityAccountScope,
        account: AccountBinding,
    },
    ObserveAccountIntent {
        scope: IdentityAccountScope,
        account: AccountBinding,
    },
    ValidateProfile {
        scope: IdentityAccountScope,
        profile: crate::execution_profiles::ExecutionProfile,
    },
    /// Freeze initialization configuration and verify private provenance after UID drop.
    CaptureInitialization {
        scope: IdentityAccountScope,
        directory: PathBuf,
        request_digest: String,
        base_config_path: Option<PathBuf>,
        initialize: crate::process::RuntimeInitialization,
    },
    CaptureLaunch {
        scope: IdentityAccountScope,
        directory: PathBuf,
        request_digest: String,
        base_config_path: Option<PathBuf>,
        account: Option<AccountBinding>,
        settings: crate::start_settings::StartSettings,
    },
    Enrollment {
        scope: IdentityAccountScope,
        operation: IdentityEnrollmentOperation,
    },
    Usage {
        scope: IdentityAccountScope,
        account: AccountBinding,
        refresh: bool,
    },
    SetDefault {
        scope: IdentityAccountScope,
        command_id: Uuid,
        account: AccountBinding,
        expected_revision: u64,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdentityEnrollmentOperation {
    Start {
        request: EnrollmentRequest,
    },
    Resolve {
        request: EnrollmentRequest,
    },
    Drive {
        enrollment_id: Uuid,
    },
    Status {
        enrollment_id: Uuid,
    },
    Cancel {
        command_id: Uuid,
        enrollment_id: Uuid,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityAuthorityRight {
    Use,
    Enroll,
    /// Nonexecuting offline bookkeeping only; never account or launch authority.
    Recover,
}

/// An independent private pipe checks CURRENT supervisor authority at each
/// credential/network/publication boundary. Only a boolean crosses back.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityAuthorityRequest {
    pub schema: u32,
    pub actor: EnrollmentActor,
    pub right: IdentityAuthorityRight,
    pub connection_id: Uuid,
    pub account_id: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityAuthorityReply {
    pub schema: u32,
    pub allowed: bool,
}

/// A private projection of an authenticated grant. The root caller retains all
/// current-grant checks; a helper cannot turn this into supervisor authority.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityAccountScope {
    pub full_access: bool,
    pub can_use: bool,
    pub can_enroll: bool,
    pub account_ids: Vec<Uuid>,
    pub enrollment_connections: Vec<Uuid>,
    pub actor: EnrollmentActor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityConfigFacts {
    pub account: AccountBinding,
    pub capability_revision: u64,
    pub policy_digest: String,
    pub config_digest: String,
    pub account_root_digest: String,
    pub uid: u32,
    pub gid: u32,
    pub supplementary_groups: Vec<u32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum IdentityHelperResponse {
    Facts {
        facts: IdentityConfigFacts,
    },
    /// Safe account/model/default observations or retained launch-file identity.
    Value {
        value: serde_json::Value,
    },
    /// No configuration, filesystem, credential or subprocess diagnostics.
    Unavailable,
}

/// Root supplies positive retirement proof over its authenticated private pipe.
/// This contains no canonical history, credentials or execution instruction.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundRecoveryRequest {
    pub directory: PathBuf,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub command_id: Uuid,
    pub local_process_retired: bool,
    /// Exact current-incarnation guardian cleanup, distinct from never-launched retirement.
    pub current_scope_cleanup_observed: bool,
    pub actor: EnrollmentActor,
    pub acknowledge_cleanup: Option<Uuid>,
    pub acknowledge_resources: Vec<Uuid>,
    pub reconcile_tools: Option<Uuid>,
    pub expected_revision: Option<u64>,
}
