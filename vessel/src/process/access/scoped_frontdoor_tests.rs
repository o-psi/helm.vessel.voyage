//! Real private grant/catalogue/intent boundaries. No provider, runtime or Root
//! owner is created; each refused path must preserve its exact durable metadata.
use super::*;
use crate::process::{database, test_support::Fixture};
use std::path::PathBuf;
use voyage_protocol::vessel::{VoyageCommand, VoyageRequest};
const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn seed(f: &Fixture) -> ConnectionGrant {
    let mut g = f.connection();
    g.rights = vec![
        ProcessRight::Catalogue,
        ProcessRight::Create,
        ProcessRight::Observe,
    ];
    g.token_hash = store::hash(TOKEN);
    f.save_connection(&g);
    g
}
fn file(f: &Fixture, g: &ConnectionGrant) -> PathBuf {
    store::connection_path(&f.0, g.grant_id)
}
fn command(value: Value) -> VesselCommand {
    serde_json::from_value(value).unwrap()
}
#[tokio::test]
async fn exact_current_connection_rejects_every_independent_authority_drift_without_rewriting_session_scope()
 {
    for field in 0..12 {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let g = seed(&f);
        let session = Uuid::new_v4();
        let derived = s.connection_session(&g, session, &f.0).unwrap();
        let path = store::grant_path(&f.0, derived.grant_id);
        let retained = std::fs::read(&path).unwrap();
        let mut changed = g.clone();
        match field {
            0 => changed.schema_version = 2,
            1 => changed.principal_id = Uuid::new_v4(),
            2 => changed.vessel_id = Uuid::new_v4(),
            3 => changed.revision += 1,
            4 => changed.rights.reverse(),
            5 => changed.accounts.push(Uuid::new_v4()),
            6 => changed.enrollment_connections.push(Uuid::new_v4()),
            7 => changed.workspaces[0].id = Uuid::new_v4(),
            8 => changed.expires_at_ms -= 1,
            9 => changed.revoked = true,
            10 => changed.full_access = true,
            _ => changed.token_hash = store::hash("independently-replaced-token"),
        };
        f.save_connection(&changed);
        let parent_bytes = std::fs::read(file(&f, &g)).unwrap();
        if field == 11 {
            assert!(
                s.connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
                    .await
                    .is_err()
            );
        } else {
            assert!(
                s.connection_session(&g, session, &f.0).is_err(),
                "field {field}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), retained);
        assert_eq!(std::fs::read(file(&f, &g)).unwrap(), parent_bytes);
    }
}
#[tokio::test]
async fn connection_intent_binds_exact_command_principal_revision_and_payload_without_external_replay()
 {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let g = seed(&f);
    let id = Uuid::new_v4();
    let session = Uuid::new_v4();
    let start = VesselCommand::Start {
        command_id: id,
        session_id: session,
        workspace: f.0.clone(),
    };
    s.bind_connection_operation(&g, id, &start).await.unwrap();
    let path =
        f.0.join("access/connection-commands")
            .join(format!("{id}.json"));
    let before = std::fs::read(&path).unwrap();
    s.bind_connection_operation(&g, id, &start).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    for value in [
        VesselCommand::Start {
            command_id: id,
            session_id: Uuid::new_v4(),
            workspace: f.0.clone(),
        },
        VesselCommand::Start {
            command_id: id,
            session_id: session,
            workspace: f.0.join("other"),
        },
        VesselCommand::Restart {
            command_id: id,
            session_id: session,
            incarnation: Uuid::new_v4(),
        },
    ] {
        assert!(s.bind_connection_operation(&g, id, &value).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    for principal in [false, true] {
        let mut changed = g.clone();
        if principal {
            changed.principal_id = Uuid::new_v4();
        } else {
            changed.revision += 1;
        }
        f.save_connection(&changed);
        assert!(
            s.bind_connection_operation(&changed, id, &start)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    assert!(s.registrations.lock().await.unwrap().is_empty());
    assert!(!f.0.join("sessions").join(session.to_string()).exists());
}
#[tokio::test]
async fn start_identity_path_and_configuration_refusals_precede_intent_or_owner_creation() {
    for variant in 0..7 {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let g = seed(&f);
        let session = Uuid::new_v4();
        let id = Uuid::new_v4();
        let parent = std::fs::read(file(&f, &g)).unwrap();
        let request = match variant {
            0 => VesselCommand::Start {
                command_id: Uuid::nil(),
                session_id: session,
                workspace: f.0.clone(),
            },
            1 => VesselCommand::Start {
                command_id: id,
                session_id: Uuid::nil(),
                workspace: f.0.clone(),
            },
            2 => VesselCommand::Start {
                command_id: id,
                session_id: session,
                workspace: f.0.join("..").join(f.0.file_name().unwrap()),
            },
            3 => {
                let outside = f.0.join("outside");
                crate::process::registry::private_directory(&outside).unwrap();
                VesselCommand::Start {
                    command_id: id,
                    session_id: session,
                    workspace: outside,
                }
            }
            4 => VesselCommand::StartConfigured {
                command_id: id,
                session_id: session,
                workspace: f.0.clone(),
                config_path: f.0.join("unapproved.json"),
            },
            5 => VesselCommand::ResolveStart {
                command_id: id,
                session_id: session,
                workspace: f.0.clone(),
                config_path: Some(f.0.join("unapproved.json")),
            },
            _ => {
                let mut registration = f.registration();
                registration.session_id = session;
                registration.workspace = f.0.join("different");
                database::save(&f.0, &registration).await.unwrap();
                VesselCommand::Start {
                    command_id: id,
                    session_id: session,
                    workspace: f.0.clone(),
                }
            }
        };
        assert!(
            s.connected(g.grant_id, TOKEN, request).await.is_err(),
            "variant {variant}"
        );
        assert_eq!(std::fs::read(file(&f, &g)).unwrap(), parent);
        assert!(
            !f.0.join("access/connection-commands")
                .join(format!("{id}.json"))
                .exists()
        );
        assert!(
            !f.0.join("sessions")
                .join(session.to_string())
                .join("runtime.sock")
                .exists()
        );
    }
}
#[tokio::test]
async fn workspace_rights_cancel_and_lifecycle_admission_never_use_owner_flag_as_an_extra_right() {
    let operations = [
        json!({"op":"workspace_file","path":"no-file.txt"}),
        json!({"op":"snapshot"}),
        json!({"op":"cancel","command_id":Uuid::new_v4(),"expected_revision":0,"expires_at_ms":u64::MAX,"run_id":Uuid::new_v4()}),
        json!({"op":"terminal","operation":{"action":"snapshot"}}),
    ];
    for owner in [false, true] {
        for op in &operations {
            let f = Fixture::new();
            let s = f.supervisor().await;
            let mut g = seed(&f);
            g.rights = if owner {
                serde_json::from_value(json!([
                    "catalogue",
                    "account_use",
                    "account_enroll",
                    "create",
                    "observe",
                    "history",
                    "execute",
                    "steer",
                    "decide",
                    "cancel",
                    "lifecycle",
                    "terminal"
                ]))
                .unwrap()
            } else {
                vec![ProcessRight::Catalogue]
            };
            g.full_access = owner;
            if owner {
                g.workspaces.clear();
                g.accounts.clear();
                g.enrollment_connections.clear();
            }
            f.save_connection(&g);
            if owner && op["op"] != "workspace_file" {
                continue;
            }
            let request = serde_json::from_value::<VoyageCommand>(op.clone()).unwrap();
            let session = Uuid::new_v4();
            let before = std::fs::read(file(&f, &g)).unwrap();
            let result = s
                .connected(
                    g.grant_id,
                    TOKEN,
                    VesselCommand::Voyage(VoyageRequest {
                        session_id: session,
                        incarnation: None,
                        command: request,
                    }),
                )
                .await;
            assert!(result.is_err());
            assert!(!f.0.join("sessions").join(session.to_string()).exists());
            assert_eq!(std::fs::read(file(&f, &g)).unwrap(), before);
        }
    }
}
#[tokio::test]
async fn scoped_branch_recovery_and_unknown_owner_operations_are_refused_without_receipt_replay() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let g = seed(&f);
    let session = Uuid::new_v4();
    let command_id = Uuid::new_v4();
    let before = std::fs::read(file(&f, &g)).unwrap();
    for value in [
        json!({"op":"branch","command_id":command_id,"session_id":session,"branch_id":Uuid::new_v4(),"incarnation":Uuid::new_v4(),"expected_revision":0,"expires_at_ms":u64::MAX,"name":null}),
        json!({"op":"recover","command_id":command_id,"session_id":session,"incarnation":Uuid::new_v4()}),
        json!({"op":"update_prepare","operation_id":command_id,"channel":"nightly"}),
        json!({"op":"update_status","operation_id":command_id}),
    ] {
        let c = command(value);
        assert!(s.connected(g.grant_id, TOKEN, c).await.is_err());
        assert_eq!(std::fs::read(file(&f, &g)).unwrap(), before);
        assert!(
            !f.0.join("access/connection-commands")
                .join(format!("{command_id}.json"))
                .exists()
        );
    }
}

#[tokio::test]
async fn frozen_saved_authority_is_checked_on_same_authenticated_grant_before_durable_admission() {
    for field in 0..4 {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let mut g = seed(&f);
        let fingerprint = store::connection_authority_fingerprint(&g).unwrap();
        let caps = s
            .connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
            .await
            .unwrap();
        assert_eq!(caps["authorization_fingerprint"], fingerprint);
        match field {
            0 => g.accounts.push(Uuid::new_v4()),
            1 => g.enrollment_connections.push(Uuid::new_v4()),
            2 => g.workspaces[0].path = f.0.join("other"),
            _ => g.rights.push(ProcessRight::Execute),
        };
        f.save_connection(&g);
        let id = Uuid::new_v4();
        let sid = Uuid::new_v4();
        let request = VesselCommand::Start {
            command_id: id,
            session_id: sid,
            workspace: f.0.clone(),
        };
        let reply = s
            .granted_with_authority(
                g.grant_id,
                TOKEN.into(),
                Some(g.vessel_id),
                Some(fingerprint),
                request,
            )
            .await
            .unwrap_err();
        assert!(
            reply
                .to_string()
                .contains("saved connection authority changed")
        );
        assert!(
            !f.0.join("access/connection-commands")
                .join(format!("{id}.json"))
                .exists()
        );
        assert!(!f.0.join("sessions").join(sid.to_string()).exists());
        assert!(s.registrations.lock().await.unwrap().is_empty());
    }
}
#[tokio::test]
async fn complete_saved_fingerprint_preserves_legacy_owner_bytes_and_ignores_derived_discovery() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = seed(&f);
    g.full_access = true;
    g.workspaces.clear();
    g.accounts.clear();
    g.enrollment_connections.clear();
    g.rights = serde_json::from_value(json!([
        "catalogue",
        "account_use",
        "account_enroll",
        "create",
        "observe",
        "history",
        "execute",
        "steer",
        "decide",
        "cancel",
        "lifecycle",
        "terminal"
    ]))
    .unwrap();
    f.save_connection(&g);
    let raw = std::fs::read(file(&f, &g)).unwrap();
    let first = s
        .connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    let mut r = f.registration();
    r.workspace = f.0.join("new-discovery");
    crate::process::registry::private_directory(&r.workspace).unwrap();
    database::save(&f.0, &r).await.unwrap();
    let second = s
        .connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    assert_ne!(first["workspaces"], second["workspaces"]);
    assert_eq!(
        first["authorization_fingerprint"],
        second["authorization_fingerprint"]
    );
    assert_eq!(std::fs::read(file(&f, &g)).unwrap(), raw);
    assert!(!g.rights.contains(&ProcessRight::WorkspaceRead));
}
#[test]
fn process_saved_fingerprint_includes_nested_scope_and_every_serialized_authority_field() {
    let f = Fixture::new();
    let g = f.session();
    let base = store::process_authority_fingerprint(&g).unwrap();
    for field in 0..9 {
        let mut changed = g.clone();
        match field {
            0 => changed.accounts.push(Uuid::new_v4()),
            1 => changed.enrollment_connections.push(Uuid::new_v4()),
            2 => {
                changed.parent_grant = Some(GrantBinding {
                    grant_id: Uuid::new_v4(),
                    principal_id: g.principal_id,
                    revision: g.revision,
                })
            }
            3 => {
                changed.connection_binding = Some(GrantBinding {
                    grant_id: Uuid::new_v4(),
                    principal_id: g.principal_id,
                    revision: g.revision,
                })
            }
            4 => changed.token_hash = "different-auth-identity".into(),
            5 => changed.workspace = f.0.join("other"),
            6 => changed.expires_at_ms -= 1,
            7 => changed.revoked = true,
            _ => {
                changed.participant_binding =
                    Some(voyage_protocol::process::ParticipantGrantBinding {
                        binding_id: Uuid::new_v4(),
                        revision: 1,
                    })
            }
        };
        assert_ne!(
            base,
            store::process_authority_fingerprint(&changed).unwrap()
        );
    }
    assert_eq!(base, store::process_authority_fingerprint(&g).unwrap());
}

#[tokio::test]
async fn already_loaded_connection_and_session_rechecks_refuse_same_revision_token_rotation() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut connection = seed(&f);
    let retained = connection.clone();
    connection.token_hash = store::hash("rotated-synthetic-auth-identity");
    f.save_connection(&connection);
    assert!(store::current_connection(&f.0, &retained).is_err());
    let registration = f.registration();
    let mut session = f.session();
    session.session_id = registration.session_id;
    f.save_session(&session);
    let held = session.clone();
    session.token_hash = store::hash("changed-session-auth-identity");
    f.save_session(&session);
    assert!(super::super::execution_epoch::check(&f.0, &held).is_err());
    assert!(
        crate::process::accounts::Scope::Session(held)
            .check(&f.0, &f.0, ProcessRight::AccountUse)
            .is_err()
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}

#[tokio::test]
async fn accepted_private_notification_then_late_grant_drift_is_unknown_and_retains_exact_receipt()
{
    use voyage_protocol::notifications::{Destination, NotificationKind, NotificationOperation};
    for drift in ["token", "rights", "revision", "revoked"] {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let g = seed(&f);
        let r = f.registration();
        database::save(&f.0, &r).await.unwrap();
        let destination = Destination {
            id: Uuid::new_v4(),
            recipient_grant_id: g.grant_id,
            recipient_principal_id: g.principal_id,
            recipient_grant_revision: g.revision,
            source_vessel_id: g.vessel_id,
            source_session_id: r.session_id,
            event_kinds: vec![NotificationKind::Test],
            expires_at_ms: store::now().unwrap() + 60_000,
            notification_ttl_ms: 30_000,
            quiet_hours_utc: None,
        };
        s.notifications(
            NotificationOperation::Configure {
                command_id: Uuid::new_v4(),
                destination: destination.clone(),
            },
            None,
        )
        .await
        .unwrap();
        let id = Uuid::new_v4();
        // This is the actual production scoped dispatch branch, using only the
        // owned private SQLite acceptance store. No runtime/provider is started.
        let accepted = s
            .notifications(
                NotificationOperation::Accept {
                    command_id: id,
                    destination_id: destination.id,
                },
                Some(GrantBinding {
                    grant_id: g.grant_id,
                    principal_id: g.principal_id,
                    revision: g.revision,
                }),
            )
            .await
            .unwrap();
        assert!(accepted["accepted_at_ms"].as_u64().is_some());
        let path = f.0.join("notifications/notifications.sqlite3");
        let receipt = || {
            let db = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let command: (String, String, i64) = db
                .query_row(
                    "SELECT id,destination_id,operation FROM commands WHERE id=?1",
                    [id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            let state: (String, i64) = db
                .query_row(
                    "SELECT payload,accepted FROM destinations WHERE id=?1",
                    [destination.id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let count: i64 = db
                .query_row("SELECT count(*) FROM commands", [], |row| row.get(0))
                .unwrap();
            (command, state, count)
        };
        let before = receipt();
        assert_eq!(before.0, (id.to_string(), destination.id.to_string(), 2));
        assert_eq!(before.2, 2);
        let mut changed = g.clone();
        match drift {
            "token" => changed.token_hash = store::hash("late-synthetic-auth-rotation"),
            "rights" => changed.rights.clear(),
            "revision" => changed.revision += 1,
            _ => changed.revoked = true,
        }
        f.save_connection(&changed);
        // Deterministic boundary-level regression: place the independent change
        // after actual dispatch settlement, before its factored final checker.
        // This does not claim an end-to-end scheduler/native race observation.
        let wire = super::super::super::api::response(s.finish_connected_reply(&g, accepted));
        assert!(wire.error.is_some());
        assert!(
            wire.outcome_unknown,
            "late {drift} must not promise definite nonadmission"
        );
        assert!(
            wire.result.is_null(),
            "revoked reply must not expose its payload"
        );
        assert_eq!(
            receipt(),
            before,
            "exact accepted operation was not replayed or rewritten"
        );
        assert!(
            s.registrations
                .lock()
                .await
                .unwrap()
                .contains_key(&r.session_id)
        );
    }
}

#[tokio::test]
async fn readonly_success_stays_redacted_and_preflight_saved_pin_refusal_is_definite() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = seed(&f);
    let frozen = store::connection_authority_fingerprint(&g).unwrap();
    let normal = super::super::super::api::response(
        s.connected_with_authority(
            g.grant_id,
            TOKEN,
            Some(&frozen),
            VesselCommand::Capabilities,
        )
        .await,
    );
    assert!(normal.error.is_none());
    assert!(!normal.outcome_unknown);
    let redacted = s
        .finish_connected_reply(
            &g,
            json!({"catalogue":{"summary":"private fixture history"}}),
        )
        .unwrap();
    assert!(redacted["catalogue"]["summary"].is_null());
    g.accounts.push(Uuid::new_v4());
    f.save_connection(&g);
    let id = Uuid::new_v4();
    let session = Uuid::new_v4();
    let refused = super::super::super::api::response(
        s.connected_with_authority(
            g.grant_id,
            TOKEN,
            Some(&frozen),
            VesselCommand::Start {
                command_id: id,
                session_id: session,
                workspace: f.0.clone(),
            },
        )
        .await,
    );
    assert!(refused.error.is_some());
    assert!(!refused.outcome_unknown);
    assert!(
        !f.0.join("access/connection-commands")
            .join(format!("{id}.json"))
            .exists()
    );
    assert!(!f.0.join("sessions").join(session.to_string()).exists());
    assert!(s.registrations.lock().await.unwrap().is_empty());
    // With no complete Vessel effect classifier, an authority failure AFTER a
    // successful read is also conservatively unknown; it never releases data.
    let late_read = super::super::super::api::response(s.finish_connected_reply(
        &store::authenticate_connection(&f.0, g.grant_id, TOKEN).unwrap(),
        json!({"safe":true}),
    ));
    assert!(late_read.error.is_none());
    assert!(!late_read.outcome_unknown);
    let mut changed = g.clone();
    changed.revoked = true;
    f.save_connection(&changed);
    let withheld =
        super::super::super::api::response(s.finish_connected_reply(&g, json!({"safe":true})));
    assert!(withheld.error.is_some());
    assert!(withheld.outcome_unknown);
    assert!(withheld.result.is_null());
}
