//! Real SQLite steering lifecycle/atomicity/reopen with exact receipt and image fences.
use super::*;
use crate::model::{Role, Usage};

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    journal: Journal,
    session: Session,
    guard: ExecutionGuard,
    run: RunRecord,
    actor: SteeringActor,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("journal");
        let mut journal = Journal::open(directory.clone()).unwrap();
        let session = Session::new(root.path().into(), "owned fixture".into());
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_decisions(&guard).unwrap();
        journal.initialize_assignments(&guard).unwrap();
        journal.initialize_observations(&guard).unwrap();
        journal.initialize_cleanup_progress(&guard).unwrap();
        let actor = SteeringActor {
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
        };
        journal
            .initialize_command_bindings(&guard, actor.principal_id)
            .unwrap();
        let request = TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: actor.machine_id,
            principal_id: actor.principal_id,
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: 61000,
            prompt: "Owned initial input".into(),
            parts: vec![],
        };
        let run = journal.admit_turn(&guard, &request, 1000).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        Self {
            _root: root,
            directory,
            journal,
            session,
            guard,
            run,
            actor,
        }
    }
    fn request(&self, text: &str) -> SteeringAdmission {
        SteeringAdmission {
            coordination: None,
            receipt_id: Uuid::new_v4(),
            session_id: self.session.id,
            run_id: self.run.id,
            actor: self.actor,
            expected_revision: self.journal.load_session(self.session.id).unwrap().revision,
            expires_at_ms: 61000,
            text: text.into(),
            parts: vec![],
        }
    }
    fn persisted(&self) -> String {
        // Include the full private graph: indexed provenance, revisions, events,
        // observations, scheduling and command receipts, not only JSON records.
        let tables = self
            .journal
            .connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let mut data = Vec::new();
        for table in tables {
            assert!(table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            let mut query = self
                .journal
                .connection
                .prepare(&format!("SELECT * FROM \"{table}\""))
                .unwrap();
            let columns = query.column_count();
            let mut rows = query
                .query_map([], |row| {
                    let values = (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            data.push(format!("{table}:{rows:?}"));
        }
        data.join("\n")
    }
    fn apply(&mut self, records: &[SteeringRecord], now: i64) {
        let mut messages = self
            .journal
            .load_session(self.session.id)
            .unwrap()
            .session
            .messages;
        messages.extend(records.iter().map(SteeringRecord::applied_message));
        self.journal
            .checkpoint_canonical_at(&self.guard, self.run.id, &messages, &Usage::default(), now)
            .unwrap();
    }
}

#[test]
fn exact_queued_and_applied_receipts_preserve_canonical_order_actor_and_ids_across_reopen() {
    let mut f = Fixture::new();
    let first = f.request("First accepted steering 世界");
    let one = f
        .journal
        .queue_steering(&f.guard, &first, 1100)
        .unwrap()
        .record;
    let second = f.request("Second accepted steering café");
    let two = f
        .journal
        .queue_steering(&f.guard, &second, 1200)
        .unwrap()
        .record;
    assert_eq!(one.ordinal, 1);
    assert_eq!(two.ordinal, 2);
    let before = f.persisted();
    let duplicate = f.journal.queue_steering(&f.guard, &first, 1300).unwrap();
    assert!(duplicate.duplicate);
    assert!(f.persisted() == before);
    f.apply(&[one, two], 1400);
    let saved = f.journal.load_session(f.session.id).unwrap();
    let texts: Vec<_> = saved
        .session
        .messages
        .iter()
        .filter_map(|m| {
            m.steering
                .as_ref()
                .map(|s| (s.id, m.content.clone(), s.status.clone()))
        })
        .collect();
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].0, first.receipt_id);
    assert_eq!(texts[1].0, second.receipt_id);
    assert_eq!(texts[0].1, first.text);
    let record = f.journal.steering_record(first.receipt_id).unwrap();
    assert!(record.status == SteeringStatus::Applied);
    assert!(record.canonical_index.is_some());
    let expected = serde_json::to_string(&record).unwrap();
    let directory = f.directory.clone();
    let sid = f.session.id;
    drop(f.guard);
    drop(f.journal);
    let mut reopened = Journal::open(directory).unwrap();
    let guard = reopened.acquire_execution(sid).unwrap();
    assert_eq!(
        serde_json::to_string(&reopened.steering_record(first.receipt_id).unwrap()).unwrap(),
        expected
    );
    assert!(
        reopened
            .queue_steering(&guard, &first, 1500)
            .unwrap()
            .duplicate
    );
    drop(guard);
}

#[test]
fn payload_actor_run_and_observation_drift_never_replace_an_existing_receipt_or_admit_another_run()
{
    let mut f = Fixture::new();
    let request = f.request("Original immutable steering");
    f.journal.queue_steering(&f.guard, &request, 1100).unwrap();
    let before = f.persisted();
    for field in 0..7 {
        let mut changed = request.clone();
        match field {
            0 => changed.text = "Changed payload".into(),
            1 => changed.actor.principal_id = Uuid::new_v4(),
            2 => changed.actor.machine_id = Uuid::new_v4(),
            3 => changed.run_id = Uuid::new_v4(),
            4 => changed.session_id = Uuid::new_v4(),
            5 => changed.expected_revision += 1,
            _ => changed.expires_at_ms += 1,
        };
        assert!(f.journal.queue_steering(&f.guard, &changed, 1200).is_err());
        assert!(f.persisted() == before);
    }
    assert_eq!(f.journal.steering_page(f.run.id, 0, 64).unwrap().len(), 1);
}

#[test]
fn rejecting_queued_receipts_is_exact_durable_and_never_appends_rejected_user_text() {
    let mut f = Fixture::new();
    let request = f.request("Rejected input not canonical");
    f.journal.queue_steering(&f.guard, &request, 1100).unwrap();
    let record = f
        .journal
        .reject_steering(
            &f.guard,
            f.run.id,
            request.receipt_id,
            SteeringRejection::Closed,
        )
        .unwrap();
    assert!(record.status == SteeringStatus::NotApplied);
    assert!(record.canonical_index.is_none());
    let before = f.persisted();
    assert!(
        f.journal
            .reject_steering(
                &f.guard,
                f.run.id,
                request.receipt_id,
                SteeringRejection::Closed
            )
            .unwrap()
            .status
            == SteeringStatus::NotApplied
    );
    assert!(f.persisted() == before);
    assert!(
        f.journal
            .reject_steering(
                &f.guard,
                f.run.id,
                request.receipt_id,
                SteeringRejection::Expired
            )
            .is_err()
    );
    assert!(f.persisted() == before);
    assert!(
        !f.journal
            .load_session(f.session.id)
            .unwrap()
            .session
            .messages
            .iter()
            .any(|m| m.content == request.text)
    );
    let directory = f.directory.clone();
    let id = request.receipt_id;
    drop(f.guard);
    drop(f.journal);
    let reopened = Journal::open(directory).unwrap();
    assert_eq!(
        reopened.steering_record(id).unwrap().reason,
        Some(SteeringRejection::Closed)
    );
}

#[test]
fn corrupted_sqlite_receipt_provenance_fails_closed_instead_of_projecting_forged_settlement() {
    for field in ["ordinal", "principal_id", "digest", "record"] {
        let mut f = Fixture::new();
        let request = f.request("Immutable private journal evidence");
        f.journal.queue_steering(&f.guard, &request, 1100).unwrap();
        match field {
            "ordinal" => {
                f.journal
                    .connection
                    .execute(
                        "UPDATE steering SET ordinal=99 WHERE id=?1",
                        [request.receipt_id.to_string()],
                    )
                    .unwrap();
            }
            "principal_id" => {
                f.journal
                    .connection
                    .execute(
                        "UPDATE steering SET principal_id=?2 WHERE id=?1",
                        params![request.receipt_id.to_string(), Uuid::new_v4().to_string()],
                    )
                    .unwrap();
            }
            "digest" => {
                f.journal
                    .connection
                    .execute(
                        "UPDATE steering SET digest=zeroblob(32) WHERE id=?1",
                        [request.receipt_id.to_string()],
                    )
                    .unwrap();
            }
            _ => {
                f.journal
                    .connection
                    .execute(
                        "UPDATE steering SET record='{}' WHERE id=?1",
                        [request.receipt_id.to_string()],
                    )
                    .unwrap();
            }
        }
        let before = f.persisted();
        assert!(f.journal.steering_record(request.receipt_id).is_err());
        assert!(f.journal.steering_page(f.run.id, 0, 64).is_err());
        assert!(f.journal.queue_steering(&f.guard, &request, 1200).is_err());
        assert!(f.persisted() == before);
    }
}

#[test]
fn queue_capacity_and_terminal_boundary_preserve_receipt_ids_and_pending_obligations_without_replay()
 {
    let mut f = Fixture::new();
    let mut requests = Vec::new();
    for i in 0..MAX_PENDING_STEERING {
        let request = f.request(&format!("Bounded queued item {i}"));
        f.journal
            .queue_steering(&f.guard, &request, 1100 + i as i64)
            .unwrap();
        requests.push(request);
    }
    let before = f.persisted();
    let overflow = f.request("Must not reserve beyond pending budget");
    assert!(f.journal.queue_steering(&f.guard, &overflow, 2000).is_err());
    assert!(f.persisted() == before);
    assert!(f.journal.steering_record(overflow.receipt_id).is_err());
    assert!(
        f.journal
            .finish(
                &f.guard,
                f.run.id,
                RunState::Completed,
                None,
                Some("Not a completed result while queued input remains")
            )
            .is_err()
    );
    assert!(f.persisted() == before);
    for request in requests {
        f.journal
            .reject_steering(
                &f.guard,
                f.run.id,
                request.receipt_id,
                SteeringRejection::RunCompleted,
            )
            .unwrap();
    }
    f.journal
        .finish(
            &f.guard,
            f.run.id,
            RunState::Completed,
            None,
            Some("Owned journal completion"),
        )
        .unwrap();
    let late = f.request("Late command not queued");
    let before = f.persisted();
    assert!(f.journal.queue_steering(&f.guard, &late, 3000).is_err());
    assert!(f.persisted() == before);
}

#[test]
fn mixed_image_steering_consumes_exact_session_bytes_and_checkpoints_parts_atomically() {
    use voyage_protocol::content::ContentPart;
    let mut f = Fixture::new();
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 1)
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    let bytes = image.into_inner();
    let mut store = crate::images::Store::open(&f.directory, f.session.id).unwrap();
    let attachment = store
        .put(
            f.actor.principal_id,
            Uuid::new_v4(),
            "owned-steering.png",
            &bytes,
        )
        .unwrap();
    let mut request = f.request("Mixed canonical steering text");
    request.parts = vec![
        ContentPart::Text {
            text: request.text.clone(),
        },
        ContentPart::Image {
            attachment: attachment.clone(),
        },
    ];
    let images = f.journal.resolve_steering_images(&request).unwrap();
    assert_eq!(images[&attachment.id], bytes);
    let queued = f
        .journal
        .queue_steering(&f.guard, &request, 1100)
        .unwrap()
        .record;
    f.apply(&[queued], 1200);
    let saved = f.journal.load_session(f.session.id).unwrap();
    let message = saved
        .session
        .messages
        .iter()
        .find(|m| {
            m.steering
                .as_ref()
                .is_some_and(|s| s.id == request.receipt_id)
        })
        .unwrap();
    assert!(message.role == Role::User);
    assert!(message.parts == request.parts);
    assert_eq!(message.content, request.text);
    let before = f.persisted();
    let mut changed = request.clone();
    if let ContentPart::Image { attachment } = &mut changed.parts[1] {
        attachment.sha256 = "f".repeat(64);
    }
    assert!(f.journal.resolve_steering_images(&changed).is_err());
    assert!(f.journal.queue_steering(&f.guard, &changed, 1300).is_err());
    assert!(f.persisted() == before);
}

