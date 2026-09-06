//! Shared bounded adapters; the HTTP caller authenticates operator administration.
use super::*;
use crate::enrollment::control::Peer;
use voyage_protocol::control::{Address, ClientOperation, Lease, Mutation, Receipt, Record, Reply};
impl EnrollmentApi {
    pub fn enable_control(&mut self) -> Result<(), EnrollmentError> {
        self.control_generation = Some(
            self.store
                .lock()
                .map_err(|_| EnrollmentError::Storage)?
                .enable_control()?,
        );
        Ok(())
    }
    pub fn control_enabled(&self) -> bool {
        self.control_generation.is_some()
    }
    async fn control_operation<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut EnrollmentStore, u64) -> Result<T, EnrollmentError> + Send + 'static,
    ) -> Result<T, EnrollmentError> {
        let generation = self.control_generation.ok_or(EnrollmentError::Denied)?;
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| EnrollmentError::Busy)?;
        let store = self.store.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = store.lock().map_err(|_| EnrollmentError::Storage)?;
            f(&mut store, generation)
        });
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .map_err(|_| EnrollmentError::Busy)?
            .map_err(|_| EnrollmentError::Storage)?
    }
    pub(crate) async fn control_peer(
        &self,
        peer: Peer,
        operation: ClientOperation,
    ) -> Result<Reply, EnrollmentError> {
        self.control_operation(move |store, g| store.control_peer(g, &peer, &operation, clock))
            .await
    }
    /// Caller must authenticate the operator and enforce the canonical HTTP origin.
    pub async fn control_mutate(&self, request: Mutation) -> Result<Receipt, EnrollmentError> {
        self.control_operation(move |store, g| store.control_mutate(g, &request, clock))
            .await
    }
    /// Caller must authenticate operator metadata access. Filter returned lease
    /// observations against current live transport before publishing to a client.
    pub async fn control_inspect(
        &self,
        address: Address,
    ) -> Result<(Record, Vec<Lease>), EnrollmentError> {
        self.control_operation(move |store, g| store.control_inspect(g, address, clock))
            .await
    }
}
fn clock() -> Result<i64, EnrollmentError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| EnrollmentError::Invalid)?;
    i64::try_from(elapsed.as_millis()).map_err(|_| EnrollmentError::Invalid)
}
