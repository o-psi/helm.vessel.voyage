use super::*;
use axum::body::to_bytes;
use serde_json::{Value, json};
use tower::ServiceExt;
use voyage_protocol::enrollment::SigningKey;
const TOKEN: &str = "fixture-owner-token-with-more-than-32-bytes";
const ORIGIN: &str = "https://vessel.example";
fn setup() -> (tempfile::TempDir, EnrollmentApi) {
    let d = tempfile::tempdir().unwrap();
    let store = EnrollmentStore::open(&d.path().join("enrollment"), ORIGIN, false).unwrap();
    (d, EnrollmentApi::new(store, TOKEN).unwrap())
}
fn request(path: &str, body: Value, operator: bool) -> Request<Body> {
    let mut r = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-voyage-request", "2");
    if operator {
        r = r.header("authorization", format!("Bearer {TOKEN}"));
    }
    r.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}
async fn call(router: &Router, path: &str, body: Value, operator: bool) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(request(path, body, operator))
        .await
        .unwrap();
    assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), MAX_PROOF_BYTES)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn actual_http_extractors_crypto_and_database_enroll_recover_rotate_revoke() {
    let (_d, api) = setup();
    let router = api.clone().router();
    let key = SigningKey::generate().unwrap();
    let (status, invitation) = call(
        &router,
        "/v2/enrollment/invitations",
        json!({"ttl_ms":60_000}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let operation = ProofOperation::Enroll {
        transaction_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        invitation_id: Uuid::parse_str(invitation["id"].as_str().unwrap()).unwrap(),
        public_key: key.public_key(),
    };
    let (status, c) = call(
        &router,
        "/v2/enrollment/challenge",
        json!({"operation":operation,"invitation_key":invitation["key"]}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let c: Challenge = serde_json::from_value(c).unwrap();
    let proof = SignedChallenge {
        signature: key.sign(&c).unwrap(),
        challenge: c,
        new_signature: None,
    };
    let request = json!({"proof":proof,"invitation_key":invitation["key"]});
    let (status, receipt) = call(&router, "/v2/enrollment/complete", request.clone(), false).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(&router, "/v2/enrollment/complete", request, false)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let machine_id = Uuid::parse_str(receipt["machine_id"].as_str().unwrap()).unwrap();
    // Recovery after losing the enrollment receipt needs no invitation secret,
    // but still needs possession of the original private key and a fresh proof.
    let (_, c) = call(
        &router,
        "/v2/enrollment/challenge",
        json!({"operation":operation}),
        false,
    )
    .await;
    let c: Challenge = serde_json::from_value(c).unwrap();
    let proof = SignedChallenge {
        signature: key.sign(&c).unwrap(),
        challenge: c,
        new_signature: None,
    };
    let (status, recovered) = call(
        &router,
        "/v2/enrollment/complete",
        json!({"proof":proof}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt, recovered);
    let new = SigningKey::generate().unwrap();
    let op = ProofOperation::Rotate {
        machine_id,
        epoch: 1,
        transaction_id: Uuid::new_v4(),
        new_public_key: new.public_key(),
    };
    let (_, c) = call(
        &router,
        "/v2/enrollment/challenge",
        json!({"operation":op}),
        false,
    )
    .await;
    let c: Challenge = serde_json::from_value(c).unwrap();
    let proof = SignedChallenge {
        signature: key.sign(&c).unwrap(),
        new_signature: Some(new.sign(&c).unwrap()),
        challenge: c,
    };
    let (status, r) = call(
        &router,
        "/v2/enrollment/complete",
        json!({"proof":proof}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(r["epoch"], 2);
    let revoke =
        json!({"machine_id":machine_id,"expected_epoch":2,"transaction_id":Uuid::new_v4()});
    assert_eq!(
        call(&router, "/v2/enrollment/revoke", revoke.clone(), false)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&router, "/v2/enrollment/revoke", revoke, true).await.0,
        StatusCode::OK
    );
    assert!(api.store.lock().unwrap().current(machine_id, 2).is_err());
    assert_eq!(
        call(
            &router,
            "/v2/enrollment/challenge",
            json!({"operation":ProofOperation::Connect{machine_id,epoch:2}}),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}
#[tokio::test]
async fn operator_auth_csrf_origin_queries_and_media_type_fail_closed() {
    let (_d, api) = setup();
    let router = api.router();
    for mode in 0..8 {
        let mut r = request("/v2/enrollment/invitations", json!({"ttl_ms":1000}), true);
        match mode {
            0 => {
                r.headers_mut().remove("authorization");
            }
            1 => {
                r.headers_mut()
                    .insert("authorization", HeaderValue::from_static("Bearer wrong"));
            }
            2 => {
                r.headers_mut().remove("x-voyage-request");
            }
            3 => {
                r.headers_mut()
                    .insert("origin", HeaderValue::from_static("https://evil.example"));
            }
            4 => {
                r.headers_mut()
                    .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
            }
            5 => {
                r.headers_mut()
                    .insert("content-type", HeaderValue::from_static("text/plain"));
            }
            6 => {
                *r.uri_mut() = "/v2/enrollment/invitations?secret=sentinel"
                    .parse()
                    .unwrap();
            }
            _ => {
                r.headers_mut()
                    .append("authorization", HeaderValue::from_static("Bearer wrong"));
            }
        }
        let response = router.clone().oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let mut r = request("/v2/enrollment/invitations", json!({"ttl_ms":1000}), false);
    r.headers_mut().insert(
        "authorization",
        HeaderValue::from_str(&format!(
            "Basic {}",
            STANDARD.encode(format!("owner:{TOKEN}"))
        ))
        .unwrap(),
    );
    r.headers_mut()
        .insert("origin", HeaderValue::from_static(ORIGIN));
    assert_eq!(router.oneshot(r).await.unwrap().status(), StatusCode::OK);
}
#[tokio::test]
async fn malformed_oversized_and_duplicate_input_never_echoes_content() {
    let (_d, api) = setup();
    let router = api.router();
    for content in [
        "{\"ttl_ms\":1,\"ttl_ms\":2}".to_owned(),
        "{\"ttl_ms\":1,\"private-sentinel\":1}".into(),
        "{\"private-sentinel\"".into(),
    ] {
        let mut r = request("/v2/enrollment/invitations", json!({}), true);
        *r.body_mut() = Body::from(content);
        let response = router.clone().oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("sentinel"));
    }
    let mut r = request("/v2/enrollment/invitations", json!({}), true);
    *r.body_mut() = Body::from(vec![b'x'; MAX_PROOF_BYTES + 1]);
    assert_eq!(
        router.oneshot(r).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}
#[tokio::test]
async fn admission_slots_are_bounded_and_weak_operator_tokens_are_rejected() {
    let (_d, api) = setup();
    let limiter = api.operator_requests.clone();
    let _held = limiter.acquire_many(4).await.unwrap();
    assert_eq!(
        call(
            &api.router(),
            "/v2/enrollment/invitations",
            json!({"ttl_ms":1000}),
            true
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let d = tempfile::tempdir().unwrap();
    let store = EnrollmentStore::open(&d.path().join("authority"), ORIGIN, false).unwrap();
    assert!(EnrollmentApi::new(store, "short").is_err());
}
