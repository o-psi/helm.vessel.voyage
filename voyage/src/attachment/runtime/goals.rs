use super::super::journal::GoalMeasurement;
use super::*;
use serde_json::Value;

impl ManagedSessionOwner {
    pub(crate) async fn settle_goal_run(
        &self,
        run: Uuid,
        measurement: Option<GoalMeasurement>,
        cleanup_observed: bool,
    ) -> anyhow::Result<Option<Value>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.settle_goal_run(
                guard,
                run,
                measurement,
                cleanup_observed,
                SystemClock.now_ms()?,
            )
        })
        .await?
    }

    pub(crate) async fn recover_goal_turn(&self) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.recover_goal_turn(guard, SystemClock.now_ms()?)
        })
        .await?
    }
}
