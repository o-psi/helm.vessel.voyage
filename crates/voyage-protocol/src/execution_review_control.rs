//! Protected administrator-review bookkeeping. This is not a public launch API.
use super::execution_identity::{ExecutionReceipt, ExecutionReview};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorOwnerChange {
    pub command_id: Uuid,
    pub principal_id: Uuid,
    pub expected_authority_revision: u64,
    pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorOwnerReceipt {
    pub command_id: Uuid,
    pub vessel_id: Uuid,
    pub principal_id: Uuid,
    pub authority_revision: u64,
    pub enabled: bool,
    /// Fences authority; each running guardian must separately observe cleanup.
    pub grants_fenced: Vec<Uuid>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionReviewControlAction {
    Cancel,
    Revoke,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReviewControl {
    pub command_id: Uuid,
    pub review_id: Uuid,
    pub digest: String,
    pub action: ExecutionReviewControlAction,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedExecutionReview {
    pub review: ExecutionReview,
    pub receipt: ExecutionReceipt,
    pub administrator_grant_id: Option<Uuid>,
}
