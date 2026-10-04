//! Account terminal goal turns once. A run's final answer is not Goal completion.
use super::*;

/// Supplied only by the executing runtime's aggregate meter, never a provider,
/// model tool, Helm request or persisted configuration. Includes subordinate use.
#[derive(Clone, Debug)]
pub(crate) struct GoalMeasurement {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    pub complete: bool,
}

impl Journal {
    pub(crate) fn settle_goal_run(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        measurement: Option<GoalMeasurement>,
        cleanup_observed: bool,
        now: i64,
    ) -> Result<Option<Value>> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(now >= 0, "invalid goal settlement time");
        // Legacy journals and ordinary runs have no Goal reservation to settle.
        if self.opened_schema < 14 {
            return Ok(None);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id,
            "goal run session mismatch"
        );
        let reservation:Option<(String,String,i64)>=tx.query_row("SELECT goal_id,state,started_at_ms FROM process_goal_turns WHERE command_id=?1 AND session_id=?2",params![run.command_id.to_string(),guard.session_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((goal_id, state, started)) = reservation else {
            return Ok(None);
        };
        let prior: Option<String> = tx
            .query_row(
                "SELECT receipt FROM process_goal_settlements WHERE command_id=?1",
                [run.command_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            return Ok(Some(serde_json::from_str(&prior)?));
        }
        ensure!(state == "reserved", "goal turn was already abandoned");
        ensure!(
            !matches!(run.state, RunState::Accepted | RunState::Running),
            "goal run is still active"
        );
        let saved = read_session(&tx, guard.session_id)?;
        let mut current = read(&tx, guard.session_id)?;
        let assessment = super::reporting::assessment(&tx, &saved.session, &run, current.revision)?;
        let goal = current.goal.as_mut().context("reserved goal missing")?;
        ensure!(
            goal.id.to_string() == goal_id,
            "goal settlement identity mismatch"
        );
        let (retained_input, retained_output, retained_complete, metered) =
            super::metering::retained_usage(&tx, run.command_id)?;
        let measured =
            measurement.as_ref().is_some_and(|m| m.complete) && (!metered || retained_complete);
        let mut observed = measurement.unwrap_or(GoalMeasurement {
            input_tokens: run.usage.input_tokens.max(retained_input),
            output_tokens: run.usage.output_tokens.max(retained_output),
            elapsed_ms: u64::try_from(now.saturating_sub(started).max(0))?,
            complete: false,
        });
        // Include admission/setup/cleanup time outside the provider meter too.
        observed.elapsed_ms = observed
            .elapsed_ms
            .max(u64::try_from(now.saturating_sub(started).max(0))?);
        ensure!(
            observed.input_tokens >= run.usage.input_tokens
                && observed.output_tokens >= run.usage.output_tokens
                && observed.input_tokens >= retained_input
                && observed.output_tokens >= retained_output,
            "aggregate goal usage is below canonical run usage"
        );
        goal.usage.input_tokens = goal
            .usage
            .input_tokens
            .checked_add(observed.input_tokens)
            .context("goal input usage overflow")?;
        goal.usage.output_tokens = goal
            .usage
            .output_tokens
            .checked_add(observed.output_tokens)
            .context("goal output usage overflow")?;
        goal.usage.elapsed_ms = goal
            .usage
            .elapsed_ms
            .checked_add(observed.elapsed_ms)
            .context("goal elapsed usage overflow")?;
        if !measured {
            goal.usage.unmeasured_runs = goal
                .usage
                .unmeasured_runs
                .checked_add(1)
                .context("goal unmeasured usage overflow")?;
        }
        let digest = progress_digest(&saved.session, run_id)?;
        let repeated = if let Some(digest) = &digest {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_settlements s JOIN process_goal_turns t USING(command_id) WHERE t.session_id=?1 AND t.goal_id=?2 AND s.progress_digest=?3)",params![guard.session_id.to_string(),goal_id,digest],|r|r.get::<_,bool>(0))?
        } else {
            true
        };
        goal.usage.no_progress_runs = if repeated {
            goal.usage
                .no_progress_runs
                .checked_add(1)
                .context("goal no-progress counter overflow")?
        } else {
            0
        };
        let unresolved:u64=tx.query_row("SELECT count(*) FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL",[guard.session_id.to_string()],|r|r.get(0))?;
        let decisions: u64 = tx.query_row(
            "SELECT count(*) FROM process_decisions WHERE run_id=?1 AND response IS NULL",
            [run_id.to_string()],
            |r| r.get(0),
        )?;
        let stop = if !cleanup_observed || unresolved > 0 || !cleanup_ready(&tx, guard.session_id)?
        {
            Some((
                GoalStatus::NeedsAttention,
                GoalStopReason::UnresolvedEffects,
            ))
        } else if run.state == RunState::Cancelled {
            if goal.limits.tokens > 0
                && goal
                    .usage
                    .input_tokens
                    .saturating_add(goal.usage.output_tokens)
                    >= goal.limits.tokens
            {
                Some((GoalStatus::Limited, GoalStopReason::TokenLimit))
            } else if goal.limits.elapsed_ms > 0 && goal.usage.elapsed_ms >= goal.limits.elapsed_ms
            {
                Some((GoalStatus::Limited, GoalStopReason::TimeLimit))
            } else if !measured && goal.limits.usage_required() {
                // Missing provider usage can cancel the metered execution itself.
                // Expose the unresolved accounting rather than suggesting a human
                // cancelled a fully measured run that can safely resume.
                Some((GoalStatus::NeedsAttention, GoalStopReason::UsageUnknown))
            } else {
                Some((GoalStatus::NeedsAttention, GoalStopReason::Cancelled))
            }
        } else if run.state == RunState::Interrupted {
            Some((GoalStatus::NeedsAttention, GoalStopReason::Interrupted))
        } else if decisions > 0 {
            Some((GoalStatus::NeedsAttention, GoalStopReason::ApprovalRequired))
        } else if goal.limits.tokens > 0
            && goal
                .usage
                .input_tokens
                .saturating_add(goal.usage.output_tokens)
                >= goal.limits.tokens
        {
            Some((GoalStatus::Limited, GoalStopReason::TokenLimit))
        } else if goal.limits.elapsed_ms > 0 && goal.usage.elapsed_ms >= goal.limits.elapsed_ms {
            Some((GoalStatus::Limited, GoalStopReason::TimeLimit))
        } else if run.state != RunState::Completed {
            Some((GoalStatus::NeedsAttention, GoalStopReason::ProviderFailure))
        } else if !measured && goal.limits.usage_required() {
            Some((GoalStatus::NeedsAttention, GoalStopReason::UsageUnknown))
        } else {
            goal.limit_reached()
                .map(|reason| (GoalStatus::Limited, reason))
        };
        let report_eligible = goal.status == GoalStatus::Active
            && run.state == RunState::Completed
            && (measured || !goal.limits.usage_required())
            && cleanup_observed
            && unresolved == 0
            && decisions == 0
            && cleanup_ready(&tx, guard.session_id)?
            && super::scheduling::obstruction(&tx, guard.session_id)?.is_none()
            && stop.is_none_or(|(_, reason)| {
                matches!(
                    reason,
                    GoalStopReason::RunLimit | GoalStopReason::NoProgress
                )
            });
        if report_eligible && let Some(assessment) = assessment {
            let blocked = assessment.report.outcome == GoalReportOutcome::Blocked;
            // An intermediate blocker is a checkpoint, not termination. Only
            // three consecutive reports of the same genuine impasse stop intent.
            let same_impasse = blocked
                && goal.assessment.as_ref().is_some_and(|previous| {
                    previous.report.outcome == GoalReportOutcome::Blocked
                        && previous.report.summary == assessment.report.summary
                });
            let repeated_impasse = if same_impasse {
                goal.usage.impasse_runs.saturating_add(1)
            } else if blocked {
                1
            } else {
                0
            };
            goal.usage.impasse_runs = repeated_impasse;
            if !blocked || repeated_impasse >= 3 {
                goal.status = if blocked {
                    GoalStatus::Blocked
                } else {
                    GoalStatus::Complete
                };
                goal.continuation_authorized = false;
            }
            goal.stop_reason = None;
            goal.assessment = Some(assessment);
        }
        if !report_eligible || goal.assessment.as_ref().is_none_or(|a| a.run_id != run_id) {
            goal.usage.impasse_runs = 0;
        }
        // A concurrent explicit pause wins over every automatic transition.
        if goal.status == GoalStatus::Active
            && let Some((status, reason)) = stop
        {
            goal.status = status;
            goal.stop_reason = Some(reason);
            goal.continuation_authorized = false;
        }
        goal.updated_at_ms = goal.updated_at_ms.max(u64::try_from(now)?);
        let keep_authority = goal.status == GoalStatus::Active && goal.continuation_authorized;
        let authority: Option<GoalAuthority> = if keep_authority {
            let value: String = tx.query_row(
                "SELECT authority FROM process_goals WHERE session_id=?1",
                [guard.session_id.to_string()],
                |r| r.get(0),
            )?;
            Some(serde_json::from_str(&value)?)
        } else {
            None
        };
        let receipt = json!({"command_id":run.command_id,"goal_id":goal.id,"run_id":run_id,"status":"settled","usage_complete":measured,"goal_status":goal.status,"stop_reason":goal.stop_reason,"usage":goal.usage,"assessment":goal.assessment});
        current.revision = current
            .revision
            .checked_add(1)
            .context("goal revision overflow")?;
        update_session(&tx, &saved)?;
        persist(&tx, guard.session_id, &current, authority.as_ref())?;
        tx.execute(
            "INSERT INTO process_goal_settlements VALUES(?1,?2,?3)",
            params![
                run.command_id.to_string(),
                serde_json::to_string(&receipt)?,
                digest
            ],
        )?;
        tx.execute(
            "UPDATE process_goal_turns SET state='settled' WHERE command_id=?1",
            [run.command_id.to_string()],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(Some(receipt))
    }

    /// Called only on startup, after ordinary interrupted-run recovery. Never
    /// returns a Submit command, and never claims an old process survived.
    pub(crate) fn recover_goal_turn(&mut self, guard: &ExecutionGuard, now: i64) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        if self.opened_schema < 14 {
            return Ok(());
        }
        let pending:Option<(String,Option<String>)>=self.connection.query_row("SELECT t.command_id,c.run_id FROM process_goal_turns t LEFT JOIN commands c ON c.id=t.command_id WHERE t.session_id=?1 AND t.state='reserved'",[guard.session_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((command, run)) = pending {
            if let Some(run) = run {
                let unresolved:u64=self.connection.query_row("SELECT count(*) FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL",[guard.session_id.to_string()],|r|r.get(0))?;
                self.settle_goal_run(guard, Uuid::parse_str(&run)?, None, unresolved == 0, now)?;
            } else {
                self.abandon_goal_turn(
                    guard,
                    Uuid::parse_str(&command)?,
                    GoalStopReason::Interrupted,
                    now,
                )?;
            }
        }
        Ok(())
    }
}

/// Only new, successful tool observations count as progress automatically.
/// Changing call IDs or repeating the same tool/result does not reset the bound.
fn progress_digest(session: &Session, run: Uuid) -> Result<Option<String>> {
    let Some(summary) = session.run_summaries.iter().find(|s| s.run_id == run) else {
        return Ok(None);
    };
    let (Some(start), Some(end)) = (summary.message_start, summary.message_end) else {
        return Ok(None);
    };
    let Some(messages) = session.messages.get(start..end) else {
        return Ok(None);
    };
    let mut evidence = Vec::new();
    for message in messages
        .iter()
        .filter(|m| m.role == Role::Tool && m.tool_success == Some(true))
    {
        let Some(id) = message.tool_call_id.as_ref() else {
            continue;
        };
        let Some(call) = messages
            .iter()
            .flat_map(|m| &m.tool_calls)
            .find(|c| &c.id == id)
        else {
            continue;
        };
        if call.name == "goal" {
            continue;
        }
        // Timing varies between identical observations and cannot count as
        // substantive progress. Keep execution/exit/completeness facts.
        let mut outcome = message.tool_outcome.clone();
        if let Some(outcome) = &mut outcome {
            outcome.elapsed_ms = None;
        }
        evidence.push(json!({"name":call.name,"arguments":call.arguments,"content":message.content,"outcome":outcome}));
    }
    if evidence.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hex::encode(Sha256::digest(serde_json::to_vec(
            &evidence,
        )?))))
    }
}
