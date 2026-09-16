use super::*;
use serde_json::json;
use std::{io::Write, os::unix::fs::OpenOptionsExt};

fn write_record(root: &std::path::Path, id: Uuid, value: &Value) {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(root.join(format!("{id}.redemption")))
        .unwrap();
    file.write_all(&serde_json::to_vec(value).unwrap()).unwrap();
}

#[test]
fn offline_redemption_recovery_checks_every_binding_without_replaying() {
    let temp = tempfile::tempdir().unwrap();
    let dir = private::Directory::open(temp.path()).unwrap();
    let id = Uuid::new_v4();
    let command = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let invitation = Uuid::new_v4();
    let connection = Uuid::new_v4();
    let value = json!({"summary":{"id":id,"endpoint":"https://example.invalid","vessel_id":Uuid::new_v4(),"invitation_id":invitation,"command_id":command,"principal_id":principal,"connection_id":connection},"request":{"protocol":VESSEL_API_VERSION,"command_id":command,"principal_id":principal,"invitation_id":invitation,"code":"offline-fixture"},"credential_ref":Uuid::new_v4(),"connection_id":connection});
    write_record(temp.path(), id, &value);
    assert_eq!(redemption_connection(&dir, id).unwrap(), connection);
    assert!(read_redemption(&dir, Uuid::new_v4()).is_err());
    for field in [
        "id",
        "command_id",
        "principal_id",
        "invitation_id",
        "connection_id",
    ] {
        let mut changed = value.clone();
        changed["summary"][field] = json!(Uuid::new_v4());
        write_record(temp.path(), id, &changed);
        assert!(read_redemption(&dir, id).is_err(), "{field}");
        assert_eq!(
            serde_json::from_slice::<Value>(
                &std::fs::read(temp.path().join(format!("{id}.redemption"))).unwrap()
            )
            .unwrap(),
            changed
        );
    }
    let mut changed = value.clone();
    changed["request"]["protocol"] = json!(VESSEL_API_VERSION + 1);
    write_record(temp.path(), id, &changed);
    assert!(read_redemption(&dir, id).is_err());
    write_record(temp.path(), id, &json!({}));
    assert!(read_redemption(&dir, id).is_err());
    let mut legacy = value;
    legacy["summary"]
        .as_object_mut()
        .unwrap()
        .remove("connection_id");
    write_record(temp.path(), id, &legacy);
    assert_eq!(redemption_connection(&dir, id).unwrap(), connection);
}

#[test]
fn public_metadata_ids_must_be_present_valid_and_non_nil() {
    let id = Uuid::new_v4();
    assert_eq!(uuid(&json!({"vessel_id":id}), "vessel_id").unwrap(), id);
    for value in [
        json!({}),
        json!({"vessel_id":null}),
        json!({"vessel_id":42}),
        json!({"vessel_id":"bad"}),
        json!({"vessel_id":Uuid::nil()}),
    ] {
        assert!(uuid(&value, "vessel_id").is_err());
    }
}

fn metadata(vessel: Uuid, principal: Uuid) -> Value {
    json!({"protocol":VESSEL_API_VERSION,"vessel_id":vessel,"principal_id":principal,"scope":"workspaces","version":"fixture","expires_at_ms":u64::MAX,"rights":["read"],"features":["duplex_socket"],"workspaces":[{"id":Uuid::new_v4(),"name":"fixture","path":"/synthetic/workspace","provider_ready":true}],"grant_revision":2})
}
fn workspace_credential(endpoint: &str, vessel: Uuid, principal: Uuid) -> Credential {
    Credential::parse(&serde_json::to_vec(&json!({"schema_version":1,"kind":"workspace","endpoint":endpoint,"vessel_id":vessel,"principal_id":principal,"grant_id":Uuid::new_v4(),"token":"fixture-token"})).unwrap()).unwrap()
}

