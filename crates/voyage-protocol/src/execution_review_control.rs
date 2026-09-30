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

/// Human-visible operations use configured references only; never UID/env/paths.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="action",rename_all="snake_case",deny_unknown_fields)]
pub enum ExecutionOperation {
    Inventory,
    Prepare {
        review_id:Uuid,
        command_id:Uuid,
        session_id:Uuid,
        workspace:std::path::PathBuf,
        identity:super::execution_identity::IdentityRef,
    },
    PrepareTransition {
        review_id:Uuid,
        command_id:Uuid,
        session_id:Uuid,
        source_incarnation:Uuid,
        identity:super::execution_identity::IdentityRef,
        stop_source:bool,
    },
    Approve { approval:super::execution_identity::ReviewApproval },
    Review { review_id:Uuid },
    Control { control:ExecutionReviewControl },
    Status { session_id:Uuid },
}
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionStatus {
    pub session_id:Uuid,
    pub incarnation:Uuid,
    pub identity:super::execution_identity::IdentitySummary,
    pub observed:Option<super::execution_identity::ObservedExecution>,
    pub process_state:super::process::ProcessState,
    pub administrator_authorized:bool,
    pub cleanup_observed:bool,
}
