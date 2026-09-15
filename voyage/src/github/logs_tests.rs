use super::*;
use crate::github::http_fixture::{Fixture, Reply};
use serde_json::{Value, json};
fn object() -> Object {
    Object::parse("https://github.com/example/project/pull/7").unwrap()
}
fn detail() -> Value {
    json!({"number":7,"html_url":object().url(),"head":{"sha":"a".repeat(40)},"base":{"sha":"b".repeat(40),"repo":{"id":42},"ref":"main"}})
}
fn snapshot() -> Reply {
    Reply::json("/repos/example/project/pulls/7", detail())
}
fn job() -> Value {
    json!({"id":9,"head_sha":"a".repeat(40),"run_id":10})
}
fn run() -> Value {
    json!({"id":10,"head_sha":"a".repeat(40)})
}
async fn fails(replies: Vec<Reply>, fragment: &str) {
    let server = Fixture::start(replies).await;
    let error = read(&server.client(), object(), 9, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains(fragment), "{error}");
    server.finish().await;
}
#[test]
fn signed_url_validation_rejects_unsafe_origins_without_resolving() {
    for url in [
        "http://example.invalid/log",
        "https://user@example.invalid/log",
        "https://example.invalid:8443/log",
        "https://example.invalid/log#fragment",
        "https://example.invalid./log",
        "not a URL",
    ] {
        assert!(download_url(url).is_err(), "{url}");
    }
    assert!(download_url(&format!("https://example.invalid/{}", "x".repeat(8192))).is_err());
    let url = download_url("https://example.invalid/log?signature=fixture").unwrap();
    assert_eq!(url.query(), Some("signature=fixture"));
}
#[test]
fn address_filter_excludes_private_transition_and_special_use_ranges() {
    for address in [
        "0.1.2.3",
        "10.0.0.1",
        "127.0.0.1",
        "169.254.1.1",
        "172.16.0.1",
        "192.168.0.1",
        "255.255.255.255",
        "192.0.2.1",
        "224.0.0.1",
        "240.0.0.1",
        "100.64.0.1",
        "100.127.255.254",
        "192.0.0.1",
        "192.88.99.1",
        "198.18.0.1",
        "198.19.1.1",
        "::1",
        "::ffff:8.8.8.8",
        "fc00::1",
        "fe80::1",
        "2001:100::1",
        "2001:db8::1",
        "2002::1",
        "3fff:100::1",
    ] {
        assert!(!public_address(address.parse().unwrap()), "{address}");
    }
    for address in [
        "8.8.8.8",
        "100.63.255.254",
        "100.128.0.1",
        "2606:4700::1111",
        "2001:4860::8888",
    ] {
        assert!(public_address(address.parse().unwrap()), "{address}");
    }
}
#[tokio::test]
async fn log_requests_validate_object_and_job_before_network() {
    let client = Client::for_test("127.0.0.1:1".parse().unwrap());
    for (object, job) in [
        (object(), 0),
        (object(), u64::MAX),
        (
            Object::parse("https://github.com/example/project/issues/7").unwrap(),
            9,
        ),
    ] {
        assert!(
            read(&client, object, job, &CancellationToken::new())
                .await
                .unwrap_err()
                .to_string()
                .contains("positive job ID")
        );
    }
}
#[tokio::test]
async fn logs_reject_wrong_job_identity_head_and_missing_run() {
    for (key, value, fragment) in [
        ("id", json!(8), "job identity"),
        ("head_sha", json!("c".repeat(40)), "job identity"),
        ("run_id", Value::Null, "run identity"),
        ("run_id", json!(0), "run identity"),
    ] {
        let mut data = job();
        data[key] = value;
        fails(
            vec![
                snapshot(),
                Reply::json("/repos/example/project/actions/jobs/9", data),
            ],
            fragment,
        )
        .await;
    }
}
#[tokio::test]
async fn logs_reject_wrong_run_and_pre_download_snapshot_changes() {
    for key in ["id", "head_sha"] {
        let mut data = run();
        data[key] = if key == "id" {
            json!(11)
        } else {
            json!("c".repeat(40))
        };
        fails(
            vec![
                snapshot(),
                Reply::json("/repos/example/project/actions/jobs/9", job()),
                Reply::json("/repos/example/project/actions/runs/10", data),
            ],
            "run identity",
        )
        .await;
    }
    let mut changed = detail();
    changed["base"]["ref"] = json!("release");
    fails(
        vec![
            snapshot(),
            Reply::json("/repos/example/project/actions/jobs/9", job()),
            Reply::json("/repos/example/project/actions/runs/10", run()),
            Reply::json("/repos/example/project/pulls/7", changed),
        ],
        "changed before log download",
    )
    .await;
}
#[tokio::test]
async fn logs_require_one_redirect_and_reject_unsafe_location_before_dns() {
    for (status, headers, fragment) in [
        (200, vec![], "unavailable or unsupported"),
        (302, vec![], "missing or ambiguous"),
        (
            302,
            vec![
                ("Location".into(), "https://one.invalid/".into()),
                ("Location".into(), "https://two.invalid/".into()),
            ],
            "missing or ambiguous",
        ),
        (
            302,
            vec![("Location".into(), "http://127.0.0.1/private".into())],
            "origin is unsupported",
        ),
    ] {
        let mut reply = Reply::json("/repos/example/project/actions/jobs/9/logs", Value::Null);
        reply.status = status;
        reply.headers = headers;
        fails(
            vec![
                snapshot(),
                Reply::json("/repos/example/project/actions/jobs/9", job()),
                Reply::json("/repos/example/project/actions/runs/10", run()),
                snapshot(),
                reply,
            ],
            fragment,
        )
        .await;
    }
}
#[tokio::test]
async fn separate_log_receiver_preserves_lossy_text_without_api_credentials() {
    let mut reply = Reply::json("/signed", Value::Null);
    reply.body = vec![b'a', 0xff, b'b'];
    let server = Fixture::start(vec![reply]).await;
    let url = reqwest::Url::parse(&format!("http://{}/signed", server.address)).unwrap();
    let (text, truncated) = receive(
        &server.client(),
        download_client("localhost", &[]).build().unwrap(),
        url,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(text, "a\u{fffd}b");
    assert!(!truncated);
    let requests = server.finish().await;
    let request = String::from_utf8_lossy(&requests[0]).to_ascii_lowercase();
    assert!(request.contains("accept: text/plain"));
    assert!(!request.contains("authorization"));
    assert!(!request.contains("offline-fixture-token"));
}
#[tokio::test]
async fn separate_log_receiver_truncates_and_refuses_redirects() {
    for status in [200, 302] {
        let mut reply = Reply::json("/signed", Value::Null);
        reply.status = status;
        reply.body = vec![b'x'; MAX_LOG_BYTES + 1];
        let server = Fixture::start(vec![reply]).await;
        let url = reqwest::Url::parse(&format!("http://{}/signed", server.address)).unwrap();
        let result = receive(
            &server.client(),
            download_client("localhost", &[]).build().unwrap(),
            url,
            &CancellationToken::new(),
        )
        .await;
        if status == 200 {
            let (text, truncated) = result.unwrap();
            assert_eq!(text.len(), MAX_LOG_BYTES);
            assert!(truncated);
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("expired, redirected, or failed")
            );
        }
        server.finish().await;
    }
}
#[tokio::test]
async fn separate_log_receiver_honors_precancelled_request() {
    let authority = Client::for_test("127.0.0.1:1".parse().unwrap());
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = receive(
        &authority,
        download_client("localhost", &[]).build().unwrap(),
        reqwest::Url::parse("http://127.0.0.1:1/").unwrap(),
        &cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "GitHub log download cancelled");
}
