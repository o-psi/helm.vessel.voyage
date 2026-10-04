use super::*;
use crate::provider::goal_meter::RequestObservation;

impl Journal {
    /// Called once for a newly admitted turn. Neither restart nor duplicate
    /// submission may construct another meter with a fresh allowance.
    pub(crate) fn begin_goal_meter(
        &mut self,
        guard: &ExecutionGuard,
        command: Uuid,
        incarnation: Uuid,
        now: i64,
    ) -> Result<Option<(u64, u64)>> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            now >= 0 && !incarnation.is_nil(),
            "invalid Goal meter identity or time"
        );
        if self.opened_schema < 16 {
            let reserved = self.opened_schema >= 13
                && self.connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM process_goal_turns WHERE command_id=?1)",
                    [command.to_string()],
                    |r| r.get::<_, bool>(0),
                )?;
            ensure!(!reserved, "Goal meter requires current accounting schema");
            return Ok(None);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<(String,String,String,i64)>=tx.query_row("SELECT goal_id,incarnation,state,started_at_ms FROM process_goal_turns WHERE command_id=?1 AND session_id=?2",params![command.to_string(),guard.session_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((goal_id, original, state, started)) = row else {
            return Ok(None);
        };
        ensure!(
            state == "reserved" && original == incarnation.to_string(),
            "Goal meter reservation is not current"
        );
        let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM commands c JOIN runs r ON c.run_id=r.id WHERE c.id=?1 AND r.session_id=?2 AND r.active=1)",params![command.to_string(),guard.session_id.to_string()],|r|r.get(0))?;
        ensure!(active, "Goal meter requires its admitted active run");
        let goal = read(&tx, guard.session_id)?
            .goal
            .context("Goal meter objective missing")?;
        ensure!(
            goal.id.to_string() == goal_id
                && goal.status == GoalStatus::Active
                && goal.continuation_authorized
                && (!goal.limits.usage_required() || goal.usage.unmeasured_runs == 0),
            "Goal meter continuation is not authorized"
        );
        let used = goal
            .usage
            .input_tokens
            .checked_add(goal.usage.output_tokens)
            .context("Goal token total overflow")?;
        let tokens = if goal.limits.tokens == 0 {
            u64::MAX
        } else {
            goal.limits
                .tokens
                .checked_sub(used)
                .filter(|n| *n > 0)
                .context("Goal token limit reached")?
        };
        let elapsed = u64::try_from(now.saturating_sub(started).max(0))?
            .checked_add(goal.usage.elapsed_ms)
            .context("Goal elapsed usage overflow")?;
        let time = if goal.limits.elapsed_ms == 0 {
            u64::MAX
        } else {
            goal.limits
                .elapsed_ms
                .checked_sub(elapsed)
                .filter(|n| *n > 0)
                .context("Goal time limit reached")?
        };
        tx.execute(
            "INSERT INTO process_goal_meters VALUES(?1,?2,NULL,?3,NULL)",
            params![command.to_string(), incarnation.to_string(), started],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(Some((tokens, time)))
    }

    pub(crate) fn record_goal_request(
        &mut self,
        guard: &ExecutionGuard,
        command: Uuid,
        incarnation: Uuid,
        observed: RequestObservation,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.opened_schema >= 16 && !observed.request_id.is_nil(),
            "invalid Goal usage observation"
        );
        ensure!(
            !observed.complete
                || observed.input_tokens.is_some() && observed.output_tokens.is_some(),
            "Goal request usage is incomplete"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_meters m JOIN commands c ON c.id=m.command_id JOIN runs r ON r.id=c.run_id WHERE m.command_id=?1 AND m.incarnation=?2 AND r.session_id=?3 AND m.settlement IS NULL AND (m.budget IS NOT NULL OR EXISTS(SELECT 1 FROM process_goal_turns t WHERE t.command_id=m.command_id AND t.incarnation=m.incarnation AND t.state='reserved')))",params![command.to_string(),incarnation.to_string(),guard.session_id.to_string()],|r|r.get(0))?;
        ensure!(current, "Goal usage belongs to an inactive reservation");
        if self.opened_schema >= 17 {
            let allocation: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM process_goal_allocations WHERE request_id=?1)",
                [observed.request_id.to_string()],
                |r| r.get(0),
            )?;
            ensure!(
                !allocation,
                "delegated usage requires its exact child receipt"
            );
        }
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT command_id,observation FROM process_goal_requests WHERE request_id=?1",
                [observed.request_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((owner, prior)) = prior {
            ensure!(owner == command.to_string(), "Goal request owner mismatch");
            let prior: RequestObservation = serde_json::from_str(&prior)?;
            if prior == observed {
                return Ok(());
            }
            ensure!(
                !prior.complete && prior.revision.checked_add(1) == Some(observed.revision),
                "Goal usage revision conflict"
            );
            ensure!(
                observed.input_tokens >= prior.input_tokens
                    && observed.output_tokens >= prior.output_tokens,
                "Goal usage cannot decrease"
            );
        } else {
            ensure!(
                observed.revision == 0
                    && observed.input_tokens.is_none()
                    && observed.output_tokens.is_none()
                    && !observed.complete,
                "Goal request must be recorded before dispatch"
            );
            let count: u64 = tx.query_row(
                "SELECT count(*) FROM process_goal_requests WHERE command_id=?1",
                [command.to_string()],
                |r| r.get(0),
            )?;
            ensure!(count < 10_000, "Goal request journal capacity reached");
        }
        tx.execute("INSERT INTO process_goal_requests VALUES(?1,?2,?3) ON CONFLICT(request_id) DO UPDATE SET observation=excluded.observation",params![observed.request_id.to_string(),command.to_string(),serde_json::to_string(&observed)?])?;
        commit(tx, &self.commit_fence)
    }
}

/// Durable observed lower bound, including all local child requests. An open
/// request remains uncertain; restart never turns it into zero usage or success.
pub(super) fn retained_usage(db: &Connection, command: Uuid) -> Result<(u64, u64, bool, bool)> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_goal_meters')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok((0, 0, false, false));
    }
    let metered: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM process_goal_meters WHERE command_id=?1)",
        [command.to_string()],
        |r| r.get(0),
    )?;
    let mut query =
        db.prepare("SELECT observation FROM process_goal_requests WHERE command_id=?1")?;
    let mut input = 0u64;
    let mut output = 0u64;
    let mut complete = metered;
    for row in query.query_map([command.to_string()], |r| r.get::<_, String>(0))? {
        let observed: RequestObservation = serde_json::from_str(&row?)?;
        input = input
            .checked_add(observed.input_tokens.unwrap_or(0))
            .context("Goal retained input overflow")?;
        output = output
            .checked_add(observed.output_tokens.unwrap_or(0))
            .context("Goal retained output overflow")?;
        complete &= observed.complete;
    }
    Ok((input, output, complete, metered))
}
