//! Private, bounded helper messages. The supervisor selects and drops the OS
//! identity before exec; these messages never grant execution authority.
use crate::accounts::{AccountBinding, EnrollmentActor, Transport};
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
    ValidateProfile {
        scope: IdentityAccountScope,
        profile: crate::execution_profiles::ExecutionProfile,
    },
    CaptureLaunch {
        scope: IdentityAccountScope,
        directory: PathBuf,
        request_digest: String,
        base_config_path: Option<PathBuf>,
        account: Option<AccountBinding>,
        settings: crate::start_settings::StartSettings,
    },
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
