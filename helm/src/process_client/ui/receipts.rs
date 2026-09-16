//! Durable execution receipts, never unsent composer state.
use super::state::{Pending, View};
use crate::process_client::transport::Client;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

// Bound legacy input as well as the immutable public command envelope.
const MAX_RECEIPT_BYTES: usize = 2 * voyage_protocol::vessel::MAX_VESSEL_BODY;

#[derive(Default, Serialize, Deserialize)]
struct Receipt {
    #[serde(default)]
    pending: Option<CommandReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settlement: Option<super::notifications::Settlement>,
}

#[derive(Serialize, Deserialize)]
struct CommandReceipt {
    #[serde(default)]
    account_host: Option<uuid::Uuid>,
    command_id: uuid::Uuid,
    incarnation: uuid::Uuid,
    #[serde(default)]
    original: Option<Box<voyage_protocol::vessel::VoyageCommand>>,
    #[serde(default)]
    receipt_only: bool,
    // Read only to recognize old lifecycle operations lacking an envelope.
    // Never serialize this field or restore it into a composer.
    #[serde(default, skip_serializing)]
    draft: String,
}

impl CommandReceipt {
    fn from_pending(pending: &Pending) -> Self {
        Self {
            account_host: pending.account_host,
            command_id: pending.command_id,
            incarnation: pending.incarnation,
            original: pending.original.clone(),
            receipt_only: matches!(
                pending.resolution(),
                voyage_protocol::vessel::VoyageCommand::Receipt { .. }
            ),
            draft: String::new(),
        }
    }

    fn into_pending(self) -> Pending {
        let receipt_only = self.receipt_only
            || (self.original.is_none()
                && (self.draft.trim() == "/branch"
                    || self.draft.trim_start().starts_with("/branch ")
                    || self.draft.trim() == "/restore"));
        Pending {
            account_host: self.account_host,
            command_id: self.command_id,
            incarnation: self.incarnation,
            original: self.original,
            receipt_only,
            draft: String::new(),
            preserve_draft: false,
        }
    }
}

fn decode(bytes: &[u8]) -> Result<Receipt> {
    let mut receipt: Receipt = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("Invalid saved execution receipt; original file preserved"))?;
    // Normalize legacy lifecycle semantics before discarding the old UI text.
    receipt.pending = receipt
        .pending
        .map(|p| CommandReceipt::from_pending(&p.into_pending()));
    Ok(receipt)
}

fn root(name: &str) -> PathBuf {
    let root = super::super::cli::default_directory().with_file_name(name);
    #[cfg(test)]
    let root = super::account_test_support::root(name, root);
    root
}

fn stable_name(id: uuid::Uuid, session: uuid::Uuid) -> Result<String> {
    let identity = serde_json::to_vec(&("connection-v1", id, session))?;
    Ok(format!("{:x}.json", Sha256::digest(identity)))
}

fn path(client: &Client, view: &View) -> Result<PathBuf> {
    let root = root("helm-command-receipts");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::create_dir_all(root.parent().expect("receipt parent"))?;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    super::super::local::check_private_directory(&root)?;
    Ok(root.join(stable_name(client.id(), view.process.session_id)?))
}

fn write_receipt(path: &Path, receipt: &Receipt, replace: bool) -> Result<()> {
    let bytes = serde_json::to_vec(receipt)?;
    ensure!(
        bytes.len() <= MAX_RECEIPT_BYTES,
        "saved execution receipt exceeds recovery limit"
    );
    let root = path.parent().expect("receipt parent");
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path)?;
    } else {
        temporary.persist_noclobber(path)?;
    }
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

