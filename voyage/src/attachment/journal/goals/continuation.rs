//! A durable reservation is consumed only by its original process incarnation.
//! Reopening this record never authorizes resending its command.
use super::*;

pub(crate) struct GoalTurnReservation {
    pub command: RuntimeCommand,
    pub authority: GoalAuthority,
}

impl Journal {
    /// The caller revalidates the returned private authority before dispatch.
    /// Goal/session revisions fence pause, edit, new human input and replacement
    /// between reservation and ordinary turn admission.
    pub(crate) fn reserve_goal_turn(
        &mut self,
        guard: &ExecutionGuard,
        expected_goal_revision: u64,
        incarnation: Uuid,
        prompt: String,
        now: i64,
    ) -> Result<GoalTurnReservation> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            now >= 0
                && !incarnation.is_nil()
                && !prompt.trim().is_empty()
                && prompt.len() <= MAX_PROMPT,
            "invalid goal turn reservation"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        super::super::lifecycle::ensure_admissible(&tx, guard.session_id)?;
        ensure_idle(&tx, guard.session_id)?;
        let mut current = read(&tx, guard.session_id)?;
        ensure!(
            current.revision == expected_goal_revision,
            "goal revision conflict"
        );
        let goal = current.goal.as_mut().context("no goal exists")?;
        ensure!(
            goal.status == GoalStatus::Active && goal.continuation_authorized,
            "goal continuation is not authorized"
        );
        ensure!(goal.limit_reached().is_none(), "goal limit reached");
        let authority: String = tx.query_row(
            "SELECT authority FROM process_goals WHERE session_id=?1",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        let authority: GoalAuthority = serde_json::from_str(&authority)?;
        ensure!(authority.valid(), "invalid goal continuation authority");
        let saved = read_session(&tx, guard.session_id)?;
        let command_id = Uuid::new_v4();
        let command = RuntimeCommand::Submit {
            coordination: None,
            command_id,
            expected_revision: saved
                .revision
                .checked_add(1)
                .context("session revision overflow")?,
            expires_at_ms: u64::try_from(now)?
                .checked_add(30_000)
                .context("goal deadline overflow")?,
            prompt,
        };
        let request = serde_json::to_string(&command)?;
        let count: i64 =
            tx.query_row("SELECT count(*) FROM process_command_bindings", [], |r| {
                r.get(0)
            })?;
        ensure!(count < MAX_COMMANDS, "command reservation capacity reached");
        tx.execute(
            "INSERT INTO process_command_bindings(id,principal,request) VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                authority.principal_id.to_string(),
                request
            ],
        )?;
        tx.execute(
            "INSERT INTO process_goal_turns VALUES(?1,?2,?3,?4,?5,'reserved',?6)",
            params![
                command_id.to_string(),
                guard.session_id.to_string(),
                goal.id.to_string(),
                incarnation.to_string(),
                now,
                request
            ],
        )?;
        goal.usage.runs = goal
            .usage
            .runs
            .checked_add(1)
            .context("goal run usage overflow")?;
        goal.updated_at_ms = u64::try_from(now)?;
        current.revision = current
            .revision
            .checked_add(1)
            .context("goal revision overflow")?;
        update_session(&tx, &saved)?;
        persist(&tx, guard.session_id, &current, Some(&authority))?;
        commit(tx, &self.commit_fence)?;
        Ok(GoalTurnReservation { command, authority })
    }

    /// Durable stop after a failed dispatch or an owner restart. A command that
    /// never reached ordinary admission is fenced before this returns. Accepted
    /// runs retain their canonical receipt and require separate reconciliation.
    pub(crate) fn abandon_goal_turn(
        &mut self,
        guard: &ExecutionGuard,
        command_id: Uuid,
        reason: GoalStopReason,
        now: i64,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(now >= 0, "invalid goal stop time");
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row:Option<(String,String,String)>=tx.query_row("SELECT goal_id,state,request FROM process_goal_turns WHERE command_id=?1 AND session_id=?2",params![command_id.to_string(),guard.session_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let (goal_id, state, request) = row.context("goal turn reservation missing")?;
        if state != "reserved" {
            return Ok(());
        }
        let accepted: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [command_id.to_string()],
            |r| r.get(0),
        )?;
        if !accepted {
            let receipt =
                json!({"command_id":command_id,"status":"not_admitted","effects_replayed":false});
            tx.execute(
                "INSERT OR IGNORE INTO process_commands VALUES(?1,?2,?3)",
                params![
                    command_id.to_string(),
                    request,
                    serde_json::to_string(&receipt)?
                ],
            )?;
            Journal::append_public_command(&tx, guard.session_id, command_id)?;
        }
        let mut current = read(&tx, guard.session_id)?;
        let goal = current.goal.as_mut().context("reserved goal missing")?;
        ensure!(
            goal.id.to_string() == goal_id,
            "reserved goal identity conflict"
        );
        // Explicit human pause remains a pause; a restart cannot override it.
        if goal.status != GoalStatus::Paused {
            goal.status = GoalStatus::NeedsAttention;
            goal.stop_reason = Some(reason);
        }
        goal.continuation_authorized = false;
        goal.updated_at_ms = u64::try_from(now)?;
        current.revision = current
            .revision
            .checked_add(1)
            .context("goal revision overflow")?;
        update_session(&tx, &read_session(&tx, guard.session_id)?)?;
        persist(&tx, guard.session_id, &current, None)?;
        tx.execute(
            "UPDATE process_goal_turns SET state='abandoned' WHERE command_id=?1",
            [command_id.to_string()],
        )?;
        commit(tx, &self.commit_fence)
    }
}
