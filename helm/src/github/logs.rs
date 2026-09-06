//! Bounded Actions job logs acquired through an authenticated fixed API route.
use super::{
    context,
    repository::{Object, ObjectKind},
    transport::Client,
};
use anyhow::{Result, ensure};
use reqwest::{StatusCode, header};
use serde::Serialize;
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const MAX_LOG_BYTES: usize = 1024 * 1024;
#[derive(Clone, Debug, Serialize)]
pub struct Log {
    pub object: Object,
    pub head: String,
    pub base: context::Base,
    pub job: u64,
    pub run: u64,
    pub run_attempt: Option<u64>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub fetched_at: chrono::DateTime<chrono::Utc>,
    pub association: &'static str,
    pub text: String,
    pub truncated: bool,
    pub incomplete: Vec<String>,
}

pub(super) async fn read(
    client: &Client,
    object: Object,
    job: u64,
    cancel: &CancellationToken,
) -> Result<Log> {
    object.validate()?;
    ensure!(
        object.kind == ObjectKind::PullRequest && job > 0 && job <= i64::MAX as u64,
        "GitHub job logs require a pull request and positive job ID"
    );
    let detail = context::details(client, &object, cancel).await?;
    let head = context::sha(&detail["head"]["sha"])?;
    let base = context::base(&detail)?;
    let prefix = format!("/repos/{}/actions", object.repository.slug());
    let job_data = client
        .get(&format!("{prefix}/jobs/{job}"), cancel)
        .await?
        .json()?;
    ensure!(
        job_data["id"].as_u64() == Some(job) && context::sha(&job_data["head_sha"])? == head,
        "GitHub job identity or head differs from the selected pull request"
    );
    let run = job_data["run_id"]
        .as_u64()
        .filter(|id| *id > 0 && *id <= i64::MAX as u64)
        .ok_or_else(|| anyhow::anyhow!("GitHub job run identity is missing"))?;
    let run_data = client
        .get(&format!("{prefix}/runs/{run}"), cancel)
        .await?
        .json()?;
    ensure!(
        run_data["id"].as_u64() == Some(run) && context::sha(&run_data["head_sha"])? == head,
        "GitHub workflow run identity or head differs"
    );
    let fresh = context::details(client, &object, cancel).await?;
    ensure!(
        context::sha(&fresh["head"]["sha"])? == head && context::base(&fresh)? == base,
        "Pull request changed before log download; reload its context"
    );
    let response = client
        .request(
            reqwest::Method::GET,
            &format!("{prefix}/jobs/{job}/logs"),
            None,
            cancel,
        )
        .await?;
    ensure!(
        response.status == StatusCode::FOUND,
        "GitHub job log download is unavailable or unsupported"
    );
    ensure!(
        response.headers.get_all(header::LOCATION).iter().count() == 1,
        "GitHub job log location is missing or ambiguous"
    );
    let location = response
        .headers
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("GitHub job log location is missing"))?;
    client.check_current()?;
    let (text, truncated) = download(client, location, cancel).await?;
    client.check_current()?;
    let fresh = context::details(client, &object, cancel).await?;
    let mut incomplete = Vec::new();
    if context::sha(&fresh["head"]["sha"])? != head || context::base(&fresh)? != base {
        incomplete
            .push("Pull request changed during log observation; reload before review.".into());
    }
    if truncated {
        incomplete.push("Log text exceeds the local 1 MiB limit and is truncated.".into());
    }
    Ok(Log {
        object,
        head,
        base,
        job,
        run,
        run_attempt: job_data["run_attempt"].as_u64(),
        status: job_data["status"].as_str().map(str::to_owned),
        conclusion: job_data["conclusion"].as_str().map(str::to_owned),
        fetched_at: chrono::Utc::now(),
        association: "Job and workflow run share the observed head SHA; this does not prove a unique pull-request trigger.",
        text,
        truncated,
        incomplete,
    })
}

fn download_url(location: &str) -> Result<reqwest::Url> {
    ensure!(
        location.len() <= 8192,
        "GitHub signed log location exceeds limit"
    );
    let url = reqwest::Url::parse(location)
        .map_err(|_| anyhow::anyhow!("GitHub signed log location is invalid"))?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url.port_or_known_default() == Some(443)
            && url.host_str().is_some_and(|host| !host.ends_with('.')),
        "GitHub signed log origin is unsupported"
    );
    Ok(url)
}
fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && a != 0
                && a < 240
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0)
                && !(a == 192 && b == 88 && c == 99)
                && !(a == 198 && (18..=19).contains(&b))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // Conservative native global-unicast subset; reject transition,
            // documentation and special-purpose ranges, including mapped IPv4.
            segments[0] & 0xe000 == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
                && segments[0] != 0x2002
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

