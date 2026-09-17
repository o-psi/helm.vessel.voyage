//! Access changes affect new admissions, not already-started external effects.
use super::{AccessMode, Policy};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub struct LiveAccess(AtomicU64);
impl LiveAccess {
    pub(crate) fn new(mode: AccessMode) -> Self {
        Self(AtomicU64::new(rank(mode)))
    }
    pub(crate) fn snapshot(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }
    pub(crate) fn mode(value: u64) -> AccessMode {
        match value & 3 {
            0 => AccessMode::ReadOnly,
            1 => AccessMode::Approval,
            _ => AccessMode::Unrestricted,
        }
    }
    /// Only the serialized, authenticated configuration handler publishes updates.
    pub(crate) fn update(&self, mode: AccessMode) {
        let previous = self.snapshot();
        // Explicit access-setting also revokes current-run filesystem grants.
        self.0.store(
            (previous.wrapping_add(4) & !3) | rank(mode),
            Ordering::Release,
        );
    }
}
fn rank(mode: AccessMode) -> u64 {
    match mode {
        AccessMode::ReadOnly => 0,
        AccessMode::Approval => 1,
        AccessMode::Unrestricted => 2,
    }
}
pub(super) fn restrict(a: AccessMode, b: AccessMode) -> AccessMode {
    if rank(a) < rank(b) { a } else { b }
}
impl Policy {
    pub(crate) fn access_binding(&self) -> Option<(Arc<LiveAccess>, u64)> {
        self.live_access.as_ref().map(|live| {
            (
                live.clone(),
                self.dispatch_access.unwrap_or_else(|| live.snapshot()),
            )
        })
    }
    pub(crate) fn with_live_access(mut self, access: Option<Arc<LiveAccess>>) -> Self {
        self.live_access = access;
        self
    }
    /// Freeze access for a tool admission and fence approvals against later changes.
    pub(crate) fn for_dispatch(&self) -> Self {
        let mut policy = self.clone();
        // Re-dispatching a captured policy must never retain a revoked overlay.
        policy.readable = self.snapshot.effective.rules().read_roots.clone();
        policy.writable = self.snapshot.effective.rules().write_roots.clone();
        policy.dispatch_sandbox = None;
        policy.dispatch_roots = None;
        policy.dispatch_error = None;
        if let Some(live) = &self.live_access {
            policy.dispatch_access = Some(live.snapshot());
        }
        if let Err(error) = policy.snapshot_roots() {
            policy.dispatch_error = Some(error.to_string());
        }
        policy
    }
}
