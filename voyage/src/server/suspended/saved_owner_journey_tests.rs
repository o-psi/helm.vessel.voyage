//! Whole saved observer over actual private SQLite/leases/attribution. No executor.
use super::*;
use crate::{
    attachment::{journal::TurnAdmission, local_actor::LocalActorStore},
    model::{Message, Role},
    session::Session,
};
use rusqlite::types::Value as SqlValue;
use std::os::unix::fs::PermissionsExt;

struct Fixture {
    _root: tempfile::TempDir,
    directory: PathBuf,
    registration: ProcessRegistration,
    actor: LocalActor,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut session = Session::new(workspace.clone(), "fixture".into());
        session.messages = (0..140)
            .map(|index| {
                let mut m = Message::new(
                    if index % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    format!("Canonical {index} café 日本語"),
                );
                if index == 17 {
                    m.content = "長い canonical text 世界 ".repeat(3000);
                    m.provider_state =
                        Some(json!({"opaque_replay":"PRIVATE-PROVIDER-STATE-SENTINEL"}));
                }
                m
            })
            .collect();
        let vessel =
            crate::attachment::journal::prepare_directory(root.path().join("vessel")).unwrap();
        let sessions =
            crate::attachment::journal::prepare_directory(vessel.join("sessions")).unwrap();
        let directory =
            crate::attachment::journal::prepare_directory(sessions.join(session.id.to_string()))
                .unwrap();
        let actor = LocalActorStore::open(&directory.join("identity"))
            .unwrap()
            .identity()
            .unwrap();
        let registration = ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id: session.id,
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: None,
            token: "saved-owned-fixture-auth-token-at-least-32".into(),
            peer_uids: None,
            workspace,
            state: voyage_protocol::process::ProcessState::Suspended,
            name: Some("Owned saved fixture".into()),
            executable: None,
        };
        write(&directory.join("registration.json"), &registration);
        crate::attachment::journal::open_private_file(&directory.join("startup.lock")).unwrap();
        let mut journal = Journal::open(directory.join("journal")).unwrap();
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_decisions(&guard).unwrap();
        journal.initialize_assignments(&guard).unwrap();
        journal.initialize_observations(&guard).unwrap();
        journal.initialize_cleanup_progress(&guard).unwrap();
        journal.initialize_session_resources(&guard).unwrap();
        journal
            .initialize_command_bindings(&guard, actor.principal_id)
            .unwrap();
        let settings = json!({"version":1,"workspace":registration.workspace,"config":{"provider":"openai-responses","model":"fixture","api_key_required":false,"base_url":"http://127.0.0.1:9/v1","access":"read-only"},"explicit":{"access":"read-only"},"selection":null,"confirmation":null});
        journal
            .retain_initial_configuration(&guard, settings.to_string())
            .unwrap();
        drop(guard);
        drop(journal);
        Self {
            _root: root,
            directory,
            registration,
            actor,
        }
    }
    fn request(&self, command: RuntimeCommand) -> RuntimeRequest {
        RuntimeRequest {
            protocol: PROCESS_PROTOCOL,
            session_id: self.registration.session_id,
            incarnation: self.registration.incarnation,
            token: self.registration.token.clone(),
            authorization: None,
            scope_authority: None,
            command,
        }
    }
    fn stopped(&self) {
        write(
            &self.directory.join("stopped.json"),
            &json!({"session_id":self.registration.session_id,"incarnation":self.registration.incarnation,"cleanup_observed":true,"suspended":true}),
        );
    }
    fn logical(&self) -> String {
        let db = rusqlite::Connection::open_with_flags(
            self.directory.join("journal/journal.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let names = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let mut all = Vec::new();
        for name in names {
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            let mut stmt = db.prepare(&format!("SELECT * FROM \"{name}\"")).unwrap();
            let columns = stmt.column_count();
            let mut rows = stmt
                .query_map([], |row| {
                    let values = (0..columns)
                        .map(|column| row.get::<_, SqlValue>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            all.push(format!("{name}:{rows:?}"));
        }
        all.join("\n")
    }
    fn leases_free(&self) {
        let startup =
            crate::attachment::journal::open_private_file(&self.directory.join("startup.lock"))
                .unwrap();
        startup.try_lock().unwrap();
        drop(startup);
        let journal = Journal::open(self.directory.join("journal")).unwrap();
        let guard = journal
            .acquire_execution(self.registration.session_id)
            .unwrap();
        drop(guard);
    }
    async fn read(&self, command: RuntimeCommand) -> Value {
        let result = observe(&self.directory, &self.request(command))
            .await
            .unwrap();
        self.leases_free();
        result
    }
}
fn write(path: &std::path::Path, value: &impl serde::Serialize) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[tokio::test]
async fn retired_saved_owner_snapshot_paging_and_utf8_chunks_preserve_every_private_canonical_row()
{
    let f = Fixture::new();
    let before = f.logical();
    let snapshot = f.read(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot["total_messages"], 140);
    assert_eq!(snapshot["observation"], "saved");
    assert_eq!(snapshot["recovery_pending"], true);
    assert_eq!(snapshot["suspended"], false);
    assert!(
        !snapshot
            .to_string()
            .contains("PRIVATE-PROVIDER-STATE-SENTINEL")
    );
    let first = f
        .read(RuntimeCommand::History {
            offset: 0,
            limit: 128,
            expected_revision: Some(0),
        })
        .await;
    assert!(first["messages"].as_array().unwrap().len() <= 128);
    let second = f
        .read(RuntimeCommand::History {
            offset: 128,
            limit: 128,
            expected_revision: Some(0),
        })
        .await;
    assert_eq!(second["messages"].as_array().unwrap().len(), 12);
    assert_eq!(second["messages"][0]["message_index"], 128);
    let mut offset = 0;
    let mut bytes = String::new();
    loop {
        let chunk = f
            .read(RuntimeCommand::MessageChunk {
                index: 17,
                offset,
                limit: 4096,
                expected_revision: 0,
            })
            .await;
        assert_eq!(chunk["encoding"], "public_message_json_utf8");
        bytes.push_str(chunk["data"].as_str().unwrap());
        offset = chunk["next_offset"].as_u64().unwrap();
        if chunk["has_more"] == false {
            break;
        }
    }
    let full: Value = serde_json::from_str(&bytes).unwrap();
    assert_eq!(full["content"], "長い canonical text 世界 ".repeat(3000));
    assert!(full.get("provider_state").is_none());
    assert!(f.logical() == before);
    f.leases_free();
    assert!(!f.directory.join("runtime.sock").exists());
}

#[tokio::test]
async fn stale_history_range_and_malformed_projection_refuse_under_the_full_observer_without_mutation()
 {
    let f = Fixture::new();
    let before = f.logical();
    let commands = vec![
        RuntimeCommand::History {
            offset: 0,
            limit: 0,
            expected_revision: Some(0),
        },
        RuntimeCommand::History {
            offset: 0,
            limit: 129,
            expected_revision: Some(0),
        },
        RuntimeCommand::History {
            offset: 0,
            limit: 1,
            expected_revision: Some(1),
        },
        RuntimeCommand::MessageChunk {
            index: 999,
            offset: 0,
            limit: 32,
            expected_revision: 0,
        },
        RuntimeCommand::MessageChunk {
            index: 17,
            offset: 0,
            limit: 0,
            expected_revision: 0,
        },
        RuntimeCommand::Events {
            after: 0,
            limit: 8,
            wait_ms: 10001,
            projection: None,
        },
        RuntimeCommand::Events {
            after: 0,
            limit: 8,
            wait_ms: 0,
            projection: Some("unsupported".into()),
        },
    ];
    for command in commands {
        assert!(observe(&f.directory, &f.request(command)).await.is_err());
        f.leases_free();
        assert!(f.logical() == before);
    }
}

#[tokio::test]
async fn saved_reads_and_exact_stop_evidence_are_distinct_without_inventing_owner_cleanup() {
    let f = Fixture::new();
    let before = f.logical();
    let snapshot = f.read(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot["recovery_pending"], true);
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Stop))
            .await
            .is_err()
    );
    f.leases_free();
    f.stopped();
    let stop = f.read(RuntimeCommand::Stop).await;
    assert_eq!(stop["status"], "stopped");
    assert_eq!(stop["cleanup"], "observed");
    let snapshot = f.read(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot["suspended"], true);
    assert_eq!(snapshot["recovery_pending"], false);
    // The file is fixture-supplied protocol evidence, NOT real process death proof.
    write(
        &f.directory.join("stopped.json"),
        &json!({"session_id":f.registration.session_id,"incarnation":Uuid::new_v4(),"cleanup_observed":true,"suspended":true}),
    );
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Stop))
            .await
            .is_err()
    );
    assert_eq!(
        f.read(RuntimeCommand::Snapshot).await["recovery_pending"],
        true
    );
    assert!(f.logical() == before);
}

