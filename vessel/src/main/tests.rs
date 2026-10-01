use super::*;
#[test]
fn operator_auth_and_bearer_are_exact_and_secret_safe() {
    let mut state = AppState {
        process_route: None,
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        browser_credentials: Default::default(),
        operator_token_hash: Some(token_hash("fixture-token")),
        public_origin: Some("https://fixture.invalid".into()),
    };
    let mut headers = HeaderMap::new();
    assert!(operator_auth(&state, &headers).is_err());
    headers.insert("authorization", "Bearer fixture-token".parse().unwrap());
    assert!(operator_auth(&state, &headers).is_ok());
    assert_eq!(bearer(&headers), Some("fixture-token"));
    headers.insert("authorization", "Bearer wrong".parse().unwrap());
    assert!(operator_auth(&state, &headers).is_err());
    state.operator_token_hash = None;
    assert!(operator_auth(&state, &headers).is_err());
    assert!(constant_time_eq(b"same", b"same"));
    assert!(!constant_time_eq(b"same", b"diff"));
    assert!(!constant_time_eq(b"a", b"aa"));
}
#[tokio::test]
async fn health_metrics_and_readiness_are_observations_not_execution() {
    let state = AppState {
        process_route: None,
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        browser_credentials: Default::default(),
        operator_token_hash: None,
        public_origin: None,
    };
    assert_eq!(health().await.0.status, "ok");
    let _ = metrics(State(state.clone())).await;
    assert!(readiness(State(state)).await.is_ok());
}

#[test]
fn approved_gateway_public_origin_remains_a_named_flag_with_required_route() {
    let cli = Cli::try_parse_from([
        "vessel",
        "--bind",
        "127.0.0.1:9480",
        "--process-directory",
        "/private/vessel",
        "--public-origin",
        "https://vessel.example.invalid",
    ])
    .unwrap();
    assert_eq!(
        cli.process_directory,
        Some(PathBuf::from("/private/vessel"))
    );
    assert_eq!(
        cli.public_origin.as_deref(),
        Some("https://vessel.example.invalid")
    );
    assert!(!cli.allow_insecure_loopback);
    assert_eq!(
        vessel::origin::validate_origin(
            cli.public_origin.as_deref().unwrap(),
            cli.allow_insecure_loopback
        )
        .unwrap(),
        "https://vessel.example.invalid"
    );
    assert!(Cli::try_parse_from(["vessel", "--process-directory", "/private/vessel"]).is_err());
}

#[test]
fn gateway_cli_keeps_explicit_literal_loopback_development_and_https_origin_validation() {
    for (origin, development, valid) in [
        ("https://vessel.example.invalid", false, true),
        ("http://127.0.0.1:9480", false, false),
        ("http://127.0.0.1:9480", true, true),
        ("http://localhost:9480", true, false),
        ("http://example.invalid", true, false),
        ("https://user:secret@example.invalid", false, false),
        ("https://example.invalid/path", false, false),
        ("https://example.invalid?token=private", false, false),
    ] {
        let mut args = vec![
            "vessel",
            "--process-directory",
            "/private/vessel",
            "--public-origin",
            origin,
        ];
        if development {
            args.push("--allow-insecure-loopback");
        }
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(
            vessel::origin::validate_origin(
                cli.public_origin.as_deref().unwrap(),
                cli.allow_insecure_loopback
            )
            .is_ok(),
            valid
        );
    }
}
