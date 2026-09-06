use super::*;
use voyage_protocol::process::RuntimeCommand;
impl ManagedSessionOwner {
    pub(crate) async fn initialize_command_bindings(&self, principal: Uuid) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.initialize_command_bindings(guard, principal)
        })
        .await?
    }
    pub(crate) async fn bind_process_command(
        &self,
        id: Uuid,
        principal: Uuid,
        command: RuntimeCommand,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.bind_process_command(guard, id, principal, &command)
        })
        .await?
    }
}
