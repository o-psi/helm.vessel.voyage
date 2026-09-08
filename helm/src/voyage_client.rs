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
        let mut http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4));
        // Discovery uses an operator credential even during loopback development.
        if reqwest::Url::parse(&origin)?
            .host_str()
            .is_some_and(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            })
        {
            http = http.no_proxy();
        }
        let http = http.build()?;
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
