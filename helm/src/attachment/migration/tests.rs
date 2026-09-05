use super::*;
#[cfg(unix)]
use crate::attachment::journal::Journal;
use crate::{Message, Role, session::Session};
use sha2::{Digest, Sha256};

struct Fixture {
    _temporary: tempfile::TempDir,
    store: SessionStore,
    coordinator: Coordinator,
    journal: PathBuf,
    source: PathBuf,
    request: TransferRequest,
    original: Vec<u8>,
}

impl Fixture {
    async fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let data = temporary.path().join("data");
        std::fs::create_dir(&data).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = SessionStore::new(data.join("sessions"));
        let mut session = Session::new(workspace.clone(), "fixture-model".into());
        session
            .messages
            .push(Message::new(Role::User, "canonical 雪 input"));
        session
            .messages
            .push(Message::new(Role::Assistant, "original answer"));
        session.switch_model("selected-model").unwrap();
        session.messages.last_mut().unwrap().provider_state =
            Some(serde_json::json!({"opaque":"local continuity state"}));
        session.usage.input_tokens = 123;
        session.usage.output_tokens = 9;
        store.save(&mut session).await.unwrap();
        let source = data.join("sessions").join(format!("{}.json", session.id));
        let original = std::fs::read(&source).unwrap();
        let request = TransferRequest {
            transfer_id: Uuid::new_v4(),
            session_id: session.id,
            expected_revision: session.revision,
            source_sha256: hex::encode(Sha256::digest(&original)),
        };
        let completion = data.join("completion");
        std::fs::create_dir(&completion).unwrap();
        let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
        let coordinator = Coordinator::open(completion.join(key), &workspace).unwrap();
        let journal = data.join("journal");
        Self {
            _temporary: temporary,
            store,
            coordinator,
            journal,
            source,
            request,
            original,
        }
    }
    async fn transfer(&self) -> Result<TransferReceipt> {
        transfer(
            self.store.clone(),
            self.coordinator.clone(),
            self.journal.clone(),
            self.request.clone(),
            CancellationToken::new(),
        )
        .await
    }
}

#[cfg(unix)]
#[tokio::test]
async fn successful_transfer_preserves_original_and_rejects_old_json_readers() {
    let fixture = Fixture::new().await;
    let result = fixture.transfer().await.unwrap();
    assert_eq!(result.session_id, fixture.request.session_id);
    assert!(!result.duplicate);
    assert!(
        fixture
            .store
            .load(fixture.request.session_id)
            .await
            .is_err()
    );
    assert!(serde_json::from_slice::<Session>(&std::fs::read(&fixture.source).unwrap()).is_err());
    let journal = Journal::open(fixture.journal.clone()).unwrap();
    let imported = journal.load_session(fixture.request.session_id).unwrap();
    let original: Session = serde_json::from_slice(&fixture.original).unwrap();
    assert_eq!(
        serde_json::to_value(&imported.session).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(imported.revision, original.revision);
    let retry = fixture.transfer().await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.journal_revision, original.revision);
}

