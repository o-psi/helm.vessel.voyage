//! One-time, fenced extraction of a session from a legacy shared journal.
use super::*;
use serde_json::json;
impl Journal {
    pub(crate) fn import_managed(
        &mut self,
        source_directory: &Path,
        session: Uuid,
        transfer: Uuid,
        revision: u64,
        workspace: &Path,
        allow_remote: bool,
    ) -> Result<()> {
        if self.json_import_finalized(session, transfer)? {
            let provenance: String = self.connection.query_row(
                "SELECT provenance FROM managed_import WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )?;
            let marker: serde_json::Value = serde_json::from_str(&provenance)?;
            ensure!(
                marker["transfer_id"] == transfer.to_string()
                    && marker["expected_revision"] == revision
                    && marker["destination"].as_str() == self.directory.to_str(),
                "finalized managed import provenance mismatch"
            );
            ensure!(
                self.load_session(session)?
                    .session
                    .workspace
                    .canonicalize()?
                    == workspace,
                "managed import workspace changed"
            );
            return Ok(());
        }
        let source_directory = source_directory.canonicalize()?;
        ensure!(
            source_directory != self.directory.canonicalize()?,
            "source and destination journal coincide"
        );
        let mut source = Journal::open(source_directory.clone())?;
        let runtime_tables: bool = source.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_commands')",
            [],
            |r| r.get(0),
        )?;
        if runtime_tables {
            let receipts: i64 =
                source
                    .connection
                    .query_row("SELECT count(*) FROM process_commands", [], |r| r.get(0))?;
            ensure!(
                receipts == 0,
                "already-supervised journals use owner transfer, not legacy managed import"
            );
        }

        // Retry must acquire the exact legacy fence even after its canonical row
        // becomes a retirement marker that old executors cannot decode.
        let lock = open_private_file(&source_directory.join(format!("{session}.execution.lock")))?;
        lock.try_lock()
            .context("source session execution still owned")?;
        let marker = json!({"format":"voyage.managed-retired","transfer_id":transfer,"session_id":session,"destination":self.directory.canonicalize()?,"source_directory":source_directory,"expected_revision":revision});
        let raw: String = source.connection.query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [session.to_string()],
            |r| r.get(0),
        )?;
        let retired = serde_json::from_str::<serde_json::Value>(&raw)? == marker;
        if !retired {
            let saved = source.load_session(session)?;
            ensure!(
                saved.session.messages.iter().all(|m| m
                    .tool_output
                    .as_ref()
                    .is_none_or(|o| o.artifacts().next().is_none())),
                "Legacy managed import cannot transfer tool artifacts; continue on the owning Vessel"
            );
            ensure!(
                saved.revision == revision && saved.session.workspace.canonicalize()? == workspace,
                "managed source revision or workspace changed"
            );
            let active: bool = source.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
                [session.to_string()],
                |r| r.get(0),
            )?;
            ensure!(
                !active
                    && catalogue::pending_cleanup(&source.connection, session)?.is_none()
                    && session_resources::pending(&source.connection, session)? == 0,
                "managed source requires observed cleanup"
            );
            let remote: bool = source.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM remote_session WHERE session_id=?1)",
                [session.to_string()],
                |r| r.get(0),
            )?;
            ensure!(
                !remote || allow_remote,
                "remote relay journals require outbound migration"
            );
            if remote {
                let count: i64 =
                    source
                        .connection
                        .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
                ensure!(count == 1, "outbound migration requires dedicated journal");
            }
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS managed_import(session_id TEXT PRIMARY KEY,provenance TEXT NOT NULL)")?;
        let prior: Option<String> = self
            .connection
            .query_row(
                "SELECT provenance FROM managed_import WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            ensure!(
                serde_json::from_str::<serde_json::Value>(&prior)? == marker,
                "managed import provenance conflict"
            );
        } else {
            ensure!(!retired, "retired source has no prepared destination");
            let count: i64 =
                self.connection
                    .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
            ensure!(count == 0, "managed import destination is occupied");
            self.connection.execute(
                "ATTACH DATABASE ?1 AS legacy",
                [source_directory
                    .join("journal.sqlite3")
                    .to_str()
                    .context("non-UTF8 journal path")?],
            )?;
            let copied = (|| -> Result<()> {
                let tx = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                tx.execute(
                    "INSERT INTO sessions SELECT * FROM legacy.sessions WHERE id=?1",
                    [session.to_string()],
                )?;
                tx.execute(
                    "INSERT INTO runs SELECT * FROM legacy.runs WHERE session_id=?1",
                    [session.to_string()],
                )?;
                tx.execute("INSERT INTO commands SELECT * FROM legacy.commands WHERE run_id IN (SELECT id FROM runs)",[])?;
                for table in [
                    "events",
                    "steering",
                    "local_cancel_intents",
                    "local_cleanup_obligations",
                    "local_tool_reconciliations",
                    "imports",
                ] {
                    tx.execute(
                        &format!(
                            "INSERT INTO {table} SELECT * FROM legacy.{table} WHERE session_id=?1"
                        ),
                        [session.to_string()],
                    )?;
                }
                if allow_remote {
                    for table in [
                        "remote_session",
                        "remote_cleanup_attestations",
                        "remote_text",
                        "remote_receipts",
                        "remote_events",
                        "remote_tools",
                        "remote_withdrawal",
                    ] {
                        tx.execute(
                            &format!("INSERT INTO {table} SELECT * FROM legacy.{table}"),
                            [],
                        )?;
                    }
                }
                // These runtime tables are absent from older journals; when present
                // preserve only rows attributed to the selected canonical session.
                for (table, filter) in [
                    ("process_decisions", "run_id IN (SELECT id FROM runs)"),
                    ("process_assignments", "run_id IN (SELECT id FROM runs)"),
                    (
                        "process_assignment_local_cleanup",
                        "run_id IN (SELECT id FROM runs)",
                    ),
                    (
                        "process_configuration",
                        "session_id IN (SELECT id FROM sessions)",
                    ),
                ] {
                    let schema: Option<String> = tx
                        .query_row(
                            "SELECT sql FROM legacy.sqlite_master WHERE type='table' AND name=?1",
                            [table],
                            |r| r.get(0),
                        )
                        .optional()?;
                    if let Some(schema) = schema {
                        tx.execute_batch(&schema)?;
                        tx.execute(
                            &format!(
                                "INSERT INTO {table} SELECT * FROM legacy.{table} WHERE {filter}"
                            ),
                            [],
                        )?;
                    }
                }
                tx.execute(
                    "INSERT INTO managed_import VALUES(?1,?2)",
                    params![session.to_string(), serde_json::to_string(&marker)?],
                )?;
                commit(tx, &self.commit_fence)
            })();
            self.connection.execute_batch("DETACH DATABASE legacy")?;
            copied?;
        }
        // Destination is durable but not yet opened by the runtime. Retiring the
        // source is the authority handoff; any old executable fails to decode it.
        source
            .connection
            .pragma_update(None, "secure_delete", "ON")?;
        let tx = source
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !retired {
            ensure!(
                tx.execute(
                    "UPDATE sessions SET state=?1 WHERE id=?2 AND revision=?3",
                    params![
                        serde_json::to_string(&marker)?,
                        session.to_string(),
                        i64::try_from(revision)?
                    ]
                )? == 1,
                "managed source revision changed"
            );
        }
        tx.execute(
            "DELETE FROM events WHERE session_id=?1",
            [session.to_string()],
        )?;
        tx.execute(
            "UPDATE runs SET record=json_set(record,'$.partial_text','') WHERE session_id=?1",
            [session.to_string()],
        )?;
        tx.execute(
            "UPDATE steering SET record=json_set(record,'$.request.text','') WHERE session_id=?1",
            [session.to_string()],
        )?;
        if allow_remote {
            tx.execute("DELETE FROM remote_events", [])?;
        }
        let decisions: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_decisions')",
            [],
            |r| r.get(0),
        )?;
        if decisions {
            tx.execute("UPDATE process_decisions SET request='{}',response=NULL WHERE run_id IN (SELECT id FROM runs WHERE session_id=?1)",[session.to_string()])?;
        }
        commit(tx, &source.commit_fence)?;
        File::open(&source_directory)?.sync_all()?;
        self.finalize_json_import(session, transfer)?;
        Ok(())
    }
    pub(crate) fn remove_managed_originals(&self, session: Uuid) -> Result<()> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='managed_import')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(());
        }
        let provenance: Option<String> = self
            .connection
            .query_row(
                "SELECT provenance FROM managed_import WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(provenance) = provenance else {
            return Ok(());
        };
        let marker: serde_json::Value = serde_json::from_str(&provenance)?;
        let source = PathBuf::from(
            marker["source_directory"]
                .as_str()
                .context("managed import source missing")?,
        );
        ensure!(
            source.is_absolute() && marker["destination"].as_str() == self.directory.to_str(),
            "managed import cleanup provenance mismatch"
        );
        match fs::symlink_metadata(&source) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "unsafe retired source directory"
            ),
        }
        let journal = Journal::open(source.clone())?;
        let lock = open_private_file(&source.join(format!("{session}.execution.lock")))?;
        lock.try_lock()
            .context("managed import source fence busy")?;
        let saved: String = journal.connection.query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [session.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            serde_json::from_str::<serde_json::Value>(&saved)? == marker,
            "managed source retirement changed"
        );
        let original: Option<String> = journal
            .connection
            .query_row(
                "SELECT transfer_id FROM imports WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(original) = original {
            let original = Uuid::parse_str(&original)?;
            let path = source.join("imports").join(format!("{original}.json"));
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    ensure!(
                        metadata.is_file() && !metadata.file_type().is_symlink(),
                        "unsafe retained managed original"
                    );
                    fs::remove_file(&path)?;
                    File::open(path.parent().context("imports parent missing")?)?.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}
