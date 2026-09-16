use super::*;
fn backend() -> SocketBackend {
    SocketBackend {
        directory: "/not-a-supervisor".into(),
        expected_vessel_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        token: "f".repeat(64),
        authority: serde_json::Value::Null,
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