#[cfg(not(unix))]
#[tokio::test]
async fn unsupported_transfer_fails_before_writing_source_or_destination() {
    let fixture = Fixture::new().await;
    let error = fixture.transfer().await.unwrap_err();
    assert!(error.to_string().contains("unsupported"));
    assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
    assert!(!fixture.journal.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn every_publication_boundary_is_resumable_with_one_authority() {
    for fault in [
        Boundary::BackupPublished,
        Boundary::BackupDurable,
        Boundary::MarkerPublished,
        Boundary::MarkerDurable,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
    ] {
        let fixture = Fixture::new().await;
        let failed = transfer_inner(
            fixture.store.clone(),
            fixture.coordinator.clone(),
            fixture.journal.clone(),
            fixture.request.clone(),
            CancellationToken::new(),
            fault,
        )
        .await;
        assert!(failed.unwrap_err().to_string().contains("injected"));
        let source_active = fixture.store.load(fixture.request.session_id).await.is_ok();
        let journal_active = Journal::open(fixture.journal.clone())
            .unwrap()
            .load_session(fixture.request.session_id)
            .is_ok();
        assert!(!(source_active && journal_active));
        assert_eq!(journal_active, fault == Boundary::AfterCommit);
        assert_eq!(
            source_active,
            matches!(fault, Boundary::BackupPublished | Boundary::BackupDurable)
        );
        let retry = fixture.transfer().await.unwrap();
        assert_eq!(retry.duplicate, fault == Boundary::AfterCommit);
        assert_eq!(
            std::fs::read(
                fixture
                    .journal
                    .join("imports")
                    .join(format!("{}.json", fixture.request.transfer_id))
            )
            .unwrap(),
            fixture.original
        );
        assert!(
            fixture
                .store
                .load(fixture.request.session_id)
                .await
                .is_err()
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_identity_hash_workspace_and_cancel_leave_source_active() {
    for case in ["hash", "revision", "workspace", "cancel", "writer"] {
        let mut fixture = Fixture::new().await;
        let cancel = CancellationToken::new();
        let mut lease = None;
        match case {
            "hash" => fixture.request.source_sha256 = "a".repeat(64),
            "revision" => fixture.request.expected_revision += 1,
            "workspace" => {
                fixture.coordinator = Coordinator::open(
                    fixture._temporary.path().join("wrong-coordinator"),
                    &fixture._temporary.path().join("workspace"),
                )
                .unwrap()
            }
            "cancel" => cancel.cancel(),
            "writer" => lease = Some(fixture.coordinator.acquire_agent_writer().unwrap()),
            _ => unreachable!(),
        }
        assert!(
            transfer(
                fixture.store.clone(),
                fixture.coordinator.clone(),
                fixture.journal.clone(),
                fixture.request.clone(),
                cancel
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
        assert!(!fixture.journal.exists());
        drop(lease);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn retry_never_overwrites_an_advanced_journal_or_accepts_restored_json() {
    use crate::attachment::journal::TurnAdmission;
    let fixture = Fixture::new().await;
    fixture.transfer().await.unwrap();
    let mut journal = Journal::open(fixture.journal.clone()).unwrap();
    let guard = journal
        .acquire_execution(fixture.request.session_id)
        .unwrap();
    journal
        .admit_turn(
            &guard,
            &TurnAdmission {
                command_id: Uuid::new_v4(),
                machine_id: Uuid::new_v4(),
                principal_id: Uuid::new_v4(),
                session_id: fixture.request.session_id,
                expected_revision: fixture.request.expected_revision,
                expires_at_ms: 100,
                prompt: "new authoritative turn".into(),
            },
            1,
        )
        .unwrap();
    let before = serde_json::to_value(
        journal
            .load_session(fixture.request.session_id)
            .unwrap()
            .session,
    )
    .unwrap();
    let result = fixture.transfer().await.unwrap();
    assert!(result.duplicate);
    assert_eq!(
        result.journal_revision,
        fixture.request.expected_revision + 1
    );
    assert_eq!(
        serde_json::to_value(
            journal
                .load_session(fixture.request.session_id)
                .unwrap()
                .session
        )
        .unwrap(),
        before
    );
    std::fs::write(&fixture.source, &fixture.original).unwrap();
    assert!(fixture.transfer().await.is_err());
    assert_eq!(
        serde_json::to_value(
            journal
                .load_session(fixture.request.session_id)
                .unwrap()
                .session
        )
        .unwrap(),
        before
    );
}

#[cfg(unix)]
#[tokio::test]
async fn transfer_crash_child() {
    let Some(root) = std::env::var_os("HELM_TRANSFER_TEST_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let id = Uuid::parse_str(&std::env::var("HELM_TRANSFER_TEST_ID").unwrap()).unwrap();
    let transfer_id =
        Uuid::parse_str(&std::env::var("HELM_TRANSFER_TEST_TRANSFER_ID").unwrap()).unwrap();
    let original = std::fs::read(root.join("data/sessions").join(format!("{id}.json"))).unwrap();
    let session: Session = serde_json::from_slice(&original).unwrap();
    let key = hex::encode(Sha256::digest(
        session.workspace.as_os_str().as_encoded_bytes(),
    ));
    let coordinator =
        Coordinator::open(root.join("data/completion").join(key), &session.workspace).unwrap();
    let index: usize = std::env::var("HELM_TRANSFER_TEST_CRASH")
        .unwrap()
        .parse()
        .unwrap();
    let faults = [
        Boundary::BackupPublished,
        Boundary::BackupDurable,
        Boundary::MarkerPublished,
        Boundary::MarkerDurable,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
    ];
    transfer_inner(
        SessionStore::new(root.join("data/sessions")),
        coordinator,
        root.join("data/journal"),
        TransferRequest {
            transfer_id,
            session_id: id,
            expected_revision: session.revision,
            source_sha256: hex::encode(Sha256::digest(original)),
        },
        CancellationToken::new(),
        faults[index],
    )
    .await
    .unwrap();
    panic!("expected process termination");
}

#[cfg(unix)]
#[tokio::test]
async fn process_death_releases_both_fences_and_resumes_every_boundary() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    for index in 0..6 {
        let fixture = Fixture::new().await;
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "attachment::migration::tests::transfer_crash_child",
                "--nocapture",
            ])
            .env("HELM_TRANSFER_TEST_ROOT", fixture._temporary.path())
            .env(
                "HELM_TRANSFER_TEST_ID",
                fixture.request.session_id.to_string(),
            )
            .env(
                "HELM_TRANSFER_TEST_TRANSFER_ID",
                fixture.request.transfer_id.to_string(),
            )
            .env("HELM_TRANSFER_TEST_CRASH", index.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("transfer crash child timed out");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(status.code(), Some(77));
        let journal = Journal::open(fixture.journal.clone()).unwrap();
        let source_active = fixture.store.load(fixture.request.session_id).await.is_ok();
        let journal_active = journal.load_session(fixture.request.session_id).is_ok();
        assert!(!(source_active && journal_active));
        assert_eq!(journal_active, index == 5);
        let receipt = fixture.transfer().await.unwrap();
        assert_eq!(receipt.duplicate, index == 5);
        assert!(
            fixture
                .store
                .load(fixture.request.session_id)
                .await
                .is_err()
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn pending_tool_intent_stays_ambiguous_and_blocks_admission() {
    use crate::attachment::journal::TurnAdmission;
    let mut fixture = Fixture::new().await;
    let mut session = fixture
        .store
        .load(fixture.request.session_id)
        .await
        .unwrap();
    let mut intent = Message::new(Role::Assistant, "uncertain tool");
    intent.tool_calls.push(crate::model::ToolCall {
        id: "uncertain-effect".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"never execute this"}),
    });
    session.messages.push(intent);
    fixture.store.save(&mut session).await.unwrap();
    fixture.original = std::fs::read(&fixture.source).unwrap();
    fixture.request.expected_revision = session.revision;
    fixture.request.source_sha256 = hex::encode(Sha256::digest(&fixture.original));
    fixture.transfer().await.unwrap();
    let mut journal = Journal::open(fixture.journal.clone()).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: session.revision,
        expires_at_ms: 100,
        prompt: "resume".into(),
    };
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert!(journal.lookup_command(&request).unwrap().is_none());
    assert_eq!(
        serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap(),
        serde_json::to_value(session).unwrap()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_hardlinks_permissions_and_corrupt_backups_fail_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for case in ["symlink", "hardlink", "public", "backup", "marker"] {
        let fixture = Fixture::new().await;
        match case {
            "symlink" => {
                let other = fixture.source.with_extension("other");
                std::fs::rename(&fixture.source, &other).unwrap();
                symlink(other, &fixture.source).unwrap();
            }
            "hardlink" => {
                std::fs::hard_link(&fixture.source, fixture.source.with_extension("alias")).unwrap()
            }
            "public" => {
                std::fs::set_permissions(&fixture.source, std::fs::Permissions::from_mode(0o644))
                    .unwrap()
            }
            "backup" | "marker" => {
                assert!(
                    transfer_inner(
                        fixture.store.clone(),
                        fixture.coordinator.clone(),
                        fixture.journal.clone(),
                        fixture.request.clone(),
                        CancellationToken::new(),
                        Boundary::BeforeCommit
                    )
                    .await
                    .is_err()
                );
                if case == "backup" {
                    std::fs::write(
                        fixture
                            .journal
                            .join("imports")
                            .join(format!("{}.json", fixture.request.transfer_id)),
                        b"corrupt",
                    )
                    .unwrap();
                } else {
                    let mut marker: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(&fixture.source).unwrap()).unwrap();
                    marker["provenance"]["source_revision"] = serde_json::json!(999);
                    std::fs::write(&fixture.source, serde_json::to_vec(&marker).unwrap()).unwrap();
                }
            }
            _ => unreachable!(),
        }
        assert!(fixture.transfer().await.is_err());
        if fixture.journal.exists() {
            assert!(
                Journal::open(fixture.journal.clone())
                    .unwrap()
                    .load_session(fixture.request.session_id)
                    .is_err()
            );
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn backup_capacity_is_bounded_and_existing_retry_remains_possible() {
    use std::os::unix::fs::OpenOptionsExt;
    let fixture = Fixture::new().await;
    assert!(
        transfer_inner(
            fixture.store.clone(),
            fixture.coordinator.clone(),
            fixture.journal.clone(),
            fixture.request.clone(),
            CancellationToken::new(),
            Boundary::BackupDurable
        )
        .await
        .is_err()
    );
    let imports = fixture.journal.join("imports");
    for n in 0..255 {
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(imports.join(format!("orphan-{n}.json")))
            .unwrap();
    }
    let mut another = fixture.request.clone();
    another.transfer_id = Uuid::new_v4();
    let error = transfer(
        fixture.store.clone(),
        fixture.coordinator.clone(),
        fixture.journal.clone(),
        another,
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("capacity"));
    assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
    fixture.transfer().await.unwrap(); // own exact existing backup consumes no new capacity
}

#[cfg(unix)]
#[tokio::test]
async fn backup_byte_limit_and_capacity_lock_reject_before_source_transition() {
    use std::os::unix::fs::OpenOptionsExt;
    let fixture = Fixture::new().await;
    assert!(
        transfer_inner(
            fixture.store.clone(),
            fixture.coordinator.clone(),
            fixture.journal.clone(),
            fixture.request.clone(),
            CancellationToken::new(),
            Boundary::BackupDurable
        )
        .await
        .is_err()
    );
    let filler = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(fixture.journal.join("imports/orphan.json"))
        .unwrap();
    filler.set_len(256 * 1024 * 1024).unwrap();
    let mut request = fixture.request.clone();
    request.transfer_id = Uuid::new_v4();
    assert!(
        transfer(
            fixture.store.clone(),
            fixture.coordinator.clone(),
            fixture.journal.clone(),
            request,
            CancellationToken::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("capacity")
    );
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(fixture.journal.join("imports.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(
        fixture
            .transfer()
            .await
            .unwrap_err()
            .to_string()
            .contains("capacity busy")
    );
    assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
}

#[cfg(unix)]
#[tokio::test]
async fn session_fence_cancellation_and_bound_store_rejection_preserve_source() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .store
        .with_execution(fixture.request.session_id)
        .await
        .unwrap();
    assert!(
        transfer(
            owner.clone(),
            fixture.coordinator.clone(),
            fixture.journal.clone(),
            fixture.request.clone(),
            CancellationToken::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("unowned")
    );
    let cancel = CancellationToken::new();
    let attempt = tokio::spawn(transfer(
        fixture.store.clone(),
        fixture.coordinator.clone(),
        fixture.journal.clone(),
        fixture.request.clone(),
        cancel.clone(),
    ));
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    cancel.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), attempt)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
    assert!(!fixture.journal.exists());
    drop(owner);
    fixture.transfer().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unsupported_source_fields_and_system_guidance_fail_before_transition() {
    for case in ["unknown", "system", "overflow"] {
        let mut fixture = Fixture::new().await;
        let mut value: serde_json::Value = serde_json::from_slice(&fixture.original).unwrap();
        match case {
            "unknown" => {
                value["future_field"] = serde_json::json!("preserve rather than silently omit")
            }
            "system" => value["messages"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::to_value(Message::new(Role::System, "runtime only")).unwrap()),
            "overflow" => {
                value["revision"] = serde_json::json!(u64::MAX);
                fixture.request.expected_revision = u64::MAX;
            }
            _ => unreachable!(),
        }
        fixture.original = serde_json::to_vec(&value).unwrap();
        std::fs::write(&fixture.source, &fixture.original).unwrap();
        fixture.request.source_sha256 = hex::encode(Sha256::digest(&fixture.original));
        assert!(fixture.transfer().await.is_err());
        assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
        assert!(!fixture.journal.exists());
    }
}

#[cfg(unix)]
#[tokio::test]
async fn database_failure_after_marker_is_pending_and_retries_atomically() {
    let fixture = Fixture::new().await;
    let _journal = Journal::open(fixture.journal.clone()).unwrap();
    let db = rusqlite::Connection::open(fixture.journal.join("journal.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER fail_transfer BEFORE INSERT ON imports BEGIN SELECT RAISE(ABORT, 'injected provenance failure'); END;").unwrap();
    assert!(fixture.transfer().await.is_err());
    assert!(
        fixture
            .store
            .load(fixture.request.session_id)
            .await
            .is_err()
    );
    assert!(_journal.load_session(fixture.request.session_id).is_err());
    assert_eq!(
        db.query_row("SELECT count(*) FROM imports", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_transfer").unwrap();
    assert!(!fixture.transfer().await.unwrap().duplicate);
    assert!(_journal.load_session(fixture.request.session_id).is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_journal_requires_explicit_upgrade_before_source_transition() {
    let fixture = Fixture::new().await;
    drop(Journal::open(fixture.journal.clone()).unwrap());
    let db = rusqlite::Connection::open(fixture.journal.join("journal.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE local_tool_reconciliations; DROP TABLE local_cleanup_obligations; DROP TABLE local_cancel_intents; DROP TABLE steering; DROP TABLE imports; UPDATE attachment_schema SET version=2 WHERE id=1;")
        .unwrap();
    assert!(
        fixture
            .transfer()
            .await
            .unwrap_err()
            .to_string()
            .contains("explicit quiescent")
    );
    assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.original);
    assert_eq!(
        db.query_row("SELECT version FROM attachment_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    Journal::open(fixture.journal.clone())
        .unwrap()
        .upgrade_quiescent()
        .unwrap();
    fixture.transfer().await.unwrap();
}
