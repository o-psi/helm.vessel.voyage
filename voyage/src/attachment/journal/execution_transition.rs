//! Retired handoff is metadata/configuration bookkeeping, never execution or
//! cleanup attestation. Only identity-scoped private helpers call this module.
use super::*;
use sha2::{Digest, Sha256};
use voyage_protocol::execution_transition::*;
const TABLE: &str = "execution_transitions";
const MAX_HASH_BYTES: usize = 64 * 1024 * 1024;
const MAX_HASH_ROWS: usize = 100_000;
fn hash(domain: &[u8], bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(bytes);
    hex::encode(h.finalize())
}
fn valid_hash(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn table(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )?)
}
fn settings(db: &Connection, id: Uuid) -> Result<String> {
    ensure!(
        table(db, "process_configuration")?,
        "frozen configuration unavailable"
    );
    Ok(db.query_row("SELECT settings FROM process_configuration WHERE session_id=?1 AND length(CAST(settings AS BLOB))<=1048576",[id.to_string()],|r|r.get(0))?)
}
fn pending_digest(db: &Connection) -> Result<String> {
    let mut query =
        db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")?;
    let names = query
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ensure!(names.len() <= 128, "journal table bound exceeded");
    let mut digest = Sha256::new();
    digest.update(b"voyage/retired-pending-work/v1\0");
    let mut bytes = 0usize;
    let mut rows = 0usize;
    for name in names {
        if name.starts_with("sqlite_")
            || name.starts_with("notification_")
            || matches!(
                name.as_str(),
                "sessions"
                    | "process_configuration"
                    | "events"
                    | "process_observations"
                    | "process_observation_pruned"
                    | TABLE
            )
        {
            continue;
        }
        ensure!(
            !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
            "unsupported journal table identity"
        );
        digest.update((name.len() as u64).to_be_bytes());
        digest.update(name.as_bytes());
        let mut statement = db.prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))?;
        let columns = statement.column_count();
        digest.update((columns as u64).to_be_bytes());
        for column in statement.column_names() {
            digest.update((column.len() as u64).to_be_bytes());
            digest.update(column.as_bytes());
        }
        let mut cursor = statement.query([])?;
        while let Some(row) = cursor.next()? {
            rows += 1;
            ensure!(rows <= MAX_HASH_ROWS, "journal row bound exceeded");
            digest.update([0xff]);
            for i in 0..columns {
                use rusqlite::types::ValueRef;
                let (tag, value): (u8, Vec<u8>) = match row.get_ref(i)? {
                    ValueRef::Null => (0, vec![]),
                    ValueRef::Integer(v) => (1, v.to_be_bytes().to_vec()),
                    ValueRef::Real(v) => (2, v.to_bits().to_be_bytes().to_vec()),
                    ValueRef::Text(v) => {
                        ensure!(
                            v.len() <= MAX_HASH_BYTES.saturating_sub(bytes),
                            "journal digest byte bound exceeded"
                        );
                        (3, v.to_vec())
                    }
                    ValueRef::Blob(v) => {
                        ensure!(
                            v.len() <= MAX_HASH_BYTES.saturating_sub(bytes),
                            "journal digest byte bound exceeded"
                        );
                        (4, v.to_vec())
                    }
                };
                bytes = bytes
                    .checked_add(value.len())
                    .context("journal digest size overflow")?;
                ensure!(
                    bytes <= MAX_HASH_BYTES,
                    "journal digest byte bound exceeded"
                );
                digest.update([tag]);
                digest.update((value.len() as u64).to_be_bytes());
                digest.update(value);
            }
        }
    }
    Ok(hex::encode(digest.finalize()))
}
fn facts(db: &Connection, session: Uuid, incarnation: Uuid) -> Result<RetiredJournalFacts> {
    ensure!(
        db.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))? == 1,
        "handoff requires a dedicated runtime journal"
    );
    let saved = read_session(db, session)?;
    let cfg = settings(db, session)?;
    let state: String = db.query_row(
        "SELECT state FROM sessions WHERE id=?1",
        [session.to_string()],
        |r| r.get(0),
    )?;
    let mut pending = std::collections::BTreeSet::new();
    let mut query=db.prepare("SELECT id,record,active FROM runs WHERE session_id=?1 AND (active=1 OR json_extract(record, '$.terminal_reason')='execution identity handoff; retained work requires reconciliation') ORDER BY rowid")?;
    for row in query.query_map([session.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (id, encoded, active) = row?;
        let run: RunRecord = serde_json::from_str(&encoded)?;
        ensure!(
            run.session_id == session
                && (active == 0
                    || run
                        .source_incarnation
                        .is_none_or(|source| source == incarnation)),
            "retired run incarnation differs"
        );
        pending.insert(Uuid::parse_str(&id)?);
    }
    if table(db, "process_session_resources")? {
        let mut query=db.prepare("SELECT id FROM process_session_resources WHERE session_id=?1 AND state NOT IN ('observed','operator_attested') ORDER BY rowid")?;
        for id in query.query_map([session.to_string()], |r| r.get::<_, String>(0))? {
            pending.insert(Uuid::parse_str(&id?)?);
        }
    }
    if table(db, "process_assignments")? {
        let mut query=db.prepare("SELECT a.id FROM process_assignments a JOIN runs r ON r.id=a.run_id WHERE r.session_id=?1 AND a.cleanup<>1 ORDER BY a.rowid")?;
        for id in query.query_map([session.to_string()], |r| r.get::<_, String>(0))? {
            pending.insert(Uuid::parse_str(&id?)?);
        }
    }
    ensure!(
        pending.len() <= 512,
        "retained cleanup identity bound exceeded"
    );
    Ok(RetiredJournalFacts {
        session_id: session,
        source_incarnation: incarnation,
        revision: saved.revision,
        frozen_config_digest: crate::identity_helper::config_digest(cfg.as_bytes()),
        history_digest: hash(b"voyage/retired-canonical-history/v1\0", state.as_bytes()),
        pending_work_digest: pending_digest(db)?,
        unresolved_cleanup: pending.into_iter().collect(),
    })
}
struct StoredTransition {
    source_digest: String,
    prepared: PreparedTransitionReceipt,
    completed: Option<TransitionReceipt>,
    commit_digest: Option<String>,
    aborted: Option<AbortedTransitionReceipt>,
    abort_digest: Option<String>,
}
fn stored(db: &Connection, command: Uuid) -> Result<Option<StoredTransition>> {
    if !table(db, TABLE)? {
        return Ok(None);
    }
    type StoredRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let row:Option<StoredRow>=db.query_row("SELECT request_digest,prepared_receipt,target_receipt,commit_request_digest,abort_receipt,abort_request_digest FROM execution_transitions WHERE command_id=?1 OR commit_command_id=?1 OR abort_command_id=?1",[command.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    row.map(
        |(digest, prepared, completed, commit_digest, aborted, abort_digest)| {
            Ok(StoredTransition {
                source_digest: digest,
                prepared: serde_json::from_str(&prepared)?,
                completed: completed.map(|c| serde_json::from_str(&c)).transpose()?,
                commit_digest,
                aborted: aborted.map(|v| serde_json::from_str(&v)).transpose()?,
                abort_digest,
            })
        },
    )
    .transpose()
}
fn collisions(db: &Connection, id: Uuid) -> Result<()> {
    for (name, column) in [
        ("commands", "id"),
        ("process_commands", "id"),
        ("process_command_bindings", "id"),
        ("steering", "id"),
        ("process_goal_turns", "command_id"),
        ("process_goal_meters", "command_id"),
        ("process_goal_settlements", "command_id"),
        ("process_transfers", "command_id"),
    ] {
        if table(db, name)? {
            ensure!(
                !db.query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM {name} WHERE {column}=?1)"),
                    [id.to_string()],
                    |r| r.get::<_, bool>(0)
                )?,
                "handoff command collides with prior work"
            );
        }
    }
    Ok(())
}
fn source_directory(path: &Path, prepared: &PreparedTransitionReceipt) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        unsafe { libc::geteuid() } == prepared.source_uid
            && unsafe { libc::getegid() } == prepared.source_gid
            && metadata.is_dir()
            && metadata.uid() == prepared.source_uid
            && metadata.dev() == prepared.source_directory_device
            && metadata.ino() == prepared.source_directory_inode,
        "abort requires original source identity and directory"
    );
    Ok(())
}
fn checked_aborted(
    db: &Connection,
    path: &Path,
    receipt: AbortedTransitionReceipt,
) -> Result<TransitionResponse> {
    source_directory(path, &receipt.prepared)?;
    let current = facts(
        db,
        receipt.prepared.session_id,
        receipt.prepared.source_incarnation,
    )?;
    ensure!(
        current.revision == receipt.resulting_revision
            && current.frozen_config_digest == receipt.prepared.previous_config_digest
            && current.history_digest == receipt.history_digest
            && current.pending_work_digest == receipt.pending_work_digest,
        "aborted source differs from receipt"
    );
    Ok(TransitionResponse::Aborted { receipt })
}
fn checked_completed(db: &Connection, receipt: TransitionReceipt) -> Result<TransitionResponse> {
    ensure!(
        unsafe { libc::geteuid() } == receipt.prepared.target_uid
            && unsafe { libc::getegid() } == receipt.prepared.target_gid,
        "committed receipt requires its target identity"
    );
    let current = facts(
        db,
        receipt.prepared.session_id,
        receipt.prepared.source_incarnation,
    )?;
    ensure!(
        current.revision == receipt.resulting_revision
            && current.frozen_config_digest == receipt.config_digest
            && current.history_digest == receipt.history_digest
            && current.pending_work_digest == receipt.pending_work_digest,
        "committed journal differs from receipt"
    );
    Ok(TransitionResponse::Committed { receipt })
}
fn retired(db: &Connection, session: Uuid) -> Result<()> {
    ensure!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
            [session.to_string()],
            |r| r.get::<_, bool>(0)
        )?,
        "active work remains during target handoff"
    );
    if table(db, "steering")? {
        ensure!(!db.query_row("SELECT EXISTS(SELECT 1 FROM steering s JOIN runs r ON r.id=s.run_id WHERE r.session_id=?1 AND s.status='queued')",[session.to_string()],|r|r.get::<_,bool>(0))?,"queued work remains during target handoff");
    }
    if table(db, "process_goals")? {
        let encoded: Option<Option<String>> = db
            .query_row(
                "SELECT state FROM process_goals WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(Some(encoded)) = encoded {
            let goal: voyage_protocol::goals::Goal = serde_json::from_str(&encoded)?;
            ensure!(
                !goal.continuation_authorized
                    && goal.status != voyage_protocol::goals::GoalStatus::Active,
                "goal continuation remains authorized"
            );
        }
    }
    Ok(())
}
fn interrupt_work(tx: &Transaction<'_>, session: Uuid, revision: u64) -> Result<()> {
    if table(tx, "steering")? {
        let mut query=tx.prepare("SELECT DISTINCT r.id FROM runs r JOIN steering s ON s.run_id=r.id WHERE r.session_id=?1 AND s.status='queued'")?;
        let ids = query
            .query_map([session.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(query);
        for id in ids {
            steering::settle(
                tx,
                &read_run(tx, Uuid::parse_str(&id)?)?,
                &RunState::Interrupted,
                revision,
            )?;
        }
    }
    let mut query =
        tx.prepare("SELECT id FROM runs WHERE session_id=?1 AND active=1 ORDER BY rowid")?;
    let ids = query
        .query_map([session.to_string()], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(query);
    for id in ids {
        let mut run = read_run(tx, Uuid::parse_str(&id)?)?;
        if table(tx, "steering")? {
            steering::settle(tx, &run, &RunState::Interrupted, revision)?;
        }
        run.state = RunState::Interrupted;
        run.terminal_reason =
            Some("execution identity handoff; retained work requires reconciliation".into());
        ensure!(
            tx.execute(
                "UPDATE runs SET record=?1,active=0 WHERE id=?2 AND active=1",
                params![serde_json::to_string(&run)?, id]
            )? == 1,
            "retired run changed"
        );
    }
    if table(tx, "process_session_resources")? {
        tx.execute("UPDATE process_session_resources SET state='cleanup_unknown' WHERE session_id=?1 AND state='owned'",[session.to_string()])?;
    }
    if table(tx, "process_goals")? {
        let encoded: Option<String> = tx
            .query_row(
                "SELECT state FROM process_goals WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(encoded) = encoded {
            let mut goal: voyage_protocol::goals::Goal = serde_json::from_str(&encoded)?;
            goal.continuation_authorized = false;
            if goal.status == voyage_protocol::goals::GoalStatus::Active {
                goal.status = voyage_protocol::goals::GoalStatus::NeedsAttention;
                goal.stop_reason = Some(voyage_protocol::goals::GoalStopReason::UnresolvedEffects);
            }
            tx.execute("UPDATE process_goals SET state=?1,authority=NULL,revision=revision+1 WHERE session_id=?2",params![serde_json::to_string(&goal)?,session.to_string()])?;
        }
    }
    Ok(())
}
impl Journal {
    pub(crate) fn ensure_transition_configuration_ready(&self, session: Uuid) -> Result<()> {
        if table(&self.connection, TABLE)? {
            ensure!(!self.connection.query_row("SELECT EXISTS(SELECT 1 FROM execution_transitions WHERE session_id=?1 AND target_receipt IS NULL AND abort_receipt IS NULL)",[session.to_string()],|r|r.get::<_,bool>(0))?,"execution handoff configuration remains uncommitted");
        }
        Ok(())
    }
    pub(crate) fn retired_transition(
        &mut self,
        guard: &ExecutionGuard,
        request: &TransitionRequest,
        target_config: Option<&str>,
    ) -> Result<TransitionResponse> {
        self.check_guard(guard, request.session_id)?;
        ensure!(
            request.schema == SCHEMA
                && !request.session_id.is_nil()
                && !request.source_incarnation.is_nil(),
            "invalid retired request"
        );
        let request_digest = hash(
            b"voyage/retired-handoff-request/v1\0",
            &serde_json::to_vec(request)?,
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let response = match &request.operation {
            TransitionOperation::Observe { .. } => TransitionResponse::Facts {
                facts: facts(&tx, request.session_id, request.source_incarnation)?,
            },
            TransitionOperation::Lookup { command_id, .. } => {
                let StoredTransition {
                    prepared,
                    completed: complete,
                    aborted,
                    ..
                } = stored(&tx, *command_id)?.context("handoff receipt unavailable")?;
                ensure!(
                    prepared.session_id == request.session_id
                        && prepared.source_incarnation == request.source_incarnation,
                    "handoff receipt belongs elsewhere"
                );
                if let Some(receipt) = aborted {
                    return checked_aborted(&tx, request.operation.directory(), receipt);
                }
                match complete {
                    Some(receipt) => checked_completed(&tx, receipt)?,
                    None => TransitionResponse::Prepared { receipt: prepared },
                }
            }
            TransitionOperation::SourceFreeze {
                command_id,
                transition_id,
                target_incarnation,
                expected,
                target_uid,
                target_gid,
                target_config_digest,
                review_digest,
                ..
            } => {
                ensure!(
                    !command_id.is_nil()
                        && !transition_id.is_nil()
                        && !target_incarnation.is_nil()
                        && *target_incarnation != request.source_incarnation
                        && valid_hash(target_config_digest)
                        && valid_hash(review_digest),
                    "invalid handoff identity/digest"
                );
                if let Some(StoredTransition {
                    source_digest: prior,
                    prepared,
                    completed: complete,
                    aborted,
                    ..
                }) = stored(&tx, *command_id)?
                {
                    ensure!(prior == request_digest, "handoff command payload changed");
                    if let Some(receipt) = aborted {
                        return checked_aborted(&tx, request.operation.directory(), receipt);
                    }
                    return match complete {
                        Some(receipt) => checked_completed(&tx, receipt),
                        None => Ok(TransitionResponse::Prepared { receipt: prepared }),
                    };
                }
                collisions(&tx, *command_id)?;
                ensure!(
                    facts(&tx, request.session_id, request.source_incarnation)? == *expected,
                    "retired facts changed since review"
                );
                tx.execute_batch("CREATE TABLE IF NOT EXISTS execution_transitions(command_id TEXT PRIMARY KEY,transition_id TEXT NOT NULL UNIQUE,session_id TEXT NOT NULL,request_digest TEXT NOT NULL,prepared_receipt TEXT NOT NULL,commit_command_id TEXT UNIQUE,commit_request_digest TEXT,target_receipt TEXT,abort_command_id TEXT UNIQUE,abort_request_digest TEXT,abort_receipt TEXT)")?;
                ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_transitions WHERE session_id=?1 AND target_receipt IS NULL AND abort_receipt IS NULL)",[request.session_id.to_string()],|r|r.get::<_,bool>(0))?,"prior handoff remains prepared");
                let next = expected
                    .revision
                    .checked_add(1)
                    .context("revision overflow")?;
                interrupt_work(&tx, request.session_id, next)?;
                ensure!(
                    tx.execute(
                        "UPDATE sessions SET revision=?1 WHERE id=?2 AND revision=?3",
                        params![
                            i64::try_from(next)?,
                            request.session_id.to_string(),
                            i64::try_from(expected.revision)?
                        ]
                    )? == 1,
                    "stale handoff revision"
                );
                let retained = facts(&tx, request.session_id, request.source_incarnation)?;
                ensure!(
                    retained.history_digest == expected.history_digest
                        && retained.frozen_config_digest == expected.frozen_config_digest,
                    "handoff changed canonical text or old configuration"
                );
                let receipt = PreparedTransitionReceipt {
                    command_id: *command_id,
                    transition_id: *transition_id,
                    session_id: request.session_id,
                    source_incarnation: request.source_incarnation,
                    target_incarnation: *target_incarnation,
                    source_uid: unsafe { libc::geteuid() },
                    source_gid: unsafe { libc::getegid() },
                    source_directory_device: {
                        use std::os::unix::fs::MetadataExt;
                        std::fs::metadata(request.operation.directory())?.dev()
                    },
                    source_directory_inode: {
                        use std::os::unix::fs::MetadataExt;
                        std::fs::metadata(request.operation.directory())?.ino()
                    },
                    target_uid: *target_uid,
                    target_gid: *target_gid,
                    previous_revision: expected.revision,
                    prepared_revision: next,
                    previous_config_digest: expected.frozen_config_digest.clone(),
                    target_config_digest: target_config_digest.clone(),
                    review_digest: review_digest.clone(),
                    history_digest: expected.history_digest.clone(),
                    pending_work_digest: expected.pending_work_digest.clone(),
                    retained_pending_work_digest: retained.pending_work_digest,
                    unresolved_cleanup: expected.unresolved_cleanup.clone(),
                };
                tx.execute("INSERT INTO execution_transitions(command_id,transition_id,session_id,request_digest,prepared_receipt) VALUES(?1,?2,?3,?4,?5)",params![command_id.to_string(),transition_id.to_string(),request.session_id.to_string(),request_digest,serde_json::to_string(&receipt)?])?;
                TransitionResponse::Prepared { receipt }
            }
            TransitionOperation::AbortSource {
                command_id,
                expected,
                ..
            } => {
                ensure!(
                    !command_id.is_nil() && *command_id != expected.command_id,
                    "invalid abort command"
                );
                ensure!(
                    expected.session_id == request.session_id
                        && expected.source_incarnation == request.source_incarnation,
                    "abort request belongs elsewhere"
                );
                source_directory(request.operation.directory(), expected)?;
                let prior =
                    stored(&tx, expected.command_id)?.context("source marker unavailable")?;
                ensure!(
                    prior.prepared == *expected && prior.completed.is_none(),
                    "source already committed or marker changed"
                );
                if let Some(receipt) = prior.aborted {
                    ensure!(
                        prior.abort_digest.as_deref() == Some(request_digest.as_str()),
                        "abort payload changed"
                    );
                    return checked_aborted(&tx, request.operation.directory(), receipt);
                }
                collisions(&tx, *command_id)?;
                ensure!(
                    stored(&tx, *command_id)?.is_none(),
                    "abort command already used"
                );
                let current = facts(&tx, request.session_id, request.source_incarnation)?;
                ensure!(
                    current.revision == expected.prepared_revision
                        && current.history_digest == expected.history_digest
                        && current.frozen_config_digest == expected.previous_config_digest
                        && current.pending_work_digest == expected.retained_pending_work_digest,
                    "prepared source changed"
                );
                retired(&tx, request.session_id)?;
                let next = current
                    .revision
                    .checked_add(1)
                    .context("revision overflow")?;
                ensure!(
                    tx.execute(
                        "UPDATE sessions SET revision=?1 WHERE id=?2 AND revision=?3",
                        params![
                            i64::try_from(next)?,
                            request.session_id.to_string(),
                            i64::try_from(current.revision)?
                        ]
                    )? == 1,
                    "source abort revision changed"
                );
                let receipt = AbortedTransitionReceipt {
                    command_id: *command_id,
                    prepared: expected.clone(),
                    resulting_revision: next,
                    history_digest: current.history_digest,
                    pending_work_digest: current.pending_work_digest,
                };
                ensure!(tx.execute("UPDATE execution_transitions SET abort_command_id=?1,abort_request_digest=?2,abort_receipt=?3 WHERE command_id=?4 AND target_receipt IS NULL AND abort_receipt IS NULL",params![command_id.to_string(),request_digest,serde_json::to_string(&receipt)?,expected.command_id.to_string()])?==1,"source abort marker changed");
                TransitionResponse::Aborted { receipt }
            }
            TransitionOperation::TargetCommit {
                command_id,
                expected,
                ..
            } => {
                ensure!(
                    !command_id.is_nil()
                        && *command_id != expected.command_id
                        && unsafe { libc::geteuid() } == expected.target_uid
                        && unsafe { libc::getegid() } == expected.target_gid,
                    "target helper identity/command differs"
                );
                let StoredTransition {
                    prepared,
                    completed: complete,
                    commit_digest: prior_commit,
                    aborted,
                    ..
                } = stored(&tx, expected.command_id)?.context("source handoff unavailable")?;
                ensure!(aborted.is_none(), "source handoff was aborted");
                ensure!(
                    prepared == *expected
                        && expected.session_id == request.session_id
                        && expected.source_incarnation == request.source_incarnation,
                    "prepared handoff changed"
                );
                if let Some(receipt) = complete {
                    ensure!(
                        prior_commit.as_deref() == Some(request_digest.as_str()),
                        "target commit payload changed"
                    );
                    return checked_completed(&tx, receipt);
                }
                collisions(&tx, *command_id)?;
                ensure!(
                    stored(&tx, *command_id)?.is_none(),
                    "target command ID already used"
                );
                retired(&tx, request.session_id)?;
                let current = facts(&tx, request.session_id, request.source_incarnation)?;
                ensure!(
                    current.revision == expected.prepared_revision
                        && current.history_digest == expected.history_digest
                        && current.frozen_config_digest == expected.previous_config_digest
                        && current.pending_work_digest == expected.retained_pending_work_digest,
                    "prepared journal changed before target commit"
                );
                let configuration = target_config.context("target configuration unavailable")?;
                ensure!(
                    configuration.len() <= MAX_TARGET_CONFIG
                        && crate::identity_helper::config_digest(configuration.as_bytes())
                            == expected.target_config_digest,
                    "target configuration digest differs"
                );
                let next = current
                    .revision
                    .checked_add(1)
                    .context("revision overflow")?;
                ensure!(tx.execute("UPDATE process_configuration SET settings=?1 WHERE session_id=?2 AND settings=?3",params![configuration,request.session_id.to_string(),settings(&tx,request.session_id)?])?==1,"frozen configuration changed concurrently");
                ensure!(
                    tx.execute(
                        "UPDATE sessions SET revision=?1 WHERE id=?2 AND revision=?3",
                        params![
                            i64::try_from(next)?,
                            request.session_id.to_string(),
                            i64::try_from(current.revision)?
                        ]
                    )? == 1,
                    "target commit revision changed"
                );
                let after = facts(&tx, request.session_id, request.source_incarnation)?;
                ensure!(
                    after.history_digest == expected.history_digest
                        && after.pending_work_digest == expected.retained_pending_work_digest,
                    "target commit changed canonical history or retained work"
                );
                let receipt = TransitionReceipt {
                    command_id: *command_id,
                    prepared: prepared.clone(),
                    resulting_revision: next,
                    config_digest: after.frozen_config_digest,
                    history_digest: after.history_digest,
                    pending_work_digest: after.pending_work_digest,
                };
                ensure!(tx.execute("UPDATE execution_transitions SET commit_command_id=?1,commit_request_digest=?2,target_receipt=?3 WHERE command_id=?4 AND target_receipt IS NULL",params![command_id.to_string(),request_digest,serde_json::to_string(&receipt)?,expected.command_id.to_string()])?==1,"target receipt changed concurrently");
                TransitionResponse::Committed { receipt }
            }
        };
        commit(tx, &self.commit_fence)?;
        Ok(response)
    }
}

#[cfg(test)]
#[path = "execution_transition_tests.rs"]
mod tests;
