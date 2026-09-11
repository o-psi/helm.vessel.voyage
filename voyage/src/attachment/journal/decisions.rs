//! Decisions are durable execution requests; only the active owner resolves them.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
impl Journal {
    pub(crate) fn finish_decision(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        outcome: &str,
    ) -> Result<()> {
        ensure!(
            matches!(outcome, "invalidated" | "cancelled" | "expired"),
            "invalid decision completion"
        );
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE process_decisions SET response=?3 WHERE id=?1 AND response IS NULL AND run_id IN (SELECT id FROM runs WHERE session_id=?2)",
            params![id.to_string(), guard.session_id.to_string(), serde_json::to_string(outcome)?],
        )?;
        commit(tx, &self.commit_fence)
    }

    pub(crate) fn initialize_decisions(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_decisions(id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), incarnation TEXT NOT NULL, expires INTEGER NOT NULL, request TEXT NOT NULL, response TEXT)")?;
        Ok(())
    }
    pub(crate) fn create_decision(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        incarnation: Uuid,
        id: Uuid,
        expires: i64,
        request: Value,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id && matches!(run.state, RunState::Running),
            "decision run is not active"
        );
        let count: i64 = tx.query_row("SELECT count(*) FROM process_decisions", [], |row| {
            row.get(0)
        })?;
        ensure!(count < 10_000, "decision capacity reached");
        let encoded = serde_json::to_string(&request)?;
        ensure!(encoded.len() <= 16 * 1024, "decision request exceeds limit");
        tx.execute(
            "INSERT INTO process_decisions VALUES(?1,?2,?3,?4,?5,NULL)",
            params![
                id.to_string(),
                run_id.to_string(),
                incarnation.to_string(),
                expires,
                encoded
            ],
        )?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn decisions(&self, incarnation: Uuid, now: i64) -> Result<Value> {
        let mut statement=self.connection.prepare("SELECT d.id,d.run_id,d.expires,d.request FROM process_decisions d JOIN runs r ON r.id=d.run_id WHERE d.incarnation=?1 AND d.response IS NULL AND d.expires>?2 AND r.active=1 ORDER BY d.rowid LIMIT 64")?;
        let rows = statement.query_map(params![incarnation.to_string(), now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (id, run, expires, request) = row?;
            result.push(json!({"decision_id":id,"run_id":run,"incarnation":incarnation,"expires_at_ms":expires,"request":serde_json::from_str::<Value>(&request)?}));
        }
        Ok(json!(result))
    }
    pub(crate) fn decision_response(&self, id: Uuid, now: i64) -> Result<Option<Value>> {
        let (expires, response): (i64, Option<String>) = self.connection.query_row(
            "SELECT expires,response FROM process_decisions WHERE id=?1",
            [id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if response.is_none() && expires <= now {
            return Ok(Some(json!("expired")));
        }
        response
            .map(|encoded| serde_json::from_str(&encoded).map_err(Into::into))
            .transpose()
    }
    pub(crate) fn respond_decision(
        &mut self,
        guard: &ExecutionGuard,
        incarnation: Uuid,
        command: &RuntimeCommand,
        admit: impl FnOnce() -> Result<i64>,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        let RuntimeCommand::Respond {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            decision_id,
            response,
        } = command
        else {
            anyhow::bail!("not a decision response")
        };
        ensure!(!command_id.is_nil(), "nil command ID");
        let encoded = serde_json::to_string(command)?;
        ensure!(encoded.len() <= 8192, "decision response exceeds limit");
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // SQLite may have waited behind a writer. Current responder authority
        // and the deadline clock must be sampled after that wait. A later grant
        // revocation does not undo already committed consent or replay effects.
        let now = admit()?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((request, receipt)) = prior {
            ensure!(request == encoded, "command ID payload conflict");
            return Ok(serde_json::from_str(&receipt)?);
        }
        let collisions:i64=tx.query_row("SELECT (SELECT count(*) FROM commands WHERE id=?1)+(SELECT count(*) FROM steering WHERE id=?1)",[command_id.to_string()],|row|row.get(0))?;
        ensure!(collisions == 0, "command ID already used");
        let expiry = i64::try_from(*expires_at_ms)?;
        ensure!(
            expiry > now && expiry - now <= 300_000,
            "invalid response deadline"
        );
        ensure!(
            read_session(&tx, guard.session_id)?.revision == *expected_revision,
            "session revision conflict"
        );
        let run = read_run(&tx, *run_id)?;
        ensure!(
            run.session_id == guard.session_id && matches!(run.state, RunState::Running),
            "decision run is not active"
        );
        let cancelled: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM local_cancel_intents WHERE run_id=?1)",
            [run_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(!cancelled, "run cancellation already requested");
        let(actual_run,actual_incarnation,deadline,request,answered):(String,String,i64,String,bool)=tx.query_row("SELECT run_id,incarnation,expires,request,response IS NOT NULL FROM process_decisions WHERE id=?1",[decision_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
        ensure!(
            actual_run == run_id.to_string()
                && actual_incarnation == incarnation.to_string()
                && deadline > now
                && !answered,
            "stale, expired or already answered decision"
        );
        let request: Value = serde_json::from_str(&request)?;
        if request["kind"] == "approval" {
            ensure!(
                matches!(response.as_str(), Some("approved" | "denied")),
                "approval response must be approved or denied"
            );
        } else {
            let answer: crate::tools::QuestionAnswer = serde_json::from_value(response.clone())?;
            match answer {
                crate::tools::QuestionAnswer::Selected { index, answer } => ensure!(
                    request["question"]["options"][index].as_str() == Some(&answer),
                    "answer does not match option"
                ),
                crate::tools::QuestionAnswer::Custom { answer } => ensure!(
                    !answer.trim().is_empty()
                        && answer.len() <= 4096
                        && !answer.chars().any(char::is_control),
                    "invalid custom answer"
                ),
                _ => {}
            }
        }
        tx.execute(
            "UPDATE process_decisions SET response=?1 WHERE id=?2",
            params![serde_json::to_string(response)?, decision_id.to_string()],
        )?;
        let receipt = json!({"command_id":command_id,"decision_id":decision_id,"run_id":run_id,"status":"applied"});
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                encoded,
                serde_json::to_string(&receipt)?
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_authority_and_time_are_checked_inside_write_transaction() {
        let root = tempfile::tempdir().unwrap();
        let session = Session::new(root.path().into(), "fixture".into());
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_decisions(&guard).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        let run = journal
            .admit_turn(
                &guard,
                &TurnAdmission {
                    coordination: None,
                    operator_name: None,
                    command_id: Uuid::new_v4(),
                    machine_id: Uuid::new_v4(),
                    principal_id: Uuid::new_v4(),
                    session_id: session.id,
                    expected_revision: 0,
                    expires_at_ms: 10000,
                    prompt: "fixture".into(),
                    parts: vec![],
                },
                1,
            )
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        let incarnation = Uuid::new_v4();
        let decision = Uuid::new_v4();
        journal
            .create_decision(
                &guard,
                run.id,
                incarnation,
                decision,
                100,
                json!({"kind":"approval"}),
            )
            .unwrap();
        let revision = read_session(&journal.connection, session.id)
            .unwrap()
            .revision;
        let command = RuntimeCommand::Respond {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: 1000,
            run_id: run.id,
            decision_id: decision,
            response: json!("approved"),
        };
        let other = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
        other.busy_timeout(Duration::ZERO).unwrap();
        let rejected = journal
            .respond_decision(&guard, incarnation, &command, || {
                // Admission callback must run under the decision write transaction.
                assert!(other.execute_batch("BEGIN IMMEDIATE").is_err());
                anyhow::bail!("responder revoked while queued")
            })
            .unwrap_err();
        assert!(rejected.to_string().contains("responder revoked"));
        assert_eq!(journal.decision_response(decision, 2).unwrap(), None);
        let expired = journal
            .respond_decision(&guard, incarnation, &command, || Ok(100))
            .unwrap_err();
        assert!(expired.to_string().contains("expired"), "{expired:#}");
        assert_eq!(journal.decision_response(decision, 2).unwrap(), None);
        let receipt = journal
            .respond_decision(&guard, incarnation, &command, || Ok(99))
            .unwrap();
        assert_eq!(receipt["status"], "applied");
        assert_eq!(
            journal.decision_response(decision, 101).unwrap(),
            Some(json!("approved"))
        );
        assert_eq!(
            journal
                .respond_decision(&guard, incarnation, &command, || Ok(101))
                .unwrap(),
            receipt
        );
    }
}