#[tokio::test]
async fn actual_startup_and_execution_leases_bound_observation_and_release_without_an_executor() {
    let f = Fixture::new();
    let before = f.logical();
    let startup =
        crate::attachment::journal::open_private_file(&f.directory.join("startup.lock")).unwrap();
    startup.try_lock().unwrap();
    let request = f.request(RuntimeCommand::Snapshot);
    let directory = f.directory.clone();
    let pending = tokio::spawn(async move { observe(&directory, &request).await });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!pending.is_finished());
    drop(startup);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    f.leases_free();
    let journal = Journal::open(f.directory.join("journal")).unwrap();
    let guard = journal
        .acquire_execution(f.registration.session_id)
        .unwrap();
    let request = f.request(RuntimeCommand::Snapshot);
    let directory = f.directory.clone();
    let pending = tokio::spawn(async move { observe(&directory, &request).await });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!pending.is_finished());
    drop(guard);
    drop(journal);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    f.leases_free();
    assert!(f.logical() == before);
}

#[tokio::test]
async fn full_observer_authentication_and_relinquished_registration_fail_before_saved_disclosure() {
    let f = Fixture::new();
    let before = f.logical();
    for field in 0..4 {
        let mut request = f.request(RuntimeCommand::Snapshot);
        match field {
            0 => request.protocol += 1,
            1 => request.session_id = Uuid::new_v4(),
            2 => request.incarnation = Uuid::new_v4(),
            _ => request.token = "foreign synthetic token".into(),
        };
        assert!(observe(&f.directory, &request).await.is_err());
        f.leases_free();
        assert!(f.logical() == before);
    }
    let mut registration = f.registration.clone();
    registration.state = voyage_protocol::process::ProcessState::Relinquished;
    write(&f.directory.join("registration.json"), &registration);
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Snapshot))
            .await
            .is_err()
    );
    assert!(f.logical() == before);
    f.leases_free();
}

