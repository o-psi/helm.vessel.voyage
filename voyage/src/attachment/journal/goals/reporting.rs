//! Model assessments bind to a current root run and canonical tool observations.
//! Reporting cannot grant continuation, mutate limits or bypass terminal gates.
use super::*;

#[derive(Clone)]
pub(crate) struct GoalReportContext {
    pub run: Uuid,
    command: Uuid,
    goal: Uuid,
    revision: u64,
    incarnation: Uuid,
}

impl Journal {
    pub(crate) fn goal_report_context(
        &self,
        session: Uuid,
        run: Uuid,
    ) -> Result<Option<GoalReportContext>> {
        if self.opened_schema < 20 {
            return Ok(None);
        }
        let record = read_run(&self.connection, run)?;
        ensure!(
            record.session_id == session,
            "Goal reporting session mismatch"
        );
        let row: Option<(String,String)> = self.connection.query_row("SELECT goal_id,incarnation FROM process_goal_turns WHERE command_id=?1 AND state='reserved'", [record.command_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((goal, incarnation)) = row else {
            return Ok(None);
        };
        let snapshot = self.goal(session)?;
        ensure!(
            snapshot
                .goal
                .as_ref()
                .is_some_and(|g| g.id.to_string() == goal),
            "Goal reporting identity mismatch"
        );
        Ok(Some(GoalReportContext {
            run,
            command: record.command_id,
            goal: Uuid::parse_str(&goal)?,
            revision: snapshot.revision,
            incarnation: Uuid::parse_str(&incarnation)?,
        }))
    }

    pub(crate) fn goal_model_read(
        &self,
        session: Uuid,
        context: &GoalReportContext,
    ) -> Result<Value> {
        validate_context(&self.connection, session, context, false)?;
        let saved = read_session(&self.connection, session)?;
        let messages = run_messages(&saved.session, context.run)?;
        let mut evidence = Vec::new();
        for result in messages
            .iter()
            .filter(|m| m.role == Role::Tool && m.tool_success.is_some())
        {
            if let Some(id) = &result.tool_call_id
                && let Some(call) = messages
                    .iter()
                    .flat_map(|m| &m.tool_calls)
                    .find(|c| &c.id == id && c.name != "goal")
            {
                evidence.push(json!({"call_id":id,"tool":call.name,"success":result.tool_success}));
            }
        }
        if evidence.len() > 32 {
            evidence.drain(..evidence.len() - 32);
        }
        Ok(json!({"goal":self.goal(session)?,"run_id":context.run,"evidence":evidence}))
    }

    pub(crate) fn report_goal(
        &mut self,
        guard: &ExecutionGuard,
        context: &GoalReportContext,
        call_id: &str,
        report: &GoalReport,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.opened_schema >= 20,
            "Goal reporting requires current schema"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_context(&tx, guard.session_id, context, true)?;
        let saved = read_session(&tx, guard.session_id)?;
        let messages = run_messages(&saved.session, context.run)?;
        let canonical_report = |arguments: &Value| {
            matches!(
                arguments.get("action").and_then(Value::as_str),
                Some("report" | "status")
            ) && arguments
                .get("report")
                .cloned()
                .and_then(|value| serde_json::from_value::<GoalReport>(value).ok())
                .as_ref()
                == Some(report)
        };
        ensure!(
            messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .flat_map(|m| &m.tool_calls)
                .filter(|c| c.id == call_id && c.name == "goal" && canonical_report(&c.arguments))
                .count()
                == 1,
            "Goal report must match its canonical tool call"
        );
        let digest = evidence_digest(messages, report)?;
        let encoded = serde_json::to_string(report)?;
        let prior: Option<(String,String,String)> = tx.query_row("SELECT call_id,report,evidence_digest FROM process_goal_reports WHERE command_id=?1",[context.command.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some(prior) = prior {
            ensure!(
                prior == (call_id.to_owned(), encoded, digest),
                "Goal assessment already recorded for this run"
            );
        } else {
            tx.execute(
                "INSERT INTO process_goal_reports VALUES(?1,?2,?3,?4,?5)",
                params![
                    context.command.to_string(),
                    context.revision,
                    call_id,
                    encoded,
                    digest
                ],
            )?;
        }
        commit(tx, &self.commit_fence)?;
        Ok(
            json!({"status":"recorded","run_id":context.run,"outcome":report.outcome,"pending_terminal_checks":true}),
        )
    }
}

fn validate_context(
    db: &Connection,
    session: Uuid,
    context: &GoalReportContext,
    reporting: bool,
) -> Result<()> {
    let run = read_run(db, context.run)?;
    ensure!(
        run.session_id == session
            && run.command_id == context.command
            && matches!(run.state, RunState::Accepted | RunState::Running),
        "Goal report run is no longer active"
    );
    let current: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM process_goal_turns t JOIN process_goal_meters m USING(command_id) WHERE t.command_id=?1 AND t.goal_id=?2 AND t.incarnation=?3 AND m.incarnation=t.incarnation AND t.state='reserved')",params![context.command.to_string(),context.goal.to_string(),context.incarnation.to_string()],|r|r.get(0))?;
    ensure!(current, "Goal reporting binding is stale");
    let snapshot = read(db, session)?;
    let goal = snapshot.goal.context("Goal missing")?;
    ensure!(goal.id == context.goal, "Goal report objective changed");
    if reporting {
        ensure!(
            snapshot.revision == context.revision
                && goal.status == GoalStatus::Active
                && goal.continuation_authorized
                && run.state == RunState::Running,
            "Goal reporting no longer authorized"
        );
    }
    Ok(())
}

fn run_messages(session: &Session, run: Uuid) -> Result<&[Message]> {
    let summary = session
        .run_summaries
        .iter()
        .find(|s| s.run_id == run)
        .context("Goal run history unavailable")?;
    let start = summary
        .message_start
        .context("Goal run history was compacted")?;
    let end = summary
        .message_end
        .context("Goal run history was compacted")?;
    session
        .messages
        .get(start..end)
        .context("Goal run history range invalid")
}

fn bounded_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= 2048
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}
pub(super) fn evidence_digest(messages: &[Message], report: &GoalReport) -> Result<String> {
    ensure!(
        bounded_text(&report.summary) && report.evidence.len() <= 16,
        "Goal report needs a bounded summary and optional evidence"
    );
    let mut ids = std::collections::BTreeSet::new();
    let mut observed = Vec::new();
    for evidence in &report.evidence {
        ensure!(
            bounded_text(&evidence.conclusion)
                && evidence.call_id.len() <= 256
                && ids.insert(&evidence.call_id),
            "invalid or repeated Goal evidence"
        );
        let calls: Vec<_> = messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .flat_map(|m| &m.tool_calls)
            .filter(|c| c.id == evidence.call_id && c.name != "goal")
            .collect();
        let results: Vec<_> = messages
            .iter()
            .filter(|m| {
                m.role == Role::Tool
                    && m.tool_call_id.as_ref() == Some(&evidence.call_id)
                    && m.tool_success.is_some()
            })
            .collect();
        ensure!(
            calls.len() == 1 && results.len() == 1,
            "Goal evidence is missing, ambiguous or from another run"
        );
        if report.outcome == GoalReportOutcome::Complete {
            ensure!(
                results[0].tool_success == Some(true)
                    && results[0]
                        .tool_outcome
                        .as_ref()
                        .is_none_or(|o| o.incomplete.is_none()),
                "completion evidence must be successful and complete"
            );
        }
        observed.push(json!({"call":calls[0],"result":results[0]}));
    }
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&observed)?)))
}