fn migrate_source(client: &Client, source: &Path, destination: &Path) -> Result<()> {
    super::super::local::check_private_directory(source.parent().expect("source parent"))?;
    let receipt = decode(&read_private(source, MAX_RECEIPT_BYTES)?)?;
    let owner = client.id().to_string();
    // Honor claims made by earlier Helm versions without touching helm-views.
    let old_claim = source.with_extension("migration");
    if old_claim.try_exists()? {
        let binding = read_private(&old_claim, 128)?;
        ensure!(
            binding.is_empty() || binding == owner.as_bytes(),
            "Legacy receipt already belongs to another immutable connection; original retained"
        );
    }
    // Shared across all destinations claiming this exact source identity.
    let root = destination.parent().expect("receipt parent");
    let source_identity = serde_json::to_vec(source)?;
    let claim_path = root.join(format!("{:x}.migration", Sha256::digest(source_identity)));
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut claim = options.open(&claim_path)?;
    claim.try_lock().map_err(|_| {
        anyhow::anyhow!("Receipt migration is open in another Helm; original retained")
    })?;
    let binding = read_private(&claim_path, 128)?;
    if binding.is_empty() {
        claim.write_all(owner.as_bytes())?;
        claim.sync_all()?;
        #[cfg(unix)]
        std::fs::File::open(root)?.sync_all()?;
    } else {
        ensure!(
            binding == owner.as_bytes(),
            "Legacy receipt already belongs to another immutable connection; original retained"
        );
    }
    if !destination.try_exists()? {
        write_receipt(destination, &receipt, false)?;
    }
    Ok(())
}

fn migrate_legacy(client: &Client, view: &View, destination: &Path) -> Result<()> {
    let session = view.process.session_id;
    let old_root = root("helm-views");
    let mut sources = vec![old_root.join(stable_name(client.id(), session)?)];
    if !client.managed().is_some_and(|c| c.legacy_route.is_none()) {
        let legacy = client.legacy_route();
        // A startup connection may already have receipts in the new store.
        if legacy.id() != client.id() {
            sources.insert(
                1,
                root("helm-command-receipts").join(stable_name(legacy.id(), session)?),
            );
            sources.push(old_root.join(stable_name(legacy.id(), session)?));
        }
        let identity = serde_json::to_vec(&(
            Option::<&str>::None,
            &legacy.access_file,
            &legacy.directory,
            session,
        ))?;
        sources.push(old_root.join(format!("{:x}.json", Sha256::digest(identity))));
    }
    for source in sources {
        if source.try_exists()? {
            return migrate_source(client, &source, destination);
        }
    }
    Ok(())
}

pub fn load(client: &Client, view: &mut View) -> Result<()> {
    let path = path(client, view)?;
    if !path.try_exists()? {
        migrate_legacy(client, view, &path)?;
        if !path.try_exists()? {
            return Ok(());
        }
    }
    let receipt = decode(&read_private(&path, MAX_RECEIPT_BYTES)?)?;
    view.pending = receipt.pending.map(CommandReceipt::into_pending);
    view.settlement = receipt.settlement;
    Ok(())
}

pub fn save(client: &Client, view: &View) -> Result<()> {
    let path = path(client, view)?;
    // Persist an empty receipt too: it prevents cleared commands being resurrected
    // from immutable legacy files on the next launch.
    write_receipt(
        &path,
        &Receipt {
            pending: view.pending.as_ref().map(CommandReceipt::from_pending),
            settlement: view.settlement.clone(),
        },
        true,
    )
}

