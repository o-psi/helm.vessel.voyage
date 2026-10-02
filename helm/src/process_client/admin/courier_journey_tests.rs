//! Whole courier loops over owned correlated duplex peers. Signatures are opaque fixtures.
use super::*;
#[path = "courier_fixture_tests.rs"]
mod fixture;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use fixture::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

fn seed(j: &Journey, bytes: &[u8], phase: Option<&str>, version: u32, terminal: Option<Value>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    write(
        &j.args.journal,
        &json!({"version":version,"phase":phase,"args":j.args,
        "source":j.source.identity,"destination":j.destination.identity,
        "transfer_id":Uuid::new_v4(),"prepare_command":Uuid::new_v4(),"export_command":Uuid::new_v4(),"activate_command":Uuid::new_v4(),
        "expires_at_ms":now+180000,"preparation":null,"manifest":null,"result":terminal}),
    );
    let mut saved = journal(&j.args);
    saved["preparation"] = serde_json::to_value(j.preparation()).unwrap();
    write(&j.args.journal, &saved);
    saved["manifest"] = serde_json::to_value(j.manifest(bytes)).unwrap();
    write(&j.args.journal, &saved);
}
fn ids(j: &Journey) -> Value {
    let saved = journal(&j.args);
    json!({"args":saved["args"],"source":saved["source"],"destination":saved["destination"],"transfer_id":saved["transfer_id"],"prepare_command":saved["prepare_command"],"export_command":saved["export_command"],"activate_command":saved["activate_command"],"expires_at_ms":saved["expires_at_ms"]})
}
fn no_mutations(j: &Journey) {
    for op in [
        "prepare_transfer",
        "accept_transfer",
        "upload_transfer_chunk",
        "activate_transfer",
    ] {
        assert_eq!(
            j.destination.count(op),
            0,
            "unexpected destination effect {op}"
        );
    }
    for op in ["export_transfer", "transfer_chunk"] {
        assert_eq!(j.source.count(op), 0, "unexpected source effect {op}");
    }
}
fn fresh_journal(j: &Journey) {
    let saved = journal(&j.args);
    assert_eq!(saved["version"], 1);
    assert_eq!(saved["phase"], "receiving");
    assert_eq!(saved["args"], serde_json::to_value(&j.args).unwrap());
    assert_eq!(
        saved["source"],
        serde_json::to_value(&j.source.identity).unwrap()
    );
    assert_eq!(
        saved["destination"],
        serde_json::to_value(&j.destination.identity).unwrap()
    );
    let values: Vec<Uuid> = [
        "transfer_id",
        "prepare_command",
        "export_command",
        "activate_command",
    ]
    .iter()
    .map(|key| serde_json::from_value(saved[*key].clone()).unwrap())
    .collect();
    assert!(values.iter().all(|v| !v.is_nil()));
    for (index, value) in values.iter().enumerate() {
        assert!(!values[index + 1..].contains(value));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let deadline = saved["expires_at_ms"].as_u64().unwrap();
    assert!(deadline > now && deadline <= now + 300000);
    assert!(
        saved["preparation"].is_null() && saved["manifest"].is_null() && saved["result"].is_null()
    );
    assert_eq!(
        fs::metadata(&j.args.journal).unwrap().permissions().mode() & 0o077,
        0
    );
}
fn reply(j: &Journey) -> Value {
    json!({"transfer_id":journal(&j.args)["transfer_id"],"session_id":j.args.session,"incarnation":Uuid::new_v4(),"status":"complete"})
}
fn status_value(
    j: &Journey,
    bytes: &[u8],
    state: &str,
    received: Option<u64>,
    completion: Option<Value>,
) -> Value {
    let saved = journal(&j.args);
    let manifest = j.manifest(bytes);
    json!({"transfer_id":saved["transfer_id"],"activate_command_id":saved["activate_command"],
        "manifest_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&manifest).unwrap())),
        "source_vessel_id":j.source.identity.vessel_id,"destination_vessel_id":j.destination.identity.vessel_id,"session_id":j.args.session,
        "artifact_sha256":manifest.payload.artifact_sha256,"artifact_bytes":bytes.len(),"received_bytes":received,"state":state,"completion":completion})
}
async fn status_packet(j: &mut Journey, bytes: &[u8]) -> Packet {
    let packet = j.destination.next().await;
    assert!(matches!(packet.command, VesselCommand::Capabilities));
    packet.ok(json!({"features":["signed_transfer_status_v1"]}));
    let packet = j.destination.next().await;
    let saved = journal(&j.args);
    let manifest = j.manifest(bytes);
    assert_eq!(
        serde_json::to_value(&packet.command).unwrap(),
        json!({"op":"transfer_status","transfer_id":saved["transfer_id"],"activate_command_id":saved["activate_command"],"expected_manifest_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&manifest).unwrap())),"manifest":manifest})
    );
    packet
}
async fn chunk_packet(j: &mut Journey, offset: u64, bytes: &[u8], total: u64) -> Packet {
    let packet = j.source.next().await;
    let saved = journal(&j.args);
    assert_eq!(
        serde_json::to_value(&packet.command).unwrap(),
        json!({"op":"transfer_chunk","transfer_id":saved["transfer_id"],"offset":offset,"limit":65536})
    );
    packet.ok(json!({"transfer_id":saved["transfer_id"],"offset":offset,"total_bytes":total,"next_offset":offset+bytes.len()as u64,"data":STANDARD.encode(bytes)}));
    j.destination.next().await
}
async fn activate(j: &mut Journey, task: tokio::task::JoinHandle<Result<Value>>) -> Value {
    let packet = j.activate_packet().await;
    assert_eq!(journal(&j.args)["phase"], "activation_pending");
    assert!(journal(&j.args)["result"].is_null());
    let value = reply(j);
    packet.ok(value.clone());
    assert_eq!(result(task).await.unwrap(), value);
    value
}
async fn new_to_manifest(j: &mut Journey, bytes: &[u8]) -> tokio::task::JoinHandle<Result<Value>> {
    let task = j.begin(j.args.clone());
    j.identities().await;
    let packet = j.destination.next().await;
    fresh_journal(j);
    let saved = journal(&j.args);
    assert_eq!(
        serde_json::to_value(&packet.command).unwrap(),
        json!({
            "op":"prepare_transfer","command_id":saved["prepare_command"],
            "transfer_id":saved["transfer_id"],"source_vessel_id":j.source.identity.vessel_id,
            "session_id":j.args.session,"workspace":j.args.workspace,
            "config_path":j.args.config_path,"expires_at_ms":saved["expires_at_ms"]
        })
    );
    packet.ok(serde_json::to_value(j.preparation()).unwrap());
    j.export(bytes).await;
    task
}
async fn resume_receiving(
    j: &mut Journey,
    _bytes: &[u8],
    received: u64,
) -> tokio::task::JoinHandle<Result<Value>> {
    let task = j.begin(j.args.clone());
    j.identities().await;
    // Reuse the typed helper for positive recovery. Adverse proofs below retain
    // direct packet control to assert every malformed binding is refused.
    j.recovery_status(TransferStatusState::Receiving, Some(received), None)
        .await;
    task
}

#[tokio::test]
async fn scoped_source_is_refused_before_lock_or_any_endpoint_contact() {
    let mut j = Journey::new().await;
    j.source.client = Client::access(j.root.join("not-an-invented-credential"));
    assert!(result(j.begin(j.args.clone())).await.is_err());
    assert!(!j.args.journal.exists());
    assert!(!j.args.journal.with_extension("courier-lock").exists());
    assert!(
        j.source.calls.lock().unwrap().is_empty() && j.destination.calls.lock().unwrap().is_empty()
    );
    j.finish().await;
}
#[tokio::test]
async fn relative_arguments_and_public_parent_refuse_without_endpoint_or_journal_effects() {
    let j = Journey::new().await;
    for field in 0..3 {
        let mut args = j.args.clone();
        match field {
            0 => args.journal = "relative.json".into(),
            1 => args.destination_directory = "relative-destination".into(),
            _ => args.workspace = "relative-workspace".into(),
        };
        assert!(result(j.begin(args)).await.is_err());
    }
    fs::set_permissions(&j.root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result(j.begin(j.args.clone())).await.is_err());
    fs::set_permissions(&j.root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!j.args.journal.exists());
    assert!(
        j.source.calls.lock().unwrap().is_empty() && j.destination.calls.lock().unwrap().is_empty()
    );
    j.finish().await;
}
#[tokio::test]
async fn unsafe_directory_symlink_dangling_hardlink_or_public_journal_is_preserved_before_contact()
{
    for kind in 0..5 {
        let j = Journey::new().await;
        let target = j.root.join("original");
        fs::write(&target, b"owned original bytes").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        match kind {
            0 => fs::create_dir(&j.args.journal).unwrap(),
            1 => symlink(&target, &j.args.journal).unwrap(),
            2 => symlink(j.root.join("missing-target"), &j.args.journal).unwrap(),
            3 => fs::hard_link(&target, &j.args.journal).unwrap(),
            _ => {
                write(&j.args.journal, &json!({"retained":"unrelated"}));
                fs::set_permissions(&j.args.journal, fs::Permissions::from_mode(0o644)).unwrap();
            }
        }
        let before = fs::symlink_metadata(&j.args.journal).unwrap();
        let leaf_bytes = before.is_file().then(|| fs::read(&j.args.journal).unwrap());
        let leaf_link = before
            .file_type()
            .is_symlink()
            .then(|| fs::read_link(&j.args.journal).unwrap());
        assert!(result(j.begin(j.args.clone())).await.is_err());
        let after = fs::symlink_metadata(&j.args.journal).unwrap();
        assert_eq!(
            (
                after.dev(),
                after.ino(),
                after.uid(),
                after.gid(),
                after.nlink()
            ),
            (
                before.dev(),
                before.ino(),
                before.uid(),
                before.gid(),
                before.nlink()
            )
        );
        assert_eq!(after.file_type(), before.file_type());
        if let Some(bytes) = leaf_bytes {
            assert_eq!(fs::read(&j.args.journal).unwrap(), bytes);
        }
        if let Some(link) = leaf_link {
            assert_eq!(fs::read_link(&j.args.journal).unwrap(), link);
        }
        assert_eq!(fs::read(&target).unwrap(), b"owned original bytes");
        assert_eq!(
            fs::symlink_metadata(&j.args.journal)
                .unwrap()
                .permissions()
                .mode(),
            before.permissions().mode()
        );
        if kind == 2 {
            assert_eq!(
                fs::read_link(&j.args.journal).unwrap(),
                j.root.join("missing-target")
            );
            assert!(!j.root.join("missing-target").exists());
        }
        assert!(
            j.source.calls.lock().unwrap().is_empty()
                && j.destination.calls.lock().unwrap().is_empty()
        );
        j.finish().await;
    }
}
#[tokio::test]
async fn retired_ssh_journal_is_never_reinterpreted_or_rewritten_as_a_local_route() {
    let j = Journey::new().await;
    seed(&j, b"blob", Some("receiving"), 1, None);
    let mut saved = journal(&j.args);
    saved["args"]["destination_ssh"] = json!("retired-synthetic-route");
    write(&j.args.journal, &saved);
    let before = fs::read(&j.args.journal).unwrap();
    assert!(result(j.begin(j.args.clone())).await.is_err());
    assert_eq!(fs::read(&j.args.journal).unwrap(), before);
    assert!(
        j.source.calls.lock().unwrap().is_empty() && j.destination.calls.lock().unwrap().is_empty()
    );
    j.finish().await;
}
#[tokio::test]
async fn same_endpoint_and_changed_frozen_arguments_or_identity_refuse_before_courier_effects() {
    {
        let mut j = Journey::new().await;
        j.destination.identity = j.source.identity.clone();
        let task = j.begin(j.args.clone());
        j.identities().await;
        assert!(result(task).await.is_err());
        assert!(!j.args.journal.exists());
        no_mutations(&j);
        j.finish().await;
    }
    for field in 0..6 {
        let mut j = Journey::new().await;
        seed(&j, b"blob", Some("receiving"), 1, None);
        let before = fs::read(&j.args.journal).unwrap();
        let mut args = j.args.clone();
        match field {
            0 => args.expected_revision += 1,
            1 => args.incarnation = Uuid::new_v4(),
            2 => j.source.identity.vessel_id = Uuid::new_v4(),
            3 => j.source.identity.public_key = STANDARD.encode([15; 32]),
            4 => j.destination.identity.vessel_id = Uuid::new_v4(),
            _ => j.destination.identity.public_key = STANDARD.encode([16; 32]),
        };
        let task = j.begin(args);
        j.identities().await;
        assert!(result(task).await.is_err());
        assert_eq!(fs::read(&j.args.journal).unwrap(), before);
        no_mutations(&j);
        j.finish().await;
    }
}
#[tokio::test]
async fn fresh_two_chunk_flow_persists_all_identities_before_effects_and_activates_exactly_once() {
    let mut j = Journey::new().await;
    let bytes = vec![b'x'; 65536 + 17];
    let task = new_to_manifest(&mut j, &bytes).await;
    let pinned = ids(&j);
    j.accept(&bytes).await;
    j.chunk(0, &bytes[..65536], bytes.len() as u64).await;
    j.chunk(65536, &bytes[65536..], bytes.len() as u64).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(j.destination.count("activate_transfer"), 1);
    assert_eq!(j.destination.count("upload_transfer_chunk"), 2);
    assert_eq!(j.source.count("export_transfer"), 1);
    assert_eq!(j.destination.count("transfer_status"), 0);
    j.finish().await;
}
#[tokio::test]
async fn retained_terminal_result_reopens_with_identity_reads_only_and_byte_identical_journal() {
    let mut j = Journey::new().await;
    let old = json!({"status":"complete","historical":"opaque typed fixture result"});
    seed(
        &j,
        b"blob",
        Some("activation_pending"),
        1,
        Some(old.clone()),
    );
    let before = fs::read(&j.args.journal).unwrap();
    let task = j.begin(j.args.clone());
    j.identities().await;
    assert_eq!(result(task).await.unwrap(), old);
    assert_eq!(fs::read(&j.args.journal).unwrap(), before);
    no_mutations(&j);
    assert_eq!(j.destination.count("capabilities"), 0);
    j.finish().await;
}
#[tokio::test]
async fn lost_prepare_is_not_automatically_replayed_and_explicit_retry_retains_original_ids() {
    let mut j = Journey::new().await;
    let bytes = b"retry same preparation";
    let task = j.begin(j.args.clone());
    j.identities().await;
    let packet = j.destination.next().await;
    fresh_journal(&j);
    let pinned = ids(&j);
    assert!(matches!(
        packet.command,
        VesselCommand::PrepareTransfer { .. }
    ));
    packet.lose();
    assert!(result(task).await.is_err());
    assert_eq!(j.destination.count("prepare_transfer"), 1);
    assert_eq!(ids(&j), pinned);
    let task = j.begin(j.args.clone());
    j.identities().await;
    j.prepare().await;
    j.export(bytes).await;
    j.accept(bytes).await;
    j.chunk(0, bytes, bytes.len() as u64).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(j.destination.count("prepare_transfer"), 2);
    assert_eq!(j.source.count("export_transfer"), 1);
    assert_eq!(j.destination.count("activate_transfer"), 1);
    j.finish().await;
}
#[tokio::test]
async fn unknown_export_retains_preparation_and_explicit_retry_uses_exact_original_export_command()
{
    let mut j = Journey::new().await;
    let bytes = b"retry exact export";
    let task = j.begin(j.args.clone());
    j.identities().await;
    j.prepare().await;
    let pinned = ids(&j);
    let packet = j.source.next().await;
    assert!(matches!(
        packet.command,
        VesselCommand::ExportTransfer { .. }
    ));
    packet.refused(true);
    assert!(result(task).await.is_err());
    assert!(!journal(&j.args)["preparation"].is_null());
    assert!(journal(&j.args)["manifest"].is_null());
    let task = j.begin(j.args.clone());
    j.identities().await;
    j.export(bytes).await;
    j.accept(bytes).await;
    j.chunk(0, bytes, bytes.len() as u64).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(j.destination.count("prepare_transfer"), 1);
    assert_eq!(j.source.count("export_transfer"), 2);
    j.finish().await;
}
#[tokio::test]
async fn manifest_retained_before_accept_recovers_by_readonly_receiving_proof_without_new_export() {
    let mut j = Journey::new().await;
    let bytes = b"preaccept retained artifact";
    let task = new_to_manifest(&mut j, bytes).await;
    let packet = j.destination.next().await;
    assert!(matches!(
        packet.command,
        VesselCommand::AcceptTransfer { .. }
    ));
    packet.refused(true);
    assert!(result(task).await.is_err());
    let pinned = ids(&j);
    let task = resume_receiving(&mut j, bytes, 0).await;
    j.accept(bytes).await;
    j.chunk(0, bytes, bytes.len() as u64).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(j.destination.count("prepare_transfer"), 1);
    assert_eq!(j.source.count("export_transfer"), 1);
    assert_eq!(j.destination.count("accept_transfer"), 2);
    j.finish().await;
}
#[tokio::test]
async fn valid_full_receiving_proof_skips_upload_but_preserves_same_accept_and_activation_ids() {
    let mut j = Journey::new().await;
    let bytes = b"fully received artifact";
    seed(&j, bytes, Some("activation_pending"), 1, None);
    let pinned = ids(&j);
    let task = resume_receiving(&mut j, bytes, bytes.len() as u64).await;
    j.accept(bytes).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(j.source.count("transfer_chunk"), 0);
    assert_eq!(j.destination.count("upload_transfer_chunk"), 0);
    assert_eq!(j.destination.count("activate_transfer"), 1);
    j.finish().await;
}
#[tokio::test]
async fn lost_or_unknown_upload_stops_activation_and_only_explicit_bound_receiving_retry_continues()
{
    for lost in [false, true] {
        let mut j = Journey::new().await;
        let bytes = b"exact upload retry";
        let task = new_to_manifest(&mut j, bytes).await;
        j.accept(bytes).await;
        let pinned = ids(&j);
        let packet = chunk_packet(&mut j, 0, bytes, bytes.len() as u64).await;
        assert!(matches!(
            packet.command,
            VesselCommand::UploadTransferChunk { .. }
        ));
        if lost {
            packet.lose();
        } else {
            packet.refused(true);
        }
        assert!(result(task).await.is_err());
        assert_eq!(j.destination.count("activate_transfer"), 0);
        assert_eq!(journal(&j.args)["phase"], "receiving");
        assert_eq!(ids(&j), pinned);
        let task = resume_receiving(&mut j, bytes, 3).await;
        j.accept(bytes).await;
        j.chunk(0, bytes, bytes.len() as u64).await;
        activate(&mut j, task).await;
        assert_eq!(ids(&j), pinned);
        assert_eq!(j.destination.count("upload_transfer_chunk"), 2);
        assert_eq!(j.destination.count("activate_transfer"), 1);
        j.finish().await;
    }
}
#[tokio::test]
async fn lost_or_unknown_activation_is_persisted_pending_and_pending_status_never_replays_it() {
    for lost in [false, true] {
        let mut j = Journey::new().await;
        let bytes = b"pending activation";
        let task = new_to_manifest(&mut j, bytes).await;
        j.accept(bytes).await;
        j.chunk(0, bytes, bytes.len() as u64).await;
        let packet = j.activate_packet().await;
        assert_eq!(journal(&j.args)["phase"], "activation_pending");
        if lost {
            packet.lose();
        } else {
            packet.refused(true);
        }
        assert!(result(task).await.is_err());
        let before = fs::read(&j.args.journal).unwrap();
        let pinned = ids(&j);
        let task = j.begin(j.args.clone());
        j.identities().await;
        let packet = status_packet(&mut j, bytes).await;
        packet.ok(status_value(&j, bytes, "pending", None, None));
        assert!(result(task).await.is_err());
        assert_eq!(fs::read(&j.args.journal).unwrap(), before);
        assert_eq!(ids(&j), pinned);
        assert_eq!(j.destination.count("activate_transfer"), 1);
        assert_eq!(j.destination.count("upload_transfer_chunk"), 1);
        j.finish().await;
    }
}
#[tokio::test]
async fn historical_complete_status_reconciles_only_bound_receipt_then_reopens_without_effects() {
    let mut j = Journey::new().await;
    let bytes = b"completed artifact";
    seed(&j, bytes, Some("activation_pending"), 1, None);
    let pinned = ids(&j);
    let incarnation = Uuid::new_v4();
    let task = j.begin(j.args.clone());
    j.identities().await;
    let packet = status_packet(&mut j, bytes).await;
    packet.ok(status_value(
        &j,
        bytes,
        "complete",
        None,
        Some(json!({"session_id":j.args.session,"incarnation":incarnation})),
    ));
    let expected = json!({"transfer_id":journal(&j.args)["transfer_id"],"activate_command_id":journal(&j.args)["activate_command"],"status":"complete","session_id":j.args.session,"incarnation":incarnation});
    assert_eq!(result(task).await.unwrap(), expected);
    assert_eq!(journal(&j.args)["result"], expected);
    assert_eq!(ids(&j), pinned);
    no_mutations(&j);
    let before = fs::read(&j.args.journal).unwrap();
    let task = j.begin(j.args.clone());
    j.identities().await;
    assert_eq!(result(task).await.unwrap(), expected);
    assert_eq!(fs::read(&j.args.journal).unwrap(), before);
    assert_eq!(j.destination.count("transfer_status"), 1);
    no_mutations(&j);
    j.finish().await;
}
#[tokio::test]
async fn legacy_manifest_receiving_resume_preserves_frozen_ids_and_deadline_without_new_preparation()
 {
    let mut j = Journey::new().await;
    let bytes = b"legacy retained artifact";
    seed(&j, bytes, None, 0, None);
    let pinned = ids(&j);
    let task = resume_receiving(&mut j, bytes, 0).await;
    j.accept(bytes).await;
    j.chunk(0, bytes, bytes.len() as u64).await;
    activate(&mut j, task).await;
    assert_eq!(ids(&j), pinned);
    assert_eq!(journal(&j.args)["version"], 1);
    assert_eq!(j.source.count("export_transfer"), 0);
    assert_eq!(j.destination.count("prepare_transfer"), 0);
    j.finish().await;
}
#[tokio::test]
async fn legacy_pending_unsupported_unknown_lost_or_refused_status_retains_original_without_effects()
 {
    for kind in 0..5 {
        let mut j = Journey::new().await;
        let bytes = b"ambiguous legacy retained artifact";
        seed(&j, bytes, None, 0, None);
        let before = fs::read(&j.args.journal).unwrap();
        let task = j.begin(j.args.clone());
        j.identities().await;
        if kind == 1 {
            let packet = j.destination.next().await;
            assert!(matches!(packet.command, VesselCommand::Capabilities));
            packet.ok(json!({"features":[]}));
        } else {
            let packet = status_packet(&mut j, bytes).await;
            match kind {
                0 => packet.ok(status_value(&j, bytes, "pending", None, None)),
                2 => packet.lose(),
                3 => packet.refused(false),
                _ => packet.refused(true),
            }
        }
        assert!(result(task).await.is_err());
        assert_eq!(fs::read(&j.args.journal).unwrap(), before);
        no_mutations(&j);
        j.finish().await;
    }
}
#[tokio::test]
async fn malformed_chunk_and_status_bindings_stop_before_following_upload_or_activation() {
    let bytes = b"blob";
    for field in 0..10 {
        let mut j = Journey::new().await;
        let task = new_to_manifest(&mut j, bytes).await;
        j.accept(bytes).await;
        let pinned = ids(&j);
        let packet = j.source.next().await;
        assert!(matches!(
            packet.command,
            VesselCommand::TransferChunk { .. }
        ));
        let mut chunk = json!({"transfer_id":journal(&j.args)["transfer_id"],"offset":0,"total_bytes":bytes.len(),"next_offset":bytes.len(),"data":STANDARD.encode(bytes)});
        match field {
            0 => chunk["transfer_id"] = json!(Uuid::new_v4()),
            1 => chunk["offset"] = json!(1),
            2 => chunk["total_bytes"] = json!(bytes.len() + 1),
            3 => chunk["next_offset"] = json!(0),
            4 => chunk["next_offset"] = json!(bytes.len() + 1),
            5 => chunk["data"] = json!("not-base64!"),
            6 => chunk["data"] = json!(STANDARD.encode(b"x")),
            7 => chunk["data"] = Value::Null,
            8 => chunk["next_offset"] = json!(1.5),
            _ => chunk["data"] = json!("x".repeat(90001)),
        };
        packet.ok(chunk);
        assert!(result(task).await.is_err());
        assert_eq!(j.destination.count("upload_transfer_chunk"), 0);
        assert_eq!(j.destination.count("activate_transfer"), 0);
        assert_eq!(ids(&j), pinned);
        assert!(journal(&j.args)["result"].is_null());
        j.finish().await;
    }
    // Upload already crossed its mutation boundary; a malformed reply cannot
    // qualify the next activation or cause an automatic replacement operation.
    for field in 0..4 {
        let mut j = Journey::new().await;
        let task = new_to_manifest(&mut j, bytes).await;
        j.accept(bytes).await;
        let pinned = ids(&j);
        let packet = chunk_packet(&mut j, 0, bytes, bytes.len() as u64).await;
        assert!(matches!(
            packet.command,
            VesselCommand::UploadTransferChunk { .. }
        ));
        let mut acknowledgement = json!({"transfer_id":journal(&j.args)["transfer_id"],
            "next_offset":bytes.len(),"stored_bytes":bytes.len()});
        match field {
            0 => acknowledgement["transfer_id"] = json!(Uuid::new_v4()),
            1 => acknowledgement["next_offset"] = json!(bytes.len() - 1),
            2 => acknowledgement["stored_bytes"] = json!(bytes.len() - 1),
            _ => acknowledgement["stored_bytes"] = json!(bytes.len() + 1),
        }
        packet.ok(acknowledgement);
        assert!(result(task).await.is_err());
        assert_eq!(j.destination.count("upload_transfer_chunk"), 1);
        assert_eq!(j.destination.count("activate_transfer"), 0);
        assert_eq!(ids(&j), pinned);
        assert_eq!(journal(&j.args)["phase"], "receiving");
        assert!(journal(&j.args)["result"].is_null());
        j.finish().await;
    }
    for field in 0..17 {
        let mut j = Journey::new().await;
        seed(&j, bytes, Some("activation_pending"), 1, None);
        let before = fs::read(&j.args.journal).unwrap();
        let task = j.begin(j.args.clone());
        j.identities().await;
        let packet = status_packet(&mut j, bytes).await;
        let mut proof = status_value(&j, bytes, "receiving", Some(0), None);
        match field {
            0 => proof["transfer_id"] = json!(Uuid::new_v4()),
            1 => proof["activate_command_id"] = json!(Uuid::new_v4()),
            2 => proof["manifest_digest"] = json!("f".repeat(64)),
            3 => proof["source_vessel_id"] = json!(Uuid::new_v4()),
            4 => proof["destination_vessel_id"] = json!(Uuid::new_v4()),
            5 => proof["session_id"] = json!(Uuid::new_v4()),
            6 => proof["artifact_sha256"] = json!("f".repeat(64)),
            7 => proof["artifact_bytes"] = json!(bytes.len() + 1),
            8 => proof["received_bytes"] = json!(bytes.len() + 1),
            9 => {
                proof["completion"] =
                    json!({"session_id":j.args.session,"incarnation":Uuid::new_v4()})
            }
            10 => {
                proof["state"] = json!("pending");
                proof["received_bytes"] = json!(0);
            }
            11 => {
                proof["state"] = json!("complete");
                proof["received_bytes"] = Value::Null;
            }
            12 => {
                proof["state"] = json!("complete");
                proof["received_bytes"] = Value::Null;
                proof["completion"] =
                    json!({"session_id":Uuid::new_v4(),"incarnation":Uuid::new_v4()});
            }
            13 => {
                proof["state"] = json!("complete");
                proof["received_bytes"] = Value::Null;
                proof["completion"] =
                    json!({"session_id":j.args.session,"incarnation":Uuid::nil()});
            }
            14 => proof["unknown_field"] = json!("not permitted"),
            15 => proof["state"] = json!("unknown"),
            _ => proof["received_bytes"] = Value::Null,
        };
        packet.ok(proof);
        assert!(result(task).await.is_err());
        assert_eq!(fs::read(&j.args.journal).unwrap(), before);
        no_mutations(&j);
        j.finish().await;
    }
}
#[tokio::test]
async fn actual_courier_lock_refuses_concurrent_run_and_awaited_abort_releases_owned_obligations() {
    let mut j = Journey::new().await;
    let first = j.begin(j.args.clone());
    let packet = j.source.next().await;
    assert!(matches!(packet.command, VesselCommand::Identity));
    assert!(result(j.begin(j.args.clone())).await.is_err());
    assert_eq!(j.source.count("identity"), 1);
    assert_eq!(j.destination.count("identity"), 0);
    assert!(!j.args.journal.exists());
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    packet.ok(serde_json::to_value(&j.source.identity).unwrap());
    // A separate readonly roundtrip confirms the owned peer has handled the old
    // abandoned reply before disconnect; this is not a resumed courier effect.
    let client = j.source.client.clone();
    let probe = tokio::spawn(async move { client.request(VesselCommand::Identity).await });
    let packet = j.source.next().await;
    assert!(matches!(packet.command, VesselCommand::Identity));
    packet.ok(serde_json::to_value(&j.source.identity).unwrap());
    assert!(result(probe).await.is_ok());
    let held = lock(&j.args.journal).unwrap();
    drop(held);
    no_mutations(&j);
    j.finish().await;
}
