use super::chatgpt_oauth::{OAuthTokens, login_identity, validate_tokens};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

fn token(subject: &str, account: &str) -> String {
    format!(
        "fixture.{}.signature",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "sub": subject, "https://api.openai.com/auth": {"chatgpt_account_id": account}
            }))
            .unwrap()
        )
    )
}
fn tokens(subject: &str) -> OAuthTokens {
    OAuthTokens {
        access_token: token(subject, "billing-context"),
        refresh_token: "synthetic-refresh".into(),
        id_token: None,
        account_id: "billing-context".into(),
        expires_at: 4_000_000_000,
    }
}
#[test]
fn same_billing_different_login_is_different_identity() {
    assert_ne!(
        login_identity(&tokens("personal-user")).unwrap(),
        login_identity(&tokens("work-user")).unwrap()
    );
    let mut refreshed = tokens("personal-user");
    refreshed.refresh_token = "synthetic-rotated".into();
    refreshed.expires_at += 100;
    assert_eq!(
        login_identity(&tokens("personal-user")).unwrap(),
        login_identity(&refreshed).unwrap()
    );
}
#[test]
fn conflicting_claims_fail_without_echoing_tokens() {
    let mut t = tokens("personal-user");
    t.id_token = Some(token("other-user", "billing-context"));
    assert!(validate_tokens(&t).is_err());
    t.id_token = None;
    t.account_id = "different-billing-context".into();
    let error = validate_tokens(&t).unwrap_err().to_string();
    assert!(!error.contains(&t.access_token));
    assert!(!error.contains(&t.account_id));
}
#[test]
fn opaque_legacy_tokens_do_not_establish_login_identity() {
    let mut t = tokens("personal-user");
    t.access_token = "synthetic-opaque".into();
    assert_eq!(login_identity(&t).unwrap(), None);
    // Existing opaque credentials can remain usable until refresh, but cannot
    // authorize an unverified same-identity replacement.
    assert!(validate_tokens(&t).is_ok());
}

// The child entry is inert in ordinary test enumeration. Parent passes only a
// temporary fixture path and numeric loopback endpoint, never operator settings.
#[test]
fn independent_refresh_child() {
    let Some(root) = std::env::var_os("VOYAGE_TEST_REFRESH_ROOT") else {
        return;
    };
    let endpoint = std::env::var("VOYAGE_TEST_REFRESH_ENDPOINT").unwrap();
    let parsed = reqwest::Url::parse(&endpoint).unwrap();
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    let r = crate::accounts::Registry::new(root.into());
    let (_, accounts) = r.list(|_| true).unwrap();
    assert_eq!(accounts.len(), 1);
    let binding = r
        .freeze(
            accounts[0].id,
            voyage_protocol::accounts::Transport::ChatgptOauth,
        )
        .unwrap();
    let endpoints = super::OAuthEndpoints {
        token: format!("{endpoint}/token"),
        models: format!("{endpoint}/models"),
        ..Default::default()
    };
    let p = super::ChatGptOauthProvider::from_store(
        super::ChatGptTokenStore::bound(r, binding),
        endpoints,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let models = runtime.block_on(super::Provider::models(&p)).unwrap();
    assert_eq!(models[0].id, "fixture-model");
}

#[test]
fn independent_processes_reuse_one_slow_oauth_refresh() {
    use std::{
        io::{Read, Write},
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("registry");
    let r = crate::accounts::Registry::new(root.clone());
    let c = r.ensure_chatgpt_connection().unwrap();
    let mut expired = tokens("personal-user");
    expired.expires_at = 1;
    r.add_oauth(c.id, "personal".into(), "Personal".into(), expired)
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut refreshes = 0;
        let mut models = 0;
        while models < 2 && Instant::now() < deadline {
            let (mut stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(e) => panic!("fixture accept: {e}"),
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut byte = [0u8; 1];
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
                assert!(bytes.len() < 16384);
                if bytes.ends_with(b"\r\n\r\n") {
                    break bytes.len();
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, size)| size.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length < 16384);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let response = if headers.starts_with("POST /token ") {
                refreshes += 1;
                assert_eq!(
                    refreshes, 1,
                    "refresh was replayed across independent processes"
                );
                // Exceeds the original three-second reader wait.
                std::thread::sleep(Duration::from_secs(4));
                json!({"access_token":token("personal-user", "billing-context"),
                    "refresh_token":"synthetic-new-refresh", "account_id":"billing-context", "expires_in":3600})
            } else {
                assert!(headers.starts_with("GET /models?"));
                models += 1;
                json!({"models":[{"slug":"fixture-model", "display_name":"Fixture", "description":"synthetic"}]})
            };
            let response = response.to_string();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        }
        (refreshes, models)
    });
    let executable = std::env::current_exe().unwrap();
    let mut children: Vec<_> = (0..2)
        .map(|_| {
            std::process::Command::new(&executable)
                .args([
                    "--exact",
                    "provider::account_identity_tests::independent_refresh_child",
                    "--nocapture",
                ])
                .env("VOYAGE_TEST_REFRESH_ROOT", &root)
                .env("VOYAGE_TEST_REFRESH_ENDPOINT", &endpoint)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(18);
    let mut success = true;
    for child in &mut children {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                success &= status.success();
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                success = false;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert_eq!(server.join().unwrap(), (1, 2));
    assert!(success, "independent refresh child failed or timed out");
}

#[test]
fn refreshed_credentials_share_stream_redaction_without_debug_disclosure() {
    let redactor = crate::tools::Redactor::new(["synthetic-original".into()]);
    let child = redactor.with_additional(["child-private".into()]);
    redactor.remember_credential("synthetic-rotated").unwrap();
    assert!(child.contains_secret("synthetic-rotated"));
    assert_eq!(child.redact("synthetic-rotated"), "[REDACTED]");
    let prefix = crate::tools::Redactor::new(["synthetic".into()]);
    prefix.remember_credential("synthetic-rotated").unwrap();
    assert_eq!(prefix.redact("synthetic-rotated"), "[REDACTED]");
    assert_eq!(
        child.stable_prefix("prefix synthetic-ro", false),
        "prefix ".len()
    );
    assert!(!format!("{redactor:?}").contains("synthetic"));
}