#[tokio::test]
async fn missing_or_changed_actor_does_not_initialize_identity_or_mutate_saved_receipts() {
    let f = Fixture::new();
    let before = f.logical();
    let path = f.directory.join("identity/actor.json");
    let old = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Snapshot))
            .await
            .is_err()
    );
    assert!(!path.exists());
    f.leases_free();
    assert!(f.logical() == before);
    std::fs::write(&path, &old).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut identity: Value = serde_json::from_slice(&old).unwrap();
    identity["version"] = json!(99);
    write(&path, &identity);
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Snapshot))
            .await
            .is_err()
    );
    assert!(f.logical() == before);
    f.leases_free();
}

#[tokio::test]
async fn exact_metadata_receipt_survives_owner_drop_and_unknown_lookup_never_reserves_a_turn() {
    let f = Fixture::new();
    let mut journal = Journal::open(f.directory.join("journal")).unwrap();
    let guard = journal
        .acquire_execution(f.registration.session_id)
        .unwrap();
    let id = Uuid::new_v4();
    let command = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 0,
        expires_at_ms: 61000,
        name: "Saved metadata name".into(),
    };
    journal
        .process_metadata(&guard, f.actor, &command, 1000)
        .unwrap();
    drop(guard);
    drop(journal);
    let before = f.logical();
    let result = f.read(RuntimeCommand::Receipt { command_id: id }).await;
    assert_eq!(result["command_id"], id.to_string());
    let unknown = Uuid::new_v4();
    let result = f
        .read(RuntimeCommand::Receipt {
            command_id: unknown,
        })
        .await;
    assert_eq!(result["status"], "unknown");
    assert_eq!(result["command_id"], unknown.to_string());
    assert!(f.logical() == before);
    assert!(!f.directory.join("runtime.sock").exists());
}

