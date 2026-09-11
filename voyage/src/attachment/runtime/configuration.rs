use super::*;
impl ManagedSessionOwner {
    pub(crate) async fn check_access_revision(&self, expected: u64) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.check_access_revision(&store.guard, expected)
        })
        .await?
    }

    pub(crate) async fn retain_initial_configuration(
        &self,
        config: &crate::Config,
        workspace: &std::path::Path,
    ) -> anyhow::Result<()> {
        if self.saved_configuration().await?.is_some() {
            return Ok(());
        }
        let settings = serde_json::to_string(&crate::launch_config::LaunchConfig::capture(
            config, workspace,
        )?)?;
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.retain_initial_configuration(guard, settings)
        })
        .await?
    }

    pub(crate) async fn materialize_account_configuration(
        &self,
        account: &voyage_protocol::accounts::AccountBinding,
    ) -> anyhow::Result<()> {
        let previous = self
            .saved_configuration()
            .await?
            .context("initial configuration missing")?;
        let mut value: serde_json::Value = serde_json::from_str(&previous)?;
        ensure!(
            value["config"]["account"].is_null(),
            "account already selected"
        );
        value["config"]["account"] = serde_json::to_value(account)?;
        let settings = serde_json::to_string(&value)?;
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.materialize_account_configuration(guard, &previous, &settings)
        })
        .await?
    }

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
