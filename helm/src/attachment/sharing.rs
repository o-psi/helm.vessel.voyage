//! Current, in-memory single-owner sharing authority. No authentication or durable
//! storage is supplied here. Trusted local control-plane updates are separate from
//! authenticated operation checks. Never cache an authorization boolean: recheck
//! under the actual dispatch/publication commit fence. This registry cannot fence
//! another process or a disk transaction; those integrations remain required.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::RwLock};
use uuid::Uuid;
use voyage_protocol::attachment::Capability;

const MAX_POLICIES: usize = 4096;
const MAX_REVISION: u64 = i64::MAX as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Disclosure {
    None,
    Metadata,
    Live,
    Transcript,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    ViewMetadata,
    ViewLive,
    ViewHistory,
    Submit,
    Cancel,
    Rename,
    ChangeModel,
    Branch,
    Archive,
    Delete,
    ChangeSharing,
    ApproveWrite,
    ApproveCommand,
}

// Bound retained grants to the typed capability vocabulary even if a trusted
// configuration repeats entries. Duplicates never amplify or imply permissions.
fn unique_capabilities(values: &[Capability]) -> Vec<Capability> {
    let mut capabilities = Vec::new();
    for capability in values {
        if !capabilities.contains(capability) {
            capabilities.push(*capability);
        }
    }
    capabilities
}

/// Trusted installation identity. An epoch change invalidates earlier policies
/// and grants. No Deserialize: request identity claims are not authentication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    machine: Uuid,
    owner: Uuid,
    epoch: u64,
}
impl Scope {
    pub fn new(machine: Uuid, owner: Uuid, epoch: u64) -> Option<Self> {
        (!machine.is_nil() && !owner.is_nil() && (1..MAX_REVISION).contains(&epoch)).then_some(
            Self {
                machine,
                owner,
                epoch,
            },
        )
    }
}

/// Construct only after verifying authentication and the granted capabilities.
/// These are principal rights, not the installation's delegation or local policy.
#[derive(Clone, Debug)]
pub struct AuthenticatedPrincipal {
    scope: Scope,
    principal_id: Uuid,
    capabilities: Vec<Capability>,
}
impl AuthenticatedPrincipal {
    pub fn new(scope: Scope, principal_id: Uuid, capabilities: &[Capability]) -> Option<Self> {
        (!principal_id.is_nil()).then(|| Self {
            scope,
            principal_id,
            capabilities: unique_capabilities(capabilities),
        })
    }
}

/// Trusted attribution from the current installation's canonical run journal,
/// never caller-supplied IDs. Even CancelAny requires a real matching run target.
#[derive(Clone, Copy, Debug)]
pub struct RunAttribution {
    session_id: Uuid,
    run_id: Uuid,
    principal_id: Uuid,
}
impl RunAttribution {
    pub fn new(session_id: Uuid, run_id: Uuid, principal_id: Uuid) -> Option<Self> {
        (![session_id, run_id, principal_id].iter().any(Uuid::is_nil)).then_some(Self {
            session_id,
            run_id,
            principal_id,
        })
    }
}

/// Trusted local consent configuration. Remote changes require independently
/// authorized and confirmed lifecycle handling, not a direct call to update_local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharingSettings {
    pub disclosure: Disclosure,
    pub archived: bool,
    pub approve_write: bool,
    pub approve_command: bool,
}
impl SharingSettings {
    pub fn new(disclosure: Disclosure, archived: bool) -> Self {
        Self {
            disclosure,
            archived,
            approve_write: false,
            approve_command: false,
        }
    }
    pub fn with_approvals(mut self, write: bool, command: bool) -> Self {
        self.approve_write = write;
        self.approve_command = command;
        self
    }
    fn permits(self, action: Action) -> bool {
        if self.disclosure == Disclosure::None {
            return false;
        }
        if action == Action::ViewMetadata {
            return true;
        }
        if self.disclosure == Disclosure::Metadata {
            return false;
        }
        if self.archived {
            return matches!(
                action,
                Action::Archive | Action::Delete | Action::ChangeSharing
            ) || (action == Action::ViewHistory
                && self.disclosure == Disclosure::Transcript);
        }
        match action {
            Action::ViewHistory | Action::Branch => self.disclosure == Disclosure::Transcript,
            Action::ApproveWrite => self.approve_write,
            Action::ApproveCommand => self.approve_command,
            Action::ViewMetadata
            | Action::ViewLive
            | Action::Submit
            | Action::Cancel
            | Action::Rename
            | Action::ChangeModel
            | Action::Archive
            | Action::Delete
            | Action::ChangeSharing => true,
        }
    }
}

