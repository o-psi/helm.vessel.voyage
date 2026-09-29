//! Late authenticated observations add lower bounds without rewriting terminal
//! receipts, refunding uncertainty or restoring continuation authority.
use super::*;
use crate::provider::goal_meter::RequestObservation;
use voyage_protocol::execution_budget::{ExecutionBudget, ExecutionUsage};

#[derive(Clone)]
pub(crate) struct GoalAllocation {
    pub destination: Uuid,
    pub budget: ExecutionBudget,
}

fn same_identity(a: &ExecutionUsage, b: &ExecutionUsage) -> bool {
    a.budget == b.budget
        && a.session_id == b.session_id
        && a.run_id == b.run_id
        && a.elapsed_ms == b.elapsed_ms
        && a.complete == b.complete
}

impl Journal {
    pub(crate) fn goal_allocations(
        &self,
        session: Uuid,
        offset: u64,
        limit: u32,
    ) -> Result<Vec<GoalAllocation>> {
        self.check_schema()?;
        ensure!(
            (1..=128).contains(&limit) && offset <= i64::MAX as u64,
            "invalid allocation page"
        );
        if self.opened_schema < 17 {
            return Ok(Vec::new());
        }
        let mut query=self.connection.prepare("SELECT a.destination,a.budget FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) JOIN commands c ON c.id=q.command_id JOIN runs r ON r.id=c.run_id WHERE r.session_id=?1 AND r.active=0 ORDER BY a.rowid LIMIT ?2 OFFSET ?3")?;
        query
            .query_map(params![session.to_string(), limit, offset], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (target, budget) = row?;
                Ok(GoalAllocation {
                    destination: Uuid::parse_str(&target)?,
                    budget: serde_json::from_str(&budget)?,
                })
            })
            .collect()
    }

    /// The original execution_usage stays immutable. This separately named view
    /// adds only observed late counts; incomplete accounting remains incomplete.
    pub(crate) fn delegated_observed_usage(
        &self,
        session: Uuid,
        command: Uuid,
    ) -> Result<Option<ExecutionUsage>> {
        let Some(mut usage) = self.delegated_command_usage(session, command)? else {
            return Ok(None);
        };
        if self.opened_schema >= 18 {
            let extra:Option<(u64,u64,bool)>=self.connection.query_row("SELECT input_tokens,output_tokens,local_cleanup_observed FROM process_goal_reconciliations WHERE command_id=?1",[command.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((input, output, local_cleanup_observed)) = extra {
                usage.input_tokens = usage
                    .input_tokens
                    .checked_add(input)
                    .context("late input overflow")?;
                usage.output_tokens = usage
                    .output_tokens
                    .checked_add(output)
                    .context("late output overflow")?;
                usage.cleanup_observed |=
                    local_cleanup_observed && cleanup_ready(&self.connection, session)?;
            }
        }
        Ok(Some(usage))
    }

    pub(crate) fn reconcile_goal_allocation(
        &mut self,
        guard: &ExecutionGuard,
        destination: Uuid,
        receipt: ExecutionUsage,
        observed: Option<ExecutionUsage>,
        now: i64,
    ) -> Result<bool> {
        self.check_guard(guard, guard.session_id)?;
        self.require_content_schema(guard, false)?;
        ensure!(
            self.opened_schema >= 18 && now >= 0,
            "Goal reconciliation schema or time unavailable"
        );
        let observed = observed.unwrap_or_else(|| receipt.clone());
        ensure!(
            same_identity(&receipt, &observed)
                && observed.input_tokens >= receipt.input_tokens
                && observed.output_tokens >= receipt.output_tokens
                && (!receipt.cleanup_observed || observed.cleanup_observed)
                && (!receipt.complete
                    || observed.input_tokens == receipt.input_tokens
                        && observed.output_tokens == receipt.output_tokens),
            "invalid observed usage extension"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<(String,String,Option<String>,bool,String,String)>=tx.query_row("SELECT a.destination,a.budget,a.receipt,a.cleanup_observed,q.command_id,q.observation FROM process_goal_allocations a JOIN process_goal_requests q USING(request_id) JOIN commands c ON c.id=q.command_id JOIN runs r ON r.id=c.run_id WHERE a.request_id=?1 AND r.session_id=?2 AND r.id=?3 AND r.active=0",params![receipt.budget.command_id.to_string(),guard.session_id.to_string(),receipt.budget.parent_run_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let (target, budget, prior, clean, command, observation) =
            row.context("late observation has no terminal parent allocation")?;
        let budget: ExecutionBudget = serde_json::from_str(&budget)?;
        ensure!(
            target == destination.to_string()
                && budget == receipt.budget
                && budget.parent_session_id == guard.session_id
                && receipt.session_id == budget.session_id
                && !receipt.run_id.is_nil(),
            "late child attribution mismatch"
        );
        if let Some(prior) = prior.as_ref() {
            ensure!(
                serde_json::from_str::<ExecutionUsage>(prior)? == receipt,
                "original child receipt is immutable"
            );
        }
        let old: RequestObservation = serde_json::from_str(&observation)?;
        ensure!(
            observed.input_tokens >= old.input_tokens.unwrap_or(0)
                && observed.output_tokens >= old.output_tokens.unwrap_or(0)
                && (!clean || observed.cleanup_observed),
            "late child observation regressed"
        );
        if prior.is_some()
            && old.input_tokens == Some(observed.input_tokens)
            && old.output_tokens == Some(observed.output_tokens)
            && clean == observed.cleanup_observed
        {
            return Ok(false);
        }
        let input = observed.input_tokens - old.input_tokens.unwrap_or(0);
        let output = observed.output_tokens - old.output_tokens.unwrap_or(0);
        let command_id = Uuid::parse_str(&command)?;
        // A canonical terminal run can still be settling its live meter. Only
        // reconcile once its original aggregate receipt has become durable.
        let settled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_meters m WHERE m.command_id=?1 AND (m.settlement IS NOT NULL OR EXISTS(SELECT 1 FROM process_goal_settlements s WHERE s.command_id=m.command_id)))",[&command],|r|r.get(0))?;
        ensure!(settled, "parent terminal accounting is still active");
        let observation = RequestObservation {
            request_id: budget.command_id,
            revision: old
                .revision
                .checked_add(1)
                .context("observation revision overflow")?,
            input_tokens: Some(observed.input_tokens),
            output_tokens: Some(observed.output_tokens),
            complete: receipt.complete,
        };
        tx.execute(
            "UPDATE process_goal_requests SET observation=?1 WHERE request_id=?2",
            params![
                serde_json::to_string(&observation)?,
                budget.command_id.to_string()
            ],
        )?;
        tx.execute("UPDATE process_goal_allocations SET receipt=?1,cleanup_observed=?2 WHERE request_id=?3",params![serde_json::to_string(&receipt)?,observed.cleanup_observed,budget.command_id.to_string()])?;
        let (old_input,old_output)=tx.query_row("SELECT input_tokens,output_tokens FROM process_goal_reconciliations WHERE command_id=?1",[&command],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,u64>(1)?))).optional()?.unwrap_or_default();
        tx.execute("INSERT INTO process_goal_reconciliations(command_id,input_tokens,output_tokens) VALUES(?1,?2,?3) ON CONFLICT(command_id) DO UPDATE SET input_tokens=excluded.input_tokens,output_tokens=excluded.output_tokens",params![command,i64::try_from(old_input.checked_add(input).context("late input overflow")?)?,i64::try_from(old_output.checked_add(output).context("late output overflow")?)?])?;
        super::metering::retained_usage(&tx, command_id)?;
        // Charge only the originating Goal, never a replacement. Preserve its
        // unknown-run count and stopped/paused state until a human acts.
        let goal_id: Option<String> = tx
            .query_row(
                "SELECT goal_id FROM process_goal_turns WHERE command_id=?1",
                [&command],
                |r| r.get(0),
            )
            .optional()?;
        let mut snapshot = read(&tx, guard.session_id)?;
        if let Some(goal) = snapshot
            .goal
            .as_mut()
            .filter(|g| goal_id.as_deref() == Some(g.id.to_string().as_str()))
        {
            goal.usage.input_tokens = goal
                .usage
                .input_tokens
                .checked_add(input)
                .context("Goal input overflow")?;
            goal.usage.output_tokens = goal
                .usage
                .output_tokens
                .checked_add(output)
                .context("Goal output overflow")?;
            goal.updated_at_ms = goal.updated_at_ms.max(u64::try_from(now)?);
            let authority: Option<String> = tx.query_row(
                "SELECT authority FROM process_goals WHERE session_id=?1",
                [guard.session_id.to_string()],
                |r| r.get(0),
            )?;
            let authority: Option<GoalAuthority> =
                authority.map(|v| serde_json::from_str(&v)).transpose()?;
            snapshot.revision = snapshot
                .revision
                .checked_add(1)
                .context("Goal revision overflow")?;
            let saved = read_session(&tx, guard.session_id)?;
            update_session(&tx, &saved)?;
            persist(&tx, guard.session_id, &snapshot, authority.as_ref())?;
        }
        commit(tx, &self.commit_fence)?;
        Ok(true)
    }
}