#[test]
fn out_of_order_expired_or_corrupted_canonical_apply_rolls_back_every_receipt_and_message() {
    let mut f = Fixture::new();
    let first = f.request("First ordered input");
    let one = f
        .journal
        .queue_steering(&f.guard, &first, 1100)
        .unwrap()
        .record;
    let second = f.request("Second ordered input");
    let two = f
        .journal
        .queue_steering(&f.guard, &second, 1200)
        .unwrap()
        .record;
    let prefix = f
        .journal
        .load_session(f.session.id)
        .unwrap()
        .session
        .messages;
    let before = f.persisted();
    for kind in ["reversed", "changed", "expired"] {
        let mut messages = prefix.clone();
        match kind {
            "reversed" => {
                messages.push(two.applied_message());
                messages.push(one.applied_message());
            }
            "changed" => {
                let mut changed = one.applied_message();
                changed.content = "Not the admitted exact input".into();
                messages.push(changed);
            }
            _ => messages.push(one.applied_message()),
        };
        let clock = if kind == "expired" {
            first.expires_at_ms
        } else {
            1300
        };
        assert!(
            f.journal
                .checkpoint_canonical_at(&f.guard, f.run.id, &messages, &Usage::default(), clock)
                .is_err()
        );
        assert!(
            f.persisted() == before,
            "{kind} must not partially update queue/session"
        );
    }
    f.apply(&[one, two], 1400);
    assert!(
        f.journal
            .steering_page(f.run.id, 0, 64)
            .unwrap()
            .iter()
            .all(|r| r.status == SteeringStatus::Applied)
    );
}

