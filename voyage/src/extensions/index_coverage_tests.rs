use super::*;
#[test]
fn package_urls_cannot_carry_credentials_queries_or_non_https_origins() {
    assert_eq!(
        url("https://packages.example.invalid/index.json")
            .unwrap()
            .host_str(),
        Some("packages.example.invalid")
    );
    for raw in [
        "http://packages.example.invalid/index.json",
        "file:///tmp/index",
        "https://user@packages.example.invalid/index.json",
        "https://user:pass@packages.example.invalid/index.json",
        "https://packages.example.invalid/index.json?token=secret",
        "https://packages.example.invalid/index.json#fragment",
        "not a url",
    ] {
        assert!(url(raw).is_err(), "{raw}");
    }
    assert!(url(&format!("https://example.invalid/{}", "x".repeat(2048))).is_err());
    assert!(serde_json::from_value::<Configuration>(serde_json::json!({"format":1,"url":"https://example.invalid","sha256":"a".repeat(64),"unknown":true})).is_err());
    assert!(serde_json::from_value::<Index>(serde_json::json!({"format":1,"packages":[{"id":"test","url":"https://example.invalid","sha256":"a".repeat(64),"command":"untrusted"}]})).is_err());
}
#[tokio::test]
async fn invalid_download_digest_is_rejected_before_any_io() {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(
        download(
            &client,
            url("https://offline.invalid/package").unwrap(),
            "invalid",
            1
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("digest")
    );
}
