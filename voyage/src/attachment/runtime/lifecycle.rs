use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
impl ManagedSessionOwner {
    pub(crate) async fn apply_lifecycle(
        &self,
        command: RuntimeCommand,
        configuration: Option<String>,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            anyhow::ensure!(
                store.turn.upgrade().is_none(),
                "run callbacks or cleanup still owned"
            );
            let Store { journal, guard, .. } = &mut *store;
            let receipt = journal.apply_lifecycle(
                guard,
                &command,
                SystemClock.now_ms()?,
                configuration.as_deref(),
            )?;
            if let RuntimeCommand::Delete { command_id, .. } = command {
                journal.finish_deletion(guard, command_id)?;
                Ok(journal.process_receipt(command_id)?.unwrap_or(receipt))
            } else {
                Ok(receipt)
            }
        })
        .await?
    }
}