#[tokio::test]
async fn negative_delivery_resolution_closes_exact_original_id_without_running_or_replaying_it() {
    let f = Fixture::new();
    f.stopped();
    let id = Uuid::new_v4();
    let expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        + 60000;
    let original = RuntimeCommand::Submit {
        budget: None,
        coordination: None,
        command_id: id,
        expected_revision: 0,
        expires_at_ms: expiry,
        prompt: "Original never-dispatched owned input".into(),
    };
    let resolved = f
        .read(RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(original.clone())),
        })
        .await;
    assert_eq!(resolved["status"], "not_admitted");
    let settled = f.logical();
    assert_eq!(
        f.read(RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(original))
        })
        .await,
        resolved
    );
    assert!(f.logical() == settled);
    let changed = RuntimeCommand::Submit {
        budget: None,
        coordination: None,
        command_id: id,
        expected_revision: 0,
        expires_at_ms: expiry,
        prompt: "Changed delayed payload".into(),
    };
    assert!(
        observe(
            &f.directory,
            &f.request(RuntimeCommand::Resolve {
                command_id: id,
                original: Some(Box::new(changed))
            })
        )
        .await
        .is_err()
    );
    f.leases_free();
    assert!(f.logical() == settled);
    let mut journal = Journal::open(f.directory.join("journal")).unwrap();
    let guard = journal
        .acquire_execution(f.registration.session_id)
        .unwrap();
    let request = TurnAdmission {
        budget: None,
        coordination: None,
        operator_name: None,
        command_id: id,
        machine_id: f.actor.installation_id,
        principal_id: f.actor.principal_id,
        session_id: f.registration.session_id,
        expected_revision: 0,
        expires_at_ms: 61000,
        prompt: "Original never-dispatched owned input".into(),
        parts: vec![],
    };
    assert!(journal.admit_turn(&guard, &request, 1000).is_err());
    drop(guard);
    drop(journal);
    assert!(f.logical() == settled);
    assert!(!f.directory.join("runtime.sock").exists());
}

#[tokio::test]
async fn full_saved_event_projections_remain_session_scoped_bounded_and_without_private_payload() {
    let f = Fixture::new();
    let mut journal = Journal::open(f.directory.join("journal")).unwrap();
    let guard = journal
        .acquire_execution(f.registration.session_id)
        .unwrap();
    journal
        .process_metadata(
            &guard,
            f.actor,
            &RuntimeCommand::Rename {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: 61000,
                name: "Observed owned metadata".into(),
            },
            1000,
        )
        .unwrap();
    drop(guard);
    drop(journal);
    let before = f.logical();
    for projection in [
        None,
        Some("public-v1".into()),
        Some(voyage_protocol::live_events::PROJECTION.into()),
    ] {
        let page = f
            .read(RuntimeCommand::Events {
                after: 0,
                limit: 128,
                wait_ms: 10000,
                projection,
            })
            .await;
        assert!(!page["events"].as_array().unwrap().is_empty());
        assert!(!page.to_string().contains("PRIVATE-PROVIDER-STATE-SENTINEL"));
        assert!(!page.to_string().contains(&f.registration.token));
    }
    assert!(f.logical() == before);
    f.leases_free();
}

#[tokio::test]
async fn missing_workspace_or_stale_socket_cannot_hide_saved_history_or_be_recreated_by_observation()
 {
    let f = Fixture::new();
    f.stopped();
    let before = f.logical();
    std::fs::remove_dir(&f.registration.workspace).unwrap();
    std::fs::write(
        f.directory.join("runtime.sock"),
        b"stale owned fixture endpoint",
    )
    .unwrap();
    let snapshot = f.read(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot["total_messages"], 140);
    assert_eq!(snapshot["recovery_pending"], true);
    assert!(snapshot["access"].is_null());
    assert!(!f.registration.workspace.exists());
    assert!(
        observe(&f.directory, &f.request(RuntimeCommand::Stop))
            .await
            .is_err()
    );
    f.leases_free();
    assert!(f.logical() == before);
    std::fs::remove_file(f.directory.join("runtime.sock")).unwrap();
    let id = Uuid::new_v4();
    let original = RuntimeCommand::Submit {
        budget: None,
        coordination: None,
        command_id: id,
        expected_revision: 0,
        expires_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 60000,
        prompt: "Original missing-workspace input".into(),
    };
    assert_eq!(
        f.read(RuntimeCommand::Resolve {
            command_id: id,
            original: Some(Box::new(original))
        })
        .await["status"],
        "not_admitted"
    );
    assert!(!f.registration.workspace.exists());
    assert!(!f.directory.join("runtime.sock").exists());
    f.leases_free();
}