/// Revalidate immutable proof against the final canonical transcript. A report
/// remains a model assessment of the objective, never an authority override.
pub(super) fn assessment(
    db: &Connection,
    session: &Session,
    run: &RunRecord,
    revision: u64,
) -> Result<Option<GoalAssessment>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_goal_reports')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let row: Option<(u64,String,String)> = db.query_row("SELECT goal_revision,report,evidence_digest FROM process_goal_reports WHERE command_id=?1",[run.command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((original, encoded, digest)) = row else {
        return Ok(None);
    };
    if original != revision {
        return Ok(None);
    }
    let report: GoalReport = serde_json::from_str(&encoded)?;
    let observed = evidence_digest(run_messages(session, run.id)?, &report)?;
    ensure!(observed == digest, "Goal evidence changed after reporting");
    Ok(Some(GoalAssessment {
        run_id: run.id,
        report,
        evidence_sha256: digest,
    }))
}

impl Journal {
    /// Root-owner-only explicit request creation. Never replaces unfinished work,
    /// accepts model budget increases, or manufactures a human mutation receipt.
    pub(crate) fn model_create_goal(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        authority: GoalAuthority,
        incarnation: Uuid,
        call: &str,
        objective: &str,
        tokens: Option<u64>,
        now: i64,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            authority.valid() && !incarnation.is_nil() && now >= 0,
            "invalid Goal control"
        );
        ensure!(
            tokens.is_none_or(|n| n > 0),
            "explicit Goal token budget must be positive"
        );
        let limits = GoalLimits {
            tokens: tokens.unwrap_or(0),
            ..GoalLimits::default()
        };
        ensure!(
            valid_objective(objective) && limits.valid(),
            "invalid Goal objective or quota"
        );
        self.require_content_schema(guard, false)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id
                && run.state == RunState::Running
                && run.machine_id == authority.installation_id
                && run.principal_id == authority.principal_id,
            "Goal create requires its admitted owner run"
        );
        let saved = read_session(&tx, guard.session_id)?;
        let messages = run_messages(&saved.session, run_id)?;
        let invocation = messages
            .iter()
            .filter(|m| m.role == Role::Assistant)
            .flat_map(|m| &m.tool_calls)
            .find(|c| c.id == call && c.name == "goal")
            .context("Goal create needs its canonical call")?;
        ensure!(
            invocation.arguments.get("action").and_then(Value::as_str) == Some("create")
                && invocation
                    .arguments
                    .get("objective")
                    .and_then(Value::as_str)
                    == Some(objective)
                && invocation
                    .arguments
                    .get("token_budget")
                    .and_then(Value::as_u64)
                    == tokens,
            "Goal create canonical arguments changed"
        );
        // Explicit request provenance remains canonical user task data. The model
        // attests interpretation; it cannot infer a Goal from ordinary work.
        ensure!(
            messages
                .iter()
                .any(|m| m.role == Role::User && m.coordination.is_none()),
            "Goal creation needs user request context"
        );
        let mut snapshot = read(&tx, guard.session_id)?;
        ensure!(
            snapshot.goal.is_none(),
            "existing Goal requires explicit human replacement or clear"
        );
        let id = Uuid::new_v4();
        snapshot.goal = Some(Goal {
            id,
            session_id: guard.session_id,
            objective: objective.into(),
            status: GoalStatus::Active,
            continuation_authorized: true,
            limits,
            usage: GoalUsage {
                runs: 1,
                ..GoalUsage::default()
            },
            created_at_ms: u64::try_from(now)?,
            updated_at_ms: u64::try_from(now)?,
            stop_reason: None,
            assessment: None,
        });
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .context("Goal revision overflow")?;
        update_session(&tx, &saved)?;
        persist(&tx, guard.session_id, &snapshot, Some(&authority))?;
        // Bind the remainder/current settlement to the new Goal without replaying
        // admission or charging it for an earlier objective's completed work.
        let request = serde_json::to_string(
            &json!({"source":"model_create","call_id":call,"run_id":run_id}),
        )?;
        tx.execute(
            "INSERT INTO process_goal_turns VALUES(?1,?2,?3,?4,?5,'reserved',?6)",
            params![
                run.command_id.to_string(),
                guard.session_id.to_string(),
                id.to_string(),
                incarnation.to_string(),
                now,
                request
            ],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO process_goal_meters VALUES(?1,?2,NULL,?3,NULL)",
            params![run.command_id.to_string(), incarnation.to_string(), now],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(json!({"status":"created","goal":snapshot,"explicit_user_request_required":true}))
    }
}

