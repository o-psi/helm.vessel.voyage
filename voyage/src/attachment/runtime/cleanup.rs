use super::*;
impl ManagedRunCheckpoint {
    pub(crate) async fn local_cancel_requested(&self) -> anyhow::Result<bool> {
        let shared = self.store.clone();
        let token = self.token.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(store.run_id == token.run_id, "cancellation run changed");
            store
                .journal
                .local_cancel_requested(store.session_id, token.run_id)
        })
        .await?
    }
    pub(crate) fn cleanup_exclusive(&self, owner_references: usize) -> bool {
        // One retained cleanup checkpoint; optionally its still-borrowed RunOwner.
        Arc::strong_count(&self.token) == owner_references
    }
    pub(crate) async fn cleanup_progress(
        &self,
        value: serde_json::Value,
    ) -> Result<(), CheckpointError> {
        self.storage(move |store| {
            store
                .journal
                .record_cleanup_progress(&store.guard, store.run_id, value)
        })
        .await
    }
    pub(crate) async fn finish_cleanup(
        &self,
        owner_references: usize,
    ) -> Result<(), CheckpointError> {
        if !self.cleanup_exclusive(owner_references) {
            return Err(CheckpointError);
        }
        self.storage(|store| {
            let run = store.journal.run(store.run_id)?;
            if matches!(run.state, RunState::Accepted | RunState::Running) {
                store.journal.finish(
                    &store.guard,
                    store.run_id,
                    RunState::Interrupted,
                    Some("run interrupted before finalization"),
                    None,
                )?;
            }
            store
                .journal
                .confirm_local_cleanup_observed(&store.guard, store.run_id)
        })
        .await
    }
}
