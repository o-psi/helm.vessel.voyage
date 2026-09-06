//! Parent-owned assignment obligations survive ambiguous remote delivery and cleanup.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::{AssignmentObservation, AssignmentRequest};
impl Journal {
    pub(crate) fn initialize_assignments(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_assignments(id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id),principal TEXT NOT NULL,participant TEXT NOT NULL,request TEXT NOT NULL,state TEXT NOT NULL,observation TEXT,result_digest TEXT,cleanup INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS process_assignment_local_cleanup(run_id TEXT PRIMARY KEY REFERENCES runs(id))")?;
        Ok(())
    }
    pub(crate) fn record_assignment(
        &mut self,
        guard: &ExecutionGuard,
        actor: Uuid,
        participant: Uuid,
        request: &AssignmentRequest,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            request.parent_session_id == guard.session_id
                && !request.assignment_id.is_nil()
                && !participant.is_nil(),
            "assignment identity mismatch"
        );
        let encoded = serde_json::to_string(request)?;
        ensure!(
            encoded.len() <= 256 * 1024 && !request.task.trim().is_empty(),
            "assignment context exceeds limit or task is empty"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String, String, String)> = tx
            .query_row(
                "SELECT principal,participant,request,state FROM process_assignments WHERE id=?1",
                [request.assignment_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        if let Some((principal, vessel, payload, state)) = prior {
            ensure!(
                principal == actor.to_string()
                    && vessel == participant.to_string()
                    && payload == encoded,
                "assignment ID payload or participant conflict"
            );
            return Ok(
                json!({"assignment_id":request.assignment_id,"state":state,"duplicate":true}),
            );
        }
        let run = read_run(&tx, request.parent_run_id)?;
        ensure!(
            run.session_id == guard.session_id && matches!(run.state, RunState::Running),
            "assignment requires active parent run"
        );
        let ambiguous:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM process_assignments WHERE run_id=?1 AND state='acceptance_unknown')",[run.id.to_string()],|r|r.get(0))?;
        ensure!(
            !ambiguous,
            "resolve uncertain assignment acceptance before assigning more work"
        );
        ensure!(
            request.expires_at_ms
                > u64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)?
                        .as_millis()
                )?,
            "assignment expired"
        );
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM process_assignments WHERE run_id=?1",
            [run.id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(count < 256, "parent assignment capacity exceeded");
        tx.execute("INSERT INTO process_assignments(id,run_id,principal,participant,request,state) VALUES(?1,?2,?3,?4,?5,'acceptance_unknown')",params![request.assignment_id.to_string(),run.id.to_string(),actor.to_string(),participant.to_string(),encoded])?;
        commit(tx, &self.commit_fence)?;
        Ok(
            json!({"assignment_id":request.assignment_id,"state":"acceptance_unknown","duplicate":false}),
        )
    }
    pub(crate) fn update_assignment(
        &mut self,
        guard: &ExecutionGuard,
        observation: &AssignmentObservation,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            observation.parent_session_id == guard.session_id,
            "assignment parent mismatch"
        );
        let encoded = serde_json::to_string(observation)?;
        ensure!(
            encoded.len() <= 1024 * 1024,
            "assignment observation exceeds limit"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (run,participant,old,old_digest,clean):(String,String,Option<String>,Option<String>,bool)=tx.query_row("SELECT run_id,participant,observation,result_digest,cleanup FROM process_assignments WHERE id=?1",[observation.assignment_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            run == observation.parent_run_id.to_string()
                && participant == observation.participant_vessel_id.to_string()
                && !observation.child_session_id.is_nil(),
            "assignment observation attribution mismatch"
        );
        let terminal = matches!(
            observation.state.as_str(),
            "completed" | "cancelled" | "failed" | "interrupted" | "incomplete" | "rejected"
        );
        ensure!(
            !observation.cleanup_observed || terminal,
            "nonterminal assignment cannot claim final cleanup"
        );
        let digest = observation
            .result
            .as_ref()
            .map(|result| {
                serde_json::to_vec(result).map(|bytes| hex::encode(Sha256::digest(bytes)))
            })
            .transpose()?;
        if let Some(previous) = old {
            let previous: AssignmentObservation = serde_json::from_str(&previous)?;
            ensure!(
                previous.child_session_id == observation.child_session_id
                    && previous
                        .child_incarnation
                        .is_none_or(|id| Some(id) == observation.child_incarnation)
                    && previous
                        .run_id
                        .is_none_or(|id| Some(id) == observation.run_id),
                "assignment child identity changed"
            );
            ensure!(
                !clean || observation.cleanup_observed,
                "observed cleanup cannot regress"
            );
            if matches!(
                previous.state.as_str(),
                "completed" | "cancelled" | "failed" | "interrupted" | "incomplete" | "rejected"
            ) {
                ensure!(
                    previous.state == observation.state,
                    "terminal assignment state changed"
                );
            }
        }
        if let Some(old) = old_digest {
            ensure!(
                Some(&old) == digest.as_ref(),
                "assignment result changed after acceptance"
            );
        }
        let state = if terminal && observation.cleanup_observed {
            observation.state.as_str()
        } else if terminal {
            "cleanup_unknown"
        } else if observation.run_id.is_some() {
            "accepted"
        } else {
            "acceptance_unknown"
        };
        tx.execute("UPDATE process_assignments SET state=?1,observation=?2,result_digest=?3,cleanup=?4 WHERE id=?5",params![state,encoded,digest,observation.cleanup_observed,observation.assignment_id.to_string()])?;
        commit(tx, &self.commit_fence)?;
        let local: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM process_assignment_local_cleanup WHERE run_id=?1)",
            [run.clone()],
            |r| r.get(0),
        )?;
        if local && pending(&self.connection, observation.parent_run_id)? == 0 {
            self.confirm_local_cleanup_observed(guard, observation.parent_run_id)?;
        }
        Ok(
            json!({"assignment_id":observation.assignment_id,"state":state,"cleanup_observed":observation.cleanup_observed,"participant_vessel_id":observation.participant_vessel_id,"result_sha256":digest}),
        )
    }
    pub(crate) fn assignment_request(
        &self,
        run: Uuid,
        id: Uuid,
    ) -> Result<Option<AssignmentRequest>> {
        let request: Option<String> = self
            .connection
            .query_row(
                "SELECT request FROM process_assignments WHERE run_id=?1 AND id=?2",
                params![run.to_string(), id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        request
            .map(|request| serde_json::from_str(&request).map_err(Into::into))
            .transpose()
    }
    pub(crate) fn assignment_observations(&self, run: Uuid) -> Result<Value> {
        let mut query = self.connection.prepare("SELECT id,participant,state,result_digest,cleanup FROM process_assignments WHERE run_id=?1 ORDER BY rowid LIMIT 256")?;
        let rows = query.query_map([run.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (id, participant, state, digest, cleanup) = row?;
            result.push(json!({"assignment_id":id,"participant_vessel_id":participant,"state":state,"cleanup_observed":cleanup,"result_sha256":digest}));
        }
        Ok(json!(result))
    }
    pub(crate) fn assignment_result(&self, run: Uuid, id: Uuid) -> Result<Value> {
        let observation: Option<String> = self.connection.query_row(
            "SELECT observation FROM process_assignments WHERE id=?1 AND run_id=?2",
            params![id.to_string(), run.to_string()],
            |r| r.get(0),
        )?;
        observation
            .map(|encoded| {
                ensure!(
                    encoded.len() <= 1024 * 1024,
                    "assignment result exceeds limit"
                );
                Ok(serde_json::from_str::<Value>(&encoded)?)
            })
            .transpose()
            .map(|value| value.unwrap_or(Value::Null))
    }
}
pub(super) fn pending(db: &Connection, run: Uuid) -> Result<i64> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_assignments')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    Ok(db.query_row(
        "SELECT count(*) FROM process_assignments WHERE run_id=?1 AND cleanup=0",
        [run.to_string()],
        |r| r.get(0),
    )?)
}