impl Journal {
    pub(crate) fn model_edit_goal(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        authority: GoalAuthority,
        call: &str,
        objective: &str,
        now: i64,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            authority.valid() && now >= 0 && valid_objective(objective),
            "invalid Goal refinement"
        );
        self.require_content_schema(guard, false)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id
                && run.state == RunState::Running
                && run.machine_id == authority.installation_id
                && run.principal_id == authority.principal_id,
            "Goal refinement requires admitted owner run"
        );
        let saved = read_session(&tx, guard.session_id)?;
        let messages = run_messages(&saved.session, run_id)?;
        ensure!(
            messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .flat_map(|m| &m.tool_calls)
                .any(|c| c.id == call
                    && c.name == "goal"
                    && c.arguments == json!({"action":"edit","objective":objective})),
            "Goal refinement needs canonical call"
        );
        ensure!(
            messages
                .iter()
                .any(|m| m.role == Role::User && m.coordination.is_none()),
            "Goal refinement needs human request context"
        );
        let mut snapshot = read(&tx, guard.session_id)?;
        let goal = snapshot.goal.as_mut().context("No Goal to refine")?;
        ensure!(
            goal.status == GoalStatus::Active && goal.continuation_authorized,
            "model refinement cannot resume or replace stopped work"
        );
        goal.objective = objective.into();
        goal.assessment = None;
        goal.usage.impasse_runs = 0;
        goal.updated_at_ms = u64::try_from(now)?;
        snapshot.revision = snapshot
            .revision
            .checked_add(1)
            .context("Goal revision overflow")?;
        update_session(&tx, &saved)?;
        persist(&tx, guard.session_id, &snapshot, Some(&authority))?;
        commit(tx, &self.commit_fence)?;
        Ok(json!({"status":"edited","goal":snapshot,"usage_and_limits_preserved":true}))
    }
}
