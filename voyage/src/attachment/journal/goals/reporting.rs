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
        let expected = json!({"action":"report","report":report});
        ensure!(
            messages
                .iter()
                .filter(|m| m.role == Role::Assistant)
                .flat_map(|m| &m.tool_calls)
                .filter(|c| c.id == call_id && c.name == "goal" && c.arguments == expected)
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
fn evidence_digest(messages: &[Message], report: &GoalReport) -> Result<String> {
    ensure!(
        bounded_text(&report.summary) && (1..=16).contains(&report.evidence.len()),
        "Goal report needs bounded summary and evidence"
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
