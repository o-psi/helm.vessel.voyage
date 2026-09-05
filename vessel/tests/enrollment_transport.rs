//! Real subprocess/TCP fixture, using synthetic credentials and temporary state.
//! This proves enrollment HTTP, not Helm session transport or production TLS.
#![cfg(unix)]
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::enrollment::{Challenge, ProofOperation, SignedChallenge, SigningKey};
const TOKEN: &str = "fixture-only-owner-token-at-least-32-bytes";
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(root: &Path, address: &str) -> Server {
    Server(
        Command::new(env!("CARGO_BIN_EXE_vessel"))
            .args([
                "--bind",
                address,
                "--public-origin",
                &format!("http://{address}"),
                "--allow-insecure-loopback",
                "--attachment-directory",
            ])
            .arg(root.join("authority"))
            .arg("--database")
            .arg(root.join("legacy.db"))
            .env("VESSEL_OPERATOR_TOKEN", TOKEN)
            .env("RUST_LOG", "error")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}
async fn ready(client: &Client, origin: &str, server: &mut Server) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "Vessel exited before readiness"
        );
        if client
            .get(format!("{origin}/ready"))
            .send()
            .await
            .is_ok_and(|r| r.status() == StatusCode::OK)
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Vessel readiness timed out"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
async fn post(
    client: &Client,
    origin: &str,
    path: &str,
    body: Value,
    operator: bool,
) -> (StatusCode, Value) {
    let mut r = client
        .post(format!("{origin}{path}"))
        .header("x-voyage-request", "2")
        .json(&body);
    if operator {
        r = r.bearer_auth(TOKEN);
    }
    let response = r.send().await.unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let status = response.status();
    (status, response.json().await.unwrap())
}
async fn challenge(
    client: &Client,
    origin: &str,
    op: &ProofOperation,
    key: Option<&str>,
) -> Challenge {
    let (status, value) = post(
        client,
        origin,
        "/v2/enrollment/challenge",
        json!({"operation":op,"invitation_key":key}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    serde_json::from_value(value).unwrap()
}
fn proof(key: &SigningKey, c: Challenge) -> SignedChallenge {
    SignedChallenge {
        signature: key.sign(&c).unwrap(),
        challenge: c,
        new_signature: None,
    }
}
#[tokio::test]
async fn binary_enrollment_restart_recovery_revoke_and_legacy_preservation() {
    let root = tempfile::tempdir().unwrap();
    {
        let db = rusqlite::Connection::open(root.path().join("legacy.db")).unwrap();
        db.execute_batch("CREATE TABLE control_plane(id INTEGER,state TEXT);INSERT INTO control_plane VALUES(1,'legacy-sentinel-not-json');").unwrap();
    }
    let before = std::fs::read(root.path().join("legacy.db")).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = socket.local_addr().unwrap().to_string();
    drop(socket);
    let origin = format!("http://{address}");
    let client = Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let mut server = start(root.path(), &address);
    ready(&client, &origin, &mut server).await;
    assert_eq!(
        post(
            &client,
            &origin,
            "/v2/enrollment/invitations",
            json!({"ttl_ms":60_000}),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, invitation) = post(
        &client,
        &origin,
        "/v2/enrollment/invitations",
        json!({"ttl_ms":60_000}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let key = SigningKey::generate().unwrap();
    // Simulate durable client-side preparation before redemption, then restore
    // that same private key after a lost response. This is not an installed CLI.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut keyfile = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(root.path().join("fixture-key.pkcs8"))
        .unwrap();
    keyfile.write_all(key.as_pkcs8()).unwrap();
    keyfile.sync_all().unwrap();
    drop(keyfile);
    let machine_id = Uuid::new_v4();
    let op = ProofOperation::Enroll {
        machine_id,
        transaction_id: Uuid::new_v4(),
        invitation_id: Uuid::parse_str(invitation["id"].as_str().unwrap()).unwrap(),
        public_key: key.public_key(),
    };
    let c = challenge(&client, &origin, &op, invitation["key"].as_str()).await;
    let (status, receipt) = post(
        &client,
        &origin,
        "/v2/enrollment/complete",
        json!({"proof":proof(&key,c),"invitation_key":invitation["key"]}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    drop(server);
    drop(key);
    let mut server = start(root.path(), &address);
    ready(&client, &origin, &mut server).await;
    let key =
        SigningKey::from_pkcs8(&std::fs::read(root.path().join("fixture-key.pkcs8")).unwrap())
            .unwrap();
    let c = challenge(&client, &origin, &op, None).await;
    let (status, recovered) = post(
        &client,
        &origin,
        "/v2/enrollment/complete",
        json!({"proof":proof(&key,c)}),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(receipt, recovered);
    let new = SigningKey::generate().unwrap();
    let rotate = ProofOperation::Rotate {
        machine_id,
        epoch: 1,
        transaction_id: Uuid::new_v4(),
        new_public_key: new.public_key(),
    };
    let c = challenge(&client, &origin, &rotate, None).await;
    let mut p = proof(&key, c);
    p.new_signature = Some(new.sign(&p.challenge).unwrap());
    assert_eq!(
        post(
            &client,
            &origin,
            "/v2/enrollment/complete",
            json!({"proof":p}),
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    let connect = ProofOperation::Connect {
        machine_id,
        epoch: 2,
    };
    let c = challenge(&client, &origin, &connect, None).await;
    assert_eq!(
        post(
            &client,
            &origin,
            "/v2/enrollment/complete",
            json!({"proof":proof(&new,c)}),
            false
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        post(
            &client,
            &origin,
            "/v2/enrollment/revoke",
            json!({"machine_id":machine_id,"expected_epoch":2,"transaction_id":Uuid::new_v4()}),
            true
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(server);
    let mut server = start(root.path(), &address);
    ready(&client, &origin, &mut server).await;
    assert_eq!(
        post(
            &client,
            &origin,
            "/v2/enrollment/challenge",
            json!({"operation":connect}),
            false
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        client
            .post(format!("{origin}/v1/helms/pair"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    drop(server);
    assert_eq!(
        std::fs::read(root.path().join("legacy.db")).unwrap(),
        before
    );
}
#[test]
fn enrollment_is_explicit_and_unsafe_startup_fails_without_creating_state() {
    let root = tempfile::tempdir().unwrap();
    for extra in [
        vec!["--public-origin", "http://example.com"],
        vec![
            "--public-origin",
            "https://example.com",
            "--bind",
            "0.0.0.0:9480",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_vessel"))
            .arg("--attachment-directory")
            .arg(root.path().join("authority"))
            .args(extra)
            .env("VESSEL_OPERATOR_TOKEN", TOKEN)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!root.path().join("authority").exists());
        assert!(!String::from_utf8_lossy(&output.stderr).contains(TOKEN));
    }
}
