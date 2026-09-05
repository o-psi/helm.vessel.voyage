//! Read-only Vessel discovery for voyage setup. Credentials are transient and
//! enrollment/presence observations confer no execution authority.
use anyhow::{Result, bail, ensure};
use serde::Deserialize;
use std::time::Duration;
use uuid::Uuid;
use zeroize::Zeroizing;

const MAX_DISCOVERY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelmPresence {
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
}

pub struct Client {
    origin: String,
    token: Zeroizing<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(origin: &str, token: String) -> Result<Self> {
        let token = Zeroizing::new(token);
        // Cleartext is permitted only for literal loopback addresses. Redirects
        // must never move the bearer credential to a different destination.
        let origin = crate::attachment::client::validate_origin(origin, true)?;
        ensure!(
            (32..=1024).contains(&token.len()) && !token.chars().any(char::is_control),
            "Enter a 32–1024 byte Vessel operator token without control characters"
        );
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .build()?;
        Ok(Self {
            origin,
            token,
            http,
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub async fn discover(&self) -> Result<Vec<HelmPresence>> {
        let mut response = self
            .http
            .get(format!("{}/v1/diagnostics", self.origin))
            .bearer_auth(self.token.as_str())
            .send()
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Cannot reach Vessel; check the address, TLS and network, then retry"
                )
            })?;
        match response.status().as_u16() {
            200 => {}
            401 | 403 => {
                bail!("Vessel rejected the operator token; reconnect with an authorized token")
            }
            300..=399 => {
                bail!("Vessel redirected discovery; enter its final HTTPS origin explicitly")
            }
            _ => bail!("Vessel discovery is unavailable; retry when the server is ready"),
        }
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_DISCOVERY_BYTES as u64),
            "Vessel discovery response exceeds the size limit"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Vessel discovery was interrupted; retry"))?
        {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= MAX_DISCOVERY_BYTES,
                "Vessel discovery response exceeds the size limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        decode(&bytes)
    }
}

fn decode(bytes: &[u8]) -> Result<Vec<HelmPresence>> {
    #[derive(Deserialize)]
    struct Discovery {
        connections: Vec<Connection>,
    }
    #[derive(Deserialize)]
    struct Connection {
        machine_id: Uuid,
        owner_id: Uuid,
        epoch: u64,
    }
    let data: Discovery = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("Vessel returned an invalid Helm inventory"))?;
    ensure!(
        data.connections.len() <= 64,
        "Vessel inventory exceeds 64 Helms"
    );
    let mut seen = std::collections::BTreeSet::new();
    let mut owner = None;
    let mut helms = Vec::new();
    for connection in data.connections {
        ensure!(
            !connection.machine_id.is_nil()
                && !connection.owner_id.is_nil()
                && connection.epoch > 0
                && seen.insert(connection.machine_id),
            "Vessel returned invalid or duplicate Helm identities"
        );
        ensure!(
            owner.is_none_or(|id| id == connection.owner_id),
            "Vessel returned an ambiguous owner inventory"
        );
        owner = Some(connection.owner_id);
        helms.push(HelmPresence {
            machine_id: connection.machine_id,
            owner_id: connection.owner_id,
            epoch: connection.epoch,
        });
    }
    helms.sort_by_key(|helm| helm.machine_id);
    Ok(helms)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_origin_without_exposing_credentials() {
        for origin in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/?token=secret",
        ] {
            assert!(Client::new(origin, "x".repeat(32)).is_err());
        }
        assert!(Client::new("http://127.0.0.1:9480", "x".repeat(32)).is_ok());
        assert!(Client::new("https://example.com", "short".into()).is_err());
    }
    #[test]
    fn inventory_is_bounded_and_rejects_ambiguous_identities() {
        let machine = Uuid::new_v4();
        let owner = Uuid::new_v4();
        let connection = serde_json::json!({"machine_id":machine,"owner_id":owner,"epoch":1});
        let bytes =
            serde_json::to_vec(&serde_json::json!({"connections":[connection.clone()]})).unwrap();
        assert_eq!(decode(&bytes).unwrap()[0].machine_id, machine);
        assert!(
            decode(
                &serde_json::to_vec(
                    &serde_json::json!({"connections":[connection.clone(),connection]})
                )
                .unwrap()
            )
            .is_err()
        );
        assert!(
            decode(br#"{"connections":[{"token":"sensitive"}]}"#)
                .unwrap_err()
                .to_string()
                .find("sensitive")
                .is_none()
        );
        assert!(decode(br#"{"connections":[]}"#).unwrap().is_empty());
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    async fn server(response: String) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![];
            loop {
                let mut byte = [0];
                if socket.read(&mut byte).await.unwrap() == 0 {
                    break;
                }
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(request.len() < 8192);
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (origin, task)
    }
    #[tokio::test]
    async fn authenticated_discovery_is_read_only_and_errors_do_not_echo_server_data() {
        let token = "operator-private-canary-01234567890123456789";
        let (origin, request) = server("HTTP/1.1 200 OK\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"connections\":[]}".into()).await;
        assert!(
            Client::new(&origin, token.into())
                .unwrap()
                .discover()
                .await
                .unwrap()
                .is_empty()
        );
        let request = request.await.unwrap();
        assert!(request.starts_with("GET /v1/diagnostics HTTP/1.1"));
        assert!(request.contains(&format!("Bearer {token}")));
        let (origin, request) = server("HTTP/1.1 401 Unauthorized\r\nContent-Length: 17\r\nConnection: close\r\n\r\nprivate-from-body".into()).await;
        let error = Client::new(&origin, token.into())
            .unwrap()
            .discover()
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-from-body"));
        assert!(error.contains("rejected"));
        request.await.unwrap();
    }
    #[tokio::test]
    async fn redirects_and_oversized_responses_fail_closed() {
        let redirected = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let response = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://{}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            redirected.local_addr().unwrap()
        );
        let (origin, request) = server(response).await;
        assert!(
            Client::new(&origin, "x".repeat(32))
                .unwrap()
                .discover()
                .await
                .unwrap_err()
                .to_string()
                .contains("redirected")
        );
        request.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), redirected.accept())
                .await
                .is_err()
        );
        let (origin, request) = server(
            "HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\nConnection: close\r\n\r\n".into(),
        )
        .await;
        assert!(
            Client::new(&origin, "x".repeat(32))
                .unwrap()
                .discover()
                .await
                .unwrap_err()
                .to_string()
                .contains("size limit")
        );
        request.await.unwrap();
    }
}
