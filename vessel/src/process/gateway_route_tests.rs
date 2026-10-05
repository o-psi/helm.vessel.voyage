use super::super::{access::store, database, test_support::Fixture};
use super::*;
use voyage_protocol::host_browser::HostBrowserOperation;

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn gateway_browser_wire_shape_roundtrips() {
    let command = VesselCommand::Voyage(VoyageRequest {
        session_id: Uuid::new_v4(),
        incarnation: Some(Uuid::new_v4()),
        command: VoyageCommand::HostBrowser {
            operation: HostBrowserOperation::Status {},
        },
    });
    let encoded = serde_json::to_string(&command).unwrap();
    assert!(serde_json::from_str::<VesselCommand>(&encoded).is_ok());
}

async fn call(supervisor: Arc<Supervisor>, request: gateway_ipc::GatewayRequest) -> VesselResponse {
    let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(serve_gateway_connection(
        server,
        supervisor,
        "https://fixture.invalid".into(),
    ));
    gateway_ipc::write_frame(&mut client, &request)
        .await
        .unwrap();
    let reply = gateway_ipc::read_frame(&mut client).await.unwrap().unwrap();
    task.await.unwrap().unwrap();
    reply
}

#[tokio::test]
async fn system_pipe_constructs_only_granted_commands_and_keeps_pairing_separate() {
    let fixture = Fixture::new();
    let supervisor = Arc::new(fixture.supervisor().await);
    let mut grant = fixture.session();
    grant.token_hash = store::hash(TOKEN);
    fixture.save_session(&grant);
    let auth = gateway_ipc::GrantAuth {
        expected_authority_fingerprint: None,
        expected_vessel_id: None,
        grant_id: grant.grant_id,
        token: TOKEN.into(),
    };
    let capabilities = call(
        supervisor.clone(),
        gateway_ipc::GatewayRequest::Command {
            auth: auth.clone(),
            command: VesselCommand::Capabilities,
        },
    )
    .await;
    assert!(capabilities.error.is_none(), "{:?}", capabilities.error);
    assert_eq!(capabilities.result["scope"], "session");
    assert_eq!(
        capabilities.result["session_id"],
        grant.session_id.to_string()
    );
    let private = call(
        supervisor.clone(),
        gateway_ipc::GatewayRequest::Command {
            auth: auth.clone(),
            command: VesselCommand::Granted {
                expected_authority_fingerprint: None,
                expected_vessel_id: None,
                grant_id: grant.grant_id,
                token: TOKEN.into(),
                command: Box::new(VesselCommand::Capabilities),
            },
        },
    )
    .await;
    assert_eq!(private.error.as_deref(), Some("private envelope refused"));
    assert!(!private.outcome_unknown);
    let denied = call(
        supervisor.clone(),
        gateway_ipc::GatewayRequest::Command {
            auth: gateway_ipc::GrantAuth {
                token: "b".repeat(64),
                ..auth.clone()
            },
            command: VesselCommand::Capabilities,
        },
    )
    .await;
    assert!(denied.error.is_some());
    assert_eq!(denied.result, Value::Null);
    let stale = call(
        supervisor.clone(),
        gateway_ipc::GatewayRequest::Command {
            auth: gateway_ipc::GrantAuth {
                expected_vessel_id: Some(Uuid::new_v4()),
                ..auth.clone()
            },
            command: VesselCommand::Capabilities,
        },
    )
    .await;
    assert!(stale.error.is_some());
    assert!(!stale.outcome_unknown);
    assert_eq!(stale.result, Value::Null);
    let discovery = call(supervisor, gateway_ipc::GatewayRequest::PairPreflight {}).await;
    assert!(discovery.error.is_none());
    assert_eq!(discovery.result["protocol"], 1);
    assert!(discovery.result["vessel_id"].is_string());
}

