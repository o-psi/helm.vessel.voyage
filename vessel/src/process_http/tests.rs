use super::*;
fn state(directory: Option<PathBuf>) -> AppState {
    AppState {
        process_directory: directory,
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        operator_token_hash: None,
        public_origin: Some("https://fixture.invalid".into()),
    }
}
#[test]
fn expected_host_pin_rejects_missing_identity_ambiguity() {
    let mut headers = HeaderMap::new();
    assert_eq!(expected_vessel(&headers).unwrap(), None);
    for value in ["invalid", "00000000-0000-0000-0000-000000000000"] {
        headers.insert("x-voyage-vessel", value.parse().unwrap());
        assert_eq!(
            expected_vessel(&headers).unwrap_err(),
            StatusCode::BAD_REQUEST
        );
    }
    let id = Uuid::new_v4();
    headers.insert("x-voyage-vessel", id.to_string().parse().unwrap());
    assert_eq!(expected_vessel(&headers).unwrap(), Some(id));
    headers.append(
        "x-voyage-vessel",
        Uuid::new_v4().to_string().parse().unwrap(),
    );
    assert!(expected_vessel(&headers).is_err());
}
#[tokio::test]
async fn command_gateway_refuses_malformed_auth_before_routing() {
    let request = || VesselRequest {
        protocol: 1,
        command: VesselCommand::Capabilities,
    };
    assert_eq!(
        command(State(state(None)), HeaderMap::new(), Json(request()))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let root = std::env::temp_dir().join(format!("absent-vessel-{}", Uuid::new_v4()));
    for case in 0..6 {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", "a".repeat(64)).parse().unwrap(),
        );
        headers.insert(
            "x-voyage-grant",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        let mut r = request();
        match case {
            0 => {
                headers.remove("authorization");
            }
            1 => {
                headers.append("authorization", "Bearer duplicate".parse().unwrap());
            }
            2 => {
                headers.insert("authorization", "Bearer short".parse().unwrap());
            }
            3 => {
                headers.insert("x-voyage-grant", "invalid".parse().unwrap());
            }
            4 => {
                headers.insert("x-voyage-vessel", "invalid".parse().unwrap());
            }
            _ => r.protocol = 99,
        };
        let response = command(State(state(Some(root.clone()))), headers, Json(r)).await;
        assert!(matches!(
            response.status(),
            StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
        ));
    }
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
}
#[tokio::test]
async fn event_gateway_validates_subscription_identity_and_auth_shape() {
    let root = std::env::temp_dir().join(format!("absent-vessel-{}", Uuid::new_v4()));
    let subscription = voyage_protocol::vessel::VesselEventSubscription {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        after: 0,
    };
    for subscriptions in [vec![], vec![subscription.clone(), subscription.clone()]] {
        let response = events(
            State(state(Some(root.clone()))),
            HeaderMap::new(),
            Json(VesselEventRequest {
                protocol: 1,
                subscriptions,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(
        pair_capabilities(State(state(None))).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn public_boundary_checks_origin_and_sets_noncacheable_security_headers() {
    let state = state(None);
    let app = Router::new()
        .route("/fixture", get(|| async { "fixture" }))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            super::boundary,
        ))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (origin, status) in [
        (None, 200),
        (Some("https://fixture.invalid"), 200),
        (Some("https://other.invalid"), 403),
    ] {
        let mut request = client.get(format!("{endpoint}/fixture"));
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        if status == 200 {
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(response.headers()["x-content-type-options"], "nosniff");
            assert_eq!(response.headers()["x-accel-buffering"], "no");
        }
    }
    server.abort();
    let _ = server.await;
}
#[tokio::test]
async fn valid_auth_transport_failure_remains_explicitly_unknown() {
    let root = std::env::temp_dir().join(format!("missing-gateway-{}", Uuid::new_v4()));
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        format!("Bearer {}", "a".repeat(64)).parse().unwrap(),
    );
    headers.insert(
        "x-voyage-grant",
        Uuid::new_v4().to_string().parse().unwrap(),
    );
    let response = command(
        State(state(Some(root.clone()))),
        headers,
        Json(VesselRequest {
            protocol: 1,
            command: VesselCommand::Capabilities,
        }),
    )
    .await;
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let result: VesselResponse = serde_json::from_slice(&body).unwrap();
    assert!(result.outcome_unknown);
    assert!(result.error.unwrap().contains("routing unavailable"));
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
}
