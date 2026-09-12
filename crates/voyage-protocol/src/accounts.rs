//! Safe executing-host account contracts. Credentials are deliberately absent.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    OpenaiResponses,
    OpenaiChat,
    ChatgptOauth,
    Anthropic,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionDescriptor {
    pub id: Uuid,
    pub revision: u64,
    pub label: String,
    /// Exact approved base endpoint; compatibility does not authorize another endpoint.
    pub endpoint: String,
    pub transports: Vec<Transport>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Ready,
    SignInRequired,
    Removed,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialAvailability {
    Available,
    Expired,
    EnvironmentUnavailable,
    EnvironmentChanged,
    RefreshPendingOrUncertain,
    Missing,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountDescriptor {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub alias: String,
    pub label: String,
    pub metadata_revision: u64,
    pub identity_generation: u64,
    pub credential_revision: u64,
    /// Permission-relevant credential changes; ordinary OAuth refresh does not advance this.
    pub capability_revision: u64,
    /// Local observation only, not entitlement validation or permission to use the account.
    pub availability: CredentialAvailability,
    pub state: AccountState,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountBinding {
    pub account_id: Uuid,
    pub connection_id: Uuid,
    pub identity_generation: u64,
    pub connection_revision: u64,
    pub transport: Transport,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnrollmentActor {
    pub principal: String,
    pub workspace: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnrollmentRequest {
    pub command_id: Uuid,
    pub enrollment_id: Uuid,
    pub connection_id: Uuid,
    pub alias: String,
    pub label: String,
    pub actor: EnrollmentActor,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    Starting,
    Pending,
    Exchanging,
    Succeeded,
    Cancelled,
    Expired,
    Denied,
    Uncertain,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnrollmentStatus {
    pub enrollment_id: Uuid,
    pub state: EnrollmentState,
    pub account_id: Option<Uuid>,
    pub expires_at: u64,
    /// Cancellation/expiry cannot prove that an upstream authorization effect was undone.
    pub effects_may_have_occurred: bool,
}
/// Sensitive human-only response: NEVER put in events, receipts, tools or history.
/// Intentionally does not implement Debug.
#[derive(Clone, Serialize, Deserialize)]
pub struct PrivateEnrollmentStatus {
    pub status: EnrollmentStatus,
    pub user_code: Option<String>,
    pub verification_uri: Option<String>,
    /// Fixed diagnostic vocabulary only; never provider bodies or credential material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<EnrollmentFailure>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentPhase {
    RequestCode,
    Poll,
    Exchange,
    Publication,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentFailureKind {
    Timeout,
    Connection,
    Rejected,
    InvalidResponse,
    Storage,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnrollmentFailure {
    pub phase: EnrollmentPhase,
    pub kind: EnrollmentFailureKind,
    pub http_status: Option<u16>,
}
