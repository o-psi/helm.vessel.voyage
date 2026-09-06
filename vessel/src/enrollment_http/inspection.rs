//! Bounded shared adapters; callers authenticate operator access before invocation.
use super::*;
use voyage_protocol::enrollment::inspection::{Audit, Machines, Request};
impl EnrollmentApi {
    pub async fn inspect_machines(&self, request: Request) -> Result<Machines, EnrollmentError> {
        self.operation(move |store, now| store.inspect_machines(&request, now))
            .await
            .map_err(|e| e.0)
    }
    pub async fn inspect_audit(&self, request: Request) -> Result<Audit, EnrollmentError> {
        self.operation(move |store, now| store.inspect_audit(&request, now))
            .await
            .map_err(|e| e.0)
    }
}

impl EnrollmentApi {
    /// Admit an already-authenticated operator HTTP read before consuming its body.
    pub fn inspection_permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, EnrollmentError> {
        self.operator_requests
            .clone()
            .try_acquire_owned()
            .map_err(|_| EnrollmentError::Busy)
    }
}