#[test]
fn steering_and_turn_command_id_collisions_never_create_alias_receipts_or_reopen_execution() {
    let mut f = Fixture::new();
    let mut request = f.request("No alias input");
    request.receipt_id = f.run.command_id;
    let before = f.persisted();
    assert!(f.journal.queue_steering(&f.guard, &request, 1100).is_err());
    assert!(f.persisted() == before);
    request.receipt_id = Uuid::new_v4();
    let queued = f
        .journal
        .queue_steering(&f.guard, &request, 1100)
        .unwrap()
        .record;
    f.apply(&[queued], 1200);
    let before = f.persisted();
    let metadata = voyage_protocol::process::RuntimeCommand::Rename {
        command_id: request.receipt_id,
        expected_revision: f.journal.load_session(f.session.id).unwrap().revision,
        expires_at_ms: 61000,
        name: "No receipt alias".into(),
    };
    let actor = crate::attachment::local_actor::LocalActor {
        installation_id: f.actor.machine_id,
        principal_id: f.actor.principal_id,
    };
    assert!(
        f.journal
            .process_metadata(&f.guard, actor, &metadata, 1300)
            .is_err()
    );
    assert!(f.persisted() == before);
    let history = f
        .journal
        .load_session(f.session.id)
        .unwrap()
        .session
        .messages;
    let mut alias = history.clone();
    alias.push(
        f.journal
            .steering_record(request.receipt_id)
            .unwrap()
            .applied_message(),
    );
    assert!(
        f.journal
            .checkpoint_canonical_at(&f.guard, f.run.id, &alias, &Usage::default(), 1400)
            .is_err()
    );
    assert!(f.persisted() == before);
}
