//! Staged contracts for privileged supervision. These records do not grant OS or
//! connection authority. No transport advertises support until its complete launch,
//! storage, approval and recovery boundary is implemented.
use crate::process::ProcessPeerUids;
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU64, path::PathBuf};
use uuid::Uuid;

pub const EXECUTION_SCHEMA: u32 = 1;
pub const MAX_REVIEW_LIFETIME_MS: u64 = 15 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationScope {
    User,
    System,
}

/// Host-local references, never a client-supplied UID, executable or environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRef {
    pub id: Uuid,
    pub revision: NonZeroU64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountContextRef {
    pub id: Uuid,
    pub revision: NonZeroU64,
}

/// A nonzero UID alone is not proof of an unprivileged account. Existing host
/// grants must be inventoried; unknown rights must be presented as unknown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClass {
    Ordinary,
    Administrator,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySummary {
    pub identity: IdentityRef,
    pub label: String,
    pub authority: AuthorityClass,
    pub account_context: AccountContextRef,
    pub available: bool,
    pub unavailable_reason: Option<ExecutionFailure>,
}

/// Supervisor-controlled host configuration. Client requests refer only to
/// `identity`; they never supply these operating-system details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredExecutionIdentity {
    pub identity: IdentityRef,
    pub label: String,
    pub user_name: String,
    pub uid: u32,
    pub gid: u32,
    pub supplementary_groups: Vec<u32>,
    pub home: PathBuf,
    pub account_context: AccountContextRef,
    pub authority: AuthorityClass,
    pub enabled: bool,
}

/// Authoritative incarnation binding in the supervisor catalogue. The runtime
/// receives only the fields needed for its own launch and peer checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub identity: IdentityRef,
    pub account_context: AccountContextRef,
    pub peer_uids: ProcessPeerUids,
    pub administrator_grant_id: Option<Uuid>,
    pub host_identity_digest: String,
    pub policy_digest: String,
}

