//! Allocation amounts are reserved before durable intent and before remote effects.
use super::*;
use anyhow::{Context, ensure};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub(crate) struct AllocationRequest {
    pub command_id: Uuid,
    pub destination: Uuid,
    pub session_id: Uuid,
    pub tokens: u64,
    pub elapsed_ms: u64,
    pub expires_at_ms: u64,
}
#[derive(Debug)]
pub(super) struct Allocation {
    pub destination: Uuid,
    pub budget: ExecutionBudget,
    pub receipt: Option<ExecutionUsage>,
    pub observer_started: bool,
}

impl GoalMeter {
    pub(crate) async fn allocate(
        &self,
        command_id: Uuid,
        destination: Uuid,
        session_id: Uuid,
    ) -> anyhow::Result<ExecutionBudget> {
        let _serial = self.delegation_lock.lock().await;
        ensure!(
            !command_id.is_nil() && !destination.is_nil() && !session_id.is_nil(),
            "invalid Goal allocation identity"
        );
        let observer = self
            .observer
            .as_ref()
            .context("Goal allocation requires durable accounting")?;
        let tokens = {
            let mut t = self
                .totals
                .lock()
                .map_err(|_| anyhow::anyhow!("Goal usage unavailable"))?;
            if let Some(prior) = t.allocations.get(&command_id) {
                ensure!(
                    prior.destination == destination && prior.budget.session_id == session_id,
                    "Goal allocation destination conflict"
                );
                return Ok(prior.budget.clone());
            }
            ensure!(
                !t.uncertain && !t.cleanup_unobserved() && t.allocations.len() < 128,
                "Goal allocations are unavailable or exhausted"
            );
            let used = t
                .input
                .checked_add(t.output)
                .and_then(|v| v.checked_add(t.reserved))
                .context("Goal usage overflow")?;
            let tokens = self.token_allowance.saturating_sub(used) / 2;
            ensure!(
                tokens > 0 && !self.remaining_time().is_zero(),
                "Goal budget cannot allocate child work"
            );
            t.reserved = t
                .reserved
                .checked_add(tokens)
                .context("Goal allocation overflow")?;
            tokens
        };
        // A dropped future retains the reservation. The parent cannot certify
        // missing child usage as zero, even if durable publication was interrupted.
        let elapsed_ms = u64::try_from(self.remaining_time().as_millis())?;
        let now = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis(),
        )?;
        let request = AllocationRequest {
            command_id,
            destination,
            session_id,
            tokens,
            elapsed_ms,
            expires_at_ms: now
                .checked_add(elapsed_ms)
                .context("Goal deadline overflow")?,
        };
        let budget = match observer.allocate(request.clone()).await {
            Ok(budget) => budget,
            Err(error) => {
                self.mark_uncertain();
                return Err(error);
            }
        };
        if !budget.valid_for(command_id)
            || budget.session_id != session_id
            || budget.tokens > tokens
            || budget.elapsed_ms > elapsed_ms
            || budget.expires_at_ms > request.expires_at_ms
        {
            self.mark_uncertain();
            anyhow::bail!("durable Goal allocation widened its bounds");
        }
        let mut t = self
            .totals
            .lock()
            .map_err(|_| anyhow::anyhow!("Goal usage unavailable"))?;
        t.reserved -= tokens - budget.tokens;
        t.allocations.insert(
            command_id,
            Allocation {
                destination,
                budget: budget.clone(),
                receipt: None,
                observer_started: false,
            },
        );
        Ok(budget)
    }

    pub(crate) async fn settle_allocation(
        &self,
        destination: Uuid,
        usage: ExecutionUsage,
    ) -> anyhow::Result<()> {
        let _serial = self.delegation_lock.lock().await;
        {
            let t = self
                .totals
                .lock()
                .map_err(|_| anyhow::anyhow!("Goal usage unavailable"))?;
            let saved = t
                .allocations
                .get(&usage.budget.command_id)
                .context("unknown Goal allocation")?;
            ensure!(
                saved.destination == destination
                    && saved.budget == usage.budget
                    && usage.session_id == saved.budget.session_id
                    && !usage.run_id.is_nil(),
                "Goal child accounting identity mismatch"
            );
            if let Some(prior) = &saved.receipt {
                ensure!(prior == &usage, "Goal child receipt changed");
                return Ok(());
            }
        }
        let observer = self
            .observer
            .as_ref()
            .context("Goal allocation requires durable accounting")?;
        if let Err(error) = observer.settle_allocation(destination, usage.clone()).await {
            self.mark_uncertain();
            return Err(error);
        }
        let mut t = self
            .totals
            .lock()
            .map_err(|_| anyhow::anyhow!("Goal usage unavailable"))?;
        let input = t.input.checked_add(usage.input_tokens);
        let output = t.output.checked_add(usage.output_tokens);
        let (Some(input), Some(output)) = (input, output) else {
            t.uncertain = true;
            anyhow::bail!("Goal aggregate usage overflow");
        };
        t.input = input;
        t.output = output;
        t.reserved = t
            .reserved
            .checked_sub(usage.budget.tokens)
            .context("Goal allocation reservation mismatch")?;
        t.uncertain |= !usage.complete;
        let command_id = usage.budget.command_id;
        t.allocations
            .get_mut(&command_id)
            .context("Goal allocation disappeared")?
            .receipt = Some(usage);
        self.allocation_changed.notify_waiters();
        Ok(())
    }

    pub(crate) fn observe_allocation_once(&self, command: Uuid) -> bool {
        let Ok(mut t) = self.totals.lock() else {
            return false;
        };
        let Some(allocation) = t.allocations.get_mut(&command) else {
            return false;
        };
        if allocation.observer_started {
            return false;
        }
        allocation.observer_started = true;
        true
    }

    pub(crate) async fn wait_allocations(&self, cancel: tokio_util::sync::CancellationToken) {
        let deadline = tokio::time::Instant::now() + self.remaining_time() + Duration::from_secs(5);
        loop {
            let notified = self.allocation_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .totals
                .lock()
                .map_or(true, |t| t.reserved == 0 || t.uncertain)
            {
                return;
            }
            tokio::select! {_=notified=>{},_=cancel.cancelled()=>return,_=tokio::time::sleep_until(deadline)=>return}
        }
    }

    fn mark_uncertain(&self) {
        if let Ok(mut t) = self.totals.lock() {
            t.uncertain = true;
        }
        self.allocation_changed.notify_waiters();
    }
}
