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
        if Self::mode(previous) != mode {
            self.0.store(
                (previous.wrapping_add(4) & !3) | rank(mode),
                Ordering::Release,
            );
        }
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
        if let Some(live) = &self.live_access {
            policy.dispatch_access = Some(live.snapshot());
        }
        policy
    }
}
