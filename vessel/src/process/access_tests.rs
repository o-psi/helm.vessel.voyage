use super::super::{database, test_support::Fixture};
use super::*;
use voyage_protocol::vessel::{VoyageCommand, VoyageRequest};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn legacy_owner_capabilities_preserve_grant_bytes_and_refuse_new_workspace_right() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut grant = f.connection();
    grant.full_access = true;
    grant.workspaces.clear();
    grant.accounts.clear();
    grant.enrollment_connections.clear();
    grant.token_hash = store::hash(TOKEN);
    grant.rights = serde_json::from_value(json!([
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
    f.save_connection(&grant);
    let path = store::connection_path(&f.0, grant.grant_id);
    let retained = std::fs::read(&path).unwrap();
    for _ in 0..2 {
        let caps = s
            .connected(grant.grant_id, TOKEN, VesselCommand::Capabilities)
            .await
            .unwrap();
        assert_eq!(caps["scope"], "owner");
        assert_eq!(caps["rights"], serde_json::to_value(&grant.rights).unwrap());
        assert_eq!(caps["vessel_id"], json!(grant.vessel_id));
        assert_eq!(caps["principal_id"], json!(grant.principal_id));
        assert_eq!(std::fs::read(&path).unwrap(), retained);
    }
    // The right check precedes registration lookup: this nonexistent session
    // must not be opened or prepared, and an owner flag cannot supply the right.
    let refused = s
        .connected(
            grant.grant_id,
            TOKEN,
            VesselCommand::Voyage(VoyageRequest {
                session_id: Uuid::new_v4(),
                incarnation: None,
                command: VoyageCommand::WorkspaceFile {
                    path: "new-right.txt".into(),
                },
            }),
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("workspace permission denied"));
    assert_eq!(std::fs::read(&path).unwrap(), retained);
    assert!(
        store::authenticate_connection(
            &f.0,
            grant.grant_id,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        )
        .err()
        .unwrap()
        .to_string()
        .contains("access denied")
    );
    grant.revoked = true;
    f.save_connection(&grant);
    assert!(
        s.connected(grant.grant_id, TOKEN, VesselCommand::Capabilities)
            .await
            .unwrap_err()
            .to_string()
            .contains("access revoked")
    );
}

fn issue(f: &Fixture) -> VesselCommand {
    VesselCommand::Grant {
        command_id: Uuid::new_v4(),
        grant_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        rights: vec![ProcessRight::Observe],
        accounts: vec![],
        enrollment_connections: vec![],
        expires_at_ms: store::now().unwrap() + 60_000,
        endpoint: "http://127.0.0.1:12345".into(),
    }
}
fn scoped(grant: &ProcessGrant, command: VesselCommand) -> VesselCommand {
    VesselCommand::Granted {
        expected_vessel_id: None,
        grant_id: grant.grant_id,
        token: TOKEN.into(),
        command: Box::new(command),
    }
}

#[tokio::test]
async fn grant_issue_retry_payload_conflict_revocation_and_authentication_are_durable() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let command = issue(&f);
    let first = s.grant(command.clone()).await.unwrap();
    assert_eq!(s.grant(command.clone()).await.unwrap(), first);
    let credential: AccessCredential = serde_json::from_value(first).unwrap();
    assert_eq!(credential.token.len(), 64);
    let grant = store::authenticate(&f.0, credential.grant_id, &credential.token).unwrap();
    assert_eq!(grant.revision, 1);
    assert_ne!(grant.token_hash, credential.token);
    assert!(store::authenticate(&f.0, grant.grant_id, TOKEN).is_err());
    assert!(store::authenticate(&f.0, grant.grant_id, "short").is_err());
    let mut conflict = command;
    if let VesselCommand::Grant { rights, .. } = &mut conflict {
        rights.push(ProcessRight::History);
    }
    assert!(s.grant(conflict).await.is_err());
    let revoke = VesselCommand::RevokeGrant {
        command_id: Uuid::new_v4(),
        grant_id: grant.grant_id,
        expected_revision: 1,
    };
    let result = s.revoke_grant(revoke.clone()).await.unwrap();
    assert_eq!(result["revision"], 2);
    assert_eq!(result["revoked"], true);
    let repeated = s.revoke_grant(revoke).await.unwrap();
    assert_eq!(repeated["revision"], 2);
    assert!(
        store::authenticate(&f.0, grant.grant_id, &credential.token)
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
    assert!(
        s.revoke_grant(VesselCommand::RevokeGrant {
            command_id: Uuid::new_v4(),
            grant_id: grant.grant_id,
            expected_revision: 1
        })
        .await
        .unwrap_err()
        .to_string()
        .contains("revision conflict")
    );
}

#[tokio::test]
async fn grant_validation_rejects_bad_identities_rights_expiry_and_origins_without_publication() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    for case in 0..12 {
        let mut command = issue(&f);
        let id = if let VesselCommand::Grant {
            command_id,
            grant_id,
            principal_id,
            session_id,
            rights,
            expires_at_ms,
            endpoint,
            workspace,
            ..
        } = &mut command
        {
            match case {
                0 => *command_id = Uuid::nil(),
                1 => *grant_id = Uuid::nil(),
                2 => *principal_id = Uuid::nil(),
                3 => *session_id = Uuid::nil(),
                4 => rights.clear(),
                5 => *rights = vec![ProcessRight::Observe, ProcessRight::Observe],
                6 => *rights = vec![ProcessRight::Catalogue],
                7 => *rights = vec![ProcessRight::Create],
                8 => *expires_at_ms = 0,
                9 => *expires_at_ms = u64::MAX,
                10 => *endpoint = "http://example.invalid".into(),
                11 => *workspace = f.0.join("absent"),
                _ => unreachable!(),
            }
            *grant_id
        } else {
            unreachable!()
        };
        assert!(s.grant(command).await.is_err(), "case {case}");
        assert!(!store::grant_path(&f.0, id).exists(), "case {case}");
        assert!(!store::credential_path(&f.0, id).exists(), "case {case}");
    }
    assert!(s.grant(VesselCommand::Capabilities).await.is_err());
    assert!(s.revoke_grant(VesselCommand::Capabilities).await.is_err());
}

#[tokio::test]
async fn session_scope_enforces_command_rights_and_target_before_owner_contact() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.session();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![ProcessRight::Observe];
    f.save_session(&g);
    let caps = s
        .handle(scoped(&g, VesselCommand::Capabilities))
        .await
        .unwrap();
    assert_eq!(caps["scope"], "session");
    assert_eq!(caps["grant_revision"], g.revision);
    for feature in ["execution_budget", "goals"] {
        assert!(
            caps["features"]
                .as_array()
                .unwrap()
                .contains(&json!(feature))
        );
    }
    assert!(
        !caps["features"]
            .as_array()
            .unwrap()
            .contains(&json!("start_settings"))
    );
    let cases = [
        (
            VesselCommand::Inspect {
                session_id: Uuid::new_v4(),
            },
            "session grant denied",
        ),
        (VesselCommand::Identity, "local account-owner"),
        (
            VesselCommand::Voyage(VoyageRequest {
                session_id: g.session_id,
                incarnation: None,
                command: VoyageCommand::History {
                    offset: 0,
                    limit: 1,
                    expected_revision: None,
                },
            }),
            "permission denied",
        ),
        (
            VesselCommand::Voyage(VoyageRequest {
                session_id: g.session_id,
                incarnation: None,
                command: VoyageCommand::Resolve {
                    command_id: Uuid::new_v4(),
                    original: None,
                },
            }),
            "unavailable to scoped",
        ),
        (
            VesselCommand::Start {
                command_id: Uuid::new_v4(),
                session_id: g.session_id,
                workspace: f.0.clone(),
            },
            "permission denied",
        ),
    ];
    for (command, message) in cases {
        let error = s.handle(scoped(&g, command)).await.unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert!(
            error
                .downcast_ref::<super::super::routing::OutcomeUnknown>()
                .is_none()
        );
    }
    g.revoked = true;
    f.save_session(&g);
    assert!(
        s.handle(scoped(&g, VesselCommand::Capabilities))
            .await
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
    g.revoked = false;
    g.expires_at_ms = 0;
    f.save_session(&g);
    assert!(
        s.handle(scoped(&g, VesselCommand::Capabilities))
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
}

#[tokio::test]
async fn connection_derivation_is_stable_scoped_and_detects_each_retained_authority_conflict() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.rights
        .extend([ProcessRight::Catalogue, ProcessRight::Create]);
    f.save_connection(&g);
    let session = Uuid::new_v4();
    let binding = s.connection_session(&g, session, &f.0).unwrap();
    let second = s.connection_session(&g, session, &f.0).unwrap();
    assert_eq!(binding.grant_id, second.grant_id);
    let path = store::grant_path(&f.0, binding.grant_id);
    let original: ProcessGrant = store::load(&path).unwrap();
    assert!(original.token_hash.is_empty());
    assert!(!original.rights.contains(&ProcessRight::Create));
    assert!(!original.rights.contains(&ProcessRight::Catalogue));
    assert_eq!(original.accounts, g.accounts);
    assert_eq!(original.enrollment_connections, g.enrollment_connections);
    assert_eq!(
        original.connection_binding.as_ref().unwrap().grant_id,
        g.grant_id
    );
    for case in 0..11 {
        let mut altered = original.clone();
        match case {
            0 => altered.grant_id = Uuid::new_v4(),
            1 => altered.principal_id = Uuid::new_v4(),
            2 => altered.session_id = Uuid::new_v4(),
            3 => altered.workspace = f.0.join("other"),
            4 => altered.revision += 1,
            5 => altered.revoked = true,
            6 => altered.expires_at_ms -= 1,
            7 => altered.rights.clear(),
            8 => altered.accounts.clear(),
            9 => altered.enrollment_connections.clear(),
            10 => altered.connection_binding = None,
            _ => unreachable!(),
        }
        store::save(&path, &altered).unwrap();
        assert!(
            s.connection_session(&g, session, &f.0)
                .unwrap_err()
                .to_string()
                .contains("derived session authority conflict"),
            "case {case}"
        );
    }
    store::save(&path, &original).unwrap();
    assert!(
        s.connection_session(&g, Uuid::new_v4(), &f.0.join("absent"))
            .is_err()
    );
    let other = s.connection_session(&g, Uuid::new_v4(), &f.0).unwrap();
    assert_ne!(other.grant_id, binding.grant_id);
    g.revoked = true;
    f.save_connection(&g);
    assert!(
        s.connection_session(&g, session, &f.0)
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
}

#[tokio::test]
async fn workspace_catalogue_filters_unapproved_sessions_and_rechecks_revocation() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![ProcessRight::Catalogue, ProcessRight::Observe];
    f.save_connection(&g);
    let r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    let mut outside = f.registration();
    outside.workspace = f.0.join("unapproved");
    registry::private_directory(&outside.workspace).unwrap();
    database::save(&f.0, &outside).await.unwrap();
    let caps = s
        .connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    assert_eq!(caps["scope"], "workspaces");
    for feature in ["execution_budget", "goals", "start_settings"] {
        assert!(
            caps["features"]
                .as_array()
                .unwrap()
                .contains(&json!(feature))
        );
    }
    let catalogue = s
        .connected(g.grant_id, TOKEN, VesselCommand::Catalogue)
        .await
        .unwrap();
    let entries = catalogue.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["session_id"], r.session_id.to_string());
    let page = s
        .connected(
            g.grant_id,
            TOKEN,
            VesselCommand::CatalogueChanges {
                after: Some(0),
                limit: 128,
                wait_ms: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["entries"][0]["session_id"], r.session_id.to_string());
    assert!(page["entries"][0]["catalogue"]["summary"].is_null());
    assert_eq!(page["cursor"], page["latest_cursor"]);
    assert!(
        s.connected(
            g.grant_id,
            TOKEN,
            VesselCommand::CatalogueChanges {
                after: None,
                limit: 1,
                wait_ms: 10_001
            }
        )
        .await
        .is_err()
    );

    assert!(
        s.connected(
            g.grant_id,
            TOKEN,
            VesselCommand::Inspect {
                session_id: outside.session_id
            }
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("workspace scope denied")
    );
    assert!(
        s.connected(g.grant_id, TOKEN, VesselCommand::Identity)
            .await
            .is_err()
    );
    g.rights.clear();
    f.save_connection(&g);
    assert!(
        s.connected(g.grant_id, TOKEN, VesselCommand::Catalogue)
            .await
            .unwrap_err()
            .to_string()
            .contains("permission denied")
    );
    g.revoked = true;
    f.save_connection(&g);
    assert!(
        s.connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
            .await
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
}

#[tokio::test]
async fn owner_connection_discovers_dynamic_workspaces_and_never_upgrades_scoped_grants() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut owner = f.connection();
    owner.full_access = true;
    owner.workspaces.clear();
    owner.accounts.clear();
    owner.enrollment_connections.clear();
    owner.rights = ProcessRight::all();
    owner.token_hash = store::hash(TOKEN);
    f.save_connection(&owner);
    let caps = s
        .connected(owner.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    assert_eq!(caps["scope"], "owner");
    assert!(!caps["workspaces"].as_array().unwrap().is_empty());
    let mut r = f.registration();
    r.workspace = f.0.join("created-after-pairing");
    registry::private_directory(&r.workspace).unwrap();
    database::save(&f.0, &r).await.unwrap();
    let caps = s
        .connected(owner.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    assert_eq!(
        caps["workspaces"][0]["path"],
        r.workspace.to_string_lossy().as_ref()
    );
    let catalogue = s
        .connected(owner.grant_id, TOKEN, VesselCommand::Catalogue)
        .await
        .unwrap();
    assert_eq!(catalogue.as_array().unwrap().len(), 1);
    let binding = s
        .connection_session(&owner, r.session_id, &r.workspace)
        .unwrap();
    let derived: ProcessGrant = store::load(&store::grant_path(&f.0, binding.grant_id)).unwrap();
    assert!(derived.full_access);
    assert_eq!(derived.connection_binding.unwrap().grant_id, owner.grant_id);
    let scoped = f.connection();
    f.save_connection(&scoped);
    assert!(
        s.connection_session(&scoped, r.session_id, &r.workspace)
            .is_err()
    );
    let mut legacy = serde_json::to_value(&owner).unwrap();
    legacy.as_object_mut().unwrap().remove("full_access");
    let legacy: ConnectionGrant = serde_json::from_value(legacy).unwrap();
    assert!(!legacy.full_access);
    assert!(store::current_connection(&f.0, &legacy).is_err());
    let mut contradictory = owner.clone();
    contradictory.accounts.push(Uuid::new_v4());
    f.save_connection(&contradictory);
    assert!(store::current_connection(&f.0, &contradictory).is_err());
    owner.revoked = true;
    f.save_connection(&owner);
    assert!(
        s.connected(owner.grant_id, TOKEN, VesselCommand::Catalogue)
            .await
            .is_err()
    );
    assert!(
        s.connection_session(&owner, r.session_id, &r.workspace)
            .is_err()
    );
}

#[tokio::test]
async fn sidebar_branch_requires_full_owner_and_each_existing_right() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![
        ProcessRight::Create,
        ProcessRight::History,
        ProcessRight::Lifecycle,
    ];
    let command = VesselCommand::Branch {
        command_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: store::now().unwrap() + 60_000,
        branch_id: Uuid::new_v4(),
        name: None,
        through_message: None,
    };
    f.save_connection(&g);
    assert!(
        s.connected(g.grant_id, TOKEN, command.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("branch requires local account-owner authority")
    );
    g.full_access = true;
    g.workspaces.clear();
    g.accounts.clear();
    g.enrollment_connections.clear();
    for missing in [
        ProcessRight::Create,
        ProcessRight::History,
        ProcessRight::Lifecycle,
    ] {
        g.rights = vec![
            ProcessRight::Create,
            ProcessRight::History,
            ProcessRight::Lifecycle,
        ];
        g.rights.retain(|right| *right != missing);
        f.save_connection(&g);
        assert!(
            s.connected(g.grant_id, TOKEN, command.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("contradictory owner authority")
        );
    }
    g.rights = ProcessRight::all();
    f.save_connection(&g);
    let r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    let mut stale = command;
    if let VesselCommand::Branch { session_id, .. } = &mut stale {
        *session_id = r.session_id;
    }
    // Full-owner routing reaches the existing exact-incarnation branch guard.
    assert!(
        s.connected(g.grant_id, TOKEN, stale)
            .await
            .unwrap_err()
            .to_string()
            .contains("stale source incarnation")
    );
}

#[tokio::test]
async fn sidebar_restart_preserves_lifecycle_right_and_workspace_boundary() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![ProcessRight::Observe];
    f.save_connection(&g);
    let mut r = f.registration();
    r.workspace = f.0.join("outside");
    registry::private_directory(&r.workspace).unwrap();
    database::save(&f.0, &r).await.unwrap();
    let command = VesselCommand::Restart {
        command_id: Uuid::new_v4(),
        session_id: r.session_id,
        incarnation: r.incarnation,
    };
    assert!(
        s.connected(g.grant_id, TOKEN, command.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("workspace permission denied")
    );
    g.rights.push(ProcessRight::Lifecycle);
    f.save_connection(&g);
    assert!(
        s.connected(g.grant_id, TOKEN, command)
            .await
            .unwrap_err()
            .to_string()
            .contains("workspace scope denied")
    );
}

#[test]
fn sidebar_voyage_mutations_keep_existing_lifecycle_authorization() {
    let id = Uuid::new_v4();
    for command in [
        VoyageCommand::Rename {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            name: "name".into(),
        },
        VoyageCommand::Archive {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            archived: false,
        },
        VoyageCommand::Delete {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            confirm_session_id: id,
        },
        VoyageCommand::Clear {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            confirm_session_id: id,
        },
        VoyageCommand::Compact {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            retain: 8,
            preserve_canonical: true,
        },
    ] {
        assert_eq!(
            crate::process::api::required_right(&command),
            Some(ProcessRight::Lifecycle)
        );
    }
}

#[tokio::test]
async fn remote_updates_are_not_workspace_or_session_execution_rights() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![
        ProcessRight::Execute,
        ProcessRight::Lifecycle,
        ProcessRight::Create,
    ];
    f.save_connection(&g);
    let caps = s
        .connected(g.grant_id, TOKEN, VesselCommand::Capabilities)
        .await
        .unwrap();
    assert_eq!(caps["remote_updates"], false);
    for command in [
        VesselCommand::UpdatePrepare {
            operation_id: Uuid::new_v4(),
            channel: "nightly".into(),
        },
        VesselCommand::UpdateApply {
            operation_id: Uuid::new_v4(),
            release_id: "a".repeat(64),
        },
        VesselCommand::UpdateStatus {
            operation_id: Uuid::new_v4(),
        },
        VesselCommand::UpdateDiscard {
            operation_id: Uuid::new_v4(),
        },
    ] {
        assert!(
            s.connected(g.grant_id, TOKEN, command)
                .await
                .unwrap_err()
                .to_string()
                .contains("account-owner authority")
        );
    }
}

#[tokio::test]
async fn catalogue_long_poll_rechecks_revocation_before_returning() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut g = f.connection();
    g.token_hash = store::hash(TOKEN);
    g.rights = vec![ProcessRight::Catalogue];
    f.save_connection(&g);
    let checkpoint = s
        .connected(
            g.grant_id,
            TOKEN,
            VesselCommand::CatalogueChanges {
                after: None,
                limit: 1,
                wait_ms: 0,
            },
        )
        .await
        .unwrap();
    let read = s.connected(
        g.grant_id,
        TOKEN,
        VesselCommand::CatalogueChanges {
            after: checkpoint["cursor"].as_u64(),
            limit: 1,
            wait_ms: 100,
        },
    );
    let revoke = async {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        g.revoked = true;
        f.save_connection(&g);
    };
    let (result, ()) = tokio::join!(read, revoke);
    assert!(result.unwrap_err().to_string().contains("revoked"));
}

#[tokio::test]
async fn session_catalogue_changes_filter_peers_and_redact_real_history_projection() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut grant = f.session();
    grant.token_hash = store::hash(TOKEN);
    grant.rights = vec![ProcessRight::Observe];
    let mut own = f.registration();
    own.session_id = grant.session_id;
    database::save(&f.0, &own).await.unwrap();
    let other = f.registration();
    database::save(&f.0, &other).await.unwrap();
    let summary = json!({"session_id":own.session_id,"revision":3,"observation_cursor":8,
        "name":"Retained","model":"private-model","created_at":null,"last_turn_end":null,
        "total_messages":2,"run_id":null,"run_state":null,"archived":false,"deleted":false,"pending_cleanup_run":null});
    rusqlite::Connection::open(f.0.join("catalogue.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE catalogue SET summary=?1 WHERE session_id=?2",
            rusqlite::params![summary.to_string(), own.session_id.to_string()],
        )
        .unwrap();
    f.save_session(&grant);
    let command = VesselCommand::CatalogueChanges {
        after: Some(0),
        limit: 128,
        wait_ms: 0,
    };
    let redacted = s.handle(scoped(&grant, command.clone())).await.unwrap();
    assert_eq!(redacted["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        redacted["entries"][0]["session_id"],
        own.session_id.to_string()
    );
    assert!(redacted["entries"][0]["catalogue"]["summary"].is_null());
    grant.rights.push(ProcessRight::History);
    f.save_session(&grant);
    let history = s.handle(scoped(&grant, command.clone())).await.unwrap();
    assert_eq!(
        history["entries"][0]["catalogue"]["summary"]["model"],
        "private-model"
    );
    grant.rights.clear();
    f.save_session(&grant);
    assert!(s.handle(scoped(&grant, command)).await.is_err());
}
