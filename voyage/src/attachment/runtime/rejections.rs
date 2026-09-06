use super::*;
use serde_json::Value;
impl ManagedSessionOwner {
    pub(crate) async fn reject_unadmitted(
        &self,
        id: Uuid,
        principal: Uuid,
        reason: String,
    ) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.reject_unadmitted(guard, id, principal, &reason)
        })
        .await?
    }
}
