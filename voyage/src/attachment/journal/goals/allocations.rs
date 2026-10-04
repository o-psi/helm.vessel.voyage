//! Parent-owned allocation intent precedes remote effects; exact child receipts
//! enter the same aggregate ledger once. Missing observations are never refunds.
use super::*;
use crate::provider::goal_meter::{AllocationRequest, RequestObservation};
use voyage_protocol::execution_budget::{ExecutionBudget, ExecutionUsage};

pub(super) fn current(
    db: &Connection,
    session: Uuid,
    command: Uuid,
    incarnation: Uuid,
) -> Result<(Uuid, Option<ExecutionBudget>, i64)> {
    let row:Option<(String,Option<String>,i64)>=db.query_row("SELECT r.id,m.budget,m.started_at_ms FROM process_goal_meters m JOIN commands c ON c.id=m.command_id JOIN runs r ON r.id=c.run_id WHERE m.command_id=?1 AND m.incarnation=?2 AND r.session_id=?3 AND m.settlement IS NULL AND (m.budget IS NOT NULL OR EXISTS(SELECT 1 FROM process_goal_turns t WHERE t.command_id=m.command_id AND t.incarnation=m.incarnation AND t.state='reserved'))",params![command.to_string(),incarnation.to_string(),session.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (run, budget, started) = row.context("Goal allocation belongs to an inactive meter")?;
    Ok((
        Uuid::parse_str(&run)?,
        budget.map(|v| serde_json::from_str(&v)).transpose()?,
        started,
    ))
}

impl Journal {
    pub(crate) fn allocate_goal_child(
        &mut self,
        guard: &ExecutionGuard,
        command: Uuid,
        incarnation: Uuid,
        request: AllocationRequest,
        now: i64,
    ) -> Result<ExecutionBudget> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.opened_schema >= 19
                && now >= 0
                && !request.command_id.is_nil()
                && !request.destination.is_nil()
                && request.session_id != guard.session_id
                && !request.session_id.is_nil()
                && request.tokens > 0
                && request.elapsed_ms > 0,
            "invalid Goal allocation"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run, parent, started) = current(&tx, guard.session_id, command, incarnation)?;
        ensure!(
            matches!(
                read_run(&tx, run)?.state,
                RunState::Accepted | RunState::Running
            ),
            "Goal allocation requires active execution"
        );
        let (allowance, end) = if let Some(parent) = parent {
            ensure!(
                parent.valid_for(command),
                "parent delegated budget identity mismatch"
            );
            (
                parent.tokens,
                parent
                    .expires_at_ms
                    .min(u64::try_from(started)?.saturating_add(parent.elapsed_ms)),
            )
        } else {
            let goal = read(&tx, guard.session_id)?
                .goal
                .context("Goal allocation objective missing")?;
            let reserved:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_turns WHERE command_id=?1 AND goal_id=?2)",params![command.to_string(),goal.id.to_string()],|r|r.get(0))?;
            ensure!(
                reserved && (!goal.limits.usage_required() || goal.usage.unmeasured_runs == 0),
                "Goal allocation objective or accounting changed"
            );
            let used = goal
                .usage
                .input_tokens
                .checked_add(goal.usage.output_tokens)
                .context("Goal usage overflow")?;
            (
                goal.limits.token_allowance().saturating_sub(used),
                u64::try_from(started)?.saturating_add(
                    goal.limits
                        .time_allowance_ms()
                        .saturating_sub(goal.usage.elapsed_ms),
                ),
            )
        };
        let (input, output, _, _) = super::metering::retained_usage(&tx, command)?;
        let mut retained = 0u64;
        let mut count = 0usize;
        for row in tx.prepare("SELECT a.budget,a.receipt,a.closure FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) WHERE q.command_id=?1")?.query_map([command.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?)))? {
            let (budget,receipt,closure)=row?;count+=1;
            let budget:ExecutionBudget=serde_json::from_str(&budget)?;
            if let Some(receipt)=receipt {
                let receipt:ExecutionUsage=serde_json::from_str(&receipt)?;
                ensure!(receipt.complete && receipt.cleanup_observed,"Goal child usage or cleanup is unresolved");
            } else if closure.is_none() {retained=retained.checked_add(budget.tokens).context("Goal allocation total overflow")?;}
        }
        ensure!(count < 128, "Goal child allocation limit reached");
        let used = input
            .checked_add(output)
            .and_then(|v| v.checked_add(retained))
            .context("Goal aggregate usage overflow")?;
        let tokens = request.tokens.min(allowance.saturating_sub(used) / 2);
        let elapsed_ms = request.elapsed_ms.min(
            request
                .expires_at_ms
                .min(end)
                .saturating_sub(u64::try_from(now)?),
        );
        let budget = ExecutionBudget {
            command_id: request.command_id,
            session_id: request.session_id,
            parent_session_id: guard.session_id,
            parent_run_id: run,
            tokens,
            elapsed_ms,
            expires_at_ms: u64::try_from(now)?
                .checked_add(elapsed_ms)
                .context("Goal deadline overflow")?,
        };
        ensure!(
            budget.valid_for(request.command_id),
            "Goal has no finite child allowance remaining"
        );
        let requests: u64 = tx.query_row(
            "SELECT count(*) FROM process_goal_requests WHERE command_id=?1",
            [command.to_string()],
            |r| r.get(0),
        )?;
        ensure!(requests < 10_000, "Goal request journal capacity reached");
        let observation = RequestObservation {
            request_id: request.command_id,
            revision: 0,
            input_tokens: None,
            output_tokens: None,
            complete: false,
        };
        tx.execute(
            "INSERT INTO process_goal_requests VALUES(?1,?2,?3)",
            params![
                request.command_id.to_string(),
                command.to_string(),
                serde_json::to_string(&observation)?
            ],
        )?;
        tx.execute(
            "INSERT INTO process_goal_allocations(request_id,destination,budget,receipt,dispatch_gated) VALUES(?1,?2,?3,NULL,1)",
            params![
                request.command_id.to_string(),
                request.destination.to_string(),
                serde_json::to_string(&budget)?
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(budget)
    }

    pub(crate) fn settle_goal_allocation(
        &mut self,
        guard: &ExecutionGuard,
        command: Uuid,
        incarnation: Uuid,
        destination: Uuid,
        usage: ExecutionUsage,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.opened_schema >= 17,
            "Goal allocation schema is unavailable"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if self.opened_schema >= 19 {
            let closed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_allocations WHERE request_id=?1 AND closure IS NOT NULL)",[usage.budget.command_id.to_string()],|r|r.get(0))?;
            ensure!(!closed, "Goal allocation already has non-admission proof");
        }
        let row:Option<(String,String,Option<String>)>=tx.query_row("SELECT a.destination,a.budget,a.receipt FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) WHERE a.request_id=?1 AND q.command_id=?2",params![usage.budget.command_id.to_string(),command.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (target, budget, prior) = row.context("unknown Goal child allocation")?;
        let budget: ExecutionBudget = serde_json::from_str(&budget)?;
        ensure!(
            target == destination.to_string()
                && budget == usage.budget
                && usage.session_id == budget.session_id
                && budget.parent_session_id == guard.session_id
                && !usage.run_id.is_nil(),
            "Goal child receipt attribution mismatch"
        );
        if let Some(prior) = prior {
            ensure!(
                serde_json::from_str::<ExecutionUsage>(&prior)? == usage,
                "Goal child receipt is immutable"
            );
            return Ok(());
        }
        let (run, _, _) = current(&tx, guard.session_id, command, incarnation)?;
        ensure!(
            run == budget.parent_run_id,
            "Goal child receipt parent run mismatch"
        );
        usage
            .input_tokens
            .checked_add(usage.output_tokens)
            .context("Goal child usage overflow")?;
        let observation = RequestObservation {
            request_id: budget.command_id,
            revision: 1,
            input_tokens: Some(usage.input_tokens),
            output_tokens: Some(usage.output_tokens),
            complete: usage.complete,
        };
        tx.execute(
            "UPDATE process_goal_requests SET observation=?1 WHERE request_id=?2",
            params![
                serde_json::to_string(&observation)?,
                budget.command_id.to_string()
            ],
        )?;
        tx.execute(
            "UPDATE process_goal_allocations SET receipt=?1,cleanup_observed=?3 WHERE request_id=?2",
            params![
                serde_json::to_string(&usage)?,
                budget.command_id.to_string(),usage.cleanup_observed
            ],
        )?;
        // Reject an unrepresentable aggregate atomically rather than publish a
        // ledger whose retained lower bound can no longer be read.
        super::metering::retained_usage(&tx, command)?;
        commit(tx, &self.commit_fence)
    }
}
