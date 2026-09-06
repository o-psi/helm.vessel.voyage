use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
impl RunOwner {
    pub(crate) async fn bind_workflow(
        &mut self,
        invocation: crate::workflow::Invocation,
        secrets: crate::workflow::secrets::SecretInputs,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(self.workflow_bindings.is_none(), "workflow already bound");
        self.storage(move |store| {
            store
                .journal
                .record_workflow(&store.guard, store.run_id, invocation)
        })
        .await?;
        self.workflow_bindings = Some(secrets.bind(self.run_id)?);
        Ok(())
    }
}
impl ManagedSessionOwner {
    pub(crate) async fn control_receipt(
        &self,
        command: RuntimeCommand,
    ) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.control_receipt(&command)
        })
        .await?
    }
    pub(crate) async fn admit_control(
        &self,
        command: RuntimeCommand,
    ) -> anyhow::Result<(Value, bool)> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.admit_control(guard, command, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn complete_control(&self, id: Uuid, outcome: Value) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.complete_control(guard, id, outcome)
        })
        .await?
    }
}
