use super::*;
use crate::{Message, Role, session::Session, attachment::journal::Journal};
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
        let workspace = temporary.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let store = SessionStore::new(data.join("sessions"));
        let mut session = Session::new(workspace.clone(), "fixture-model".into());
        session.messages.push(Message::new(Role::User, "canonical 雪 input"));
        session.messages.push(Message::new(Role::Assistant, "original answer"));
        session.usage.input_tokens = 123;
        session.usage.output_tokens = 9;
        store.save(&mut session).await.unwrap();
        let source = data.join("sessions").join(format!("{}.json", session.id));
        let original = std::fs::read(&source).unwrap();
        let request = TransferRequest { transfer_id: Uuid::new_v4(), session_id: session.id,
            expected_revision: session.revision, source_sha256: hex::encode(Sha256::digest(&original)) };
        let completion = data.join("completion");
        std::fs::create_dir(&completion).unwrap();
        let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
        let coordinator = Coordinator::open(completion.join(key), &workspace).unwrap();
        let journal = data.join("journal");
        Self { _temporary: temporary, store, coordinator, journal, source, request, original }
    }
    async fn transfer(&self) -> Result<TransferReceipt> {
        transfer(self.store.clone(), self.coordinator.clone(), self.journal.clone(), self.request.clone(), CancellationToken::new()).await
    }
}

#[cfg(unix)]
#[tokio::test]
async fn successful_transfer_preserves_original_and_rejects_old_json_readers() {
    let fixture = Fixture::new().await;
    let result = fixture.transfer().await.unwrap();
    assert_eq!(result.session_id, fixture.request.session_id);
    assert!(!result.duplicate);
    assert!(fixture.store.load(fixture.request.session_id).await.is_err());
    assert!(serde_json::from_slice::<Session>(&std::fs::read(&fixture.source).unwrap()).is_err());
    let journal = Journal::open(fixture.journal.clone()).unwrap();
    let imported = journal.load_session(fixture.request.session_id).unwrap();
    let original: Session = serde_json::from_slice(&fixture.original).unwrap();
    assert_eq!(serde_json::to_value(&imported.session).unwrap(), serde_json::to_value(&original).unwrap());
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
