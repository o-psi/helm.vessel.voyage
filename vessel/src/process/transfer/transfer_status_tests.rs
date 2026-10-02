//! Actual ordinary Supervisor status/receipt/locking boundaries, without runtime execution.
//! Preparation and historical completion below are explicit private metadata fixtures.
use super::*;
use crate::process::{database, identity, test_support::Fixture};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(5);
const ARTIFACT: &[u8] = b"bounded synthetic portable checkpoint";

#[derive(PartialEq, Eq)]
struct Node {
    uid: u32,
    gid: u32,
    mode: u32,
    dev: u64,
    ino: u64,
    links: u64,
    len: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    bytes: Vec<u8>,
}
fn graph(root: &Path) -> BTreeMap<PathBuf, Node> {
    fn visit(root: &Path, path: &Path, out: &mut BTreeMap<PathBuf, Node>, total: &mut usize) {
        let m = fs::symlink_metadata(path).unwrap();
        let bytes = if m.file_type().is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if m.is_file() {
            assert!(m.len() <= 8 * 1024 * 1024, "private fixture file bound");
            let mut bytes = Vec::new();
            fs::File::open(path)
                .unwrap()
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .unwrap();
            assert!(bytes.len() <= 8 * 1024 * 1024);
            bytes
        } else {
            assert!(m.is_dir());
            Vec::new()
        };
        *total += bytes.len();
        assert!(*total <= 32 * 1024 * 1024 && out.len() < 4096);
        out.insert(
            path.strip_prefix(root).unwrap().into(),
            Node {
                uid: m.uid(),
                gid: m.gid(),
                mode: m.mode(),
                dev: m.dev(),
                ino: m.ino(),
                links: m.nlink(),
                len: m.len(),
                modified: (m.mtime(), m.mtime_nsec()),
                changed: (m.ctime(), m.ctime_nsec()),
                bytes,
            },
        );
        if m.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), out, total);
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(root, root, &mut out, &mut 0);
    out
}
struct Journey {
    source: Fixture,
    destination: Fixture,
    supervisor: Supervisor,
    manifest: SignedArtifact<TransferManifest>,
    activation: Uuid,
}
impl Journey {
    async fn new() -> Self {
        let source = Fixture::new();
        let destination = Fixture::new();
        let supervisor = destination.supervisor().await;
        let source_id = identity::public(&source.0).unwrap();
        let dest_id = identity::public(&destination.0).unwrap();
        identity::pin(&destination.0, &source_id).unwrap();
        let id = Uuid::new_v4();
        let session = Uuid::new_v4();
        let preparation = identity::sign(
            &destination.0,
            TransferPreparation {
                transfer_id: id,
                source_vessel_id: source_id.vessel_id,
                destination_vessel_id: dest_id.vessel_id,
                session_id: session,
                nonce: Uuid::new_v4(),
                workspace: destination.0.clone(),
                expires_at_ms: store::now().unwrap() + 60_000,
            },
        )
        .unwrap();
        let manifest = identity::sign(
            &source.0,
            TransferManifest {
                transfer_id: id,
                source_vessel_id: source_id.vessel_id,
                destination_vessel_id: dest_id.vessel_id,
                session_id: session,
                source_incarnation: Uuid::new_v4(),
                prepare_digest: identity::digest(&preparation).unwrap(),
                artifact_sha256: format!("{:x}", Sha256::digest(ARTIFACT)),
                artifact_bytes: ARTIFACT.len() as u64,
                generation: 1,
            },
        )
        .unwrap();
        // No validate-start helper or executable is launched: only seed a signed
        // preparation and original catalogue witness for the actual status API.
        registry::private_directory(&directory(&destination.0, id)).unwrap();
        let namespace = fs::metadata(destination.0.join("catalogue.sqlite3")).unwrap();
        save(
            &destination.0,
            id,
            &Prepared {
                preparation,
                config_path: None,
                manifest: None,
                activated: false,
                activation_command: None,
                catalogue_namespace: Some((namespace.dev(), namespace.ino())),
            },
        )
        .unwrap();
        Self {
            source,
            destination,
            supervisor,
            manifest,
            activation: Uuid::new_v4(),
        }
    }
    fn id(&self) -> Uuid {
        self.manifest.payload.transfer_id
    }
    fn request(&self) -> VesselCommand {
        VesselCommand::TransferStatus {
            transfer_id: self.id(),
            activate_command_id: self.activation,
            expected_manifest_digest: identity::digest(&self.manifest).unwrap(),
            manifest: self.manifest.clone(),
        }
    }
    fn activate(&self) -> VesselCommand {
        VesselCommand::ActivateTransfer {
            command_id: self.activation,
            transfer_id: self.id(),
        }
    }
    fn upload(&self, bytes: &[u8]) -> VesselCommand {
        VesselCommand::UploadTransferChunk {
            transfer_id: self.id(),
            offset: 0,
            data: STANDARD.encode(bytes),
        }
    }
    async fn status(&self) -> TransferStatus {
        serde_json::from_value(
            tokio::time::timeout(WAIT, self.supervisor.handle(self.request()))
                .await
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }
    async fn no_write_status(&self) -> TransferStatus {
        let before = graph(&self.destination.0);
        let status = self.status().await;
        assert!(
            graph(&self.destination.0) == before,
            "readonly status changed private graph"
        );
        assert_eq!(status.transfer_id, self.id());
        assert_eq!(status.activate_command_id, self.activation);
        assert_eq!(
            status.manifest_digest,
            identity::digest(&self.manifest).unwrap()
        );
        assert_eq!(
            status.source_vessel_id,
            self.manifest.payload.source_vessel_id
        );
        assert_eq!(
            status.destination_vessel_id,
            self.manifest.payload.destination_vessel_id
        );
        assert_eq!(status.session_id, self.manifest.payload.session_id);
        assert_eq!(
            status.artifact_sha256,
            self.manifest.payload.artifact_sha256
        );
        assert_eq!(status.artifact_bytes, ARTIFACT.len() as u64);
        assert_eq!(self.supervisor.model_slots.available_permits(), 1);
        assert!(self.supervisor.enrollment_workers.lock().await.is_empty());
        status
    }
    async fn refuse(&self, request: VesselCommand) {
        let before = graph(&self.destination.0);
        assert!(
            tokio::time::timeout(WAIT, self.supervisor.handle(request))
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            graph(&self.destination.0) == before,
            "refusal changed private graph"
        );
    }
    async fn accept(&self) {
        tokio::time::timeout(
            WAIT,
            self.supervisor.handle(VesselCommand::AcceptTransfer {
                manifest: self.manifest.clone(),
            }),
        )
        .await
        .unwrap()
        .unwrap();
    }
    fn intent(&self, command: Uuid) {
        let mut p = load(&self.destination.0, self.id()).unwrap();
        p.activation_command = Some(command);
        save(&self.destination.0, self.id(), &p).unwrap();
    }
    fn registration(&self) -> ProcessRegistration {
        let mut r = self.destination.registration();
        r.session_id = self.manifest.payload.session_id;
        r.command_id = self.activation;
        r.state = ProcessState::Starting;
        r.initialize = Some(RuntimeInitialization::Transfer {
            transfer_id: self.id(),
            artifact_path: directory(&self.destination.0, self.id()).join("artifact.json"),
            sha256: self.manifest.payload.artifact_sha256.clone(),
            prepare_digest: self.manifest.payload.prepare_digest.clone(),
            generation: self.manifest.payload.generation,
        });
        r
    }
    async fn admit(&self) -> ProcessRegistration {
        let r = self.registration();
        database::admit(
            &self.destination.0,
            &r,
            serde_json::to_vec(&self.activate()).unwrap(),
        )
        .await
        .unwrap();
        r
    }
    fn sql(&self, sql: &str) {
        let db = rusqlite::Connection::open(self.destination.0.join("catalogue.sqlite3")).unwrap();
        db.execute_batch(sql).unwrap();
    }
    fn replace_catalogue(&self) {
        let other = Fixture::new();
        // Existing, complete, compatible empty catalogue; not just a missing or
        // corrupt file. Its unrelated namespace must never prove no admission.
        let db = database::open(&other.0).unwrap();
        drop(db);
        let next = self.destination.0.join("replacement-catalogue");
        fs::copy(other.0.join("catalogue.sqlite3"), &next).unwrap();
        fs::set_permissions(&next, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(
            self.destination.0.join("catalogue.sqlite3"),
            self.destination.0.join("original-catalogue-retained"),
        )
        .unwrap();
        fs::rename(next, self.destination.0.join("catalogue.sqlite3")).unwrap();
    }
}
fn receiving(status: &TransferStatus, bytes: u64) {
    assert_eq!(status.state, TransferStatusState::Receiving);
    assert_eq!(status.received_bytes, Some(bytes));
    assert!(status.completion.is_none());
}
fn pending(status: &TransferStatus) {
    assert_eq!(status.state, TransferStatusState::Pending);
    assert!(status.received_bytes.is_none() && status.completion.is_none());
}

#[tokio::test]
async fn anchored_preaccept_partial_and_full_receiving_are_exact_readonly_observations() {
    let f = Journey::new().await;
    receiving(&f.no_write_status().await, 0);
    assert!(load(&f.destination.0, f.id()).unwrap().manifest.is_none());
    f.accept().await;
    f.supervisor.handle(f.upload(&ARTIFACT[..7])).await.unwrap();
    receiving(&f.no_write_status().await, 7);
    f.supervisor
        .handle(VesselCommand::UploadTransferChunk {
            transfer_id: f.id(),
            offset: 7,
            data: STANDARD.encode(&ARTIFACT[7..]),
        })
        .await
        .unwrap();
    receiving(&f.no_write_status().await, ARTIFACT.len() as u64);
    assert!(
        database::creation_receipt(&f.destination.0, f.activation)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn legacy_unanchored_preparation_is_pending_not_upload_or_activation_permission() {
    let f = Journey::new().await;
    let mut p = load(&f.destination.0, f.id()).unwrap();
    p.catalogue_namespace = None;
    save(&f.destination.0, f.id(), &p).unwrap();
    pending(&f.no_write_status().await);
    f.refuse(VesselCommand::AcceptTransfer {
        manifest: f.manifest.clone(),
    })
    .await;
    f.refuse(f.upload(ARTIFACT)).await;
    f.refuse(f.activate()).await;
}
#[tokio::test]
async fn exact_private_activation_intent_is_pending_and_fences_further_uploads() {
    let f = Journey::new().await;
    f.accept().await;
    f.intent(f.activation);
    pending(&f.no_write_status().await);
    f.refuse(f.upload(ARTIFACT)).await;
    pending(&f.no_write_status().await);
}
#[tokio::test]
async fn actual_admission_without_completion_is_pending_with_original_command_and_registration() {
    let f = Journey::new().await;
    f.accept().await;
    let r = f.admit().await;
    pending(&f.no_write_status().await);
    f.refuse(f.upload(ARTIFACT)).await;
    assert_eq!(
        f.supervisor
            .registration(r.session_id)
            .await
            .unwrap()
            .incarnation,
        r.incarnation
    );
    assert!(
        database::creation_receipt(&f.destination.0, f.activation)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn completed_historical_receipt_binds_original_initializer_not_current_owner_liveness() {
    let f = Journey::new().await;
    f.accept().await;
    let mut original = f.admit().await;
    original.state = ProcessState::Suspended;
    let info = ProcessInfo::from(&original);
    // Explicit logical completion fixture: no process ever ran or died here.
    database::settle_creation(&f.destination.0, f.activation, &info)
        .await
        .unwrap();
    let mut current = original.clone();
    current.incarnation = Uuid::new_v4();
    current.command_id = Uuid::new_v4();
    current.restart_from = Some(original.incarnation);
    current.state = ProcessState::Stopped;
    database::save(&f.destination.0, &current).await.unwrap();
    let mut p = load(&f.destination.0, f.id()).unwrap();
    p.catalogue_namespace = None;
    save(&f.destination.0, f.id(), &p).unwrap();
    let status = f.no_write_status().await;
    assert_eq!(status.state, TransferStatusState::Complete);
    assert!(status.received_bytes.is_none());
    let completion = status.completion.unwrap();
    assert_eq!(completion.session_id, original.session_id);
    assert_eq!(completion.incarnation, original.incarnation);
    assert_ne!(completion.incarnation, current.incarnation);
    f.refuse(f.upload(ARTIFACT)).await;
}
#[tokio::test]
async fn receipt_without_admission_or_with_foreign_initializer_cannot_become_completion() {
    for case in 0..3 {
        let f = Journey::new().await;
        let mut r = f.registration();
        if case == 0 {
            database::save(&f.destination.0, &r).await.unwrap();
        } else {
            if case == 1 {
                r.initialize = None;
            } else {
                r.config_path = Some(f.destination.0.join("foreign-config"));
            }
            database::admit(
                &f.destination.0,
                &r,
                serde_json::to_vec(&f.activate()).unwrap(),
            )
            .await
            .unwrap();
        }
        database::settle_creation(&f.destination.0, f.activation, &ProcessInfo::from(&r))
            .await
            .unwrap();
        f.refuse(f.request()).await;
    }
}
#[tokio::test]
async fn wrong_status_ids_digest_signed_manifest_or_signature_refuse_without_reserving_anything() {
    let f = Journey::new().await;
    for case in 0..11 {
        let mut request = f.request();
        if let VesselCommand::TransferStatus {
            transfer_id,
            activate_command_id,
            expected_manifest_digest,
            manifest,
        } = &mut request
        {
            match case {
                0 => *transfer_id = Uuid::nil(),
                1 => *transfer_id = Uuid::new_v4(),
                2 => *activate_command_id = Uuid::nil(),
                3 => *expected_manifest_digest = "short".into(),
                4 => *expected_manifest_digest = "f".repeat(64),
                5 => manifest.payload.session_id = Uuid::new_v4(),
                6 => manifest.payload.destination_vessel_id = Uuid::new_v4(),
                7 => manifest.payload.prepare_digest = "f".repeat(64),
                8 => manifest.payload.generation = 0,
                9 => manifest.payload.source_vessel_id = Uuid::new_v4(),
                _ => manifest.signature = STANDARD.encode([0; 64]),
            }
            if (5..=9).contains(&case) {
                *manifest = identity::sign(&f.source.0, manifest.payload.clone()).unwrap();
                *expected_manifest_digest = identity::digest(manifest).unwrap();
            }
            if case == 10 {
                *expected_manifest_digest = identity::digest(manifest).unwrap();
            }
        }
        f.refuse(request).await;
    }
    receiving(&f.no_write_status().await, 0);
}
#[tokio::test]
async fn missing_corrupt_or_incoherent_identity_and_peer_projection_refuse_without_repair() {
    for case in 0..6 {
        let f = Journey::new().await;
        let identity_dir = f.destination.0.join("identity");
        match case {
            0 => fs::remove_file(identity_dir.join("key.json")).unwrap(),
            1 => fs::remove_file(identity_dir.join("public.json")).unwrap(),
            2 => fs::write(identity_dir.join("key.json"), b"invalid private key").unwrap(),
            3 => store::save(
                &identity_dir.join("public.json"),
                &VesselIdentity {
                    vessel_id: Uuid::new_v4(),
                    public_key: STANDARD.encode([0; 32]),
                },
            )
            .unwrap(),
            4 => fs::remove_file(
                f.destination
                    .0
                    .join("trusted-vessels")
                    .join(format!("{}.json", f.manifest.payload.source_vessel_id)),
            )
            .unwrap(),
            _ => fs::set_permissions(
                identity_dir.join("public.json"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap(),
        }
        f.refuse(f.request()).await;
    }
}
#[tokio::test]
async fn missing_corrupt_replaced_unknown_schema_and_incomplete_catalogues_never_prove_absence() {
    for case in 0..5 {
        let f = Journey::new().await;
        match case {
            0 => fs::rename(
                f.destination.0.join("catalogue.sqlite3"),
                f.destination.0.join("retained-missing-catalogue"),
            )
            .unwrap(),
            1 => fs::write(f.destination.0.join("catalogue.sqlite3"), b"not SQLite").unwrap(),
            2 => f.replace_catalogue(),
            3 => f.sql("UPDATE schema_version SET version=999"),
            _ => f.sql("DROP TABLE creation_receipts"),
        }
        f.refuse(f.request()).await;
    }
}
#[tokio::test]
async fn alternate_activation_and_conflicting_original_command_keep_exact_uncertainty() {
    let f = Journey::new().await;
    f.accept().await;
    let mut r = f.registration();
    r.command_id = Uuid::new_v4();
    let other = VesselCommand::ActivateTransfer {
        command_id: r.command_id,
        transfer_id: f.id(),
    };
    database::admit(&f.destination.0, &r, serde_json::to_vec(&other).unwrap())
        .await
        .unwrap();
    pending(&f.no_write_status().await);
    f.refuse(f.upload(ARTIFACT)).await;
    let g = Journey::new().await;
    database::command(
        &g.destination.0,
        "commands",
        g.activation,
        serde_json::to_vec(&VesselCommand::ActivateTransfer {
            command_id: g.activation,
            transfer_id: Uuid::new_v4(),
        })
        .unwrap(),
        true,
    )
    .await
    .unwrap();
    g.refuse(g.request()).await;
}
#[tokio::test]
async fn unsafe_artifact_leaves_and_changed_complete_payloads_refuse_without_unlink_or_rewrite() {
    for case in 0..5 {
        let f = Journey::new().await;
        f.accept().await;
        f.supervisor.handle(f.upload(ARTIFACT)).await.unwrap();
        let path = directory(&f.destination.0, f.id()).join("artifact.json");
        match case {
            0 => fs::write(&path, vec![b'x'; ARTIFACT.len()]).unwrap(),
            1 => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            2 => fs::hard_link(&path, f.destination.0.join("artifact-alias")).unwrap(),
            3 => {
                fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(f.destination.0.join("absent-target"), &path).unwrap();
            }
            _ => {
                fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(f.destination.0.join("catalogue.sqlite3"), &path)
                    .unwrap();
            }
        }
        f.refuse(f.request()).await;
    }
}
#[tokio::test]
async fn status_capability_is_ordinary_owner_only_and_does_not_consume_model_slots() {
    let f = Journey::new().await;
    let caps = f
        .supervisor
        .handle(VesselCommand::Capabilities)
        .await
        .unwrap();
    assert!(
        caps["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "signed_transfer_status_v1")
    );
    let mut grant = f.destination.session();
    grant.session_id = f.manifest.payload.session_id;
    grant.rights = ProcessRight::all();
    grant.token_hash = format!("{:x}", Sha256::digest(b"owned-scope-token"));
    f.destination.save_session(&grant);
    let request = VesselCommand::Granted {
        expected_authority_fingerprint: None,
        expected_vessel_id: Some(f.manifest.payload.destination_vessel_id),
        grant_id: grant.grant_id,
        token: "owned-scope-token".into(),
        command: Box::new(f.request()),
    };
    f.refuse(request).await;
    receiving(&f.no_write_status().await, 0);
}
#[tokio::test]
async fn transfer_serial_fence_keeps_queued_status_and_upload_behind_exact_pending_intent() {
    let f = Journey::new().await;
    f.accept().await;
    let lock = f.supervisor.assignment_lock(f.id()).await.unwrap();
    let held = lock.lock().await;
    let status = f.supervisor.handle(f.request());
    let upload = f.supervisor.handle(f.upload(ARTIFACT));
    tokio::pin!(status, upload);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut status)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut upload)
            .await
            .is_err()
    );
    f.intent(f.activation);
    let before = graph(&f.destination.0);
    drop(held);
    let (status, upload) =
        tokio::time::timeout(WAIT, async { tokio::join!(&mut status, &mut upload) })
            .await
            .unwrap();
    pending(&serde_json::from_value(status.unwrap()).unwrap());
    assert!(upload.is_err());
    assert!(graph(&f.destination.0) == before);
}
#[tokio::test]
async fn registration_serial_fence_observes_exact_admission_committed_while_status_waited() {
    let f = Journey::new().await;
    let held = f.supervisor.registrations.observe_existing().await.unwrap();
    let status = f.supervisor.handle(f.request());
    tokio::pin!(status);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut status)
            .await
            .is_err()
    );
    f.admit().await;
    let before = graph(&f.destination.0);
    drop(held);
    let status = tokio::time::timeout(WAIT, &mut status)
        .await
        .unwrap()
        .unwrap();
    pending(&serde_json::from_value(status).unwrap());
    assert!(graph(&f.destination.0) == before);
}
#[tokio::test]
async fn delayed_status_refuses_a_valid_empty_namespace_replacement_before_first_read() {
    let f = Journey::new().await;
    let held = f.supervisor.registrations.observe_existing().await.unwrap();
    let status = f.supervisor.handle(f.request());
    tokio::pin!(status);
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut status)
            .await
            .is_err()
    );
    f.replace_catalogue();
    let before = graph(&f.destination.0);
    drop(held);
    assert!(
        tokio::time::timeout(WAIT, &mut status)
            .await
            .unwrap()
            .is_err()
    );
    assert!(graph(&f.destination.0) == before);
}
#[tokio::test]
async fn missing_runtime_image_retains_actual_admission_unknown_then_readonly_pending_without_replay()
 {
    let f = Journey::new().await;
    f.accept().await;
    f.supervisor.handle(f.upload(ARTIFACT)).await.unwrap();
    assert!(!f.supervisor.binary.exists());
    let response = crate::process::api::response(
        tokio::time::timeout(WAIT, f.supervisor.handle(f.activate()))
            .await
            .unwrap(),
    );
    assert!(response.error.is_some() && response.outcome_unknown && response.result.is_null());
    let original = f
        .supervisor
        .registration(f.manifest.payload.session_id)
        .await
        .unwrap();
    assert_eq!(original.command_id, f.activation);
    assert_eq!(original.state, ProcessState::Starting);
    assert_eq!(
        load(&f.destination.0, f.id()).unwrap().activation_command,
        Some(f.activation)
    );
    assert!(
        database::creation_receipt(&f.destination.0, f.activation)
            .await
            .unwrap()
            .is_none()
    );
    pending(&f.no_write_status().await);
    f.refuse(f.upload(ARTIFACT)).await;
    pending(&f.no_write_status().await);
    assert_eq!(
        f.supervisor
            .registration(original.session_id)
            .await
            .unwrap()
            .incarnation,
        original.incarnation
    );
    assert!(
        !registry::directory(&f.destination.0, original.session_id)
            .join("runtime.sock")
            .exists()
    );
}

// These deterministic helper-boundary journeys use the same production namespace
// scope as transfer startup. They do not inject a pause/fault into start_initialized
// or claim that two independent API activations can bypass registration serial.
fn original_namespace(f: &Journey) -> (u64, u64) {
    load(&f.destination.0, f.id())
        .unwrap()
        .catalogue_namespace
        .unwrap()
}
fn readonly_count(path: &Path, table: &str) -> i64 {
    assert!(matches!(
        table,
        "voyages" | "lifecycle_commands" | "creation_receipts"
    ));
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .unwrap();
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
#[tokio::test]
async fn nested_transfer_context_requires_same_root_and_namespace_before_operation() {
    let f = Journey::new().await;
    let namespace = original_namespace(&f);
    let before = graph(&f.destination.0);
    let source_before = graph(&f.source.0);
    tokio::time::timeout(
        WAIT,
        database::in_transfer_namespace(&f.destination.0, namespace, async {
            let entered = std::cell::Cell::new(false);
            assert!(
                database::in_transfer_namespace(&f.source.0, namespace, async {
                    entered.set(true);
                    Ok(())
                })
                .await
                .is_err()
            );
            assert!(!entered.get());
            assert!(
                database::in_transfer_namespace(
                    &f.destination.0,
                    (namespace.0, namespace.1 ^ 1),
                    async {
                        entered.set(true);
                        Ok(())
                    }
                )
                .await
                .is_err()
            );
            assert!(!entered.get());
            assert!(
                database::check_transfer_namespace(&f.source.0)
                    .await
                    .is_err()
            );
            database::in_transfer_namespace(&f.destination.0, namespace, async {
                database::check_transfer_namespace(&f.destination.0).await
            })
            .await?;
            Ok(())
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(graph(&f.destination.0) == before && graph(&f.source.0) == source_before);
    // Outside the scoped future the ordinary, unpinned no-op is restored.
    database::check_transfer_namespace(&f.source.0)
        .await
        .unwrap();
}
#[tokio::test]
async fn cancelled_context_and_concurrent_unpinned_database_operation_do_not_share_authority() {
    let f = Journey::new().await;
    let before = graph(&f.destination.0);
    let namespace = original_namespace(&f);
    let unrelated = f.source.registration();
    let pending = tokio::time::timeout(
        Duration::from_millis(25),
        database::in_transfer_namespace(
            &f.destination.0,
            namespace,
            std::future::pending::<Result<()>>(),
        ),
    );
    let ordinary = database::save(&f.source.0, &unrelated);
    let (cancelled, written) =
        tokio::time::timeout(WAIT, async { tokio::join!(pending, ordinary) })
            .await
            .unwrap();
    assert!(cancelled.is_err());
    written.unwrap();
    assert_eq!(
        database::registration(&f.source.0, unrelated.session_id)
            .await
            .unwrap()
            .incarnation,
        unrelated.incarnation
    );
    database::check_transfer_namespace(&f.source.0)
        .await
        .unwrap();
    // A fresh operation after cancellation remains outside the old context too.
    let next = f.source.registration();
    database::save(&f.source.0, &next).await.unwrap();
    assert_eq!(
        database::registration(&f.source.0, next.session_id)
            .await
            .unwrap()
            .incarnation,
        next.incarnation
    );
    assert!(graph(&f.destination.0) == before);
}
#[tokio::test]
async fn namespace_replaced_after_intent_refuses_actual_admission_without_creating_or_reserving() {
    let f = Journey::new().await;
    f.accept().await;
    f.intent(f.activation);
    let namespace = original_namespace(&f);
    tokio::time::timeout(
        WAIT,
        database::in_transfer_namespace(&f.destination.0, namespace, async {
            f.replace_catalogue();
            let before = graph(&f.destination.0);
            let registration = f.registration();
            assert!(
                database::admit(
                    &f.destination.0,
                    &registration,
                    serde_json::to_vec(&f.activate()).unwrap()
                )
                .await
                .is_err()
            );
            assert!(graph(&f.destination.0) == before);
            assert_eq!(
                readonly_count(&f.destination.0.join("catalogue.sqlite3"), "voyages"),
                0
            );
            assert_eq!(
                readonly_count(
                    &f.destination.0.join("catalogue.sqlite3"),
                    "lifecycle_commands"
                ),
                0
            );
            Ok(())
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        load(&f.destination.0, f.id()).unwrap().activation_command,
        Some(f.activation)
    );
    assert!(!registry::directory(&f.destination.0, f.manifest.payload.session_id).exists());
    f.refuse(f.request()).await;
}
#[tokio::test]
async fn namespace_replaced_after_actual_admission_fails_prelaunch_check_and_retains_original_unknown()
 {
    let f = Journey::new().await;
    f.accept().await;
    f.intent(f.activation);
    let namespace = original_namespace(&f);
    let result = tokio::time::timeout(
        WAIT,
        database::in_transfer_namespace(&f.destination.0, namespace, async {
            f.admit().await;
            f.replace_catalogue();
            let before = graph(&f.destination.0);
            let result = database::check_transfer_namespace(&f.destination.0).await;
            assert!(graph(&f.destination.0) == before);
            // Same unknown classification as the production prelaunch callsite;
            // no launch function is called by this boundary-level fixture.
            result.map_err(|error| error.context(crate::process::routing::OutcomeUnknown))
        }),
    )
    .await
    .unwrap();
    let reply = crate::process::api::response(result.map(|()| serde_json::Value::Null));
    assert!(reply.error.is_some() && reply.outcome_unknown && reply.result.is_null());
    let retained = f.destination.0.join("original-catalogue-retained");
    assert_eq!(readonly_count(&retained, "voyages"), 1);
    assert_eq!(readonly_count(&retained, "lifecycle_commands"), 1);
    assert_eq!(readonly_count(&retained, "creation_receipts"), 0);
    let db = rusqlite::Connection::open_with_flags(
        &retained,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (command, bytes): (String, Vec<u8>) = db
        .query_row(
            "SELECT command_id,request FROM lifecycle_commands WHERE namespace='commands'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(command, f.activation.to_string());
    assert!(bytes == serde_json::to_vec(&f.activate()).unwrap());
    drop(db);
    assert_eq!(
        load(&f.destination.0, f.id()).unwrap().activation_command,
        Some(f.activation)
    );
    assert!(!f.supervisor.binary.exists());
    f.refuse(f.request()).await;
    f.refuse(f.upload(ARTIFACT)).await;
}
#[tokio::test]
async fn pinned_writes_preserve_namespace_checks_across_own_admission_and_receipt_changes() {
    let f = Journey::new().await;
    f.accept().await;
    let namespace = original_namespace(&f);
    let registration = tokio::time::timeout(
        WAIT,
        database::in_transfer_namespace(&f.destination.0, namespace, async {
            let r = f.admit().await;
            database::check_transfer_namespace(&f.destination.0).await?;
            // Logical historical fixture only: this does not establish runtime health.
            let mut historical = ProcessInfo::from(&r);
            historical.state = ProcessState::Suspended;
            database::settle_creation(&f.destination.0, f.activation, &historical).await?;
            database::check_transfer_namespace(&f.destination.0).await?;
            Ok(r)
        }),
    )
    .await
    .unwrap()
    .unwrap();
    let status = f.no_write_status().await;
    assert_eq!(status.state, TransferStatusState::Complete);
    assert_eq!(
        status.completion.unwrap().incarnation,
        registration.incarnation
    );
    assert!(!f.supervisor.binary.exists());
}
