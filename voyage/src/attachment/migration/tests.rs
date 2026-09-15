use super::*;
use crate::{
    attachment::journal::Journal,
    model::{Message, Role},
    session::Session,
};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

struct Fixture {
    _root: tempfile::TempDir,
    store: SessionStore,
    coordinator: Coordinator,
    directory: PathBuf,
    source: PathBuf,
    session: Session,
    original: Vec<u8>,
    request: TransferRequest,
}

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        private_dir(&workspace);
        let data = root.path().join("data");
        private_dir(&data);
        let store = SessionStore::new(data.join("sessions"));
        let mut session = Session::new(workspace.clone(), "private-fixture".into());
        session
            .messages
            .push(Message::new(Role::User, "preserve canonical input"));
        session
            .messages
            .push(Message::new(Role::Assistant, "preserve canonical answer"));
        store.save(&mut session).await.unwrap();
        let source = data.join("sessions").join(format!("{}.json", session.id));
        let original = fs::read(&source).unwrap();
        let completion = data.join("completion");
        private_dir(&completion);
        let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
        let coordinator = Coordinator::open(completion.join(key), &workspace).unwrap();
        let request = TransferRequest {
            transfer_id: Uuid::new_v4(),
            session_id: session.id,
            expected_revision: session.revision,
            source_sha256: hex::encode(Sha256::digest(&original)),
        };
        Self {
            directory: data.join("journal"),
            _root: root,
            store,
            coordinator,
            source,
            session,
            original,
            request,
        }
    }

    async fn run(&self, boundary: Boundary) -> Result<TransferReceipt> {
        transfer_inner(
            self.store.clone(),
            self.coordinator.clone(),
            self.directory.clone(),
            self.request.clone(),
            CancellationToken::new(),
            boundary,
        )
        .await
    }

    fn backup(&self) -> PathBuf {
        self.directory
            .join("imports")
            .join(format!("{}.json", self.request.transfer_id))
    }

    fn replace_source(&mut self, value: serde_json::Value) {
        self.original = serde_json::to_vec(&value).unwrap();
        fs::write(&self.source, &self.original).unwrap();
        self.request.source_sha256 = hex::encode(Sha256::digest(&self.original));
    }
}

#[tokio::test]
async fn transfer_preserves_snapshot_and_retries_without_reimporting() {
    let f = Fixture::new().await;
    let receipt = f.run(Boundary::None).await.unwrap();
    assert_eq!(receipt.transfer_id, f.request.transfer_id);
    assert_eq!(receipt.session_id, f.session.id);
    assert_eq!(receipt.journal_revision, f.session.revision);
    assert!(!receipt.duplicate);
    assert_eq!(fs::read(f.backup()).unwrap(), f.original);
    let marker: Marker = serde_json::from_slice(&fs::read(&f.source).unwrap()).unwrap();
    assert_eq!(marker.format, "helm.session-transfer");
    assert_eq!(marker.version, 1);
    assert_eq!(marker.provenance.source_sha256, f.request.source_sha256);
    assert!(
        f.store.load(f.session.id).await.is_err(),
        "JSON authority must be retired"
    );
    let journal = Journal::open(f.directory.clone()).unwrap();
    let imported = journal.load_session(f.session.id).unwrap();
    assert_eq!(imported.revision, f.session.revision);
    assert_eq!(
        serde_json::to_value(&imported.session.messages).unwrap(),
        serde_json::to_value(&f.session.messages).unwrap()
    );
    drop(journal);
    let retry = f.run(Boundary::None).await.unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.journal_revision, receipt.journal_revision);
}

#[tokio::test]
async fn every_publication_boundary_recovers_with_exactly_one_authority() {
    for boundary in [
        Boundary::BackupPublished,
        Boundary::BackupDurable,
        Boundary::MarkerPublished,
        Boundary::MarkerDurable,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
    ] {
        let f = Fixture::new().await;
        let error = f.run(boundary).await.unwrap_err();
        assert!(error.to_string().contains("injected transfer interruption"));
        assert_eq!(fs::read(f.backup()).unwrap(), f.original);
        let journal = Journal::open(f.directory.clone()).unwrap();
        assert_eq!(
            journal.load_session(f.session.id).is_ok(),
            boundary == Boundary::AfterCommit
        );
        drop(journal);
        let still_json = matches!(
            boundary,
            Boundary::BackupPublished | Boundary::BackupDurable
        );
        assert_eq!(fs::read(&f.source).unwrap() == f.original, still_json);
        let recovered = f.run(Boundary::None).await.unwrap();
        assert_eq!(recovered.duplicate, boundary == Boundary::AfterCommit);
        assert!(f.run(Boundary::None).await.unwrap().duplicate);
        assert_eq!(fs::read(f.backup()).unwrap(), f.original);
    }
}

#[tokio::test]
async fn invalid_requests_are_refused_before_publication() {
    for case in 0..6 {
        let mut f = Fixture::new().await;
        match case {
            0 => f.request.transfer_id = Uuid::nil(),
            1 => f.request.session_id = Uuid::nil(),
            2 => f.request.source_sha256 = "A".repeat(64),
            3 => f.request.source_sha256 = "a".repeat(63),
            4 => f.request.source_sha256 = "0".repeat(64),
            _ => f.request.expected_revision += 1,
        }
        assert!(f.run(Boundary::None).await.is_err());
        assert_eq!(fs::read(&f.source).unwrap(), f.original);
        assert!(!f.directory.exists());
    }
}

