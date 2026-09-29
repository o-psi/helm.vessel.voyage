use super::super::journal::GoalMeasurement;
use super::*;
use crate::provider::goal_meter::{GoalMeter, Observer, RequestObservation};
use serde_json::Value;

struct UsageObserver {
    owner: ManagedSessionOwner,
    command: Uuid,
    incarnation: Uuid,
}
impl std::fmt::Debug for UsageObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GoalUsageObserver")
    }
}
#[async_trait]
impl Observer for UsageObserver {
    async fn record(&self, observed: RequestObservation) -> anyhow::Result<()> {
        let shared = self.owner.store.clone();
        let command = self.command;
        let incarnation = self.incarnation;
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.record_goal_request(guard, command, incarnation, observed)
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn reserve_goal_turn(
        &self,
        revision: u64,
        incarnation: Uuid,
        prompt: String,
    ) -> anyhow::Result<super::super::journal::GoalTurnReservation> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.reserve_goal_turn(guard, revision, incarnation, prompt, SystemClock.now_ms()?)
        })
        .await?
    }

    pub(crate) async fn begin_execution_meter(
        &self,
        command: Uuid,
        incarnation: Uuid,
        delegated: Option<voyage_protocol::execution_budget::ExecutionBudget>,
    ) -> anyhow::Result<Option<Arc<GoalMeter>>> {
        let shared = self.store.clone();
        let budget = tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            match delegated {
                Some(budget) => {
                    anyhow::ensure!(
                        budget.valid_for(command),
                        "delegated command identity mismatch"
                    );
                    journal
                        .begin_delegated_meter(guard, &budget, incarnation, SystemClock.now_ms()?)
                        .map(Some)
                }
                None => {
                    journal.begin_goal_meter(guard, command, incarnation, SystemClock.now_ms()?)
                }
            }
        })
        .await??;
        Ok(budget.map(|(tokens, time)| {
            GoalMeter::with_observer(
                tokens,
                std::time::Duration::from_millis(time),
                Some(Arc::new(UsageObserver {
                    owner: self.clone(),
                    command,
                    incarnation,
                })),
            )
        }))
    }

    pub(crate) async fn settle_metered_run(
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
            if let Some(receipt) = journal.settle_delegated_run(
                guard,
                run,
                measurement.clone(),
                cleanup_observed,
                SystemClock.now_ms()?,
            )? {
                return Ok(Some(serde_json::to_value(receipt)?));
            }
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
            journal.recover_goal_turn(guard, SystemClock.now_ms()?)?;
            journal.recover_delegated_meter(guard, SystemClock.now_ms()?)
        })
        .await?
    }
}
