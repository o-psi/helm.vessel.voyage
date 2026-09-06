//! Fixed-origin authenticated requests; errors never include response bodies or URLs.
use anyhow::{Result, ensure};
use futures_util::StreamExt;
use reqwest::{Method, StatusCode, header};
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub(super) const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REQUEST_BYTES: usize = 128 * 1024;
pub(super) const API_VERSION: &str = "2026-03-10";

pub(super) struct Client {
    http: reqwest::Client,
    token: Zeroizing<String>,
    origin: reqwest::Url,
    policy: Option<std::sync::Arc<crate::policy::Policy>>,
}

pub(super) struct Response {
    pub status: StatusCode,
    pub headers: header::HeaderMap,
    pub bytes: Vec<u8>,
}
impl Response {
    pub fn json(self) -> Result<Value> {
        serde_json::from_slice(&self.bytes)
            .map_err(|_| anyhow::anyhow!("GitHub returned invalid JSON"))
    }
}

impl Client {
    pub fn new(token: String) -> Result<Self> {
        ensure!(
            token.len() >= 4
                && token.len() <= 4096
                && token.bytes().all(|byte| byte.is_ascii_graphic()),
            "GitHub credential is unavailable or malformed; configure HELM_GITHUB_TOKEN"
        );
        Ok(Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .no_proxy()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|_| anyhow::anyhow!("GitHub transport is unavailable"))?,
            token: Zeroizing::new(token),
            origin: reqwest::Url::parse("https://api.github.com/").expect("fixed API origin"),
            policy: None,
        })
    }
    pub fn with_policy(mut self, policy: std::sync::Arc<crate::policy::Policy>) -> Self {
        self.policy = Some(policy);
        self
    }

    #[cfg(test)]
    pub fn fixture(token: String, origin: reqwest::Url) -> Result<Self> {
        ensure!(
            origin.host_str() == Some("127.0.0.1") && origin.scheme() == "http",
            "fixture needs loopback origin"
        );
        let mut client = Self::new(token)?;
        client.origin = origin;
        Ok(client)
    }

    /// Callers construct paths from validated typed operations, never remote URLs.
    /// Each invocation makes exactly one request. In particular, POST is not retried.
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        cancel: &CancellationToken,
    ) -> Result<Response> {
        if let Some(policy) = &self.policy {
            policy.check_current()?;
        }
        ensure!(
            path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains(['#', '\\'])
                && !path.split('/').any(|segment| matches!(segment, "." | "..")),
            "invalid GitHub operation path"
        );
        let url = self
            .origin
            .join(path)
            .map_err(|_| anyhow::anyhow!("invalid GitHub operation path"))?;
        ensure!(
            url.origin() == self.origin.origin(),
            "invalid GitHub operation origin"
        );
        let mut authorization = header::HeaderValue::from_str(&format!("Bearer {}", &*self.token))
            .map_err(|_| anyhow::anyhow!("GitHub credential is malformed"))?;
        authorization.set_sensitive(true);
        let mut request = self
            .http
            .request(method, url)
            .header(header::AUTHORIZATION, authorization)
            .header(header::ACCEPT, "application/vnd.github+json")
            .header(header::USER_AGENT, "Helm-GitHub/1")
            .header("X-GitHub-Api-Version", API_VERSION);
        if let Some(body) = body {
            let bytes =
                serde_json::to_vec(body).map_err(|_| anyhow::anyhow!("invalid GitHub request"))?;
            ensure!(
                bytes.len() <= MAX_REQUEST_BYTES,
                "GitHub request exceeds limit"
            );
            request = request
                .header(header::CONTENT_TYPE, "application/json")
                .body(bytes);
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("GitHub operation cancelled"),
            result = tokio::time::timeout(Duration::from_secs(30), async {
                let response = request.send().await.map_err(|_| anyhow::anyhow!("GitHub transport failed; publication may be uncertain"))?;
                let status = response.status();
                let headers = response.headers().clone();
                ensure!(response.content_length().is_none_or(|length| length <= MAX_RESPONSE_BYTES as u64), "GitHub response exceeds limit");
                let mut stream = response.bytes_stream();
                let mut bytes = Vec::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| anyhow::anyhow!("GitHub response was interrupted; publication may be uncertain"))?;
                    ensure!(bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES, "GitHub response exceeds limit");
                    bytes.extend_from_slice(&chunk);
                }
                if let Some(policy) = &self.policy { policy.check_current()?; }
                Ok(Response { status, headers, bytes })
            }) => result.map_err(|_| anyhow::anyhow!("GitHub deadline elapsed; publication may be uncertain"))?,
        }
    }

    pub async fn get(&self, path: &str, cancel: &CancellationToken) -> Result<Response> {
        let response = self.request(Method::GET, path, None, cancel).await?;
        check_status(&response)?;
        Ok(response)
    }
}

pub(super) fn check_status(response: &Response) -> Result<()> {
    if response.status.is_success() {
        return Ok(());
    }
    match response.status.as_u16() {
        401 => anyhow::bail!("GitHub authentication failed; verify HELM_GITHUB_TOKEN"),
        403 | 429 => {
            let retry = response
                .headers
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value <= 86400);
            if let Some(seconds) = retry {
                anyhow::bail!(
                    "GitHub denied or rate limited the operation; retry observation after {seconds} seconds"
                );
            }
            anyhow::bail!(
                "GitHub denied or rate limited the operation; verify repository permissions and API limits"
            );
        }
        404 => anyhow::bail!(
            "GitHub object unavailable; verify repository, permissions and SSO authorization"
        ),
        410 => anyhow::bail!("GitHub resource or configured API version is unavailable"),
        422 => {
            anyhow::bail!("GitHub rejected the operation parameters or content; no automatic retry")
        }
        300..=399 => anyhow::bail!(
            "GitHub redirected the operation; select its canonical repository explicitly"
        ),
        _ => anyhow::bail!(
            "GitHub operation failed with HTTP {}; no automatic retry",
            response.status.as_u16()
        ),
    }
}
