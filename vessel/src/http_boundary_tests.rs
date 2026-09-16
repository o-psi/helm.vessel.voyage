use super::*;
#[tokio::test]
async fn body_boundary_enforces_route_specific_limits_and_security_headers() {
    let app = axum::Router::new()
        .route(PAIR_PATH, axum::routing::post(|| async { "accepted" }))
        .layer(axum::middleware::from_fn(boundary));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for (size, status) in [(0, 200), (4096, 200), (4097, 413)] {
        let response = client
            .post(format!("{endpoint}{PAIR_PATH}"))
            .body(vec![b'x'; size])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }
    let response = client
        .post(format!("{endpoint}/notfound"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    task.abort();
    let _ = task.await;
}
#[test]
fn request_limits_do_not_narrow_command_or_event_payloads() {
    assert_eq!(body_limit(PAIR_PATH), 4096);
    for path in [COMMAND_PATH, EVENTS_PATH, "/other"] {
        assert_eq!(body_limit(path), MAX_VESSEL_BODY);
    }
}
