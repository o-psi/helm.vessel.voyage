use super::super::connections::{Connection, LegacyRoute, Metadata, Scope};
use super::*;
use uuid::Uuid;

#[tokio::test]
async fn client_constructors_keep_exact_route_identity_and_generation() {
    let local = Client::local("/synthetic/vessel".into());
    assert!(local.is_local());
    assert_eq!(local.label(), "local");
    assert_eq!(local.id(), local.legacy_route().id());
    assert_eq!(local.pin(), None);
    assert!(local.managed().is_none());
    let access = Client::access("/synthetic/access.json".into());
    assert!(!access.is_local());
    assert_eq!(access.label(), "grant:access.json");
    assert_eq!(access.legacy_route().directory, PathBuf::new());
    let explicit =
        Client::access_with_directory("/synthetic/access.json".into(), "/synthetic/vessel".into());
    assert_ne!(access.id(), explicit.id());
    let id = explicit.id();
    let changed = explicit.with_generation(7);
    assert_eq!(changed.id(), id);
    assert_eq!(changed.generation(), 7);
    assert_eq!(changed.with_generation(7).generation(), 7);
    let route = LegacyRoute {
        directory: "/original/vessel".into(),
        access_file: Some("/original/access".into()),
    };
    let connection = Connection {
        id: Uuid::new_v4(),
        alias: "fixture alias".into(),
        endpoint: "https://example.invalid".into(),
        vessel_id: Uuid::new_v4(),
        principal_id: None,
        grant_id: Uuid::new_v4(),
        scope: Scope::Workspaces {
            workspace_ids: vec![],
        },
        credential_ref: Uuid::new_v4(),
        autoconnect: false,
        workspace_preference: None,
        revision: 1,
        forgotten: false,
        legacy_route: Some(route.clone()),
        metadata: Metadata::default(),
    };
    let client = Client::from_connection(connection.clone(), "/synthetic/credential".into());
    assert_eq!(client.id(), connection.id);
    assert_eq!(client.pin(), Some(connection.vessel_id));
    assert_eq!(client.label(), "fixture alias");
    assert_eq!(client.legacy_route(), route);
    assert_eq!(client.managed(), Some(&connection));
    let mut no_alias = connection;
    no_alias.alias.clear();
    no_alias.legacy_route = None;
    let client = Client::from_connection(no_alias, "/synthetic/credential".into());
    assert_eq!(client.label(), "https://example.invalid");
    assert_eq!(
        client.legacy_route().access_file,
        Some(PathBuf::from("/synthetic/credential"))
    );
}

#[tokio::test]
async fn transport_owner_rules_and_public_response_failures() {
    use crate::process_client::loopback_tests::{Peer, response, send};
    use serde_json::json;
    use voyage_protocol::{duplex::ServerFrame, vessel::*};
    let mut peer = Peer::open().await;
    // Snapshot and browser preparation follow the current owner; cancellation
    // must remain bound to the observed incarnation.
    for (command, exact) in [
        (VoyageCommand::Snapshot, false),
        (VoyageCommand::PrepareBrowser, false),
        (
            VoyageCommand::Cancel {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: u64::MAX,
                run_id: Uuid::new_v4(),
            },
            true,
        ),
    ] {
        for (wrong_session, wrong_owner) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let session = Uuid::new_v4();
            let owner = Uuid::new_v4();
            let c = peer.client.clone();
            let command = command.clone();
            let task =
                tokio::spawn(async move { c.voyage_observed(session, owner, command).await });
            let (id, command) = peer.command().await;
            let VesselCommand::Voyage(request) = command else {
                panic!("voyage")
            };
            assert_eq!(request.session_id, session);
            assert_eq!(request.incarnation, exact.then_some(owner));
            let returned_owner = if wrong_owner { Uuid::new_v4() } else { owner };
            peer.voyage_reply(
                id,
                if wrong_session {
                    Uuid::new_v4()
                } else {
                    session
                },
                returned_owner,
                json!({"fixture":true}),
            )
            .await;
            let result = task.await.unwrap();
            if wrong_session || (exact && wrong_owner) {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("identity mismatch")
                );
            } else {
                assert_eq!(result.unwrap(), (json!({"fixture":true}), returned_owner));
            }
        }
    }
    for reply in [
        VesselResponse {
            error: Some("explicit refusal".into()),
            ..response(json!(null))
        },
        VesselResponse {
            outcome_unknown: true,
            ..response(json!(null))
        },
    ] {
        let c = peer.client.clone();
        let task = tokio::spawn(async move { c.request(VesselCommand::Capabilities).await });
        let (id, _) = peer.command().await;
        send(
            &mut peer.socket,
            ServerFrame::Reply {
                request_id: id,
                response: reply,
            },
        )
        .await;
        assert!(task.await.unwrap().is_err());
    }
    peer.client.disconnect();
    assert!(
        peer.client
            .request(VesselCommand::Capabilities)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn private_terminal_is_socket_bound_and_checks_exact_owner() {
    use crate::process_client::loopback_tests::Peer;
    use serde_json::json;
    use voyage_protocol::vessel::*;
    let mut peer = Peer::open().await;
    let socket = peer.client.connection_state().borrow().socket_id.unwrap();
    let session = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let run = Uuid::new_v4();
    let terminal = Uuid::new_v4();
    for mismatch in [false, true] {
        let c = peer.client.clone();
        let task = tokio::spawn(async move {
            c.private_terminal(
                socket,
                session,
                owner,
                run,
                terminal,
                TerminalAction::Snapshot,
            )
            .await
        });
        let (id, command) = peer.command().await;
        let VesselCommand::Voyage(request) = command else {
            panic!("voyage")
        };
        assert_eq!(request.incarnation, Some(owner));
        assert!(
            matches!(request.command,VoyageCommand::Terminal {run_id,terminal_id,operation:TerminalAction::Snapshot} if run_id==run && terminal_id==terminal)
        );
        peer.voyage_reply(
            id,
            session,
            if mismatch { Uuid::new_v4() } else { owner },
            json!({"revision":1}),
        )
        .await;
        assert_eq!(task.await.unwrap().is_ok(), !mismatch);
    }
    assert!(
        peer.client
            .private_terminal(
                Uuid::new_v4(),
                session,
                owner,
                run,
                terminal,
                TerminalAction::Snapshot
            )
            .await
            .is_err()
    );
    assert_eq!(
        peer.client.connection_state().borrow().socket_id,
        Some(socket)
    );
}

#[tokio::test]
async fn feature_probe_requires_explicit_socket_support() {
    use crate::process_client::loopback_tests::Peer;
    use serde_json::json;
    let mut peer = Peer::open().await;
    for (value, expected) in [
        (json!({}), false),
        (json!({"features":"duplex_socket"}), false),
        (json!({"features":["other"]}), false),
        (json!({"features":["duplex_socket"]}), true),
    ] {
        let c = peer.client.clone();
        let task = tokio::spawn(async move { c.supports_events().await });
        let (id, _) = peer.command().await;
        peer.reply(id, value).await;
        assert_eq!(task.await.unwrap(), expected);
    }
}
