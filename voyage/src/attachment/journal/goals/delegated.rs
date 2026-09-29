//! One-command delegated budgets use the same durable request meter, without
//! creating a Goal or granting a child any automatic continuation authority.
use super::*;
use voyage_protocol::execution_budget::{ExecutionBudget, ExecutionUsage};

impl Journal {
    pub(crate) fn begin_delegated_meter(
        &mut self,
        guard: &ExecutionGuard,
        budget: &ExecutionBudget,
        incarnation: Uuid,
        now: i64,
    ) -> Result<(u64, u64)> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.opened_schema >= 16
                && now >= 0
                && !incarnation.is_nil()
                && budget.valid_for(budget.command_id),
            "invalid delegated budget"
        );
        ensure!(
            budget.session_id == guard.session_id && budget.parent_session_id != guard.session_id,
            "delegated budget must target an independent child"
        );
        let remaining = budget
            .expires_at_ms
            .checked_sub(u64::try_from(now)?)
            .filter(|v| *v > 0 && *v <= 86_400_000)
            .context("delegated budget expired or exceeds time bounds")?
            .min(budget.elapsed_ms);
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN runs r ON r.id=c.run_id WHERE c.id=?1 AND r.session_id=?2 AND r.active=1)",params![budget.command_id.to_string(),guard.session_id.to_string()],|r|r.get(0))?;
        ensure!(active, "delegated meter requires its admitted active run");
        let root: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM process_goal_turns WHERE command_id=?1)",
            [budget.command_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            !root,
            "a Goal reservation cannot also be a delegated budget"
        );
        tx.execute(
            "INSERT INTO process_goal_meters VALUES(?1,?2,?3,?4,NULL)",
            params![
                budget.command_id.to_string(),
                incarnation.to_string(),
                serde_json::to_string(budget)?,
                now
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok((budget.tokens, remaining))
    }

    pub(crate) fn settle_delegated_run(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        measurement: Option<GoalMeasurement>,
        cleanup_observed: bool,
        now: i64,
    ) -> Result<Option<ExecutionUsage>> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(now >= 0, "invalid delegated settlement time");
        if self.opened_schema < 16 {
            return Ok(None);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id,
            "delegated run session mismatch"
        );
        let row:Option<(String,i64,Option<String>)>=tx.query_row("SELECT budget,started_at_ms,settlement FROM process_goal_meters WHERE command_id=?1 AND budget IS NOT NULL",[run.command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((budget, started, prior)) = row else {
            return Ok(None);
        };
        if let Some(prior) = prior {
            return Ok(Some(serde_json::from_str(&prior)?));
        }
        ensure!(
            !matches!(run.state, RunState::Accepted | RunState::Running),
            "delegated run is still active"
        );
        let budget: ExecutionBudget = serde_json::from_str(&budget)?;
        ensure!(
            budget.valid_for(run.command_id),
            "delegated receipt budget identity mismatch"
        );
        let (input, output, all_reported, _) =
            super::metering::retained_usage(&tx, run.command_id)?;
        let measured = measurement.as_ref().is_some_and(|m| m.complete);
        let elapsed = u64::try_from(now.saturating_sub(started).max(0))?;
        let usage = measurement.unwrap_or(GoalMeasurement {
            input_tokens: input.max(run.usage.input_tokens),
            output_tokens: output.max(run.usage.output_tokens),
            elapsed_ms: elapsed,
            complete: false,
        });
        ensure!(
            usage.input_tokens >= input.max(run.usage.input_tokens)
                && usage.output_tokens >= output.max(run.usage.output_tokens),
            "delegated aggregate is below retained usage"
        );
        if self.opened_schema >= 18 {
            tx.execute("INSERT INTO process_goal_reconciliations(command_id,input_tokens,output_tokens,local_cleanup_observed) VALUES(?1,0,0,?2)",params![run.command_id.to_string(),cleanup_observed])?;
        }
        let receipt = ExecutionUsage {
            budget,
            session_id: guard.session_id,
            run_id,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            elapsed_ms: usage.elapsed_ms.max(elapsed),
            complete: measured && all_reported,
            cleanup_observed: cleanup_observed && cleanup_ready(&tx, guard.session_id)?,
        };
        tx.execute(
            "UPDATE process_goal_meters SET settlement=?1 WHERE command_id=?2",
            params![serde_json::to_string(&receipt)?, run.command_id.to_string()],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(Some(receipt))
    }

    pub(crate) fn delegated_usage(&self, session: Uuid) -> Result<Option<ExecutionUsage>> {
        self.check_schema()?;
        if self.opened_schema < 16 {
            return Ok(None);
        }
        let Some(run) = self.process_latest_run(session)? else {
            return Ok(None);
        };
        self.delegated_command_usage(session, run.command_id)
    }

    pub(crate) fn delegated_command_usage(
        &self,
        session: Uuid,
        command: Uuid,
    ) -> Result<Option<ExecutionUsage>> {
        self.check_schema()?;
        if self.opened_schema < 16 {
            return Ok(None);
        }
        let saved:Option<Option<String>>=self.connection.query_row("SELECT m.settlement FROM process_goal_meters m JOIN commands c ON c.id=m.command_id JOIN runs r ON r.id=c.run_id WHERE m.command_id=?1 AND r.session_id=?2 AND m.budget IS NOT NULL",params![command.to_string(),session.to_string()],|r|r.get(0)).optional()?;
        saved
            .flatten()
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn recover_delegated_meter(
        &mut self,
        guard: &ExecutionGuard,
        now: i64,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        if self.opened_schema < 16 {
            return Ok(());
        }
        let pending:Vec<String>=self.connection.prepare("SELECT r.id FROM process_goal_meters m JOIN commands c ON c.id=m.command_id JOIN runs r ON r.id=c.run_id WHERE m.budget IS NOT NULL AND m.settlement IS NULL AND r.session_id=?1")?.query_map([guard.session_id.to_string()],|r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
        for run in pending {
            let observed = cleanup_ready(&self.connection, guard.session_id)?;
            self.settle_delegated_run(guard, Uuid::parse_str(&run)?, None, observed, now)?;
        }
        Ok(())
    }
}
