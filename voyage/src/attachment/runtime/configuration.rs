use super::*;
impl ManagedSessionOwner {
    pub(crate) async fn saved_configuration(&self) -> anyhow::Result<Option<String>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.saved_configuration(guard)
        })
        .await?
    }
    pub(crate) async fn configure(
        &self,
        command: voyage_protocol::process::RuntimeCommand,
        settings: String,
        model: String,
    ) -> anyhow::Result<serde_json::Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.configure(guard, command, settings, model, SystemClock.now_ms()?)
        })
        .await?
    }
}
