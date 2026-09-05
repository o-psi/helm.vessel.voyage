use super::*;
use crate::model::SteeringStatus;

fn pending(journal: &Journal, run: &RunRecord) -> SteeringAdmission {
    SteeringAdmission {
        receipt_id: Uuid::new_v4(),
        session_id: run.session_id,
        run_id: run.id,
        actor: SteeringActor {
            machine_id: run.machine_id,
            principal_id: run.principal_id,
        },
        expected_revision: journal.load_session(run.session_id).unwrap().revision,
        expires_at_ms: 60_000,
        text: "durable correction".into(),
    }
}

#[test]
fn queue_is_separate_from_canonical_and_application_is_atomic_fifo() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let before = journal.load_session(session.id).unwrap();
    let first = pending(&journal, &run);
    let first_record = journal.queue_steering(&guard, &first, 1).unwrap();
    assert_eq!(first_record.record.status, SteeringStatus::Queued);
    let mut second = pending(&journal, &run);
    second.text = "second".into();
    let second_record = journal.queue_steering(&guard, &second, 1).unwrap();
    let queued = journal.load_session(session.id).unwrap();
    assert_eq!(queued.revision, before.revision + 2);
    assert_eq!(
        serde_json::to_value(&queued.session.messages).unwrap(),
        serde_json::to_value(&before.session.messages).unwrap()
    );
    let mut canonical = queued.session.messages;
    canonical.push(Message::new(Role::Assistant, "response before application"));
    journal
        .checkpoint_canonical(&guard, run.id, &canonical, &Usage::default())
        .unwrap();
    let unchanged = journal.load_session(session.id).unwrap().revision;
    let mut reordered = canonical.clone();
    reordered.push(second_record.record.applied_message());
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &reordered, &Usage::default())
            .is_err()
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        unchanged
    );
    canonical.push(first_record.record.applied_message());
    canonical.push(second_record.record.applied_message());
    journal
        .checkpoint_canonical(&guard, run.id, &canonical, &Usage::default())
        .unwrap();
    let record = journal.steering_record(first.receipt_id).unwrap();
    assert_eq!(record.status, SteeringStatus::Applied);
    assert_eq!(record.canonical_index, Some(2));
    assert_eq!(
        journal
            .steering_record(second.receipt_id)
            .unwrap()
            .canonical_index,
        Some(3)
    );
    let saved = journal.load_session(session.id).unwrap();
    journal
        .checkpoint_canonical(&guard, run.id, &canonical, &Usage::default())
        .unwrap();
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        saved.revision
    );
    let replay = journal.queue_steering(&guard, &first, 90_000).unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.record.status, SteeringStatus::Applied);
}

#[test]
fn receipt_identity_collision_forgery_and_rejection_are_not_history_rewrites() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let mut steering = pending(&journal, &run);
    steering.receipt_id = request.command_id;
    assert!(journal.queue_steering(&guard, &steering, 1).is_err());
    steering.receipt_id = Uuid::new_v4();
    let record = journal.queue_steering(&guard, &steering, 1).unwrap().record;
    let mut changed = steering.clone();
    changed.text.push('!');
    assert!(journal.queue_steering(&guard, &changed, 1).is_err());
    let mut collision = request;
    collision.command_id = steering.receipt_id;
    assert!(journal.lookup_command(&collision).is_err());
    let baseline = journal.load_session(session.id).unwrap().session.messages;
    for forged in [Message::steering("unknown"), {
        let mut m = record.applied_message();
        m.content.push('!');
        m
    }] {
        let mut history = baseline.clone();
        history.push(forged);
        assert!(
            journal
                .checkpoint_canonical(&guard, run.id, &history, &Usage::default())
                .is_err()
        );
    }
    let rejected = journal
        .reject_steering(
            &guard,
            run.id,
            steering.receipt_id,
            SteeringRejection::Closed,
        )
        .unwrap();
    assert_eq!(rejected.status, SteeringStatus::NotApplied);
    assert_eq!(rejected.request.text, "durable correction");
    let revision = journal.load_session(session.id).unwrap().revision;
    journal
        .reject_steering(
            &guard,
            run.id,
            steering.receipt_id,
            SteeringRejection::Closed,
        )
        .unwrap();
    assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
    let mut history = baseline;
    history.push(record.applied_message());
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &history, &Usage::default())
            .is_err()
    );
}