/// Local observation only. A copied snapshot has no authorize/project/branch API
/// and is never an authority for a later operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub session_id: Uuid,
    pub revision: u64,
    pub settings: SharingSettings,
}
struct SessionPolicy {
    scope: Scope,
    snapshot: PolicySnapshot,
}
struct State {
    scope: Scope,
    capabilities: Vec<Capability>,
    sessions: BTreeMap<Uuid, SessionPolicy>,
}
/// Every check reads this owner's current policy. Share the owner with Arc, not
/// policy copies. Locks cover in-memory checks/updates only, not caller effects.
pub struct SharingRegistry {
    state: RwLock<State>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SharingError {
    #[error("sharing operation denied")]
    Denied,
    #[error("sharing revision or identity conflict")]
    Conflict,
    #[error("invalid sharing identity or revision")]
    Invalid,
    #[error("sharing capacity exhausted")]
    Capacity,
    #[error("sharing authority unavailable")]
    Unavailable,
}

/// No owner identity, approval opt-ins, credentials, provider state or history.
/// This is data, not an authorization token. Recheck before delayed publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Projection {
    session_id: Uuid,
    disclosure: Disclosure,
    archived: bool,
}
impl SharingRegistry {
    pub fn new(scope: Scope, installation_capabilities: &[Capability]) -> Self {
        Self {
            state: RwLock::new(State {
                scope,
                capabilities: unique_capabilities(installation_capabilities),
                sessions: BTreeMap::new(),
            }),
        }
    }

