use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
impl ManagedSessionOwner {
    pub(crate) async fn relinquish(
        &self,
        command: RuntimeCommand,
    ) -> anyhow::Result<(Value, Vec<u8>)> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            anyhow::ensure!(
                store.turn.upgrade().is_none(),
                "runtime callbacks still owned"
            );
            let Store { journal, guard, .. } = &mut *store;
            journal.relinquish(guard, &command, SystemClock.now_ms()?)
        })
        .await?
    }
}
