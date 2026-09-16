use super::*;
#[test]
fn operator_auth_and_bearer_are_exact_and_secret_safe() {
    let mut state = AppState {
        process_directory: None,
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
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
        process_directory: None,
        database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
        operator_token_hash: None,
        public_origin: None,
    };
    assert_eq!(health().await.0.status, "ok");
    let _ = metrics(State(state.clone())).await;
    assert!(readiness(State(state)).await.is_ok());
}