async fn download(
    authority: &Client,
    location: &str,
    cancel: &CancellationToken,
) -> Result<(String, bool)> {
    let url = download_url(location)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("GitHub log host is missing"))?
        .to_owned();
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("GitHub log download cancelled"),
        result = tokio::time::timeout(Duration::from_secs(30),async {
            let addresses = tokio::net::lookup_host((host.as_str(),443)).await
                .map_err(|_|anyhow::anyhow!("GitHub log host resolution failed"))?.take(17).collect::<Vec<SocketAddr>>();
            ensure!(!addresses.is_empty() && addresses.len() <= 16 && addresses.iter().all(|address|public_address(address.ip())),
                "GitHub log download resolved an unsupported or private address");
            authority.check_current()?;
            let client = download_client(&host,&addresses).build().map_err(|_|anyhow::anyhow!("GitHub log transport is unavailable"))?;
            receive(authority,client,url,cancel).await

        }) => result.map_err(|_|anyhow::anyhow!("GitHub log download deadline elapsed"))?,
    }
}

fn download_client(host: &str, addresses: &[SocketAddr]) -> reqwest::ClientBuilder {
    reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never()).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30))
        .resolve_to_addrs(host,addresses)
}

async fn receive(authority: &Client, client: reqwest::Client, url: reqwest::Url, cancel: &CancellationToken) -> Result<(String,bool)> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => anyhow::bail!("GitHub log download cancelled"),
        result = tokio::time::timeout(Duration::from_secs(30),async {
            // The separately constructed client has no credential, cookies,
            // referrer, or defaults from the authenticated API client.
            authority.check_current()?;
            let mut response = client.get(url).header(header::ACCEPT,"text/plain").send().await
                .map_err(|_|anyhow::anyhow!("GitHub signed log download failed"))?;
            ensure!(response.status() == StatusCode::OK,"GitHub signed log download expired, redirected, or failed");
            let mut bytes = Vec::new();
            let mut truncated = false;
            while let Some(chunk) = response.chunk().await.map_err(|_|anyhow::anyhow!("GitHub log download was interrupted"))? {
                let remaining = MAX_LOG_BYTES.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                if chunk.len() > remaining { truncated = true; break; }
            }
            authority.check_current()?;
            Ok((String::from_utf8_lossy(&bytes).into_owned(),truncated))
        }) => result.map_err(|_|anyhow::anyhow!("GitHub log download deadline elapsed"))?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_log_location_and_resolved_addresses_are_conservative() {
        for url in [
            "http://example.com/log",
            "https://user@example.com/log",
            "https://example.com:444/log",
            "https://example.com./log",
            "https://example.com/log#fragment",
        ] {
            assert!(download_url(url).is_err());
        }
        assert!(download_url("https://logs.example.com/path?signature=opaque").is_ok());
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "2002:7f00:1::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "2606:4700:4700::1111"] {
            assert!(public_address(address.parse().unwrap()));
        }
    }
}

#[cfg(test)]
mod tls_fixture {
    use super::*;
    #[test]
    fn tls_driver() {
        let Ok(mode) = std::env::var("HELM_GITHUB_LOG_DRIVER") else { return };
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let port:u16 = std::env::var("HELM_GITHUB_LOG_PORT").unwrap().parse().unwrap();
            let certificate = reqwest::Certificate::from_pem(&std::fs::read(std::env::var("HELM_GITHUB_LOG_CERT").unwrap()).unwrap()).unwrap();
            let authority = Client::new("fixture-authenticated-api-token".into()).unwrap();
            // Only the test supplies a local root certificate and loopback DNS
            // resolution; production download() validates public DNS first.
            let client = download_client("logs.fixture.test",&[SocketAddr::from(([127,0,0,1],port))])
                .add_root_certificate(certificate).build().unwrap();
            let url = reqwest::Url::parse(&format!("https://logs.fixture.test:{port}/{mode}?signed=fixture-private-query")).unwrap();
            let cancel = CancellationToken::new();
            if mode == "cancel" {
                let token=cancel.clone();
                let ready=std::path::PathBuf::from(std::env::var("HELM_GITHUB_LOG_READY").unwrap());
                tokio::spawn(async move {
                    tokio::time::timeout(Duration::from_secs(3),async {
                        while !ready.exists() { tokio::time::sleep(Duration::from_millis(5)).await; }
                    }).await.unwrap();
                    token.cancel();
                });
            }
            let result = receive(&authority,client,url,&cancel).await;
            match mode.as_str() {
                "success" => { let (text,truncated)=result.unwrap();assert_eq!(text,"workflow log: succeeded\n");assert!(!truncated); },
                "bounded" => { let (text,truncated)=result.unwrap();assert_eq!(text.len(),MAX_LOG_BYTES);assert!(truncated); },
                "exact" => { let (text,truncated)=result.unwrap();assert_eq!(text.len(),MAX_LOG_BYTES);assert!(!truncated); },
                "cancel" | "redirect" | "expired" | "partial" => { let text=result.unwrap_err().to_string();assert!(!text.contains("fixture-private-query"));assert!(!text.contains("fixture-authenticated-api-token")); },
                _=>panic!("unknown fixture mode"),
            }
            println!("TLS_DRIVER_PASS");
        });
    }
    #[test]
    fn secondary_https_transport_contract() {
        let status = std::process::Command::new("python3")
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/system/github_logs.py"))
            .arg("--test-binary").arg(std::env::current_exe().unwrap()).status().unwrap();
        assert!(status.success(),"actual signed HTTPS log transport fixture failed");
    }
}
