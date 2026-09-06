use super::*;
use serde_json::Value;
impl ManagedSessionOwner {
    pub(crate) async fn initialize_session_resources(&self) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.initialize_session_resources(guard)
        })
        .await?
    }
    pub async fn session_resource_adopt(
        &self,
        id: Uuid,
        run: Uuid,
        kind: String,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.session_resource_adopt(guard, id, run, &kind)
        })
        .await?
    }
    pub async fn session_resource_closed(&self, id: Uuid) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.session_resource_closed(guard, id)
        })
        .await?
    }
    pub async fn session_resources(&self) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.session_resources(store.session_id)
        })
        .await?
    }
    pub(crate) async fn attest_session_resource(
        &self,
        id: Uuid,
        actor: super::super::local_actor::LocalActor,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.attest_session_resource(guard, id, actor)
        })
        .await?
    }
    pub(crate) async fn has_cleanup_attestation(&self) -> anyhow::Result<bool> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.has_cleanup_attestation(store.session_id)
        })
        .await?
    }
}