/// Pin and bound the opened private file, not a prior pathname observation.
#[cfg(unix)]
pub(super) fn read_private(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow::anyhow!("Cannot open private recovery"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= limit as u64,
        "Invalid private recovery file"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "Private recovery exceeds limit");
    Ok(bytes)
}
#[cfg(not(unix))]
pub(super) fn read_private(_path: &std::path::Path, _limit: usize) -> Result<Vec<u8>> {
    anyhow::bail!("Private recovery storage unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use voyage_protocol::vessel::VoyageCommand;

    fn view() -> View {
        View::new(
            serde_json::from_value(serde_json::json!({
                "session_id": Uuid::new_v4(), "incarnation": Uuid::new_v4(),
                "workspace": "/synthetic", "state": "live", "name": "receipt test"
            }))
            .unwrap(),
        )
    }

    fn pending() -> Pending {
        let command_id = Uuid::new_v4();
        Pending {
            account_host: Some(Uuid::new_v4()),
            command_id,
            incarnation: Uuid::new_v4(),
            draft: "UNSENT COMPOSER SECRET".into(),
            preserve_draft: true,
            original: Some(Box::new(VoyageCommand::Submit {
                coordination: None,
                command_id,
                expected_revision: 7,
                expires_at_ms: 123456,
                prompt: "exact dispatched prompt".into(),
            })),
            receipt_only: false,
        }
    }

    #[test]
    fn receipt_serialization_excludes_preserved_composer_and_keeps_exact_command() {
        let pending = pending();
        let receipt = Receipt {
            pending: Some(CommandReceipt::from_pending(&pending)),
            settlement: None,
        };
        let bytes = serde_json::to_vec(&receipt).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        for field in ["text", "images", "markers", "shared"] {
            assert!(json.get(field).is_none());
        }
        assert!(json["pending"].get("draft").is_none());
        assert!(json["pending"].get("preserve_draft").is_none());
        assert!(!String::from_utf8_lossy(&bytes).contains("UNSENT"));
        let restored = decode(&bytes).unwrap().pending.unwrap().into_pending();
        assert_eq!(
            serde_json::to_value(&restored.original).unwrap(),
            serde_json::to_value(&pending.original).unwrap()
        );
        assert_eq!(restored.command_id, pending.command_id);
        assert_eq!(restored.incarnation, pending.incarnation);
        assert_eq!(restored.account_host, pending.account_host);
        assert!(restored.draft.is_empty());
        assert!(!restored.preserve_draft);
    }

    #[test]
    fn legacy_unsent_fields_are_ignored_even_when_not_valid_composer_data() {
        let bytes = br#"{"text":{"invalid":"UNSENT"},"images":"invalid","markers":17,"shared":false,"pending":null}"#;
        let receipt = decode(bytes).unwrap();
        assert!(receipt.pending.is_none());
        assert!(receipt.settlement.is_none());
        assert_eq!(
            serde_json::to_string(&receipt).unwrap(),
            r#"{"pending":null}"#
        );
    }

    #[test]
    fn legacy_lifecycle_resolution_survives_without_draft_text() {
        let mut pending = pending();
        pending.original = None;
        pending.draft = "/branch old target".into();
        let bytes = serde_json::to_vec(&serde_json::json!({"pending": pending})).unwrap();
        let receipt = decode(&bytes).unwrap();
        let saved = serde_json::to_vec(&receipt).unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("old target"));
        let pending = decode(&saved).unwrap().pending.unwrap().into_pending();
        assert!(matches!(
            pending.resolution(),
            VoyageCommand::Receipt { .. }
        ));
        assert!(pending.draft.is_empty());
    }

    #[cfg(unix)]
    fn private_write(path: &Path, bytes: &[u8]) {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let parent = path.parent().unwrap();
        if !parent.exists() {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(parent)
                .unwrap();
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn legacy_recovery_retains_original_and_never_restores_unsent_state() {
        let fixture = super::super::account_test_support::Fixture::new();
        let client = Client::local(fixture.0.path().join("vessel"));
        let mut view = view();
        view.draft.text = "current unsent composer".into();
        let pending = pending();
        let settlement: super::super::notifications::Settlement = serde_json::from_value(
            serde_json::json!({"run_id": Uuid::new_v4(), "since": "2026-01-01T00:00:00Z"}),
        )
        .unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "text": "legacy unsent", "images": "invalid", "markers": false,
            "shared": 42, "pending": pending, "settlement": settlement,
        }))
        .unwrap();
        let source =
            root("helm-views").join(stable_name(client.id(), view.process.session_id).unwrap());
        private_write(&source, &bytes);
        load(&client, &mut view).unwrap();
        assert_eq!(view.draft.text, "current unsent composer");
        assert!(view.images.is_empty());
        assert!(view.pending.as_ref().unwrap().draft.is_empty());
        assert_eq!(view.settlement, Some(settlement.clone()));
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(root("helm-views")).unwrap().count(), 1);
        let saved = std::fs::read(path(&client, &view).unwrap()).unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("unsent"));
        assert!(!String::from_utf8_lossy(&saved).contains("UNSENT"));
        view.pending = None;
        view.settlement = None;
        save(&client, &view).unwrap();
        load(&client, &mut view).unwrap();
        assert!(view.pending.is_none());
        assert!(view.settlement.is_none());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn legacy_tuple_recovery_honors_existing_connection_claim() {
        let fixture = super::super::account_test_support::Fixture::new();
        let client = Client::local(fixture.0.path().join("vessel"));
        let mut view = view();
        let legacy = client.legacy_route();
        let identity = serde_json::to_vec(&(
            Option::<&str>::None,
            &legacy.access_file,
            &legacy.directory,
            view.process.session_id,
        ))
        .unwrap();
        let source = root("helm-views").join(format!("{:x}.json", Sha256::digest(identity)));
        private_write(&source, br#"{"text":"unsent","pending":null}"#);
        private_write(
            &source.with_extension("migration"),
            Uuid::new_v4().to_string().as_bytes(),
        );
        assert!(load(&client, &mut view).is_err());
        assert!(!path(&client, &view).unwrap().exists());
        // A matching historical claim permits recovery without modifying it.
        std::fs::write(source.with_extension("migration"), client.id().to_string()).unwrap();
        load(&client, &mut view).unwrap();
        assert!(view.draft.text.is_empty());
        assert_eq!(
            std::fs::read_to_string(&source).unwrap(),
            r#"{"text":"unsent","pending":null}"#
        );
    }
    #[cfg(unix)]
    #[test]
    fn imported_connection_prefers_startup_receipts_and_claims_only_once() {
        use crate::process_client::connections::{Connection, Metadata, Scope};
        let fixture = super::super::account_test_support::Fixture::new();
        let startup = Client::local(fixture.0.path().join("vessel"));
        let mut view = view();
        let connection = Connection {
            id: Uuid::new_v4(),
            alias: "imported".into(),
            endpoint: "https://example.invalid".into(),
            vessel_id: Uuid::new_v4(),
            principal_id: None,
            grant_id: Uuid::new_v4(),
            scope: Scope::Session {
                session_id: view.process.session_id,
            },
            credential_ref: Uuid::new_v4(),
            autoconnect: false,
            workspace_preference: None,
            revision: 0,
            forgotten: false,
            legacy_route: Some(startup.legacy_route()),
            metadata: Metadata::default(),
        };
        let imported =
            Client::from_connection(connection.clone(), fixture.0.path().join("credential"));
        let pending = pending();
        let old =
            root("helm-views").join(stable_name(startup.id(), view.process.session_id).unwrap());
        let old_bytes =
            serde_json::to_vec(&serde_json::json!({"text": "UNSENT", "pending": pending})).unwrap();
        private_write(&old, &old_bytes);
        // An empty newer receipt must suppress stale pending commands in helm-views.
        save(&startup, &view).unwrap();
        load(&imported, &mut view).unwrap();
        assert!(view.pending.is_none());
        assert!(view.draft.text.is_empty());
        assert_eq!(std::fs::read(&old).unwrap(), old_bytes);
        let mut other = connection;
        other.id = Uuid::new_v4();
        let other = Client::from_connection(other, fixture.0.path().join("other-credential"));
        assert!(load(&other, &mut view).is_err());
        assert!(!path(&other, &view).unwrap().exists());
    }
}
