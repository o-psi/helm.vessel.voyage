//! Public scoped routing refuses authority changes before any process launch.
use super::{access::store, identity, test_support::Fixture};
use uuid::Uuid;
use voyage_protocol::process::*;

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn session_capabilities_authenticate_and_report_only_the_persisted_scope() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let mut grant = f.session();
    grant.token_hash = store::hash(TOKEN);
    grant.rights = vec![ProcessRight::Observe];
    f.save_session(&grant);
    let vessel = identity::public(&f.0).unwrap().vessel_id;
    let reply = supervisor
        .granted(
            grant.grant_id,
            TOKEN.into(),
            Some(vessel),
            VesselCommand::Capabilities,
        )
        .await
        .unwrap();
    assert_eq!(reply["scope"], "session");
    assert_eq!(reply["vessel_id"], vessel.to_string());
    assert_eq!(reply["session_id"], grant.session_id.to_string());
    assert_eq!(reply["principal_id"], grant.principal_id.to_string());
    assert_eq!(reply["grant_revision"], grant.revision);
    assert_eq!(
        reply["rights"],
        serde_json::to_value(&grant.rights).unwrap()
    );
    for token in ["short".to_owned(), "b".repeat(64)] {
        assert!(
            supervisor
                .granted(
                    grant.grant_id,
                    token,
                    Some(vessel),
                    VesselCommand::Capabilities
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("access denied")
        );
    }
    assert!(
        supervisor
            .granted(
                grant.grant_id,
                TOKEN.into(),
                Some(Uuid::new_v4()),
                VesselCommand::Capabilities
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("Vessel identity changed")
    );
    grant.revoked = true;
    f.save_session(&grant);
    assert!(
        supervisor
            .granted(
                grant.grant_id,
                TOKEN.into(),
                Some(vessel),
                VesselCommand::Capabilities
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("revoked")
    );
    grant.revoked = false;
    grant.expires_at_ms = 0;
    f.save_session(&grant);
    assert!(
        supervisor
            .granted(
                grant.grant_id,
                TOKEN.into(),
                Some(vessel),
                VesselCommand::Capabilities
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
}

#[tokio::test]
async fn lifecycle_routes_require_both_right_and_exact_session_scope() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let mut grant = f.session();
    grant.token_hash = store::hash(TOKEN);
    grant.rights.clear();
    f.save_session(&grant);
    let commands = || {
        vec![
            VesselCommand::Catalogue,
            VesselCommand::Start {
                command_id: Uuid::new_v4(),
                session_id: grant.session_id,
                workspace: f.0.clone(),
            },
            VesselCommand::Stop {
                session_id: grant.session_id,
                incarnation: Uuid::new_v4(),
            },
            VesselCommand::Restart {
                command_id: Uuid::new_v4(),
                session_id: grant.session_id,
                incarnation: Uuid::new_v4(),
            },
        ]
    };
    for command in commands() {
        assert!(
            supervisor
                .granted(grant.grant_id, TOKEN.into(), None, command)
                .await
                .unwrap_err()
                .to_string()
                .contains("permission denied")
        );
    }
    grant.rights = vec![ProcessRight::Lifecycle];
    f.save_session(&grant);
    for command in [
        VesselCommand::Start {
            command_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            workspace: f.0.clone(),
        },
        VesselCommand::Stop {
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
        },
        VesselCommand::Restart {
            command_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
        },
    ] {
        assert!(
            supervisor
                .granted(grant.grant_id, TOKEN.into(), None, command)
                .await
                .unwrap_err()
                .to_string()
                .contains("denied")
        );
    }
    let other = f.0.join("other-workspace");
    std::fs::create_dir(&other).unwrap();
    let command = VesselCommand::Start {
        command_id: Uuid::new_v4(),
        session_id: grant.session_id,
        workspace: other,
    };
    assert!(
        supervisor
            .granted(grant.grant_id, TOKEN.into(), None, command)
            .await
            .unwrap_err()
            .to_string()
            .contains("creation scope denied")
    );
    assert!(!f.0.join("new-voyage").exists());
}

#[tokio::test]
async fn workspace_connections_require_pinned_vessel_even_for_capabilities() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let mut grant = f.connection();
    grant.token_hash = store::hash(TOKEN);
    f.save_connection(&grant);
    assert!(
        supervisor
            .granted(
                grant.grant_id,
                TOKEN.into(),
                None,
                VesselCommand::Capabilities
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("requires pinned Vessel identity")
    );
    let reply = supervisor
        .granted(
            grant.grant_id,
            TOKEN.into(),
            Some(grant.vessel_id),
            VesselCommand::Capabilities,
        )
        .await
        .unwrap();
    assert_eq!(reply["vessel_id"], grant.vessel_id.to_string());
    assert_eq!(reply["principal_id"], grant.principal_id.to_string());
    grant.revoked = true;
    f.save_connection(&grant);
    assert!(
        supervisor
            .granted(
                grant.grant_id,
                TOKEN.into(),
                Some(grant.vessel_id),
                VesselCommand::Capabilities
            )
            .await
            .is_err()
    );
}
