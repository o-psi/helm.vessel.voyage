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
    // Retired remote-worker journals may retain this table; fresh independent
    // Voyage journals never create it. Scrub legacy content without requiring or
    // recreating retired infrastructure, and propagate real database failures.
    let legacy_events: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='remote_events')",
        [],
        |r| r.get(0),
    )?;
    if legacy_events {
        tx.execute("DELETE FROM remote_events", [])?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::process::RuntimeCommand;

    #[test]
    fn delete_fresh_and_legacy_journals_preserves_receipt_identity() {
        for legacy in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut journal = Journal::open(root.path().join("journal")).unwrap();
            let mut session =
                crate::session::Session::new(root.path().to_path_buf(), "fixture".into());
            session.messages = vec![crate::model::Message::new(
                crate::model::Role::User,
                "private fixture text",
            )];
            journal.create_session(&session).unwrap();
            let guard = journal.acquire_execution(session.id).unwrap();
            journal.initialize_lifecycle(&guard).unwrap();
            journal.initialize_process_commands(&guard).unwrap();
            journal
                .initialize_command_bindings(&guard, Uuid::new_v4())
                .unwrap();
            journal.initialize_decisions(&guard).unwrap();
            if legacy {
                journal.connection.execute_batch("CREATE TABLE remote_events(payload TEXT); INSERT INTO remote_events VALUES('legacy private text');").unwrap();
            }
            let command = RuntimeCommand::Delete {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: (chrono::Utc::now().timestamp_millis() + 60000) as u64,
                confirm_session_id: session.id,
            };
            let receipt = journal
                .apply_lifecycle(
                    &guard,
                    &command,
                    chrono::Utc::now().timestamp_millis(),
                    None,
                )
                .unwrap();
            assert_eq!(receipt["deleted"], true);
            assert!(
                journal
                    .load_session(session.id)
                    .unwrap()
                    .session
                    .messages
                    .is_empty()
            );
            assert_eq!(
                journal
                    .apply_lifecycle(
                        &guard,
                        &command,
                        chrono::Utc::now().timestamp_millis(),
                        None
                    )
                    .unwrap(),
                receipt
            );
            let exists: bool = journal.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='remote_events')", [], |r| r.get(0)).unwrap();
            assert_eq!(exists, legacy);
            if legacy {
                let count: i64 = journal
                    .connection
                    .query_row("SELECT count(*) FROM remote_events", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(count, 0);
            }
        }
    }
}
