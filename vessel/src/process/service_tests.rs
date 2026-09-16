use super::super::test_support::Fixture;
use super::*;

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

async fn state(f: &Fixture) -> HttpState {
    HttpState {
        supervisor: Arc::new(f.supervisor().await),
        token_hash: Sha256::digest(TOKEN.as_bytes()).into(),
        capacity: Arc::new(Semaphore::new(2)),
        event_capacity: Arc::new(Semaphore::new(1)),
        socket_capacity: Arc::new(Semaphore::new(1)),
    }
}

struct Server {
    endpoint: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(state: HttpState) -> Server {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route(voyage_protocol::vessel::COMMAND_PATH, post(local_command))
        .route(voyage_protocol::vessel::EVENTS_PATH, post(local_events))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_VESSEL_BODY))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            local_boundary,
        ))
        .with_state(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { endpoint, task }
}
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}
fn capabilities() -> VesselRequest {
    VesselRequest {
        protocol: VESSEL_API_VERSION,
        command: VesselCommand::Capabilities,
    }
}

#[tokio::test]
async fn authenticated_http_boundary_rejects_ambiguous_browser_and_invalid_credentials() {
    let f = Fixture::new();
    let s = state(&f).await;
    let server = server(s.clone()).await;
    let url = format!(
        "{}{}",
        server.endpoint,
        voyage_protocol::vessel::COMMAND_PATH
    );
    let c = client();
    assert_eq!(
        c.post(&url)
            .json(&capabilities())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    for token in [
        "",
        "Basic abc",
        "Bearer abc",
        "Bearer zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        "Bearer bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ] {
        assert_eq!(
            c.post(&url)
                .header("authorization", token)
                .json(&capabilities())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    headers.append("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    assert_eq!(
        c.post(&url)
            .headers(headers)
            .json(&capabilities())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        c.post(&url)
            .bearer_auth(TOKEN)
            .header("origin", "null")
            .json(&capabilities())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = c
        .post(&url)
        .bearer_auth(TOKEN)
        .json(&capabilities())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let response: VesselResponse = response.json().await.unwrap();
    assert!(response.error.is_none());
    assert_eq!(response.result["protocol"], VESSEL_API_VERSION);
    assert!(
        response.result["features"]
            .as_array()
            .unwrap()
            .contains(&json!("start_settings"))
    );
    assert_eq!(s.capacity.available_permits(), 2);
    let _permit = s.capacity.acquire_many(2).await.unwrap();
    assert_eq!(
        c.post(&url)
            .bearer_auth(TOKEN)
            .json(&capabilities())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn local_exchange_round_trips_identity_catalogue_errors_and_protocol_rejection() {
    let f = Fixture::new();
    let s = state(&f).await;
    let server = server(s).await;
    registry::save_local_access(
        &f.0,
        &LocalAccessCredential {
            endpoint: server.endpoint.clone(),
            token: TOKEN.into(),
        },
    )
    .unwrap();
    for command in [
        VesselCommand::Identity,
        VesselCommand::Capabilities,
        VesselCommand::Catalogue,
    ] {
        let response = super::super::exchange::exchange(
            &f.0,
            &VesselRequest {
                protocol: VESSEL_API_VERSION,
                command,
            },
        )
        .await
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert!(!response.outcome_unknown);
        assert_eq!(response.protocol, VESSEL_API_VERSION);
    }
    let response = super::super::exchange::exchange(
        &f.0,
        &VesselRequest {
            protocol: VESSEL_API_VERSION,
            command: VesselCommand::Inspect {
                session_id: Uuid::new_v4(),
            },
        },
    )
    .await
    .unwrap();
    assert!(response.error.is_some());
    assert!(!response.outcome_unknown);
    let mut request = capabilities();
    request.protocol += 1;
    assert!(
        super::super::exchange::exchange(&f.0, &request)
            .await
            .unwrap_err()
            .to_string()
            .contains("HTTP request rejected")
    );
}

#[tokio::test]
async fn event_subscription_validation_capacity_and_missing_owner_are_bounded() {
    let f = Fixture::new();
    let s = state(&f).await;
    let subscription = VesselEventSubscription {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        after: 17,
    };
    for (protocol, subscriptions) in [
        (VESSEL_API_VERSION + 1, vec![subscription.clone()]),
        (VESSEL_API_VERSION, vec![]),
        (
            VESSEL_API_VERSION,
            vec![subscription.clone(), subscription.clone()],
        ),
        (VESSEL_API_VERSION, vec![subscription.clone(); 257]),
    ] {
        let response = local_events(
            State(s.clone()),
            Json(VesselEventRequest {
                protocol,
                subscriptions,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(s.event_capacity.available_permits(), 1);
    }
    let permit = s.event_capacity.acquire().await.unwrap();
    assert_eq!(
        local_events(
            State(s.clone()),
            Json(VesselEventRequest {
                protocol: VESSEL_API_VERSION,
                subscriptions: vec![subscription.clone()]
            })
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(permit);
    let (returned, event, keep) = observe_local(s.supervisor.clone(), subscription.clone()).await;
    assert_eq!(returned.session_id, subscription.session_id);
    assert_eq!(returned.after, 17);
    assert!(event.error.is_some());
    assert!(!event.outcome_unknown);
    assert!(!keep);
    let response = local_events(
        State(s.clone()),
        Json(VesselEventRequest {
            protocol: VESSEL_API_VERSION,
            subscriptions: vec![subscription],
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = tokio::time::timeout(
        Duration::from_secs(3),
        axum::body::to_bytes(response.into_body(), 65536),
    )
    .await
    .unwrap()
    .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("event: update"));
    assert!(text.contains("\"error\":"));
    assert_eq!(s.event_capacity.available_permits(), 1);
}

#[tokio::test]
async fn socket_backend_rechecks_identity_and_rotated_local_authority() {
    use crate::duplex::Backend;
    let f = Fixture::new();
    let s = state(&f).await;
    let identity = super::super::identity::public(&f.0).unwrap();
    let backend = LocalSocketBackend {
        supervisor: s.supervisor,
        token_hash: s.token_hash,
        vessel_id: identity.vessel_id,
    };
    assert!(!backend.authorize(None).await);
    registry::save_local_access(
        &f.0,
        &LocalAccessCredential {
            endpoint: "http://127.0.0.1:1".into(),
            token: TOKEN.into(),
        },
    )
    .unwrap();
    assert!(backend.authorize(None).await);
    assert!(backend.authorize(Some(Uuid::new_v4())).await);
    let reply = backend.command(capabilities()).await;
    assert!(reply.error.is_none());
    registry::save_local_access(
        &f.0,
        &LocalAccessCredential {
            endpoint: "http://127.0.0.1:1".into(),
            token: "b".repeat(64),
        },
    )
    .unwrap();
    assert!(!backend.authorize(None).await);
    let wrong_owner = LocalSocketBackend {
        vessel_id: Uuid::new_v4(),
        ..backend
    };
    assert!(!wrong_owner.authorize(None).await);
}

// Exercise the HTTP adapter, owner selection and framed IPC together, without a
// Voyage executable or provider credentials. The peer handles exactly one call.
#[tokio::test]
async fn http_voyage_roundtrip_preserves_payload_failures_and_unknown_outcomes() {
    use crate::process::database;
    use voyage_protocol::process::{RuntimeRequest, RuntimeResponse, read_frame, write_frame};
    use voyage_protocol::vessel::{VoyageCommand, VoyageRequest};
    for fault in ["none", "refused", "unknown", "identity"] {
        let f = Fixture::new();
        let state = state(&f).await;
        let mut r = f.registration();
        r.state = ProcessState::Live;
        database::save(&f.0, &r).await.unwrap();
        let directory = registry::directory(&f.0, r.session_id);
        registry::private_directory(&directory).unwrap();
        let listener = tokio::net::UnixListener::bind(directory.join("runtime.sock")).unwrap();
        let registration = r.clone();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
            assert_eq!(request.session_id, registration.session_id);
            assert_eq!(request.incarnation, registration.incarnation);
            assert_eq!(request.token, registration.token);
            assert!(request.authorization.is_none());
            assert!(matches!(
                request.command,
                RuntimeCommand::History {
                    offset: 7,
                    limit: 11,
                    expected_revision: Some(19)
                }
            ));
            write_frame(
                &mut stream,
                &RuntimeResponse {
                    protocol: PROCESS_PROTOCOL,
                    session_id: registration.session_id,
                    incarnation: if fault == "identity" {
                        Uuid::new_v4()
                    } else {
                        registration.incarnation
                    },
                    result: json!({"rows":["synthetic"],"revision":19}),
                    error: matches!(fault, "refused" | "unknown").then(|| "fixture refusal".into()),
                    outcome_unknown: fault == "unknown",
                    resumed_from: None,
                },
            )
            .await
            .unwrap();
        });
        let server = server(state).await;
        let response = client()
            .post(format!(
                "{}{}",
                server.endpoint,
                voyage_protocol::vessel::COMMAND_PATH
            ))
            .bearer_auth(TOKEN)
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command: VesselCommand::Voyage(VoyageRequest {
                    session_id: r.session_id,
                    // Ordinary reads select the current owner, not this stale hint.
                    incarnation: Some(Uuid::new_v4()),
                    command: VoyageCommand::History {
                        offset: 7,
                        limit: 11,
                        expected_revision: Some(19),
                    },
                }),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: VesselResponse = response.json().await.unwrap();
        peer.await.unwrap();
        assert_eq!(body.protocol, VESSEL_API_VERSION);
        match fault {
            "none" => {
                assert!(body.error.is_none());
                assert!(!body.outcome_unknown);
                assert_eq!(body.result["session_id"], r.session_id.to_string());
                assert_eq!(body.result["incarnation"], r.incarnation.to_string());
                assert_eq!(body.result["result"]["rows"], json!(["synthetic"]));
            }
            "refused" => {
                assert_eq!(body.error.as_deref(), Some("fixture refusal"));
                assert_eq!(body.result["revision"], 19);
                assert!(!body.outcome_unknown);
            }
            _ => {
                assert!(body.error.is_some());
                assert_eq!(body.outcome_unknown, fault == "unknown");
                if fault == "identity" {
                    assert!(body.result.is_null());
                }
            }
        }
    }
}

#[tokio::test]
async fn local_event_observation_forwards_cursor_and_continues_only_known_success() {
    use crate::process::database;
    use voyage_protocol::process::{RuntimeRequest, RuntimeResponse, read_frame, write_frame};
    for unknown in [false, true] {
        let f = Fixture::new();
        let state = state(&f).await;
        let mut r = f.registration();
        r.state = ProcessState::Live;
        database::save(&f.0, &r).await.unwrap();
        let directory = registry::directory(&f.0, r.session_id);
        registry::private_directory(&directory).unwrap();
        let listener = tokio::net::UnixListener::bind(directory.join("runtime.sock")).unwrap();
        let registration = r.clone();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
            assert!(matches!(
                request.command,
                RuntimeCommand::Events {
                    after: 37,
                    limit: 128,
                    wait_ms: 10_000
                }
            ));
            write_frame(
                &mut stream,
                &RuntimeResponse {
                    protocol: PROCESS_PROTOCOL,
                    session_id: registration.session_id,
                    incarnation: registration.incarnation,
                    result: json!({"cursor":38,"events":[{"sequence":38}]}),
                    error: None,
                    outcome_unknown: unknown,
                    resumed_from: None,
                },
            )
            .await
            .unwrap();
        });
        let subscription = VesselEventSubscription {
            session_id: r.session_id,
            incarnation: r.incarnation,
            after: 37,
        };
        let (returned, event, keep) = observe_local(state.supervisor, subscription).await;
        peer.await.unwrap();
        assert_eq!(returned.after, 37);
        assert_eq!(event.session_id, r.session_id);
        assert_eq!(keep, !unknown);
        assert_eq!(event.outcome_unknown, unknown);
        if !unknown {
            assert_eq!(event.result["cursor"], 38);
        }
    }
}

mod account_routes {
    use super::*;
    use crate::process::accounts::Scope;
    use voyage_protocol::accounts::*;

    #[tokio::test]
    async fn account_admin_routes_reject_nonowner_defaults_before_host_registry_access() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let session = f.session();
        let connection = f.connection();
        f.save_session(&session);
        f.save_connection(&connection);
        let account = AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 1,
            connection_revision: 1,
            transport: Transport::OpenaiResponses,
        };
        for scope in [Scope::Session(session), Scope::Connection(connection)] {
            let error = s
                .host_accounts(
                    VesselCommand::AccountSetDefault {
                        command_id: Uuid::new_v4(),
                        workspace: f.0.clone(),
                        account: account.clone(),
                        expected_revision: 0,
                    },
                    scope,
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("only host owner"));
        }
        assert!(s.enrollment_workers.lock().await.is_empty());
    }

    #[tokio::test]
    async fn account_catalogue_and_usage_routes_fail_closed_on_stale_scope() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let grant = f.session();
        let mut stale = grant.clone();
        stale.revision += 1;
        f.save_session(&stale);
        let account = AccountBinding {
            account_id: grant.accounts[0],
            connection_id: grant.enrollment_connections[0],
            identity_generation: 1,
            connection_revision: 1,
            transport: Transport::OpenaiResponses,
        };
        for command in [
            VesselCommand::Accounts {
                workspace: f.0.clone(),
                transport: None,
            },
            VesselCommand::AccountDefaults {
                workspace: f.0.clone(),
            },
            VesselCommand::AccountModels {
                workspace: f.0.clone(),
                account: account.clone(),
            },
            VesselCommand::AccountUsage {
                workspace: f.0.clone(),
                account: account.clone(),
                refresh: false,
            },
            VesselCommand::AccountUsage {
                workspace: f.0.clone(),
                account,
                refresh: true,
            },
        ] {
            assert!(
                s.host_accounts(command, Scope::Session(grant.clone()))
                    .await
                    .is_err()
            );
        }
        assert!(s.enrollment_workers.lock().await.is_empty());
        assert!(s.devices.resume_candidates().unwrap().is_empty());
    }

    #[tokio::test]
    async fn enrollment_routes_deny_unapproved_connections_without_worker_or_receipt() {
        let f = Fixture::new();
        let s = f.supervisor().await;
        let session = f.session();
        let connection = f.connection();
        f.save_session(&session);
        f.save_connection(&connection);
        for scope in [Scope::Session(session), Scope::Connection(connection)] {
            for resolve in [false, true] {
                let command_id = Uuid::new_v4();
                let enrollment_id = Uuid::new_v4();
                let connection_id = Uuid::new_v4();
                let command = if resolve {
                    VesselCommand::ResolveAccountEnrollment {
                        command_id,
                        enrollment_id,
                        workspace: f.0.clone(),
                        connection_id,
                        alias: "synthetic".into(),
                        label: "Offline".into(),
                    }
                } else {
                    VesselCommand::EnrollAccount {
                        command_id,
                        enrollment_id,
                        workspace: f.0.clone(),
                        connection_id,
                        alias: "synthetic".into(),
                        label: "Offline".into(),
                    }
                };
                let error = s.host_accounts(command, scope.clone()).await.unwrap_err();
                assert!(error.to_string().contains("enrollment connection denied"));
            }
        }
        assert!(s.enrollment_workers.lock().await.is_empty());
        assert!(s.devices.resume_candidates().unwrap().is_empty());
    }
}