    /// Trusted-local create-only consent; never a remotely exposed handler.
    pub fn register_local(
        &self,
        session_id: Uuid,
        settings: SharingSettings,
    ) -> Result<PolicySnapshot, SharingError> {
        if session_id.is_nil() {
            return Err(SharingError::Invalid);
        }
        let mut state = self.state.write().map_err(|_| SharingError::Unavailable)?;
        if state.sessions.contains_key(&session_id) {
            return Err(SharingError::Conflict);
        }
        if state.sessions.len() >= MAX_POLICIES {
            return Err(SharingError::Capacity);
        }
        let snapshot = PolicySnapshot {
            session_id,
            revision: 1,
            settings,
        };
        let scope = state.scope;
        state.sessions.insert(
            session_id,
            SessionPolicy {
                scope,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }

    /// Atomic in-memory CAS. Persisted policy changes need the session
    /// coordinator's transaction; this method is not a durable acknowledgement.
    pub fn update_local(
        &self,
        session_id: Uuid,
        expected_revision: u64,
        settings: SharingSettings,
    ) -> Result<PolicySnapshot, SharingError> {
        let mut state = self.state.write().map_err(|_| SharingError::Unavailable)?;
        let scope = state.scope;
        let policy = state
            .sessions
            .get_mut(&session_id)
            .ok_or(SharingError::Conflict)?;
        if policy.snapshot.revision != expected_revision {
            return Err(SharingError::Conflict);
        }
        let revision = expected_revision
            .checked_add(1)
            .filter(|n| *n <= MAX_REVISION)
            .ok_or(SharingError::Invalid)?;
        let snapshot = PolicySnapshot {
            session_id,
            revision,
            settings,
        };
        policy.snapshot = snapshot.clone();
        policy.scope = scope;
        Ok(snapshot)
    }

    /// Advance current installation authority and invalidate all old grants and
    /// policies. Explicit local policy updates re-consent at the new epoch.
    pub fn advance_epoch_local(
        &self,
        expected_epoch: u64,
        epoch: u64,
        capabilities: &[Capability],
    ) -> Result<(), SharingError> {
        let mut state = self.state.write().map_err(|_| SharingError::Unavailable)?;
        if state.scope.epoch != expected_epoch {
            return Err(SharingError::Conflict);
        }
        if epoch <= expected_epoch || epoch >= MAX_REVISION {
            return Err(SharingError::Invalid);
        }
        state.scope.epoch = epoch;
        state.capabilities = unique_capabilities(capabilities);
        Ok(())
    }

    /// Current policy + installation delegation + authenticated capability check.
    /// A true result is not a reusable permit. Recheck at dispatch/publication
    /// commitment together with current local policy, cancellation and resources.
    pub fn authorize(
        &self,
        session_id: Uuid,
        principal: &AuthenticatedPrincipal,
        action: Action,
        run: Option<&RunAttribution>,
    ) -> bool {
        self.state
            .read()
            .is_ok_and(|state| state.authorize(session_id, principal, action, run))
    }

    pub fn project(
        &self,
        session_id: Uuid,
        principal: &AuthenticatedPrincipal,
    ) -> Option<Projection> {
        let state = self.state.read().ok()?;
        if !state.authorize(session_id, principal, Action::ViewMetadata, None) {
            return None;
        }
        let policy = &state.sessions[&session_id];
        Some(Projection {
            session_id,
            disclosure: policy.snapshot.settings.disclosure,
            archived: policy.snapshot.settings.archived,
        })
    }

    /// Authorize the current source and register child consent under one lock.
    /// Source revisions, private/archive state and capabilities cannot go stale
    /// between this check and the in-memory branch insertion. Actual session
    /// copying must still join the authoritative storage transaction.
    pub fn branch(
        &self,
        source_id: Uuid,
        expected_revision: u64,
        principal: &AuthenticatedPrincipal,
        child_id: Uuid,
        disclosure: Disclosure,
    ) -> Result<PolicySnapshot, SharingError> {
        let mut state = self.state.write().map_err(|_| SharingError::Unavailable)?;
        if !state.authorize(source_id, principal, Action::Branch, None) {
            return Err(SharingError::Denied);
        }
        let source = &state.sessions[&source_id];
        if source.snapshot.revision != expected_revision {
            return Err(SharingError::Conflict);
        }
        if child_id.is_nil() || child_id == source_id {
            return Err(SharingError::Invalid);
        }
        if disclosure > source.snapshot.settings.disclosure {
            return Err(SharingError::Denied);
        }
        if state.sessions.contains_key(&child_id) {
            return Err(SharingError::Conflict);
        }
        if state.sessions.len() >= MAX_POLICIES {
            return Err(SharingError::Capacity);
        }
        let snapshot = PolicySnapshot {
            session_id: child_id,
            revision: 1,
            settings: SharingSettings::new(disclosure, false),
        };
        let scope = state.scope;
        state.sessions.insert(
            child_id,
            SessionPolicy {
                scope,
                snapshot: snapshot.clone(),
            },
        );
        Ok(snapshot)
    }
}
impl State {
    fn authorize(
        &self,
        session_id: Uuid,
        principal: &AuthenticatedPrincipal,
        action: Action,
        run: Option<&RunAttribution>,
    ) -> bool {
        let Some(policy) = self.sessions.get(&session_id) else {
            return false;
        };
        if self.scope != principal.scope
            || policy.scope != self.scope
            || !policy.snapshot.settings.permits(action)
        {
            return false;
        }
        let capability = |capability| {
            self.capabilities.contains(&capability) && principal.capabilities.contains(&capability)
        };
        match action {
            Action::ViewMetadata => capability(Capability::ViewMetadata),
            Action::ViewLive => capability(Capability::ViewLive),
            Action::ViewHistory => capability(Capability::ViewHistory),
            Action::Submit => capability(Capability::SubmitTurn),
            Action::Cancel => run.is_some_and(|run| {
                run.session_id == session_id
                    && !run.run_id.is_nil()
                    && (capability(Capability::CancelAny)
                        || (run.principal_id == principal.principal_id
                            && capability(Capability::CancelOwn)))
            }),
            Action::Rename => capability(Capability::RenameSession),
            Action::ChangeModel => capability(Capability::ChangeModel),
            Action::Branch => {
                capability(Capability::BranchSession) && capability(Capability::ViewHistory)
            }
            Action::Archive => capability(Capability::ArchiveSession),
            Action::Delete => capability(Capability::DeleteSession),
            Action::ChangeSharing => capability(Capability::ChangeSharing),
            Action::ApproveWrite => capability(Capability::ApproveWrite),
            Action::ApproveCommand => capability(Capability::ApproveCommand),
        }
    }
}

#[cfg(test)]
mod tests;

/// Private durable declarations, never an authorization adapter.
pub mod consent;