#[tokio::test]
async fn public_pairing_discovery_checks_status_version_and_feature_without_authority() {
    use crate::process_client::loopback_tests::http_peer;
    let valid = json!({"protocol":VESSEL_API_VERSION,"vessel_id":Uuid::new_v4(),"features":["workspace_pairing"]});
    for (status, value, success) in [
        (200, valid.clone(), true),
        (404, valid.clone(), false),
        (403, valid.clone(), false),
        (
            200,
            json!({"protocol":999,"vessel_id":Uuid::new_v4(),"features":["workspace_pairing"]}),
            false,
        ),
        (
            200,
            json!({"protocol":VESSEL_API_VERSION,"vessel_id":Uuid::nil(),"features":["workspace_pairing"]}),
            false,
        ),
        (
            200,
            json!({"protocol":VESSEL_API_VERSION,"vessel_id":Uuid::new_v4(),"features":[]}),
            false,
        ),
        (200, json!({"invalid":true}), false),
        (302, valid, false),
    ] {
        let (endpoint, server) = http_peer(vec![(status, value)]).await;
        assert_eq!(pairing_capabilities(&endpoint).await.is_ok(), success);
        assert_eq!(server.await.unwrap(), vec![json!(null)]);
    }
}

#[tokio::test]
async fn metadata_inspection_rejects_identity_scope_and_bound_violations() {
    use crate::process_client::loopback_tests::{http_peer, response};
    let vessel = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let valid = metadata(vessel, principal);
    let mut cases = vec![(valid.clone(), true)];
    // Keep each independent invalid payload; the successful payload is never mutated.
    for (key, replacement) in [
        ("protocol", json!(999)),
        ("vessel_id", json!(Uuid::nil())),
        ("vessel_id", json!(Uuid::new_v4())),
        ("principal_id", json!(Uuid::new_v4())),
        ("scope", json!("session")),
        ("scope", json!("workspace")),
        ("expires_at_ms", json!(null)),
        ("expires_at_ms", json!(0)),
        ("rights", json!(vec!["read"; 65])),
        ("features", json!(vec!["feature"; 129])),
        (
            "workspaces",
            json!(vec![valid["workspaces"][0].clone(); 257]),
        ),
        (
            "workspaces",
            json!([{"id":Uuid::nil(),"name":"fixture","path":"/ok"}]),
        ),
        (
            "workspaces",
            json!([{"id":Uuid::new_v4(),"name":"fixture","path":"relative"}]),
        ),
        (
            "workspaces",
            json!([{"id":Uuid::new_v4(),"name":"x".repeat(257),"path":"/ok"}]),
        ),
        ("rights", json!("invalid")),
    ] {
        let mut value = valid.clone();
        value[key] = replacement;
        cases.push((value, false));
    }
    for (value, success) in cases {
        let (endpoint, server) =
            http_peer(vec![(200, serde_json::to_value(response(value)).unwrap())]).await;
        let credential = workspace_credential(&endpoint, vessel, principal);
        let result = inspect(&credential, Some(vessel)).await;
        assert_eq!(result.is_ok(), success);
        if let Ok((id, p, scope, metadata)) = result {
            assert_eq!(id, vessel);
            assert_eq!(p, Some(principal));
            assert!(matches!(scope, Scope::Workspaces { .. }));
            assert_eq!(metadata.workspaces.len(), 1);
        }
        let bodies = server.await.unwrap();
        assert_eq!(bodies[0]["command"]["op"], "capabilities");
    }
}

