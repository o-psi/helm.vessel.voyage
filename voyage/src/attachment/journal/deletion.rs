//! Deletion retains only identity/delivery evidence; request comparison uses hashes.
use super::*;
use serde_json::json;
pub(super) fn request_digest(request: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(request.as_bytes())))
}
pub(super) fn request_matches(saved: &str, request: &str) -> bool {
    saved == request || saved == request_digest(request)
}
pub(super) fn scrub(tx: &Transaction<'_>, session: Uuid) -> Result<()> {
    let mut query = tx.prepare("SELECT id,request,receipt FROM process_commands")?;
    let rows = query.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (id, request, receipt) = row?;
        let receipt: serde_json::Value = serde_json::from_str(&receipt)?;
        let retained = json!({"command_id":id,"status":"deleted","original_status":receipt["status"],"run_id":receipt.get("run_id"),"result_sha256":hex::encode(Sha256::digest(serde_json::to_vec(&receipt)?))});
        tx.execute(
            "UPDATE process_commands SET request=?1,receipt=?2 WHERE id=?3",
            params![
                request_digest(&request),
                serde_json::to_string(&retained)?,
                id
            ],
        )?;
    }
    drop(query);
    let mut query =
        tx.prepare("SELECT id,request FROM process_command_bindings WHERE request IS NOT NULL")?;
    let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, request) = row?;
        tx.execute(
            "UPDATE process_command_bindings SET request=?1 WHERE id=?2",
            params![request_digest(&request), id],
        )?;
    }
    drop(query);
    tx.execute("DELETE FROM remote_events", [])?;
    tx.execute(
        "UPDATE steering SET record=json_set(record,'$.request.text','') WHERE session_id=?1",
        [session.to_string()],
    )?;
    tx.execute("UPDATE process_decisions SET request='{}',response=NULL WHERE run_id IN (SELECT id FROM runs WHERE session_id=?1)",[session.to_string()])?;
    tx.execute(
        "UPDATE process_branches SET snapshot='',configuration=NULL WHERE source_session_id=?1",
        [session.to_string()],
    )?;
    for table in ["process_assignments", "process_configuration"] {
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
            [table],
            |r| r.get(0),
        )?;
        if exists {
            match table {
                "process_assignments" => {
                    tx.execute(
                        "UPDATE process_assignments SET request='',observation=NULL",
                        [],
                    )?;
                }
                "process_configuration" => {
                    tx.execute(
                        "DELETE FROM process_configuration WHERE session_id=?1",
                        [session.to_string()],
                    )?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}
impl Journal {
    pub(crate) fn finish_deletion(&mut self, guard: &ExecutionGuard, command: Uuid) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.lifecycle_status(guard.session_id)?["deleted"] == true,
            "session is not deleted"
        );
        self.remove_managed_originals(guard.session_id)?;
        // The supervisor assigns a dedicated directory per session. Never traverse
        // a payload-provided path while removing retained originals/resources.
        for path in [
            self.directory.join("imports"),
            self.directory.join("images"),
            self.directory.join("artifacts"),
            self.directory
                .parent()
                .context("journal parent missing")?
                .join("resources"),
        ] {
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "unsafe deletion resource directory"
                    );
                    fs::remove_dir_all(&path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.connection
            .execute_batch("PRAGMA secure_delete=ON; PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt: String = tx.query_row(
            "SELECT receipt FROM process_commands WHERE id=?1",
            [command.to_string()],
            |r| r.get(0),
        )?;
        let mut receipt: serde_json::Value = serde_json::from_str(&receipt)?;
        receipt["status"] = json!("applied");
        receipt["cleanup"] = json!("observed");
        tx.execute(
            "UPDATE process_commands SET receipt=?1 WHERE id=?2",
            params![serde_json::to_string(&receipt)?, command.to_string()],
        )?;
        commit(tx, &self.commit_fence)?;
        File::open(&self.directory)?.sync_all()?;
        Ok(())
    }
}
