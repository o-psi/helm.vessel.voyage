//! Named executing-host preferences. No credentials or execution permissions.
use crate::accounts::AccountBinding;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExecutionProfile {
    pub id: Uuid,
    pub name: String,
    pub account: AccountBinding,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileCatalogue {
    pub revision: u64,
    pub profiles: Vec<ExecutionProfile>,
    pub default_profile_id: Option<Uuid>,
    pub can_manage: bool,
}
