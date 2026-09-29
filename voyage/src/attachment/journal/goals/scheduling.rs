//! Admission checks and deny-only input precedence shared by the runtime driver
//! and the canonical journal. No transport or client owns continuation.
use super::*;

impl Journal {
    pub(crate) fn goal_continuation(
        &self,
        session: Uuid,
    ) -> Result<Option<(GoalSnapshot, GoalAuthority)>> {
        let snapshot = self.goal(session)?;
        if !snapshot
            .goal
            .as_ref()
            .is_some_and(|g| g.status == GoalStatus::Active && g.continuation_authorized)
        {
            return Ok(None);
        }
        let encoded: String = self.connection.query_row(
            "SELECT authority FROM process_goals WHERE session_id=?1",
            [session.to_string()],
            |r| r.get(0),
        )?;
        let authority: GoalAuthority = serde_json::from_str(&encoded)?;
        ensure!(authority.valid(), "invalid Goal continuation authority");
        Ok(Some((snapshot, authority)))
    }

    pub(crate) fn goal_obstruction(&self, session: Uuid) -> Result<Option<GoalStopReason>> {
        obstruction(&self.connection, session)
    }

    /// Compare the Goal revision so delayed failures cannot stop a replacement
    /// or override an explicit Pause/Resume. This grants no execution authority.
    pub(crate) fn stop_goal(
        &mut self,
        guard: &ExecutionGuard,
        revision: u64,
        reason: GoalStopReason,
        now: i64,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = read(&tx, guard.session_id)?;
        if current.revision != revision
            || !current
                .goal
                .as_ref()
                .is_some_and(|g| g.status == GoalStatus::Active)
        {
            return Ok(());
        }
        update_session(&tx, &read_session(&tx, guard.session_id)?)?;
        stop(&tx, guard.session_id, reason, now)?;
        commit(tx, &self.commit_fence)
    }
}

pub(super) fn obstruction(db: &Connection, session: Uuid) -> Result<Option<GoalStopReason>> {
    // Bound but unadmitted input survives process restart. Expiry alone does not
    // prove non-admission; exact resolution closes it. Rejected input is excluded.
    let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM process_command_bindings b WHERE json_extract(b.request,'$.op') IN ('submit','submit_content','workflow_submit','operator_tool','execute_tool','steer','cancel') AND NOT EXISTS(SELECT 1 FROM commands c WHERE c.id=b.id) AND NOT EXISTS(SELECT 1 FROM steering s WHERE s.id=b.id) AND NOT EXISTS(SELECT 1 FROM process_commands p WHERE p.id=b.id) AND NOT EXISTS(SELECT 1 FROM process_goal_turns g WHERE g.command_id=b.id))", [], |r| r.get(0))?;
    if pending {
        return Ok(Some(GoalStopReason::UserInput));
    }
    let decisions: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM process_decisions d JOIN runs r ON r.id=d.run_id WHERE r.session_id=?1 AND d.response IS NULL)", [session.to_string()], |r| r.get(0))?;
    if decisions {
        return Ok(Some(GoalStopReason::ApprovalRequired));
    }
    if !cleanup_ready(db, session)?
        || has_pending_tools(&read_session(db, session)?.session.messages)
    {
        return Ok(Some(GoalStopReason::UnresolvedEffects));
    }
    Ok(None)
}

/// Called inside the ordinary admission transaction after duplicate/revision
/// checks. Pending input may have arrived after the scheduler's reservation.
pub(in super::super) fn admit(
    tx: &Transaction<'_>,
    request: &TurnAdmission,
    now: i64,
) -> Result<()> {
    let snapshot = read(tx, request.session_id)?;
    if snapshot.goal.is_none() {
        return Ok(());
    }
    let reserved: Option<String> = tx.query_row("SELECT goal_id FROM process_goal_turns WHERE command_id=?1 AND session_id=?2 AND state='reserved'", params![request.command_id.to_string(),request.session_id.to_string()], |r| r.get(0)).optional()?;
    if let Some(id) = reserved {
        let goal = snapshot.goal.context("Goal missing")?;
        ensure!(
            goal.id.to_string() == id
                && goal.status == GoalStatus::Active
                && goal.continuation_authorized,
            "Goal continuation is no longer authorized"
        );
        ensure!(
            obstruction(tx, request.session_id)?.is_none(),
            "Goal continuation blocked by pending input or effects"
        );
        let encoded: String = tx.query_row(
            "SELECT authority FROM process_goals WHERE session_id=?1",
            [request.session_id.to_string()],
            |r| r.get(0),
        )?;
        let authority: GoalAuthority = serde_json::from_str(&encoded)?;
        ensure!(
            authority.installation_id == request.machine_id
                && authority.principal_id == request.principal_id,
            "Goal continuation actor changed"
        );
    } else {
        stop(tx, request.session_id, GoalStopReason::UserInput, now)?;
    }
    Ok(())
}

/// The caller already advances the session revision for its admitted action.
/// Do not change usage or cancel the current run; only stop future continuation.
pub(in super::super) fn stop(
    tx: &Transaction<'_>,
    session: Uuid,
    reason: GoalStopReason,
    now: i64,
) -> Result<()> {
    let mut snapshot = read(tx, session)?;
    let Some(goal) = snapshot
        .goal
        .as_mut()
        .filter(|g| g.status == GoalStatus::Active)
    else {
        return Ok(());
    };
    goal.status = if matches!(
        reason,
        GoalStopReason::RunLimit
            | GoalStopReason::TokenLimit
            | GoalStopReason::TimeLimit
            | GoalStopReason::NoProgress
    ) {
        GoalStatus::Limited
    } else {
        GoalStatus::NeedsAttention
    };
    goal.continuation_authorized = false;
    goal.stop_reason = Some(reason);
    goal.updated_at_ms = goal.updated_at_ms.max(u64::try_from(now)?);
    snapshot.revision = snapshot
        .revision
        .checked_add(1)
        .context("Goal revision overflow")?;
    persist(tx, session, &snapshot, None)
}