#[tokio::test]
async fn browser_pipe_requires_grant_and_records_exact_cleanup_before_dispatch() {
    let fixture = Fixture::new();
    let supervisor = Arc::new(fixture.supervisor().await);
    let registration = fixture.registration();
    database::admit(
        &fixture.0,
        &registration,
        serde_json::to_vec(&VesselCommand::Start {
            command_id: registration.command_id,
            session_id: registration.session_id,
            workspace: registration.workspace.clone(),
        })
        .unwrap(),
    )
    .await
    .unwrap();
    let mut grant = fixture.session();
    grant.session_id = registration.session_id;
    grant.token_hash = store::hash(TOKEN);
    fixture.save_session(&grant);
    let auth = gateway_ipc::GrantAuth {
        expected_authority_fingerprint: None,
        expected_vessel_id: None,
        grant_id: grant.grant_id,
        token: TOKEN.into(),
    };
    let invalid = call(
        supervisor.clone(),
        gateway_ipc::GatewayRequest::SocketOpen {
            auth: gateway_ipc::GrantAuth {
                token: "b".repeat(64),
                ..auth.clone()
            },
        },
    )
    .await;
    assert!(invalid.error.is_some());
    let (mut client, server) = tokio::net::UnixStream::pair().unwrap();
    let task = tokio::spawn(serve_gateway_connection(
        server,
        supervisor,
        "https://fixture.invalid".into(),
    ));
    gateway_ipc::write_frame(
        &mut client,
        &gateway_ipc::GatewayRequest::SocketOpen { auth: auth.clone() },
    )
    .await
    .unwrap();
    let opened: VesselResponse = gateway_ipc::read_frame(&mut client).await.unwrap().unwrap();
    assert!(opened.error.is_none());
    let root_socket = Uuid::parse_str(opened.result["socket_id"].as_str().unwrap()).unwrap();
    assert!(!root_socket.is_nil());
    gateway_ipc::write_frame(
        &mut client,
        &gateway_ipc::GatewayRequest::SocketCommand {
            auth: auth.clone(),
            command: VesselCommand::Capabilities,
        },
    )
    .await
    .unwrap();
    let refused: VesselResponse = gateway_ipc::read_frame(&mut client).await.unwrap().unwrap();
    assert_eq!(
        refused.error.as_deref(),
        Some("browser socket boundary refused")
    );
    gateway_ipc::write_frame(
        &mut client,
        &gateway_ipc::GatewayRequest::SocketCommand {
            auth: gateway_ipc::GrantAuth {
                token: "b".repeat(64),
                ..auth.clone()
            },
            command: VesselCommand::Voyage(VoyageRequest {
                session_id: registration.session_id,
                incarnation: Some(registration.incarnation),
                command: VoyageCommand::HostBrowser {
                    operation: HostBrowserOperation::Status {},
                },
            }),
        },
    )
    .await
    .unwrap();
    let changed: VesselResponse = gateway_ipc::read_frame(&mut client).await.unwrap().unwrap();
    assert_eq!(
        changed.error.as_deref(),
        Some("browser socket grant changed")
    );
    assert!(
        gateway_ipc::read_frame::<VesselResponse>(&mut client)
            .await
            .unwrap()
            .is_none()
    );
    let owner: LocalBrowserState = Arc::default();
    let response = gateway_command(
        &Arc::new(fixture.supervisor().await),
        auth,
        VesselCommand::Voyage(VoyageRequest {
            session_id: registration.session_id,
            incarnation: Some(registration.incarnation),
            command: VoyageCommand::HostBrowser {
                operation: HostBrowserOperation::Status {},
            },
        }),
        Some((
            voyage_protocol::host_browser::HostBrowserSocket {
                socket_id: root_socket,
            },
            owner.clone(),
        )),
    )
    .await;
    // The fixture has no live runtime, but the authenticated browser target is
    // registered before routing so a lost reply cannot lose cleanup provenance.
    assert!(response.error.is_some());
    assert!(
        owner
            .lock()
            .unwrap()
            .sessions
            .contains(&(registration.session_id, registration.incarnation))
    );
    task.await.unwrap().unwrap();
}
