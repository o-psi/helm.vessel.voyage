//! Resources retained between runs remain obligations of the session process.
use super::*;
use serde_json::{Value, json};
impl Journal {
    /// Move abandoned obligations out of next-run admission, without confirming
    /// cleanup. The caller holds the dead owner's exclusive execution fence.
    /// All original identities and unknown states remain durable and inspectable.
    pub(crate) fn retain_interrupted_cleanup(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!active, "cannot retain cleanup of an active run");
        // Validate the canonical binding before transferring the row.
        catalogue::pending_cleanup(&tx, guard.session_id)?;
        tx.execute("INSERT INTO process_retained_cleanup SELECT * FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL", [guard.session_id.to_string()])?;
        tx.execute(
            "DELETE FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL",
            [guard.session_id.to_string()],
        )?;
        tx.execute("UPDATE process_session_resources SET state='retained_unknown' WHERE session_id=?1 AND state='cleanup_unknown'", [guard.session_id.to_string()])?;
        commit(tx, &self.commit_fence)
    }

    pub(crate) fn retained_cleanup(&self, session: Uuid) -> Result<Value> {
        let mut query = self.connection.prepare("SELECT run_id FROM process_retained_cleanup WHERE session_id=?1 AND confirmation IS NULL ORDER BY rowid")?;
        let runs = query
            .query_map([session.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut query = self.connection.prepare("SELECT id,run_id,kind FROM process_session_resources WHERE session_id=?1 AND state='retained_unknown' ORDER BY rowid")?;
        let resources = query.query_map([session.to_string()], |r| Ok(json!({"resource_id":r.get::<_,String>(0)?,"run_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"state":"cleanup_unknown"})))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"run_ids":runs,"resources":resources,"disposition":"unresolved_retained"}))
    }
    /// The caller holds the owner fence and has verified whole-process-tree
    /// cleanup. Remote participant obligations remain independently unresolved.
    pub(crate) fn recover_process_cleanup(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute("UPDATE process_session_resources SET state='observed' WHERE session_id=?1 AND kind='root_terminals' AND state='cleanup_unknown'", [guard.session_id.to_string()])?;
        if let Some(run) = catalogue::pending_cleanup(&self.connection, guard.session_id)? {
            if super::assignments::pending(&self.connection, run)? > 0 {
                self.connection.execute(
                    "INSERT OR IGNORE INTO process_assignment_local_cleanup VALUES(?1)",
                    [run.to_string()],
                )?;
            } else {
                self.confirm_local_cleanup_observed(guard, run)?;
            }
            // The confirmation method retains local observation when a remote
            // assignment remains. Never clear that assignment here.
        }
        Ok(())
    }
    pub(crate) fn initialize_session_resources(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_retained_cleanup(run_id TEXT PRIMARY KEY REFERENCES runs(id),session_id TEXT NOT NULL REFERENCES sessions(id),installation_id TEXT NOT NULL,principal_id TEXT NOT NULL,confirmation TEXT CHECK(confirmation IN ('observed','operator_attested')))")?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_session_resources(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,run_id TEXT NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL)")?;
        self.connection.execute_batch("CREATE TRIGGER IF NOT EXISTS process_observe_resource_create AFTER INSERT ON process_session_resources BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT id,'session_resource',revision,NEW.run_id,NEW.id FROM sessions WHERE id=NEW.session_id; END; CREATE TRIGGER IF NOT EXISTS process_observe_resource_update AFTER UPDATE ON process_session_resources BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT id,'session_resource',revision,NEW.run_id,NEW.id FROM sessions WHERE id=NEW.session_id; END;")?;
        self.connection.execute("UPDATE process_session_resources SET state='cleanup_unknown' WHERE session_id=?1 AND state='owned'",[guard.session_id.to_string()])?;
        Ok(())
    }
    pub(crate) fn session_resource_adopt(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        run: Uuid,
        kind: &str,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            !id.is_nil() && !kind.is_empty() && kind.len() <= 64,
            "invalid session resource"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = read_run(&tx, run)?;
        ensure!(
            owner.session_id == guard.session_id,
            "resource run mismatch"
        );
        let prior: Option<(String, String, String)> = tx
            .query_row(
                "SELECT run_id,kind,state FROM process_session_resources WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((old_run, old_kind, state)) = prior {
            ensure!(
                old_run == run.to_string() && old_kind == kind && state == "owned",
                "resource identity conflict"
            );
            return Ok(());
        }
        ensure!(
            pending(&tx, guard.session_id)? < 256,
            "session resource capacity reached"
        );
        tx.execute(
            "INSERT INTO process_session_resources VALUES(?1,?2,?3,?4,'owned')",
            params![
                id.to_string(),
                guard.session_id.to_string(),
                run.to_string(),
                kind
            ],
        )?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn session_resource_closed(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(self.connection.execute("UPDATE process_session_resources SET state='observed' WHERE id=?1 AND session_id=?2 AND state='owned'",params![id.to_string(),guard.session_id.to_string()])?==1,"resource not owned by current process");
        Ok(())
    }
    pub(crate) fn attest_session_resource(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        actor: super::super::local_actor::LocalActor,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_resource_attestations(id TEXT PRIMARY KEY,installation_id TEXT NOT NULL,principal_id TEXT NOT NULL)")?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: String = tx.query_row(
            "SELECT state FROM process_session_resources WHERE id=?1 AND session_id=?2",
            params![id.to_string(), guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            state == "cleanup_unknown"
                || state == "retained_unknown"
                || state == "operator_attested",
            "resource is not an interrupted obligation"
        );
        if state == "operator_attested" {
            let same:bool=tx.query_row("SELECT installation_id=?1 AND principal_id=?2 FROM process_resource_attestations WHERE id=?3",params![actor.installation_id.to_string(),actor.principal_id.to_string(),id.to_string()],|r|r.get(0))?;
            ensure!(same, "resource attestation actor mismatch");
            return Ok(());
        }
        tx.execute(
            "INSERT INTO process_resource_attestations VALUES(?1,?2,?3)",
            params![
                id.to_string(),
                actor.installation_id.to_string(),
                actor.principal_id.to_string()
            ],
        )?;
        tx.execute(
            "UPDATE process_session_resources SET state='operator_attested' WHERE id=?1",
            [id.to_string()],
        )?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn session_resources(&self, session: Uuid) -> Result<Value> {
        let mut query=self.connection.prepare("SELECT id,run_id,kind,state FROM process_session_resources WHERE session_id=?1 AND state NOT IN ('observed','operator_attested','retained_unknown') ORDER BY rowid LIMIT 256")?;
        let rows=query.query_map([session.to_string()],|r|Ok(json!({"resource_id":r.get::<_,String>(0)?,"run_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"state":r.get::<_,String>(3)?})))?;
        Ok(json!(rows.collect::<rusqlite::Result<Vec<_>>>()?))
    }
}
pub(super) fn pending(db: &Connection, session: Uuid) -> Result<i64> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_session_resources')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(0);
    }
    Ok(db.query_row("SELECT count(*) FROM process_session_resources WHERE session_id=?1 AND state NOT IN ('observed','operator_attested','retained_unknown')",[session.to_string()],|r|r.get(0))?)
}
