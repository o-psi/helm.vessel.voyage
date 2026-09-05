use super::*;
use crate::model::SteeringStatus;

fn pending(journal: &Journal, run: &RunRecord) -> SteeringAdmission {
    SteeringAdmission {
        receipt_id: Uuid::new_v4(), session_id: run.session_id, run_id: run.id,
        actor: SteeringActor { machine_id: run.machine_id, principal_id: run.principal_id },
        expected_revision: journal.load_session(run.session_id).unwrap().revision,
        expires_at_ms: 60_000, text: "durable correction".into(),
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
    let mut second = pending(&journal, &run); second.text = "second".into();
    let second_record = journal.queue_steering(&guard, &second, 1).unwrap();
    let queued = journal.load_session(session.id).unwrap();
    assert_eq!(queued.revision, before.revision + 2);
    assert_eq!(serde_json::to_value(&queued.session.messages).unwrap(), serde_json::to_value(&before.session.messages).unwrap());
    let mut canonical = queued.session.messages;
    canonical.push(Message::new(Role::Assistant, "response before application"));
    journal.checkpoint_canonical(&guard, run.id, &canonical, &Usage::default()).unwrap();
    let unchanged = journal.load_session(session.id).unwrap().revision;
    let mut reordered = canonical.clone();
    reordered.push(second_record.record.applied_message());
    assert!(journal.checkpoint_canonical(&guard, run.id, &reordered, &Usage::default()).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, unchanged);
    canonical.push(first_record.record.applied_message());
    canonical.push(second_record.record.applied_message());
    journal.checkpoint_canonical(&guard, run.id, &canonical, &Usage::default()).unwrap();
    let record = journal.steering_record(first.receipt_id).unwrap();
    assert_eq!(record.status, SteeringStatus::Applied);
    assert_eq!(record.canonical_index, Some(2));
    assert_eq!(journal.steering_record(second.receipt_id).unwrap().canonical_index, Some(3));
    let saved = journal.load_session(session.id).unwrap();
    journal.checkpoint_canonical(&guard, run.id, &canonical, &Usage::default()).unwrap();
    assert_eq!(journal.load_session(session.id).unwrap().revision, saved.revision);
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
    let mut changed = steering.clone(); changed.text.push('!');
    assert!(journal.queue_steering(&guard, &changed, 1).is_err());
    let mut collision = request; collision.command_id = steering.receipt_id;
    assert!(journal.lookup_command(&collision).is_err());
    let baseline = journal.load_session(session.id).unwrap().session.messages;
    for forged in [Message::steering("unknown"), { let mut m = record.applied_message(); m.content.push('!'); m }] {
        let mut history = baseline.clone(); history.push(forged);
        assert!(journal.checkpoint_canonical(&guard, run.id, &history, &Usage::default()).is_err());
    }
    let rejected = journal.reject_steering(&guard, run.id, steering.receipt_id, SteeringRejection::Closed).unwrap();
    assert_eq!(rejected.status, SteeringStatus::NotApplied);
    assert_eq!(rejected.request.text, "durable correction");
    let revision = journal.load_session(session.id).unwrap().revision;
    journal.reject_steering(&guard, run.id, steering.receipt_id, SteeringRejection::Closed).unwrap();
    assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
    let mut history = baseline; history.push(record.applied_message());
    assert!(journal.checkpoint_canonical(&guard, run.id, &history, &Usage::default()).is_err());
}

#[test]
fn terminal_transition_resolves_pending_without_losing_receipts() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let steering = pending(&journal, &run);
    journal.queue_steering(&guard, &steering, 1).unwrap();
    let before = journal.load_session(session.id).unwrap().revision;
    journal.finish(&guard, run.id, RunState::Cancelled, Some("cancelled"), None).unwrap();
    let record = journal.steering_record(steering.receipt_id).unwrap();
    assert_eq!(record.status, SteeringStatus::NotApplied);
    assert_eq!(record.reason, Some(SteeringRejection::Cancelled));
    assert_eq!(journal.load_session(session.id).unwrap().revision, before + 1);
    assert!(journal.queue_steering(&guard, &steering, 90_000).unwrap().duplicate);
    let later = pending(&journal, &run);
    assert!(journal.queue_steering(&guard, &later, 1).is_err());
}