/// Missing capability on an old peer means unknown service identity, not ordinary
/// execution. Disabled records never promise an implemented system backend.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionCapability {
    #[default]
    Unavailable,
    Available {
        schema: u32,
        installation_scope: InstallationScope,
        identities: Vec<IdentitySummary>,
        can_review_administrator: bool,
        can_transition: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionChange {
    Start,
    ReplaceProcess,
    Transition,
}

/// Constructed by the supervisor from authenticated and protected state. A client
/// approves a saved review ID/digest; it never supplies these facts as authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewFacts {
    pub vessel_id: Uuid,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub incarnation: Uuid,
    pub requester_id: Uuid,
    pub connection_id: Uuid,
    pub connection_revision: NonZeroU64,
    pub administrative_owner_id: Uuid,
    pub authority_revision: NonZeroU64,
    pub expected_session_revision: u64,
    pub change: ExecutionChange,
    pub previous_incarnation: Option<Uuid>,
    pub identity: IdentityRef,
    pub account_context: AccountContextRef,
    pub account: crate::accounts::AccountBinding,
    pub account_capability_revision: u64,
    pub workspace: PathBuf,
    /// Digest includes resolved UID/GID/groups, account record, execution home and
    /// host isolation/capability observations. Re-resolve before launch.
    pub host_identity_digest: String,
    pub policy_digest: String,
    pub release_digest: String,
    /// Exact work and observed cleanup reviewed for this transition. No replay of
    /// uncertain effects or queued calls is implied by an execution approval.
    pub pending_work_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReview {
    pub schema: u32,
    pub review_id: Uuid,
    pub command_id: Uuid,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub facts: ReviewFacts,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewApproval {
    pub review_id: Uuid,
    pub command_id: Uuid,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedExecution {
    pub identity: IdentityRef,
    pub incarnation: Uuid,
    pub uid: u32,
    pub gid: u32,
    pub supplementary_groups: Vec<u32>,
    pub effective_capabilities: u64,
    pub permitted_capabilities: u64,
    pub inheritable_capabilities: u64,
    pub ambient_capabilities: u64,
    pub no_new_privileges: bool,
    /// Namespace identity and /proc start time belong to the observed process,
    /// not to its human-readable label or a reused numeric PID.
    pub user_namespace: String,
    pub process_start_ticks: u64,
    pub release_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutionOutcome {
    AwaitingApproval,
    Approved,
    Launching,
    Ready {
        observed: ObservedExecution,
    },
    Refused {
        reason: ExecutionFailure,
    },
    Cancelled,
    /// Authority is fenced. This is not observed process or resource cleanup.
    RevocationRequested {
        administrator_grant_id: Uuid,
    },
    /// An uncertain operation cannot automatically be retried or replaced.
    Unconfirmed {
        cleanup_obligations: Vec<Uuid>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub schema: u32,
    pub vessel_id: Uuid,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub incarnation: Uuid,
    pub command_id: Uuid,
    pub review_id: Uuid,
    pub review_digest: String,
    pub outcome: ExecutionOutcome,
}

/// A continuing administrator choice for exactly one voyage. This is authority
/// only when loaded from protected supervisor storage after authenticating the
/// owner who created it. Runtime and client copies are informational.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorGrant {
    pub schema: u32,
    pub grant_id: Uuid,
    pub vessel_id: Uuid,
    pub session_id: Uuid,
    pub administrative_owner_id: Uuid,
    pub authority_revision: NonZeroU64,
    pub identity: IdentityRef,
    pub account_context: AccountContextRef,
    pub host_identity_digest: String,
    pub policy_digest: String,
    pub created_at_ms: u64,
    pub revoked_at_ms: Option<u64>,
}

impl AdministratorGrant {
    /// Recheck before every privileged launch. The caller must obtain `self`
    /// from protected state and independently authenticate the saved owner.
    pub fn check_current(&self, current: &Self) -> Result<(), ExecutionFailure> {
        if self.schema != EXECUTION_SCHEMA
            || self.grant_id.is_nil()
            || self.vessel_id.is_nil()
            || self.session_id.is_nil()
            || self.administrative_owner_id.is_nil()
            || !digest(&self.host_identity_digest)
            || !digest(&self.policy_digest)
            || self.created_at_ms == 0
            || self.revoked_at_ms.is_some_and(|at| at < self.created_at_ms)
        {
            return Err(ExecutionFailure::InvalidReview);
        }
        if self.revoked_at_ms.is_some() {
            return Err(ExecutionFailure::OwnerRequired);
        }
        if self != current {
            return Err(ExecutionFailure::StaleReview);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFailure {
    Unsupported,
    OwnerRequired,
    IdentityUnavailable,
    AccountUnavailable,
    StaleReview,
    ExpiredReview,
    InvalidReview,
    PolicyDenied,
    OsAccessDenied,
    FilesystemReadOnly,
    IsolationUnavailable,
    CleanupUnconfirmed,
    LaunchUnconfirmed,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl ExecutionReview {
    /// Content integrity, not a signature or permission. Only a protected saved
    /// review may be approved. The domain and tuple order are part of schema 1.
    pub fn calculated_digest(&self) -> Result<String, ExecutionFailure> {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(
            "voyage/execution-review/v1",
            self.schema,
            self.review_id,
            self.command_id,
            self.created_at_ms,
            self.expires_at_ms,
            &self.facts,
        ))
        .map_err(|_| ExecutionFailure::InvalidReview)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    /// Shape/freshness only. Callers must also authenticate the administrative
    /// owner, load the review from protected storage, reserve the command durably,
    /// and refuse consumed/revoked receipts. A receipt is never a standing grant.
    pub fn check_current(
        &self,
        approval: &ReviewApproval,
        current: &ReviewFacts,
        now_ms: u64,
    ) -> Result<(), ExecutionFailure> {
        let facts = &self.facts;
        if self.schema != EXECUTION_SCHEMA
            || self.review_id.is_nil()
            || self.command_id.is_nil()
            || [
                facts.vessel_id,
                facts.session_id,
                facts.run_id,
                facts.incarnation,
                facts.requester_id,
                facts.connection_id,
                facts.administrative_owner_id,
                facts.identity.id,
                facts.account_context.id,
                facts.account.account_id,
                facts.account.connection_id,
            ]
            .iter()
            .any(Uuid::is_nil)
            || ![
                &self.digest,
                &facts.host_identity_digest,
                &facts.policy_digest,
                &facts.release_digest,
                &facts.pending_work_digest,
            ]
            .into_iter()
            .all(|s| digest(s))
            || !facts.workspace.is_absolute()
            || facts
                .workspace
                .to_str()
                .is_none_or(|s| s.chars().any(char::is_control))
            || match facts.change {
                ExecutionChange::Start => facts.previous_incarnation.is_some(),
                ExecutionChange::ReplaceProcess | ExecutionChange::Transition => facts
                    .previous_incarnation
                    .is_none_or(|id| id.is_nil() || id == facts.incarnation),
            }
            || self.expires_at_ms <= self.created_at_ms
            || self.expires_at_ms - self.created_at_ms > MAX_REVIEW_LIFETIME_MS
            || self.digest != self.calculated_digest()?
        {
            return Err(ExecutionFailure::InvalidReview);
        }
        if now_ms < self.created_at_ms || now_ms >= self.expires_at_ms {
            return Err(ExecutionFailure::ExpiredReview);
        }
        if self.review_id != approval.review_id
            || self.command_id != approval.command_id
            || self.digest != approval.digest
            || facts != current
        {
            return Err(ExecutionFailure::StaleReview);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "execution_identity_tests.rs"]
mod tests;
