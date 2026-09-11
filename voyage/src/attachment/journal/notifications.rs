//! Owner-written metadata intent, committed in the terminal/decision transaction.
//!
//! A preallocated ring reserves 256 fixed-size records before any admission. Full
//! notification capacity overwrites the oldest intent, never refuses settlement.
//! A reader behind that bounded durable horizon receives `gap: true`; evicted IDs
//! are never reconstructed or replayed from run records. A gap is NOT delivery.
//! The fixed-width sequence and padded metadata avoid growing this table at finish.
//! Ordinary journal I/O failures (including disk failure) still fail atomically.
//! No recipient, text, reason, tool arguments or request payload belongs here.
use super::*;
use serde_json::{Value, json};

const CAPACITY: i64 = 256;
const RECORD_BYTES: usize = 1024;

pub(super) fn initialize(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("CREATE TABLE notification_cursor(id INTEGER PRIMARY KEY CHECK(id=1), sequence TEXT NOT NULL);
        CREATE TABLE notification_outbox(slot INTEGER PRIMARY KEY, sequence TEXT NOT NULL, metadata TEXT NOT NULL);")?;
    tx.execute(
        "INSERT INTO notification_cursor VALUES(1,?1)",
        [sequence_key(0)],
    )?;
    for slot in 0..CAPACITY {
        tx.execute(
            "INSERT INTO notification_outbox VALUES(?1,?2,?3)",
            params![slot, sequence_key(0), " ".repeat(RECORD_BYTES)],
        )?;
    }
    Ok(())
}

fn sequence_key(value: i64) -> String {
    format!("{value:020}")
}

pub(super) fn append(
    tx: &Transaction<'_>,
    run: &RunRecord,
    decision: Option<(Uuid, i64)>,
) -> Result<()> {
    let latest: String = tx.query_row(
        "SELECT sequence FROM notification_cursor WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let sequence = latest
        .parse::<i64>()?
        .checked_add(1)
        .context("notification sequence exhausted")?;
    let kind = if decision.is_some() {
        "attention"
    } else {
        match run.state {
            RunState::Completed => "completed",
            RunState::Incomplete => "incomplete",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Interrupted => "interrupted",
            RunState::Accepted | RunState::Running => anyhow::bail!("nonterminal notification"),
        }
    };
    // The finish hook is after all classification/cancellation adjustments. A
    // terminal result does not establish descendant or resource cleanup, even for
    // Completed; never manufacture a cleanup-success notification from that state.
    let outcome = if decision.is_some() {
        Value::Null
    } else {
        json!(run.state)
    };
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|time| u64::try_from(time.as_millis()).ok());
    let event = json!({
        "sequence": sequence,
        "source_event_id": Uuid::new_v4(),
        "session_id": run.session_id,
        "run_id": run.id,
        "incarnation": run.source_incarnation,
        "kind": kind,
        "created_at_ms": created,
        "decision_id": decision.map(|(id, _)| id),
        "expires_at_ms": decision.map(|(_, expires)| expires),
        "outcome": outcome,
        "cleanup": "unknown"
    });
    let mut encoded = serde_json::to_string(&event)?;
    ensure!(
        encoded.len() <= RECORD_BYTES,
        "notification metadata exceeds reserved slot"
    );
    encoded.extend(std::iter::repeat_n(' ', RECORD_BYTES - encoded.len()));
    let written = tx.execute(
        "UPDATE notification_outbox SET sequence=?1,metadata=?2 WHERE slot=?3",
        params![sequence_key(sequence), encoded, (sequence - 1) % CAPACITY],
    )?;
    ensure!(written == 1, "reserved notification slot missing");
    tx.execute(
        "UPDATE notification_cursor SET sequence=?1 WHERE id=1",
        [sequence_key(sequence)],
    )?;
    Ok(())
}

impl Journal {
    pub(crate) fn bind_notification_incarnation(
        &mut self,
        guard: &mut ExecutionGuard,
        incarnation: Uuid,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(!incarnation.is_nil(), "nil owner incarnation");
        ensure!(
            guard
                .incarnation
                .is_none_or(|current| current == incarnation),
            "owner incarnation already bound"
        );
        // Reuse the existing exclusive-owner v8+ upgrade boundary, including its
        // other-active-session fence, before any new attributed run admission.
        self.require_content_schema(guard, true)?;
        guard.incarnation = Some(incarnation);
        Ok(())
    }

    /// Metadata-only read under the owner's mutex (or suspended execution fence).
    /// Legacy generations with no original attribution, or an unavailable clock,
    /// consume a durable intent but are not exportable: report a gap, never assign
    /// the recovery owner's incarnation or silently fabricate a timestamp.
    pub(crate) fn notification_events(
        &mut self,
        session: Uuid,
        after: u64,
        limit: u32,
    ) -> Result<Value> {
        ensure!(
            (1..=128).contains(&limit),
            "notification limit must be 1..128"
        );
        let after = i64::try_from(after).context("notification cursor overflow")?;
        self.check_schema()?;
        self.load_session(session)?;
        if self.opened_schema < 12 {
            ensure!(after == 0, "notification cursor ahead of legacy journal");
            return Ok(json!({"events": [], "next_after": 0, "has_more": false, "gap": true}));
        }
        let tx = self.connection.transaction()?;
        let latest: String = tx.query_row(
            "SELECT sequence FROM notification_cursor WHERE id=1",
            [],
            |row| row.get(0),
        )?;
        let latest = latest.parse::<i64>()?;
        ensure!(after <= latest, "notification cursor ahead of journal");
        let mut gap = after < (latest - CAPACITY).max(0);
        let mut statement = tx.prepare("SELECT sequence,metadata FROM notification_outbox WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?;
        let rows = statement
            .query_map(params![sequence_key(after), limit as i64 + 1], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = rows.len() > limit as usize;
        let mut next = after;
        let mut events = Vec::new();
        for (sequence, encoded) in rows.into_iter().take(limit as usize) {
            next = sequence.parse::<i64>()?;
            let event: Value = serde_json::from_str(&encoded)?;
            ensure!(
                event["sequence"].as_i64() == Some(next),
                "notification identity mismatch"
            );
            if event["session_id"] != session.to_string() {
                continue;
            }
            if event["incarnation"].is_null() || event["created_at_ms"].is_null() {
                gap = true;
                continue;
            }
            events.push(event);
        }
        Ok(json!({"events": events, "next_after": next, "has_more": has_more, "gap": gap}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Journal, ExecutionGuard, RunRecord) {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        let session = Session::new(root.path().into(), "fixture".into());
        journal.create_session(&session).unwrap();
        let mut guard = journal.acquire_execution(session.id).unwrap();
        journal
            .bind_notification_incarnation(&mut guard, Uuid::new_v4())
            .unwrap();
        journal.initialize_decisions(&guard).unwrap();
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
                    expires_at_ms: 1000,
                    prompt: "never export this prompt".into(),
                    parts: vec![],
                },
                1,
            )
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        (root, journal, guard, run)
    }

    #[test]
    fn only_settled_outcomes_are_exported_and_reads_are_stable() {
        for state in [
            RunState::Completed,
            RunState::Incomplete,
            RunState::Cancelled,
            RunState::Interrupted,
            RunState::Failed,
        ] {
            let (_root, mut journal, guard, run) = fixture();
            assert!(
                journal.notification_events(run.session_id, 0, 128).unwrap()["events"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let (reason, final_text) = if state == RunState::Completed {
                (None, Some("private final text"))
            } else {
                (Some("private failure details"), None)
            };
            journal
                .finish_classified(&guard, run.id, state.clone(), reason, final_text, None)
                .unwrap();
            let page = journal.notification_events(run.session_id, 0, 128).unwrap();
            assert_eq!(
                page,
                journal.notification_events(run.session_id, 0, 128).unwrap()
            );
            let event = &page["events"][0];
            assert_eq!(event["outcome"], json!(state));
            assert_eq!(event["kind"], json!(state));
            assert_eq!(event["cleanup"], "unknown");
            assert_eq!(event["incarnation"], json!(run.source_incarnation));
            assert_eq!(page["gap"], false);
            assert!(!page.to_string().contains("private"));
            assert!(!page.to_string().contains("prompt"));
            assert!(
                journal
                    .finish(&guard, run.id, RunState::Failed, Some("retry"), None)
                    .is_err()
            );
            assert_eq!(
                page,
                journal.notification_events(run.session_id, 0, 128).unwrap()
            );
        }
    }

    #[test]
    fn final_cancellation_override_not_provisional_success() {
        let (_root, mut journal, guard, run) = fixture();
        journal
            .request_cancel_local_with_clock(
                &LocalCancelRequest {
                    session_id: run.session_id,
                    run_id: run.id,
                    installation_id: run.machine_id,
                    principal_id: run.principal_id,
                    expires_at_ms: 1000,
                },
                || Ok(1),
            )
            .unwrap();
        let settled = journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("must not appear"),
            )
            .unwrap();
        assert_eq!(settled.state, RunState::Cancelled);
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["events"][0]["kind"], "cancelled");
    }

    #[test]
    fn pending_assignment_cleanup_overrides_provisional_completion() {
        let (_root, mut journal, guard, run) = fixture();
        journal.initialize_assignments(&guard).unwrap();
        journal.connection.execute(
            "INSERT INTO process_assignments(id,run_id,principal,participant,request,state) VALUES(?1,?2,?3,?4,'{}','acceptance_unknown')",
            params![Uuid::new_v4().to_string(), run.id.to_string(), run.principal_id.to_string(), Uuid::new_v4().to_string()],
        ).unwrap();
        let settled = journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("not a final success"),
            )
            .unwrap();
        assert_eq!(settled.state, RunState::Incomplete);
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["events"][0]["kind"], "incomplete");
        assert_eq!(page["events"][0]["cleanup"], "unknown");
    }

    #[test]
    fn decision_intent_is_atomic_and_metadata_only() {
        let (_root, mut journal, guard, run) = fixture();
        let id = Uuid::new_v4();
        let incarnation = run.source_incarnation.unwrap();
        assert!(
            journal
                .create_decision(&guard, run.id, Uuid::new_v4(), id, 1000, json!({}))
                .is_err()
        );
        journal
            .create_decision(
                &guard,
                run.id,
                incarnation,
                id,
                1000,
                json!({"kind":"approval", "private":"secret argument"}),
            )
            .unwrap();
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["events"][0]["kind"], "attention");
        assert_eq!(page["events"][0]["decision_id"], json!(id));
        assert_eq!(page["events"][0]["outcome"], Value::Null);
        assert_eq!(page["events"][0]["expires_at_ms"], 1000);
        assert!(!page.to_string().contains("secret"));
        assert!(
            journal
                .create_decision(&guard, run.id, incarnation, id, 1000, json!({}))
                .is_err()
        );
        assert_eq!(
            page,
            journal.notification_events(run.session_id, 0, 128).unwrap()
        );
    }

    #[test]
    fn failed_intent_rolls_back_terminal_and_decision_transactions() {
        let (_root, mut journal, guard, run) = fixture();
        journal.connection.execute_batch("CREATE TRIGGER reject_notification BEFORE UPDATE ON notification_outbox BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
        assert!(
            journal
                .finish(&guard, run.id, RunState::Failed, Some("failure"), None)
                .is_err()
        );
        assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
        let id = Uuid::new_v4();
        assert!(
            journal
                .create_decision(
                    &guard,
                    run.id,
                    run.source_incarnation.unwrap(),
                    id,
                    1000,
                    json!({})
                )
                .is_err()
        );
        let decisions: i64 = journal
            .connection
            .query_row("SELECT count(*) FROM process_decisions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(decisions, 0);
        assert_eq!(
            journal.notification_events(run.session_id, 0, 128).unwrap()["next_after"],
            0
        );
    }

    #[test]
    fn full_ring_allows_terminal_settlement_and_exposes_durable_gap() {
        let (root, mut journal, guard, run) = fixture();
        for _ in 0..CAPACITY + 3 {
            journal
                .create_decision(
                    &guard,
                    run.id,
                    run.source_incarnation.unwrap(),
                    Uuid::new_v4(),
                    1000,
                    json!({}),
                )
                .unwrap();
        }
        journal
            .finish(
                &guard,
                run.id,
                RunState::Incomplete,
                Some("unfinished obligations"),
                None,
            )
            .unwrap();
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["gap"], true);
        assert_eq!(page["has_more"], true);
        assert_eq!(page["events"].as_array().unwrap().len(), 128);
        let next = page["next_after"].as_u64().unwrap();
        let end = journal
            .notification_events(run.session_id, next, 128)
            .unwrap();
        assert_eq!(end["has_more"], false);
        assert_eq!(
            end["events"].as_array().unwrap().last().unwrap()["kind"],
            "incomplete"
        );
        let size: (i64, i64) = journal
            .connection
            .query_row(
                "SELECT count(*),sum(length(metadata)) FROM notification_outbox",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(size, (CAPACITY, CAPACITY * RECORD_BYTES as i64));
        drop(guard);
        drop(journal);
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        assert_eq!(
            page,
            journal.notification_events(run.session_id, 0, 128).unwrap()
        );
        assert!(
            journal
                .notification_events(run.session_id, u64::MAX, 128)
                .is_err()
        );
        assert!(journal.notification_events(run.session_id, 0, 129).is_err());
        assert!(journal.notification_events(run.session_id, 0, 0).is_err());
    }

    #[test]
    fn recovery_keeps_original_incarnation_and_unknown_cleanup() {
        let (root, journal, guard, run) = fixture();
        drop(guard);
        drop(journal);
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        let mut guard = journal.acquire_execution(run.session_id).unwrap();
        journal
            .bind_notification_incarnation(&mut guard, Uuid::new_v4())
            .unwrap();
        journal.recover_interrupted(&guard).unwrap();
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(
            page["events"][0]["incarnation"],
            json!(run.source_incarnation)
        );
        assert_eq!(page["events"][0]["kind"], "interrupted");
        assert_eq!(page["events"][0]["cleanup"], "unknown");
    }

    #[test]
    fn unattributed_legacy_intent_is_a_gap_not_a_fabricated_generation() {
        let (_root, mut journal, guard, mut run) = fixture();
        run.source_incarnation = None;
        journal
            .connection
            .execute(
                "UPDATE runs SET record=?1 WHERE id=?2",
                params![serde_json::to_string(&run).unwrap(), run.id.to_string()],
            )
            .unwrap();
        journal
            .finish(
                &guard,
                run.id,
                RunState::Interrupted,
                Some("old generation unknown"),
                None,
            )
            .unwrap();
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["gap"], true);
        assert_eq!(page["next_after"], 1);
        assert!(page["events"].as_array().unwrap().is_empty());
    }
    #[test]
    fn owned_legacy_upgrade_reserves_slots_and_fences_old_writer() {
        let (root, journal, guard, mut run) = fixture();
        run.source_incarnation = None;
        journal
            .connection
            .execute(
                "UPDATE runs SET record=?1 WHERE id=?2",
                params![serde_json::to_string(&run).unwrap(), run.id.to_string()],
            )
            .unwrap();
        journal.connection.execute_batch("DROP TABLE notification_outbox; DROP TABLE notification_cursor; UPDATE attachment_schema SET version=11;").unwrap();
        drop(guard);
        drop(journal);
        let old_reader = Journal::open(root.path().join("journal")).unwrap();
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        let mut guard = journal.acquire_execution(run.session_id).unwrap();
        journal
            .bind_notification_incarnation(&mut guard, Uuid::new_v4())
            .unwrap();
        assert!(old_reader.check_schema().is_err());
        assert_eq!(journal.opened_schema, 12);
        journal.recover_interrupted(&guard).unwrap();
        let page = journal.notification_events(run.session_id, 0, 128).unwrap();
        assert_eq!(page["gap"], true);
        assert_eq!(page["next_after"], 1);
        assert!(page["events"].as_array().unwrap().is_empty());
    }
}
