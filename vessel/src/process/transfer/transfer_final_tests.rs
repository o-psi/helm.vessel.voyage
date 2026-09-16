use super::*;
use crate::process::{database, identity, test_support::Fixture};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

async fn prepared() -> (
    Fixture,
    Fixture,
    Supervisor,
    SignedArtifact<TransferPreparation>,
) {
    let source = Fixture::new();
    let dest = Fixture::new();
    let mut s = dest.supervisor().await;
    identity::pin(&dest.0, &identity::public(&source.0).unwrap()).unwrap();
    identity::pin(&source.0, &identity::public(&dest.0).unwrap()).unwrap();
    // A local, deterministic validate-start executable, not a runtime or service.
    let executable = dest.0.join("validate-fixture");
    std::fs::write(&executable, "#!/bin/sh\n[ \"$1\" = validate-start ]\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    s.binary = executable;
    let command = VesselCommand::PrepareTransfer {
        command_id: Uuid::new_v4(),
        transfer_id: Uuid::new_v4(),
        source_vessel_id: identity::public(&source.0).unwrap().vessel_id,
        session_id: Uuid::new_v4(),
        workspace: dest.0.clone(),
        config_path: None,
        expires_at_ms: store::now().unwrap() + 60_000,
    };
    let first = s.transfer(command.clone()).await.unwrap();
    assert_eq!(s.transfer(command).await.unwrap(), first);
    let p = serde_json::from_value(first).unwrap();
    (source, dest, s, p)
}
fn manifest(
    source: &Fixture,
    p: &SignedArtifact<TransferPreparation>,
    bytes: &[u8],
) -> SignedArtifact<TransferManifest> {
    identity::sign(
        &source.0,
        TransferManifest {
            transfer_id: p.payload.transfer_id,
            source_vessel_id: p.payload.source_vessel_id,
            destination_vessel_id: p.payload.destination_vessel_id,
            session_id: p.payload.session_id,
            source_incarnation: Uuid::new_v4(),
            prepare_digest: identity::digest(p).unwrap(),
            artifact_sha256: format!("{:x}", Sha256::digest(bytes)),
            artifact_bytes: bytes.len() as u64,
            generation: 1,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn signed_destination_upload_is_contiguous_idempotent_and_bound() {
    let (source, dest, s, p) = prepared().await;
    let id = p.payload.transfer_id;
    let m = manifest(&source, &p, b"abcdef");
    assert!(
        s.upload_transfer(id, 0, STANDARD.encode(b"a"))
            .await
            .unwrap_err()
            .to_string()
            .contains("not accepted")
    );
    let accept = VesselCommand::AcceptTransfer {
        manifest: m.clone(),
    };
    assert_eq!(
        s.transfer(accept.clone()).await.unwrap()["state"],
        "receiving"
    );
    s.transfer(accept).await.unwrap();
    for (offset, bytes, error) in [
        (1, "a", "gap"),
        (u64::MAX, "a", "overflow"),
        (6, "a", "length"),
    ] {
        assert!(
            s.upload_transfer(id, offset, STANDARD.encode(bytes))
                .await
                .unwrap_err()
                .to_string()
                .contains(error)
        );
    }
    let chunk = VesselCommand::UploadTransferChunk {
        transfer_id: id,
        offset: 0,
        data: STANDARD.encode(b"abc"),
    };
    assert_eq!(s.transfer(chunk.clone()).await.unwrap()["stored_bytes"], 3);
    assert_eq!(s.transfer(chunk).await.unwrap()["next_offset"], 3);
    for (offset, bytes, error) in [(1, "bcd", "overlapping"), (0, "xyz", "conflict")] {
        assert!(
            s.upload_transfer(id, offset, STANDARD.encode(bytes))
                .await
                .unwrap_err()
                .to_string()
                .contains(error)
        );
    }
    s.upload_transfer(id, 3, STANDARD.encode(b"def"))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(directory(&dest.0, id).join("artifact.json")).unwrap(),
        b"abcdef"
    );
    for data in [
        String::new(),
        "!".into(),
        "a".repeat(90001),
        STANDARD.encode(vec![0; 65537]),
    ] {
        assert!(s.upload_transfer(id, 0, data).await.is_err());
    }
    let mut changed = m.payload.clone();
    changed.generation = 2;
    assert!(
        s.accept_transfer(identity::sign(&source.0, changed).unwrap())
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    let mut saved = load(&dest.0, id).unwrap();
    saved.activated = true;
    save(&dest.0, id, &saved).unwrap();
    assert!(
        s.upload_transfer(id, 0, STANDARD.encode("a"))
            .await
            .unwrap_err()
            .to_string()
            .contains("activated")
    );
}

#[tokio::test]
async fn destination_rejects_resigned_wrong_bindings_and_incomplete_activation() {
    let (source, dest, s, p) = prepared().await;
    let m = manifest(&source, &p, b"abcdef");
    for index in 0..8 {
        let mut payload = m.payload.clone();
        match index {
            0 => payload.destination_vessel_id = Uuid::new_v4(),
            1 => payload.session_id = Uuid::new_v4(),
            2 => payload.prepare_digest = "wrong".into(),
            3 => payload.generation = 0,
            4 => payload.artifact_bytes = 0,
            5 => payload.artifact_bytes = 16 * 1024 * 1024 + 1,
            6 => payload.artifact_sha256 = "short".into(),
            _ => payload.source_vessel_id = Uuid::new_v4(),
        }
        assert!(
            s.accept_transfer(identity::sign(&source.0, payload).unwrap())
                .await
                .is_err()
        );
        assert!(
            load(&dest.0, p.payload.transfer_id)
                .unwrap()
                .manifest
                .is_none()
        );
    }
    let id = p.payload.transfer_id;
    let activate = VesselCommand::ActivateTransfer {
        command_id: Uuid::new_v4(),
        transfer_id: id,
    };
    assert!(s.transfer(activate.clone()).await.is_err());
    s.accept_transfer(m).await.unwrap();
    s.upload_transfer(id, 0, STANDARD.encode("abc"))
        .await
        .unwrap();
    assert!(
        format!("{:#}", s.transfer(activate.clone()).await.unwrap_err()).contains("incomplete")
    );
    s.upload_transfer(id, 3, STANDARD.encode("xyz"))
        .await
        .unwrap();
    assert!(format!("{:#}", s.transfer(activate).await.unwrap_err()).contains("digest mismatch"));
    assert!(!load(&dest.0, id).unwrap().activated);
    assert!(
        s.transfer(VesselCommand::Notifications {
            operation: voyage_protocol::notifications::NotificationOperation::Attention
        })
        .await
        .is_err()
    );
}

#[tokio::test]
async fn stopped_source_exports_checkpoint_and_serves_verified_chunks() {
    let (source, dest, _, p) = prepared().await;
    let s = source.supervisor().await;
    let mut r = source.registration();
    r.session_id = p.payload.session_id;
    r.state = ProcessState::Stopped;
    database::save(&source.0, &r).await.unwrap();
    let session = registry::directory(&source.0, r.session_id);
    registry::private_directory(&session.join("transfers")).unwrap();
    let artifact = session
        .join("transfers")
        .join(format!("{}.json", p.payload.transfer_id));
    let checkpoint = PortableCheckpoint {
        transfer_id: p.payload.transfer_id,
        session_id: r.session_id,
        destination_vessel_id: p.payload.destination_vessel_id,
        prepare_digest: identity::digest(&p).unwrap(),
        generation: 1,
        session: serde_json::json!({"fixture":"offline"}),
        commands: vec![],
    };
    store::save(&artifact, &checkpoint).unwrap();
    let command = VesselCommand::ExportTransfer {
        command_id: Uuid::new_v4(),
        session_id: r.session_id,
        incarnation: r.incarnation,
        expected_revision: 0,
        expires_at_ms: store::now().unwrap() + 60_000,
        preparation: p.clone(),
    };
    let value = s.transfer(command.clone()).await.unwrap();
    assert_eq!(value["payload"]["generation"], 1);
    assert_eq!(s.transfer(command).await.unwrap(), value);
    assert_eq!(
        s.registration(r.session_id).await.unwrap().state,
        ProcessState::Relinquished
    );
    let id = p.payload.transfer_id;
    let mut offset = 0;
    let mut all = Vec::new();
    loop {
        let chunk = s
            .transfer(VesselCommand::TransferChunk {
                transfer_id: id,
                offset,
                limit: 17,
            })
            .await
            .unwrap();
        all.extend(STANDARD.decode(chunk["data"].as_str().unwrap()).unwrap());
        offset = chunk["next_offset"].as_u64().unwrap();
        if chunk["has_more"] == false {
            break;
        }
    }
    assert_eq!(all, std::fs::read(&artifact).unwrap());
    assert_eq!(s.transfer_chunk(id, offset, 1).await.unwrap()["data"], "");
    assert!(s.transfer_chunk(id, offset + 1, 1).await.is_err());
    for limit in [0, 65537] {
        assert!(s.transfer_chunk(id, 0, limit).await.is_err());
    }
    std::fs::write(&artifact, b"changed").unwrap();
    assert!(
        s.transfer_chunk(id, 0, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    // A world-readable artifact or a symlink is never exported.
    std::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        s.transfer_chunk(id, 0, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("unsafe")
    );
    std::fs::remove_file(&artifact).unwrap();
    std::os::unix::fs::symlink(dest.0.join("validate-fixture"), &artifact).unwrap();
    assert!(s.transfer_chunk(id, 0, 1).await.is_err());
}

#[tokio::test]
async fn preparation_rejects_identity_deadline_workspace_and_receipt_conflicts() {
    let (source, dest, s, p) = prepared().await;
    let template = VesselCommand::PrepareTransfer {
        command_id: Uuid::new_v4(),
        transfer_id: Uuid::new_v4(),
        source_vessel_id: identity::public(&source.0).unwrap().vessel_id,
        session_id: Uuid::new_v4(),
        workspace: dest.0.clone(),
        config_path: None,
        expires_at_ms: store::now().unwrap() + 60_000,
    };
    for index in 0..8 {
        let mut command = template.clone();
        if let VesselCommand::PrepareTransfer {
            command_id,
            transfer_id,
            source_vessel_id,
            workspace,
            config_path,
            expires_at_ms,
            ..
        } = &mut command
        {
            *command_id = Uuid::new_v4();
            *transfer_id = Uuid::new_v4();
            match index {
                0 => *command_id = Uuid::nil(),
                1 => *transfer_id = p.payload.transfer_id,
                2 => *source_vessel_id = p.payload.destination_vessel_id,
                3 => *expires_at_ms = 0,
                4 => *expires_at_ms = store::now().unwrap() + 1900000,
                5 => *workspace = dest.0.join("missing"),
                6 => *workspace = dest.0.join("validate-fixture"),
                _ => *config_path = Some(PathBuf::from("relative")),
            }
        }
        assert!(s.transfer(command).await.is_err());
    }
    s.check_transfer_retention().unwrap();
    let bad = directory(&dest.0, Uuid::new_v4());
    registry::private_directory(&bad).unwrap();
    std::fs::write(bad.join("prepared.json"), b"invalid").unwrap();
    assert!(s.check_transfer_retention().is_err());
}
