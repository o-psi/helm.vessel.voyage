use super::*;
fn backend() -> SocketBackend {
    SocketBackend {
        directory: "/not-a-supervisor".into(),
        expected_vessel_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        token: "f".repeat(64),
        authority: serde_json::Value::Null,
        browser_sockets: Default::default(),
    }
}
#[test]
fn origins_are_canonical_https_and_unambiguous() {
    assert!(canonical_origin("https://web.example"));
    assert!(canonical_origin("https://web.example:8443"));
    for origin in [
        "http://localhost",
        "null",
        "https://web.example/",
        "https://WEB.example",
        "https://web.example:443",
        "https://user@web.example",
        "https://web.example?token=x",
    ] {
        assert!(!canonical_origin(origin), "{origin}");
    }
    let mut headers = HeaderMap::new();
    assert!(request_origin(&headers).is_none());
    headers.append("origin", "https://web.example".parse().unwrap());
    assert!(request_origin(&headers).is_some());
    headers.append("origin", "https://web.example".parse().unwrap());
    assert!(request_origin(&headers).is_none());
}
#[test]
fn credentials_are_random_single_use_origin_bound_bounded_and_expiring() {
    let store = Credentials::default();
    let (token, expiry) = store
        .insert("https://web.example".into(), backend())
        .unwrap();
    assert_eq!(token.len(), 64);
    assert!(expiry > chrono::Utc::now().timestamp_millis() as u64);
    assert!(store.take(&token, "https://other.example").is_none());
    assert!(store.take(&token, "https://web.example").is_some());
    assert!(store.take(&token, "https://web.example").is_none());
    let (expired, _) = store
        .insert("https://web.example".into(), backend())
        .unwrap();
    assert_ne!(expired, token);
    store.0.lock().unwrap().get_mut(&expired).unwrap().deadline = tokio::time::Instant::now();
    assert!(store.take(&expired, "https://web.example").is_none());
    for _ in 0..MAX_CREDENTIALS {
        store
            .insert("https://web.example".into(), backend())
            .unwrap();
    }
    assert_eq!(
        store
            .insert("https://web.example".into(), backend())
            .unwrap_err(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
#[test]
fn first_frame_is_only_exact_authentication() {
    for frame in [
        r#"{"type":"hello","token":"x"}"#,
        r#"{"type":"authenticate","token":"x","extra":true}"#,
        r#"{"type":"authenticate"}"#,
        r#"{"type":"command","request":{}}"#,
    ] {
        assert!(serde_json::from_str::<Authentication>(frame).is_err());
    }
    assert!(
        serde_json::from_str::<Authentication>(r#"{"type":"authenticate","token":"x"}"#).is_ok()
    );
}
#[test]
fn removed_shared_draft_command_is_rejected() {
    assert!(
        serde_json::from_value::<VesselCommand>(serde_json::json!({
            "op": "drafts",
            "operation": { "op": "list" },
        }))
        .is_err()
    );
}
#[test]
fn browser_allowlist_excludes_executor_and_native_management() {
    assert!(allowed(&VesselCommand::Capabilities));
    assert!(allowed(&VesselCommand::Catalogue));
    assert!(!allowed(&VesselCommand::Granted {
        expected_vessel_id: None,
        grant_id: Uuid::new_v4(),
        token: "x".into(),
        command: Box::new(VesselCommand::Capabilities)
    }));
    let command = |command| {
        VesselCommand::Voyage(VoyageRequest {
            session_id: Uuid::new_v4(),
            incarnation: None,
            command,
        })
    };
    assert!(allowed(&command(VoyageCommand::UploadImage {
        upload_id: Uuid::new_v4(),
        name: "image.png".into(),
        data_base64: "".into(),
    })));
    assert!(!allowed(&command(VoyageCommand::PrepareBrowser)));
    assert!(!allowed(&command(VoyageCommand::OperatorTool {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: 1,
        name: "shell".into(),
        arguments: serde_json::json!({})
    })));
    assert!(!allowed(&command(VoyageCommand::History {
        offset: 0,
        limit: 129,
        expected_revision: None
    })));
    assert!(allowed(&command(VoyageCommand::History {
        offset: 0,
        limit: 128,
        expected_revision: None
    })));
    assert!(!allowed(&command(VoyageCommand::Events {
        after: 0,
        limit: 1,
        wait_ms: 10001
    })));
}
#[tokio::test]
async fn expiry_refuses_before_supervisor_dispatch_and_publication() {
    let backend = BrowserBackend(Credential {
        origin: "https://web.example".into(),
        backend: backend(),
        deadline: tokio::time::Instant::now(),
    });
    assert!(!backend.authorize(None).await);
    let response = backend
        .command(VesselRequest {
            protocol: 1,
            command: VesselCommand::Catalogue,
        })
        .await;
    assert_eq!(response.error.as_deref(), Some("browser command refused"));
    assert!(!response.outcome_unknown);
    assert!(backend.deadline().is_some());
}
#[tokio::test]
async fn origin_exception_is_specific_to_browser_socket() {
    let state = super::super::tests::state(None);
    let app = axum::Router::new()
        .route(
            SOCKET_PATH,
            axum::routing::get(|| async { "accepted" }).layer(
                axum::middleware::from_fn_with_state(state.clone(), boundary),
            ),
        )
        .route(
            "/native",
            axum::routing::get(|| async { "accepted" }).layer(
                axum::middleware::from_fn_with_state(state, super::super::boundary),
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (path, origin, expected) in [
        (SOCKET_PATH, Some("https://web.example"), 200),
        (SOCKET_PATH, None, 403),
        (SOCKET_PATH, Some("null"), 403),
        (
            "/v1/vessel/browser-socket?token=x",
            Some("https://web.example"),
            403,
        ),
        ("/native", Some("https://web.example"), 403),
    ] {
        let mut request = client.get(format!("{endpoint}{path}"));
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        assert_eq!(request.send().await.unwrap().status().as_u16(), expected);
    }
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn mint_rejects_invalid_origin_missing_identity_and_unavailable_authority() {
    let state = super::super::tests::state(Some("/not-a-supervisor".into()));
    let origin = "https://web.example";
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", "a".repeat(64)).parse().unwrap(),
    );
    headers.insert(
        "x-voyage-grant",
        Uuid::new_v4().to_string().parse().unwrap(),
    );
    let response = mint(
        State(state.clone()),
        headers.clone(),
        Json(MintRequest {
            origin: origin.into(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    headers.insert(
        "x-voyage-vessel",
        Uuid::new_v4().to_string().parse().unwrap(),
    );
    let response = mint(
        State(state.clone()),
        headers.clone(),
        Json(MintRequest {
            origin: "http://web.example".into(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = mint(
        State(state.clone()),
        headers,
        Json(MintRequest {
            origin: origin.into(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(state.browser_credentials.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn websocket_does_not_emit_hello_or_dispatch_before_exact_authentication() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let state = super::super::tests::state(None);
    let app = axum::Router::new()
        .route(
            SOCKET_PATH,
            axum::routing::get(socket).layer(axum::middleware::from_fn_with_state(
                state.clone(),
                boundary,
            )),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // Raw browser-shaped upgrade avoids adding another websocket dependency.
    for frame in [
        None,
        Some(r#"{"type":"command","request":{}}"#),
        Some(r#"{"type":"authenticate","token":"invalid"}"#),
    ] {
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(format!("GET {SOCKET_PATH} HTTP/1.1\r\nHost: {address}\r\nOrigin: https://web.example\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: voyage.vessel.v1\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = Vec::new();
        while !response.ends_with(b"\r\n\r\n") {
            response.push(stream.read_u8().await.unwrap());
            assert!(response.len() < 8192);
        }
        assert!(
            String::from_utf8(response)
                .unwrap()
                .starts_with("HTTP/1.1 101")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.read_u8())
                .await
                .is_err(),
            "server must not send Hello before auth"
        );
        if let Some(frame) = frame {
            assert!(frame.len() < 126);
            let mut bytes = vec![0x81, 0x80 | frame.len() as u8, 0, 0, 0, 0];
            bytes.extend_from_slice(frame.as_bytes());
            stream.write_all(&bytes).await.unwrap();
        }
        let mut bytes = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(6), stream.read_to_end(&mut bytes))
            .await
            .expect("unauthenticated socket must close within deadline");
        assert!(
            bytes.is_empty(),
            "no Hello or command reply before authentication"
        );
    }
    server.abort();
    let _ = server.await;
}

#[test]
fn streaming_frames_are_allowed_but_reverse_browser_execution_is_not() {
    use voyage_protocol::duplex::{ClientFrame, ReverseReply};
    let backend = BrowserBackend(Credential {
        origin: "https://web.example".into(),
        backend: backend(),
        deadline: tokio::time::Instant::now() + TTL,
    });
    assert!(backend.accepts(&ClientFrame::Subscribe {
        request_id: Uuid::new_v4(),
        request: VesselEventRequest {
            protocol: 1,
            subscriptions: vec![]
        }
    }));
    assert!(backend.accepts(&ClientFrame::Unsubscribe {
        subscription_id: Uuid::new_v4()
    }));
    assert!(!backend.accepts(&ClientFrame::ReverseReply {
        request_id: Uuid::new_v4(),
        reply: ReverseReply::Accepted
    }));
}

// Offline transport proof, not a real-browser or real grant-database fixture.
// Only the supervisor response is simulated: the public HTTP handlers, private
// credential loader/exchange, first-frame auth and duplex loop are production.
#[cfg(target_os = "linux")]
mod transport {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use voyage_protocol::duplex::{ClientFrame, ServerFrame};

    const ORIGIN: &str = "https://web.example";
    const GRANT_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const LOCAL_TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[derive(Default)]
    struct Authority {
        revoked: bool,
        revision: u64,
        commands: Vec<VesselCommand>,
        browser_socket: Option<Uuid>,
        disconnected: Vec<(Uuid, Uuid, Uuid)>,
    }
    struct Fixture {
        root: PathBuf,
        state: AppState,
        address: std::net::SocketAddr,
        vessel: Uuid,
        grant: Uuid,
        authority: Arc<Mutex<Authority>>,
        tasks: Vec<tokio::task::JoinHandle<()>>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for task in &self.tasks {
                task.abort();
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    impl Fixture {
        async fn new() -> Self {
            let vessel = Uuid::new_v4();
            let grant = Uuid::new_v4();
            let authority = Arc::new(Mutex::new(Authority::default()));
            let observed = authority.clone();
            let supervisor = axum::Router::new().route(
                voyage_protocol::vessel::COMMAND_PATH,
                axum::routing::post(move |headers: HeaderMap, Json(request): Json<VesselRequest>| {
                    let observed = observed.clone();
                    async move {
                        assert_eq!(headers["authorization"], format!("Bearer {LOCAL_TOKEN}"));
                        assert_eq!(request.protocol, VESSEL_API_VERSION);
                        let command = match request.command {
                            VesselCommand::Socket { socket, command } => {
                                observed.lock().unwrap().browser_socket = Some(socket.socket_id);
                                *command
                            }
                            VesselCommand::HostBrowserDisconnected { session_id, incarnation, socket } => {
                                observed.lock().unwrap().disconnected.push((session_id, incarnation, socket.socket_id));
                                return Json(VesselResponse { protocol: VESSEL_API_VERSION, result: serde_json::Value::Null, error: None, outcome_unknown: false });
                            }
                            command => command,
                        };
                        let VesselCommand::Granted {
                            expected_vessel_id, grant_id, token, command,
                        } = command else { panic!("exchange must wrap the public command") };
                        assert_eq!(expected_vessel_id, Some(vessel));
                        assert_eq!(grant_id, grant);
                        assert_eq!(token, GRANT_TOKEN);
                        let mut authority = observed.lock().unwrap();
                        authority.commands.push((*command).clone());
                        let result = match *command {
                            VesselCommand::Capabilities => serde_json::json!({
                                "vessel_id": vessel, "scope": "workspace", "revision": authority.revision
                            }),
                            VesselCommand::Catalogue => serde_json::json!({"fixture_catalogue": true}),
                            _ => serde_json::json!({"fixture_native": true}),
                        };
                        Json(VesselResponse {
                            protocol: VESSEL_API_VERSION,
                            result: if authority.revoked { serde_json::Value::Null } else { result },
                            error: authority.revoked.then(|| "fixture grant revoked".into()),
                            outcome_unknown: false,
                        })
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let supervisor_task =
                tokio::spawn(async move { axum::serve(listener, supervisor).await.unwrap() });
            let root = std::env::temp_dir().join(format!("browser-transport-{}", Uuid::new_v4()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .unwrap();
            // registry is private to the library. Mirror save_local_access's
            // format and permissions rather than exporting a production helper.
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.join("process-http.json"))
                .unwrap();
            std::io::Write::write_all(
                &mut file,
                &serde_json::to_vec(&voyage_protocol::process::LocalAccessCredential {
                    endpoint,
                    token: LOCAL_TOKEN.into(),
                })
                .unwrap(),
            )
            .unwrap();
            drop(file);
            let state = super::super::super::tests::state(Some(root.clone()));
            let app = axum::Router::new()
                .route(
                    CREDENTIALS_PATH,
                    axum::routing::post(mint).layer(axum::middleware::from_fn_with_state(
                        state.clone(),
                        super::super::super::boundary,
                    )),
                )
                .route(
                    SOCKET_PATH,
                    axum::routing::get(socket).layer(axum::middleware::from_fn_with_state(
                        state.clone(),
                        boundary,
                    )),
                )
                .route(
                    voyage_protocol::duplex::SOCKET_PATH,
                    axum::routing::get(super::super::super::socket).layer(
                        axum::middleware::from_fn_with_state(
                            state.clone(),
                            super::super::super::boundary,
                        ),
                    ),
                )
                .with_state(state.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let public_task =
                tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            Self {
                root,
                state,
                address,
                vessel,
                grant,
                authority,
                tasks: vec![supervisor_task, public_task],
            }
        }
        async fn mint_response(&self) -> reqwest::Response {
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap()
                .post(format!("http://{}{CREDENTIALS_PATH}", self.address))
                .bearer_auth(GRANT_TOKEN)
                .header("x-voyage-vessel", self.vessel.to_string())
                .header("x-voyage-grant", self.grant.to_string())
                .json(&serde_json::json!({"origin": ORIGIN}))
                .send()
                .await
                .unwrap()
        }
        async fn mint(&self) -> String {
            let response = self.mint_response().await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["vessel_id"], self.vessel.to_string());
            assert!(
                body["expires_at_ms"].as_u64().unwrap()
                    > chrono::Utc::now().timestamp_millis() as u64
            );
            let token = body["token"].as_str().unwrap().to_owned();
            assert_eq!(token.len(), 64);
            assert_ne!(token, GRANT_TOKEN);
            assert_ne!(token, LOCAL_TOKEN);
            token
        }
        async fn browser(&self, origin: &str, token: &str) -> Wire {
            let (mut wire, status) =
                Wire::upgrade(self.address, SOCKET_PATH, &format!("Origin: {origin}\r\n")).await;
            assert_eq!(status, 101);
            wire.send(&serde_json::json!({"type": "authenticate", "token": token}))
                .await;
            wire
        }
        fn catalogue_count(&self) -> usize {
            self.authority
                .lock()
                .unwrap()
                .commands
                .iter()
                .filter(|c| matches!(c, VesselCommand::Catalogue))
                .count()
        }
    }

    struct Wire(tokio::net::TcpStream);
    impl Wire {
        async fn upgrade(address: std::net::SocketAddr, path: &str, headers: &str) -> (Self, u16) {
            tokio::time::timeout(Duration::from_secs(3), async {
                let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
                stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n{headers}Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: voyage.vessel.v1\r\n\r\n").as_bytes()).await.unwrap();
                let mut response = Vec::new();
                while !response.ends_with(b"\r\n\r\n") {
                    response.push(stream.read_u8().await.unwrap());
                    assert!(response.len() < 8192);
                }
                let status = String::from_utf8(response).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
                (Self(stream), status)
            }).await.expect("bounded HTTP upgrade")
        }
        async fn write_frame(&mut self, opcode: u8, payload: &[u8]) {
            assert!(payload.len() <= u16::MAX as usize);
            let mut frame = vec![0x80 | opcode];
            if payload.len() < 126 {
                frame.push(0x80 | payload.len() as u8);
            } else {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            }
            let mask = [0x12, 0x34, 0x56, 0x78];
            frame.extend_from_slice(&mask);
            frame.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(i, byte)| byte ^ mask[i % 4]),
            );
            tokio::time::timeout(Duration::from_secs(3), self.0.write_all(&frame))
                .await
                .unwrap()
                .unwrap();
        }
        async fn send(&mut self, value: &impl serde::Serialize) {
            self.write_frame(1, &serde_json::to_vec(value).unwrap())
                .await;
        }
        async fn next(&mut self) -> Option<ServerFrame> {
            tokio::time::timeout(Duration::from_secs(7), async {
                loop {
                    let first = match self.0.read_u8().await {
                        Ok(first) => first,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::UnexpectedEof
                                    | std::io::ErrorKind::ConnectionReset
                            ) =>
                        {
                            return None;
                        }
                        Err(error) => panic!("websocket read: {error}"),
                    };
                    assert_eq!(first & 0x80, 0x80, "fixture expects complete frames");
                    let second = self.0.read_u8().await.unwrap();
                    assert_eq!(second & 0x80, 0, "server frames are not masked");
                    let length = match second & 0x7f {
                        126 => self.0.read_u16().await.unwrap() as usize,
                        127 => usize::try_from(self.0.read_u64().await.unwrap()).unwrap(),
                        n => n as usize,
                    };
                    assert!(length <= voyage_protocol::duplex::MAX_FRAME_BYTES);
                    let mut payload = vec![0; length];
                    self.0.read_exact(&mut payload).await.unwrap();
                    match first & 0x0f {
                        1 => return Some(serde_json::from_slice(&payload).unwrap()),
                        8 => return None,
                        9 => self.write_frame(10, &payload).await,
                        10 => {}
                        opcode => panic!("unexpected opcode {opcode}"),
                    }
                }
            })
            .await
            .expect("bounded websocket response or closure")
        }
        async fn hello(&mut self, vessel: Uuid) {
            let Some(ServerFrame::Hello {
                protocol,
                vessel_id,
                socket_id,
            }) = self.next().await
            else {
                panic!("authenticated socket must emit Hello first")
            };
            assert_eq!(protocol, VESSEL_API_VERSION);
            assert_eq!(vessel_id, vessel);
            assert!(!socket_id.is_nil());
        }
        async fn command(&mut self, command: VesselCommand) -> Uuid {
            let request_id = Uuid::new_v4();
            self.send(&ClientFrame::Command {
                request_id,
                request: Box::new(VesselRequest {
                    protocol: VESSEL_API_VERSION,
                    command,
                }),
            })
            .await;
            request_id
        }
        async fn reply(&mut self, expected: Uuid) -> serde_json::Value {
            let Some(ServerFrame::Reply {
                request_id,
                response,
            }) = self.next().await
            else {
                panic!("expected correlated reply")
            };
            assert_eq!(request_id, expected);
            assert_eq!(response.protocol, VESSEL_API_VERSION);
            assert!(response.error.is_none());
            assert!(!response.outcome_unknown);
            response.result
        }
    }

    #[tokio::test]
    async fn browser_socket_provenance_and_disconnect_cross_private_exchange() {
        let fixture = Fixture::new().await;
        let token = fixture.mint().await;
        let mut wire = fixture.browser(ORIGIN, &token).await;
        wire.hello(fixture.vessel).await;
        let session_id = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let id = wire
            .command(VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation: Some(incarnation),
                command: VoyageCommand::HostBrowser {
                    operation: voyage_protocol::host_browser::HostBrowserOperation::Status {},
                },
            }))
            .await;
        wire.reply(id).await;
        let socket = fixture
            .authority
            .lock()
            .unwrap()
            .browser_socket
            .expect("trusted socket envelope");
        assert!(!socket.is_nil());
        // Revocation must not block trusted cleanup delivery.
        fixture.authority.lock().unwrap().revoked = true;
        drop(wire);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if fixture.authority.lock().unwrap().disconnected.contains(&(
                    session_id,
                    incarnation,
                    socket,
                )) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disconnect notification reaches private supervisor despite revocation");
    }

    #[tokio::test]
    async fn mint_first_frame_hello_and_permitted_commands_use_real_exchange() {
        let fixture = Fixture::new().await;
        let token = fixture.mint().await;
        let mut wire = fixture.browser(ORIGIN, &token).await;
        wire.hello(fixture.vessel).await;
        assert!(
            !fixture
                .state
                .browser_credentials
                .0
                .lock()
                .unwrap()
                .contains_key(&token)
        );
        let id = wire.command(VesselCommand::Capabilities).await;
        assert_eq!(
            wire.reply(id).await["vessel_id"],
            fixture.vessel.to_string()
        );
        let id = wire.command(VesselCommand::Catalogue).await;
        assert_eq!(
            wire.reply(id).await,
            serde_json::json!({"fixture_catalogue": true})
        );
        assert_eq!(fixture.catalogue_count(), 1);
        let mut replay = fixture.browser(ORIGIN, &token).await;
        assert!(
            replay.next().await.is_none(),
            "consumed token must not emit Hello"
        );
        // A rejected replay must not disturb the original authorized connection.
        let id = wire.command(VesselCommand::Catalogue).await;
        wire.reply(id).await;
        assert_eq!(fixture.catalogue_count(), 2);
    }

    #[tokio::test]
    async fn wrong_origin_socket_does_not_consume_minted_credential() {
        let fixture = Fixture::new().await;
        let token = fixture.mint().await;
        let before = fixture.authority.lock().unwrap().commands.len();
        let mut wrong = fixture.browser("https://other.example", &token).await;
        assert!(wrong.next().await.is_none());
        assert!(
            fixture
                .state
                .browser_credentials
                .0
                .lock()
                .unwrap()
                .contains_key(&token)
        );
        assert_eq!(fixture.authority.lock().unwrap().commands.len(), before);
        let mut correct = fixture.browser(ORIGIN, &token).await;
        correct.hello(fixture.vessel).await;
        let id = correct.command(VesselCommand::Catalogue).await;
        correct.reply(id).await;
        assert_eq!(fixture.catalogue_count(), 1);
    }

    #[tokio::test]
    async fn revoked_or_changed_authority_refuses_first_frame_and_live_commands() {
        for revoked in [true, false] {
            let fixture = Fixture::new().await;
            let live_token = fixture.mint().await;
            let pending_token = fixture.mint().await;
            let mut live = fixture.browser(ORIGIN, &live_token).await;
            live.hello(fixture.vessel).await;
            let id = live.command(VesselCommand::Catalogue).await;
            live.reply(id).await;
            {
                let mut authority = fixture.authority.lock().unwrap();
                if revoked {
                    authority.revoked = true;
                } else {
                    authority.revision += 1;
                }
            }
            let mut pending = fixture.browser(ORIGIN, &pending_token).await;
            assert!(
                pending.next().await.is_none(),
                "stale mint cannot emit Hello"
            );
            live.command(VesselCommand::Catalogue).await;
            assert!(
                live.next().await.is_none(),
                "stale socket cannot publish a reply"
            );
            assert_eq!(
                fixture.catalogue_count(),
                1,
                "stale command must not reach supervisor"
            );
            if revoked {
                assert_eq!(
                    fixture.mint_response().await.status(),
                    StatusCode::UNAUTHORIZED
                );
            } else {
                // New mint captures new authority; stale credentials stay refused.
                let token = fixture.mint().await;
                fixture
                    .browser(ORIGIN, &token)
                    .await
                    .hello(fixture.vessel)
                    .await;
            }
        }
    }

    #[tokio::test]
    async fn minted_short_deadline_closes_live_socket_without_client_traffic() {
        let fixture = Fixture::new().await;
        let token = fixture.mint().await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        fixture
            .state
            .browser_credentials
            .0
            .lock()
            .unwrap()
            .get_mut(&token)
            .unwrap()
            .deadline = deadline;
        let mut wire = fixture.browser(ORIGIN, &token).await;
        wire.hello(fixture.vessel).await;
        let id = wire.command(VesselCommand::Catalogue).await;
        wire.reply(id).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "positive command must precede expiry"
        );
        tokio::time::timeout_at(deadline + Duration::from_secs(1), async {
            assert!(
                wire.next().await.is_none(),
                "expiry must close rather than publish"
            );
        })
        .await
        .expect("hard deadline, not the five-second authority heartbeat");
        assert_eq!(fixture.catalogue_count(), 1);
        assert!(!fixture.authority.lock().unwrap().revoked);
    }

    #[tokio::test]
    async fn account_filters_and_private_enrollment_survive_browser_socket() {
        let fixture = Fixture::new().await;
        let token = fixture.mint().await;
        let mut wire = fixture.browser(ORIGIN, &token).await;
        wire.hello(fixture.vessel).await;
        let enrollment_id = Uuid::new_v4();
        let commands = vec![
            VesselCommand::Accounts {
                workspace: "/fixture".into(),
                transport: Some(voyage_protocol::accounts::Transport::ChatgptOauth),
            },
            VesselCommand::EnrollAccount {
                command_id: Uuid::new_v4(),
                enrollment_id,
                workspace: "/fixture".into(),
                connection_id: Uuid::new_v4(),
                alias: "fixture".into(),
                label: "Fixture".into(),
            },
            VesselCommand::ResolveAccountEnrollment {
                command_id: Uuid::new_v4(),
                enrollment_id,
                workspace: "/fixture".into(),
                connection_id: Uuid::new_v4(),
                alias: "fixture".into(),
                label: "Fixture".into(),
            },
            VesselCommand::PrivateAccountEnrollment {
                enrollment_id,
                workspace: "/fixture".into(),
            },
            VesselCommand::CancelAccountEnrollment {
                command_id: Uuid::new_v4(),
                enrollment_id,
                workspace: "/fixture".into(),
            },
        ];
        for command in commands {
            let expected = serde_json::to_value(&command).unwrap();
            let id = wire.command(command).await;
            wire.reply(id).await;
            assert!(
                fixture
                    .authority
                    .lock()
                    .unwrap()
                    .commands
                    .iter()
                    .any(|c| serde_json::to_value(c).unwrap() == expected),
                "command must reach scoped supervisor unchanged"
            );
        }
        let id = wire.command(VesselCommand::Catalogue).await;
        wire.reply(id).await;
        assert_eq!(fixture.catalogue_count(), 1);
    }

    #[tokio::test]
    async fn native_header_auth_still_works_without_browser_first_frame() {
        let fixture = Fixture::new().await;
        let headers = format!(
            "Authorization: Bearer {GRANT_TOKEN}\r\nX-Voyage-Grant: {}\r\nX-Voyage-Vessel: {}\r\n",
            fixture.grant, fixture.vessel
        );
        let (mut native, status) = Wire::upgrade(
            fixture.address,
            voyage_protocol::duplex::SOCKET_PATH,
            &headers,
        )
        .await;
        assert_eq!(status, 101);
        native.hello(fixture.vessel).await;
        // Deliberately excluded by the browser allowlist but legal on native.
        let command = VesselCommand::Voyage(VoyageRequest {
            session_id: Uuid::new_v4(),
            incarnation: None,
            command: VoyageCommand::PrepareBrowser,
        });
        let id = native.command(command.clone()).await;
        assert_eq!(
            native.reply(id).await,
            serde_json::json!({"fixture_native": true})
        );
        let token = fixture.mint().await;
        let mut browser = fixture.browser(ORIGIN, &token).await;
        browser.hello(fixture.vessel).await;
        browser.command(command).await;
        assert!(browser.next().await.is_none());
        assert_eq!(
            fixture
                .authority
                .lock()
                .unwrap()
                .commands
                .iter()
                .filter(|c| matches!(c, VesselCommand::Voyage(_)))
                .count(),
            1
        );
        let (_, status) =
            Wire::upgrade(fixture.address, voyage_protocol::duplex::SOCKET_PATH, "").await;
        assert_eq!(
            status, 400,
            "browser credential support must not relax native headers"
        );
        let (_, status) = Wire::upgrade(
            fixture.address,
            voyage_protocol::duplex::SOCKET_PATH,
            &format!("{headers}Origin: {ORIGIN}\r\n"),
        )
        .await;
        assert_eq!(status, 403, "native origin boundary remains unchanged");
        let (_, status) = Wire::upgrade(
            fixture.address,
            SOCKET_PATH,
            &format!("{headers}Origin: {ORIGIN}\r\n"),
        )
        .await;
        assert_eq!(status, 400, "native headers are not browser authentication");
    }
}

#[test]
fn sidebar_allowlist_admits_only_public_lifecycle_operations() {
    let id = Uuid::new_v4();
    for command in [
        VoyageCommand::Rename {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            name: "renamed".into(),
        },
        VoyageCommand::Archive {
            command_id: id,
            expected_revision: 7,
            expires_at_ms: 99,
            archived: true,
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
        assert!(allowed(&VesselCommand::Voyage(VoyageRequest {
            session_id: id,
            incarnation: Some(id),
            command
        })));
    }
    assert!(allowed(&VesselCommand::Branch {
        command_id: id,
        session_id: id,
        incarnation: id,
        expected_revision: 7,
        expires_at_ms: 99,
        branch_id: Uuid::new_v4(),
        name: None,
        through_message: None,
    }));
    assert!(allowed(&VesselCommand::Restart {
        command_id: id,
        session_id: id,
        incarnation: id
    }));
    assert!(!allowed(&VesselCommand::Stop {
        session_id: id,
        incarnation: id
    }));
    // Runtime resolution can reserve an ID. It is deliberately not a read-only
    // receipt and must not be introduced accidentally with sidebar mutations.
    assert!(!allowed(&VesselCommand::Voyage(VoyageRequest {
        session_id: id,
        incarnation: Some(id),
        command: VoyageCommand::Resolve {
            command_id: id,
            original: None
        },
    })));
}

#[tokio::test]
async fn host_browser_requires_registered_live_socket_and_public_envelopes_are_refused() {
    let backend = backend();
    let socket_id = Uuid::new_v4();
    let request = || VesselRequest {
        protocol: VESSEL_API_VERSION,
        command: VesselCommand::Voyage(VoyageRequest {
            session_id: Uuid::new_v4(),
            incarnation: Some(Uuid::new_v4()),
            command: VoyageCommand::HostBrowser {
                operation: voyage_protocol::host_browser::HostBrowserOperation::Status {},
            },
        }),
    };
    assert!(allowed(&request().command));
    assert!(
        backend
            .socket_command(request(), socket_id)
            .await
            .error
            .is_some()
    );
    let state = std::sync::Arc::new(BrowserSocketState::default());
    state
        .closed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    backend
        .browser_sockets
        .lock()
        .unwrap()
        .insert(socket_id, state);
    assert!(
        backend
            .socket_command(request(), socket_id)
            .await
            .error
            .is_some()
    );
    backend.disconnected(socket_id);
    assert!(
        !backend
            .browser_sockets
            .lock()
            .unwrap()
            .contains_key(&socket_id)
    );
    let private = VesselCommand::HostBrowserDisconnected {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        socket: voyage_protocol::host_browser::HostBrowserSocket { socket_id },
    };
    assert!(private_envelope(&private));
    assert!(!allowed(&private));
    assert!(backend.exchange(private).await.error.is_some());
    let private = VesselCommand::Socket {
        socket: voyage_protocol::host_browser::HostBrowserSocket { socket_id },
        command: Box::new(VesselCommand::Capabilities),
    };
    assert!(private_envelope(&private));
    assert!(!allowed(&private));
    assert!(backend.exchange(private).await.error.is_some());
}
