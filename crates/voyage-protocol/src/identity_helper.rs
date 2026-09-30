//! Private, bounded helper messages. The supervisor selects and drops the OS
//! identity before exec; these messages never grant execution authority.
use crate::accounts::AccountBinding;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    Facts { facts: IdentityConfigFacts },
    /// No configuration, filesystem, credential or subprocess diagnostics.
    Unavailable,
}