#[tokio::test]
async fn prepared_import_keeps_registry_inactive_and_retains_exact_credential() {
    use crate::process_client::loopback_tests::{http_peer, response, write_private};
    let temp = tempfile::tempdir().unwrap();
    let vessel = Uuid::new_v4();
    let principal = Uuid::new_v4();
    let value = serde_json::to_value(response(metadata(vessel, principal))).unwrap();
    let (endpoint, server) = http_peer(vec![(200, value)]).await;
    let path = temp.path().join("import.json");
    write_private(
        &path,
        &json!({"schema_version":1,"kind":"workspace","endpoint":endpoint,"vessel_id":vessel,"principal_id":principal,"grant_id":Uuid::new_v4(),"token":"fixture-token"}),
    );
    let registry = Registry::open(temp.path().join("registry")).unwrap();
    let original = std::fs::read(&path).unwrap();
    let result = registry.prepare_import(path.clone()).await;
    match result {
        Ok(preview) => {
            assert_eq!(preview.connection.vessel_id, vessel);
            assert_eq!(preview.connection.principal_id, Some(principal));
            assert!(matches!(preview.connection.scope, Scope::Workspaces { .. }));
        }
        Err(error) => assert!(
            error.to_string().contains("connection storage locked"),
            "{error:#}"
        ),
    }
    assert!(registry.load().unwrap().connections.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    server.await.unwrap();
}

#[tokio::test]
async fn saved_pair_redemption_recovers_without_resubmitting_secret() {
    use crate::process_client::loopback_tests::{http_peer, response};
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::open(root.path().join("registry")).unwrap();
    let principal = registry.principal_id().unwrap();
    let vessel = Uuid::new_v4();
    let (endpoint, server) = http_peer(vec![(
        200,
        serde_json::to_value(response(metadata(vessel, principal))).unwrap(),
    )])
    .await;
    let pending = Uuid::new_v4();
    let command = Uuid::new_v4();
    let invitation = Uuid::new_v4();
    let connection = Uuid::new_v4();
    let credential_ref = Uuid::new_v4();
    let value = json!({"summary":{"id":pending,"endpoint":endpoint,"vessel_id":vessel,"invitation_id":invitation,"command_id":command,"principal_id":principal,"connection_id":connection},"request":{"protocol":VESSEL_API_VERSION,"command_id":command,"principal_id":principal,"invitation_id":invitation,"code":"retained-secret"},"credential_ref":credential_ref,"connection_id":connection});
    write_record(registry.root(), pending, &value);
    let credential = json!({"schema_version":1,"kind":"workspace","endpoint":endpoint,"vessel_id":vessel,"principal_id":principal,"grant_id":Uuid::new_v4(),"token":"saved-credential"});
    crate::process_client::loopback_tests::write_private(
        &registry.root().join(format!("{credential_ref}.credential")),
        &credential,
    );
    let preview = registry.resume_pair(pending).await.unwrap();
    assert_eq!(preview.connection.id, connection);
    assert_eq!(preview.connection.credential_ref, credential_ref);
    assert_eq!(server.await.unwrap()[0]["command"]["op"], "capabilities");
    assert_eq!(
        serde_json::from_slice::<Value>(
            &std::fs::read(registry.root().join(format!("{pending}.redemption"))).unwrap()
        )
        .unwrap(),
        value
    );
    let saved = registry
        .save_preview(&preview, "recovered".into(), false)
        .unwrap();
    assert_eq!(saved.id, connection);
    assert!(!saved.autoconnect);
}

#[tokio::test]
async fn pairing_rejects_malformed_invitation_before_private_intent() {
    use crate::process_client::loopback_tests::http_peer;
    for invitation in [
        "missing-separator".to_string(),
        "bad.secret".into(),
        format!("{}.secret", Uuid::nil()),
        format!("{}.", Uuid::new_v4()),
        format!("{}.white space", Uuid::new_v4()),
        format!("{}.{}", Uuid::new_v4(), "x".repeat(4097)),
    ] {
        let root = tempfile::tempdir().unwrap();
        let registry = Registry::open(root.path().join("registry")).unwrap();
        let (endpoint,server) = http_peer(vec![(200,json!({"protocol":VESSEL_API_VERSION,"vessel_id":Uuid::new_v4(),"features":["workspace_pairing"]}))]).await;
        assert!(registry.prepare_pair(endpoint, invitation).await.is_err());
        assert!(registry.load().unwrap().pending_pairs.is_empty());
        assert!(!std::fs::read_dir(registry.root()).unwrap().any(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_some_and(|ext| ext == "redemption")
        }));
        server.await.unwrap();
    }
}