#[tokio::test]
async fn pre_cancelled_and_already_owned_sources_cannot_transfer() {
    let f = Fixture::new().await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = transfer(
        f.store.clone(),
        f.coordinator.clone(),
        f.directory.clone(),
        f.request.clone(),
        cancel,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    let owned = f.store.with_execution(f.session.id).await.unwrap();
    let error = transfer(
        owned,
        f.coordinator.clone(),
        f.directory.clone(),
        f.request.clone(),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("fresh unowned"));
    assert!(!f.directory.exists());
    assert_eq!(fs::read(&f.source).unwrap(), f.original);
}

#[tokio::test]
async fn source_semantics_are_checked_before_retiring_json() {
    for case in 0..5 {
        let mut f = Fixture::new().await;
        let mut value: serde_json::Value = serde_json::from_slice(&f.original).unwrap();
        match case {
            0 => value["unknown_future_field"] = serde_json::json!(true),
            1 => value["id"] = serde_json::json!(Uuid::new_v4()),
            2 => {
                value["messages"] =
                    serde_json::to_value(vec![Message::new(Role::System, "legacy instructions")])
                        .unwrap()
            }
            3 => {
                value["revision"] = serde_json::json!(u64::MAX);
                f.request.expected_revision = u64::MAX;
            }
            _ => value["workspace"] = serde_json::json!(f._root.path().join("absent")),
        }
        f.replace_source(value);
        assert!(f.run(Boundary::None).await.is_err());
        assert_eq!(fs::read(&f.source).unwrap(), f.original);
        assert!(!f.directory.exists());
    }
}

#[tokio::test]
async fn private_source_rejects_public_permissions_symlinks_and_hardlinks() {
    for case in 0..3 {
        let f = Fixture::new().await;
        match case {
            0 => fs::set_permissions(&f.source, fs::Permissions::from_mode(0o644)).unwrap(),
            1 => {
                let target = f.source.with_extension("original");
                fs::rename(&f.source, &target).unwrap();
                std::os::unix::fs::symlink(target, &f.source).unwrap();
            }
            _ => fs::hard_link(&f.source, f.source.with_extension("linked")).unwrap(),
        }
        assert!(f.run(Boundary::None).await.is_err());
        assert!(!f.directory.exists());
        assert_eq!(fs::read(&f.source).unwrap(), f.original);
    }
}

#[tokio::test]
async fn coordinator_and_destination_conflicts_do_not_retire_source() {
    let mut f = Fixture::new().await;
    f.coordinator = Coordinator::open(
        f._root.path().join("wrong-coordinator"),
        &f.session.workspace,
    )
    .unwrap();
    assert!(
        f.run(Boundary::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("incorrect workspace execution fence")
    );
    assert!(!f.directory.exists());
    let f = Fixture::new().await;
    let mut journal = Journal::open(f.directory.clone()).unwrap();
    journal.create_session(&f.session).unwrap();
    drop(journal);
    assert!(
        f.run(Boundary::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("destination already owns")
    );
    assert_eq!(fs::read(&f.source).unwrap(), f.original);
    assert!(!f.backup().exists());
}

#[tokio::test]
async fn backup_and_marker_tampering_never_replays_a_transfer() {
    for case in 0..4 {
        let f = Fixture::new().await;
        assert!(f.run(Boundary::MarkerDurable).await.is_err());
        if case == 0 {
            fs::write(f.backup(), b"different source").unwrap();
        } else {
            let mut marker: serde_json::Value =
                serde_json::from_slice(&fs::read(&f.source).unwrap()).unwrap();
            match case {
                1 => marker["version"] = serde_json::json!(2),
                2 => {
                    marker["provenance"]["backup"] =
                        serde_json::json!(f._root.path().join("untrusted"))
                }
                _ => marker["format"] = serde_json::json!("other-format"),
            }
            fs::write(&f.source, serde_json::to_vec(&marker).unwrap()).unwrap();
        }
        let before = fs::read(&f.source).unwrap();
        assert!(f.run(Boundary::None).await.is_err());
        assert_eq!(fs::read(&f.source).unwrap(), before);
        assert!(
            Journal::open(f.directory.clone())
                .unwrap()
                .load_session(f.session.id)
                .is_err()
        );
    }
}

#[tokio::test]
async fn backup_collision_is_not_overwritten_and_completed_authority_is_not_replayed() {
    let f = Fixture::new().await;
    assert!(f.run(Boundary::BackupDurable).await.is_err());
    fs::write(f.backup(), b"reserved by another source").unwrap();
    assert!(
        f.run(Boundary::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("backup collision")
    );
    assert_eq!(fs::read(&f.source).unwrap(), f.original);
    assert_eq!(fs::read(f.backup()).unwrap(), b"reserved by another source");

    let f = Fixture::new().await;
    f.run(Boundary::None).await.unwrap();
    fs::write(&f.source, &f.original).unwrap();
    assert!(
        f.run(Boundary::None)
            .await
            .unwrap_err()
            .to_string()
            .contains("active JSON conflicts")
    );
    assert_eq!(fs::read(&f.source).unwrap(), f.original);
}
