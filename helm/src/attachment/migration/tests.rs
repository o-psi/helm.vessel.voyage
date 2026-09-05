use super::*;
use crate::{Message, Role, attachment::journal::Journal, session::Session};
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
