use super::*;
use serde_json::Value;
impl ManagedSessionOwner {
    pub(crate) async fn observations(&self, after: u64, limit: u32) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.observations(store.session_id, after, limit)
        })
        .await?
    }
}
