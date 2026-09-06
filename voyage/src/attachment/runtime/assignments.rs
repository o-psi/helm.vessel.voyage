use super::*;
use serde_json::Value;
use voyage_protocol::process::{AssignmentObservation, AssignmentRequest};
impl ManagedSessionOwner {
    pub async fn record_assignment(
        &self,
        actor: Uuid,
        participant: Uuid,
        request: AssignmentRequest,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.record_assignment(guard, actor, participant, &request)
        })
        .await?
    }
    pub async fn update_assignment(
        &self,
        observation: AssignmentObservation,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.update_assignment(guard, &observation)
        })
        .await?
    }
    pub async fn assignment_observations(&self, run: Uuid) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            anyhow::ensure!(
                store.journal.run(run)?.session_id == store.session_id,
                "assignment run belongs to another session"
            );
            store.journal.assignment_observations(run)
        })
        .await?
    }
    pub async fn assignment_result(&self, run: Uuid, id: Uuid) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            anyhow::ensure!(
                store.journal.run(run)?.session_id == store.session_id,
                "assignment run belongs to another session"
            );
            store.journal.assignment_result(run, id)
        })
        .await?
    }
    pub async fn assignment_request(
        &self,
        run: Uuid,
        id: Uuid,
    ) -> anyhow::Result<Option<AssignmentRequest>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            anyhow::ensure!(
                store.journal.run(run)?.session_id == store.session_id,
                "assignment run belongs to another session"
            );
            store.journal.assignment_request(run, id)
        })
        .await?
    }
}
