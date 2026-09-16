use super::super::{registry, test_support::Fixture};
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn request() -> VesselRequest {
    VesselRequest {
        protocol: VESSEL_API_VERSION,
        command: VesselCommand::Capabilities,
    }
}

#[tokio::test]
async fn exchange_rejects_nonlocal_ambiguous_and_malformed_endpoints_before_network() {
    let f = Fixture::new();
    assert!(exchange(&f.0, &request()).await.is_err());
    for endpoint in [
        "not a URL",
        "https://127.0.0.1:1",
        "http://localhost:1",
        "http://192.0.2.1:1",
        "http://user@127.0.0.1:1",
        "http://user:pass@127.0.0.1:1",
        "http://127.0.0.1:1?query",
        "http://127.0.0.1:1#fragment",
    ] {
        registry::save_local_access(
            &f.0,
            &LocalAccessCredential {
                endpoint: endpoint.into(),
                token: TOKEN.into(),
            },
        )
        .unwrap();
        assert!(exchange(&f.0, &request()).await.is_err(), "{endpoint}");
    }
    for token in ["short".to_owned(), "g".repeat(64)] {
        registry::save_local_access(
            &f.0,
            &LocalAccessCredential {
                endpoint: "http://127.0.0.1:1".into(),
                token,
            },
        )
        .unwrap();
        assert!(
            exchange(&f.0, &request())
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid local Vessel HTTP credential")
        );
    }
}

#[tokio::test]
async fn exchange_refuses_redirect_http_error_wrong_protocol_and_invalid_json() {
    for (status, body, expected) in [
        ("302 Found", "{}".to_owned(), "HTTP request rejected"),
        ("503 Service Unavailable", "{}".to_owned(), "HTTP request rejected"),
        ("200 OK", "broken-json".to_owned(), "expected value"),
        ("200 OK", serde_json::json!({"protocol": VESSEL_API_VERSION + 1, "result": {}, "error": null, "outcome_unknown": false}).to_string(), "unsupported Vessel protocol"),
    ] {
        let f = Fixture::new();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        registry::save_local_access(&f.0, &LocalAccessCredential { endpoint, token: TOKEN.into() }).unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
            let mut bytes = Vec::new();
            loop {
                let byte = stream.read_u8().await.unwrap();
                bytes.push(byte);
                if bytes.ends_with(b"\r\n\r\n") { break; }
                assert!(bytes.len() < 16_384);
            }
            let headers = String::from_utf8(bytes).unwrap().to_ascii_lowercase();
            assert!(headers.starts_with(&format!("post {} ", voyage_protocol::vessel::COMMAND_PATH)));
            assert!(headers.contains(&format!("authorization: bearer {TOKEN}")));
            let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap().trim().parse().unwrap();
            let mut payload = vec![0; length];
            stream.read_exact(&mut payload).await.unwrap();
            let request: VesselRequest = serde_json::from_slice(&payload).unwrap();
            assert!(matches!(request.command, VesselCommand::Capabilities));
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nLocation: http://192.0.2.1/never-follow\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let error = tokio::time::timeout(Duration::from_secs(4), exchange(&f.0, &request())).await.unwrap().unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
        peer.await.unwrap();
    }
}