#[test]
fn terminal_transition_resolves_pending_without_losing_receipts() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let steering = pending(&journal, &run);
    journal.queue_steering(&guard, &steering, 1).unwrap();
    let before = journal.load_session(session.id).unwrap().revision;
    journal
        .finish(&guard, run.id, RunState::Cancelled, Some("cancelled"), None)
        .unwrap();
    let record = journal.steering_record(steering.receipt_id).unwrap();
    assert_eq!(record.status, SteeringStatus::NotApplied);
    assert_eq!(record.reason, Some(SteeringRejection::Cancelled));
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        before + 1
    );
    assert!(
        journal
            .queue_steering(&guard, &steering, 90_000)
            .unwrap()
            .duplicate
    );
    let later = pending(&journal, &run);
    assert!(journal.queue_steering(&guard, &later, 1).is_err());
}

#[test]
fn write_failures_roll_back_queue_application_rejection_and_terminal_state() {
    for stage in ["queue", "apply", "reject", "terminal"] {
        let (_dir, mut journal, session, request) = setup();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        let guidance = pending(&journal, &run);
        if stage != "queue" {
            journal.queue_steering(&guard, &guidance, 1).unwrap();
        }
        let before = journal.load_session(session.id).unwrap();
        let event_count: i64 = journal
            .connection
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        let trigger = match stage {
            "queue" => {
                "CREATE TRIGGER failure BEFORE INSERT ON steering BEGIN SELECT RAISE(ABORT,'queue unavailable'); END;"
            }
            "apply" => {
                "CREATE TRIGGER failure BEFORE UPDATE ON steering WHEN NEW.status='applied' BEGIN SELECT RAISE(ABORT,'application unavailable'); END;"
            }
            _ => {
                "CREATE TRIGGER failure BEFORE UPDATE ON steering WHEN NEW.status='not_applied' BEGIN SELECT RAISE(ABORT,'rejection unavailable'); END;"
            }
        };
        journal.connection.execute_batch(trigger).unwrap();
        match stage {
            "queue" => assert!(journal.queue_steering(&guard, &guidance, 1).is_err()),
            "apply" => {
                let mut history = before.session.messages.clone();
                history.push(
                    journal
                        .steering_record(guidance.receipt_id)
                        .unwrap()
                        .applied_message(),
                );
                assert!(
                    journal
                        .checkpoint_canonical(
                            &guard,
                            run.id,
                            &history,
                            &Usage {
                                input_tokens: 7,
                                output_tokens: 3
                            }
                        )
                        .is_err()
                );
            }
            "reject" => assert!(
                journal
                    .reject_steering(
                        &guard,
                        run.id,
                        guidance.receipt_id,
                        SteeringRejection::Closed
                    )
                    .is_err()
            ),
            _ => assert!(
                journal
                    .finish(&guard, run.id, RunState::Cancelled, Some("cancelled"), None)
                    .is_err()
            ),
        }
        let after = journal.load_session(session.id).unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(
            serde_json::to_value(after.session).unwrap(),
            serde_json::to_value(before.session).unwrap()
        );
        assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
        assert_eq!(
            journal
                .connection
                .query_row("SELECT count(*) FROM events", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            event_count
        );
        if stage == "queue" {
            assert!(journal.steering_record(guidance.receipt_id).is_err());
        } else {
            assert_eq!(
                journal.steering_record(guidance.receipt_id).unwrap().status,
                SteeringStatus::Queued
            );
        }
    }
}

#[test]
fn bounds_stale_revision_forged_metadata_and_pending_completion_fail_closed() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let base = pending(&journal, &run);
    for invalid in ["empty", "size", "stale", "expiry", "nil", "run", "actor"] {
        let mut req = base.clone();
        match invalid {
            "empty" => req.text = " ".into(),
            "size" => req.text = "x".repeat(crate::agent::MAX_STEERING_BYTES + 1),
            "stale" => req.expected_revision = 0,
            "expiry" => req.expires_at_ms = 1,
            "nil" => req.receipt_id = Uuid::nil(),
            "run" => req.run_id = Uuid::new_v4(),
            _ => req.actor.principal_id = Uuid::nil(),
        };
        assert!(journal.queue_steering(&guard, &req, 1).is_err());
    }
    let record = journal.queue_steering(&guard, &base, 1).unwrap().record;
    let original = journal.load_session(session.id).unwrap().session.messages;
    for variant in ["role", "status", "provider", "duplicate"] {
        let mut message = record.applied_message();
        match variant {
            "role" => message.role = Role::Assistant,
            "status" => message.steering.as_mut().unwrap().status = SteeringStatus::Queued,
            "provider" => message.provider_state = Some(serde_json::json!({"injected":true})),
            _ => {}
        };
        let mut history = original.clone();
        history.push(message.clone());
        if variant == "duplicate" {
            history.push(message);
        }
        assert!(
            journal
                .checkpoint_canonical(&guard, run.id, &history, &Usage::default())
                .is_err()
        );
        assert_eq!(
            journal.steering_record(base.receipt_id).unwrap().status,
            SteeringStatus::Queued
        );
    }
    let mut final_history = original;
    final_history.push(Message::new(Role::Assistant, "proposal"));
    journal
        .checkpoint_canonical(&guard, run.id, &final_history, &Usage::default())
        .unwrap();
    assert!(
        journal
            .accept_checkpoint(&guard, run.id, &final_history, &Usage::default())
            .is_err()
    );
    assert!(
        journal
            .finish(&guard, run.id, RunState::Completed, None, Some("final"))
            .is_err()
    );
    for _ in 1..MAX_PENDING_STEERING {
        let req = pending(&journal, &run);
        journal.queue_steering(&guard, &req, 1).unwrap();
    }
    assert!(
        journal
            .queue_steering(&guard, &pending(&journal, &run), 1)
            .is_err()
    );
    assert!(
        journal
            .steering_page(run.id, 0, MAX_PENDING_STEERING + 1)
            .is_err()
    );
    assert_eq!(
        journal
            .steering_page(run.id, 0, MAX_PENDING_STEERING)
            .unwrap()
            .len(),
        MAX_PENDING_STEERING
    );
}

#[test]
fn explicit_schema_three_upgrade_keeps_import_provenance_and_fences_old_writer() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path().join("journal")).unwrap();
    let session = Session::new(dir.path().into(), "fixture".into());
    let provenance = import_provenance(&session, &journal.directory);
    journal.import_session(&session, &provenance).unwrap();
    journal
        .connection
        .execute_batch("DROP TABLE steering; UPDATE attachment_schema SET version=3;")
        .unwrap();
    drop(journal);
    let mut journal = Journal::open(dir.path().join("journal")).unwrap();
    let stale = Journal::open(dir.path().join("journal")).unwrap();
    assert_eq!(journal.opened_schema, 3);
    journal.upgrade_quiescent().unwrap();
    assert_eq!(journal.opened_schema, SCHEMA_VERSION);
    assert!(stale.acquire_execution(session.id).is_err());
    assert_eq!(
        journal.import_session(&session, &provenance).unwrap(),
        (0, true)
    );
    assert!(
        journal
            .steering_page(Uuid::new_v4(), 0, 1)
            .unwrap()
            .is_empty()
    );
}
