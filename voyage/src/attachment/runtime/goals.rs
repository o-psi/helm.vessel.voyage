use super::super::journal::GoalMeasurement;
use super::*;
use crate::provider::goal_meter::AllocationRequest;
use crate::provider::goal_meter::{GoalMeter, Observer, RequestObservation};
use serde_json::Value;
use voyage_protocol::execution_budget::{ExecutionBudget, ExecutionUsage};

struct UsageObserver {
    owner: ManagedSessionOwner,
    command: Uuid,
    incarnation: Uuid,
}

#[cfg(test)]
#[path = "goal_family_tests.rs"]
mod family_tests;
impl std::fmt::Debug for UsageObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GoalUsageObserver")
    }
}
#[async_trait]
impl Observer for UsageObserver {
    async fn close_allocation(
        &self,
        destination: Uuid,
        id: Uuid,
        proof: Value,
    ) -> anyhow::Result<()> {
        self.owner
            .close_goal_allocation(destination, id, Some(proof))
            .await
            .map(|_| ())
    }
    async fn dispatch(
        &self,
        dispatch: crate::provider::goal_meter::AllocationDispatch,
    ) -> anyhow::Result<()> {
        let shared = self.owner.store.clone();
        let command = self.command;
        let incarnation = self.incarnation;
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.dispatch_goal_child(guard, command, incarnation, dispatch)
        })
        .await?
    }
    async fn allocate(&self, request: AllocationRequest) -> anyhow::Result<ExecutionBudget> {
        let shared = self.owner.store.clone();
        let command = self.command;
        let incarnation = self.incarnation;
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.allocate_goal_child(guard, command, incarnation, request, SystemClock.now_ms()?)
        })
        .await?
    }
    async fn settle_allocation(
        &self,
        destination: Uuid,
        usage: ExecutionUsage,
    ) -> anyhow::Result<()> {
        let shared = self.owner.store.clone();
        let command = self.command;
        let incarnation = self.incarnation;
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            match journal.settle_goal_allocation(
                guard,
                command,
                incarnation,
                destination,
                usage.clone(),
            ) {
                Ok(()) => Ok(()),
                Err(_) => journal
                    .reconcile_goal_allocation(
                        guard,
                        destination,
                        usage,
                        None,
                        SystemClock.now_ms()?,
                    )
                    .map(|_| ()),
            }
        })
        .await?
    }
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

    pub(crate) async fn fence_offline_goal(&self) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.fence_offline_goal(guard)
        })
        .await?
    }

    pub(crate) async fn recover_goal_turn(&self) -> anyhow::Result<()> {
        let shared = self.store.clone();
        Journal::blocking_checkpoint(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.recover_goal_turn(guard, SystemClock.now_ms()?)?;
            journal.recover_delegated_meter(guard, SystemClock.now_ms()?)
        })
        .await
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn goal_allocations(
        &self,
        offset: u64,
        limit: u32,
    ) -> anyhow::Result<Vec<super::super::journal::GoalAllocation>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store
                .journal
                .goal_allocations(store.session_id, offset, limit)
        })
        .await?
    }
    pub(crate) async fn reconcile_goal_allocation(
        &self,
        destination: Uuid,
        receipt: ExecutionUsage,
        observed: Option<ExecutionUsage>,
    ) -> anyhow::Result<bool> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.reconcile_goal_allocation(
                guard,
                destination,
                receipt,
                observed,
                SystemClock.now_ms()?,
            )
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn close_goal_allocation(
        &self,
        destination: Uuid,
        id: Uuid,
        negative: Option<Value>,
    ) -> anyhow::Result<bool> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.close_goal_allocation(guard, destination, id, negative)
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn goal_continuation(
        &self,
    ) -> anyhow::Result<
        Option<(
            voyage_protocol::goals::GoalSnapshot,
            super::super::journal::GoalAuthority,
        )>,
    > {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.goal_continuation(store.session_id)
        })
        .await?
    }
    pub(crate) async fn goal_obstruction(
        &self,
    ) -> anyhow::Result<Option<voyage_protocol::goals::GoalStopReason>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.goal_obstruction(store.session_id)
        })
        .await?
    }
    pub(crate) async fn stop_goal(
        &self,
        revision: u64,
        reason: voyage_protocol::goals::GoalStopReason,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.stop_goal(guard, revision, reason, SystemClock.now_ms()?)
        })
        .await?
    }
    pub(crate) async fn abandon_goal_turn(
        &self,
        command: Uuid,
        reason: voyage_protocol::goals::GoalStopReason,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.abandon_goal_turn(guard, command, reason, SystemClock.now_ms()?)
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn goal_tool(
        &self,
        run: Uuid,
        control: Option<crate::tools::goal::GoalControl>,
        meter: Option<Arc<GoalMeter>>,
    ) -> anyhow::Result<Option<Arc<dyn crate::tools::Tool>>> {
        let shared = self.store.clone();
        let binding = tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.goal_report_context(store.session_id, run)
        })
        .await??;
        Ok((binding.is_some() || control.is_some()).then(|| {
            Arc::new(crate::tools::goal::GoalTool {
                owner: self.clone(),
                run,
                binding,
                control,
                meter,
            }) as Arc<dyn crate::tools::Tool>
        }))
    }
    pub(crate) async fn goal_model_read(
        &self,
        binding: super::super::journal::GoalReportContext,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.goal_model_read(store.session_id, &binding)
        })
        .await?
    }
    pub(crate) async fn report_goal(
        &self,
        binding: super::super::journal::GoalReportContext,
        call: String,
        report: voyage_protocol::goals::GoalReport,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.report_goal(guard, &binding, &call, &report)
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn model_create_goal(
        &self,
        run: Uuid,
        control: crate::tools::goal::GoalControl,
        call: String,
        objective: String,
        tokens: Option<u64>,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.model_create_goal(
                guard,
                run,
                control.authority,
                control.incarnation,
                &call,
                &objective,
                tokens,
                SystemClock.now_ms()?,
            )
        })
        .await?
    }
    pub(crate) async fn model_goal_binding(
        &self,
        run: Uuid,
    ) -> anyhow::Result<Option<super::super::journal::GoalReportContext>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            store.journal.goal_report_context(store.session_id, run)
        })
        .await?
    }
}

impl ManagedSessionOwner {
    pub(crate) async fn model_edit_goal(
        &self,
        run: Uuid,
        control: crate::tools::goal::GoalControl,
        call: String,
        objective: String,
    ) -> anyhow::Result<Value> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("owner poisoned"))?;
            let Store { journal, guard, .. } = &mut *store;
            journal.model_edit_goal(
                guard,
                run,
                control.authority,
                &call,
                &objective,
                SystemClock.now_ms()?,
            )
        })
        .await?
    }
}
