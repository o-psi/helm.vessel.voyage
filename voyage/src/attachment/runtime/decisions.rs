use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
impl ManagedSessionOwner {
    pub(crate) async fn dismiss_approval(&self, id: Uuid) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.dismiss_approval(guard, id)
        })
        .await?
    }

    pub(crate) async fn create_decision(
        &self,
        run: Uuid,
        incarnation: Uuid,
        id: Uuid,
        expires: i64,
        request: Value,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.create_decision(guard, run, incarnation, id, expires, request)
        })
        .await?
    }
    pub(crate) async fn decisions(&self, incarnation: Uuid) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.decisions(incarnation, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn decision_response(&self, id: Uuid) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.decision_response(id, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn respond_decision(
        &self,
        incarnation: Uuid,
        command: RuntimeCommand,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.respond_decision(guard, incarnation, &command, SystemClock.now_ms()?)
        })
        .await?
    }
}
