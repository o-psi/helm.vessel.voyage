//! Streamable HTTP 2025-06-18. No reconnect, replay, redirects or legacy SSE fallback.
use super::{MAX_FRAME_BYTES, MAX_UNRELATED_FRAMES, ToolError, failed, validate_envelope};
use reqwest::{Client, Method, Response, StatusCode, Url, header::HeaderValue};
use serde_json::{Value, json};
use tokio::sync::Mutex;

pub fn validate_http_endpoint(endpoint: &str) -> Result<(), ToolError> {
    let url = Url::parse(endpoint).map_err(|_| failed("invalid MCP HTTP endpoint"))?;
    let loopback = url
        .host_str()
        .and_then(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|ip| ip.is_loopback());
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(failed(
            "MCP endpoint requires HTTPS (HTTP allowed only for literal loopback), without credentials, query or fragment",
        ));
    }
    Ok(())
}

pub(super) struct HttpTransport {
    client: Client,
    endpoint: Url,
    credential: Option<HeaderValue>,
    session: Mutex<Option<HeaderValue>>,
    shutdown: Mutex<bool>,
}
impl HttpTransport {
    pub(super) fn new(endpoint: &str, credential: Option<&str>) -> Result<Self, ToolError> {
        validate_http_endpoint(endpoint)?;
        let credential = credential
            .map(|token| {
                if token.is_empty() {
                    return Err(failed("empty MCP HTTP credential"));
                }
                let mut header = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| failed("invalid MCP HTTP credential"))?;
                header.set_sensitive(true);
                Ok(header)
            })
            .transpose()?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| failed("MCP HTTP client construction failed"))?;
        Ok(Self {
            client,
            endpoint: Url::parse(endpoint).map_err(|_| failed("invalid MCP endpoint"))?,
            credential,
            session: Mutex::new(None),
            shutdown: Mutex::new(false),
        })
    }
    async fn post(&self, frame: &[u8], initialize: bool) -> Result<Response, ToolError> {
        let request = self
            .request_builder(Method::POST, initialize)
            .await
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(frame.to_vec());
        // A transport error may follow remote execution. Never expose reqwest's URL diagnostics.
        let response = request
            .send()
            .await
            .map_err(|_| failed("MCP HTTP request failed; external effect outcome uncertain"))?;
        if !response.status().is_success() {
            return Err(failed(format!(
                "MCP HTTP status {}; connection retired, no automatic resend",
                response.status().as_u16()
            )));
        }
        if initialize && let Some(session) = response.headers().get("Mcp-Session-Id") {
            if session.as_bytes().is_empty()
                || session.as_bytes().len() > 1024
                || !session.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b))
            {
                return Err(failed("MCP HTTP invalid session header"));
            }
            let mut session = session.clone();
            session.set_sensitive(true);
            *self.session.lock().await = Some(session);
        }
        Ok(response)
    }
    async fn request_builder(&self, method: Method, initialize: bool) -> reqwest::RequestBuilder {
        let mut request = self.client.request(method, self.endpoint.clone());
        if let Some(credential) = &self.credential {
            request = request.header("Authorization", credential);
        }
        if !initialize {
            request = request.header("MCP-Protocol-Version", "2025-06-18");
            if let Some(session) = self.session.lock().await.as_ref() {
                request = request.header("Mcp-Session-Id", session);
            }
        }
        request
    }
    pub(super) async fn send(&self, frame: &[u8]) -> Result<(), ToolError> {
        let mut response = self.post(frame, false).await?;
        if response.status() != StatusCode::ACCEPTED {
            return Err(failed("MCP HTTP notification or response requires 202"));
        }
        // Do not permit a peer to attach an unbounded body to a notification response.
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failed("MCP HTTP notification response failed"))?
        {
            if !chunk.is_empty() {
                return Err(failed("MCP HTTP 202 response must have no body"));
            }
        }
        Ok(())
    }
    pub(super) async fn request(
        &self,
        frame: &[u8],
        id: u64,
        initialize: bool,
    ) -> Result<Value, ToolError> {
        let mut response = self.post(frame, initialize).await?;
        let mime = response
            .headers()
            .get("Content-Type")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime == "application/json" {
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| failed("MCP HTTP response interrupted; outcome uncertain"))?
            {
                if bytes.len().saturating_add(chunk.len()) > MAX_FRAME_BYTES {
                    return Err(failed("MCP HTTP JSON response exceeds frame limit"));
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| failed("MCP HTTP malformed JSON"))?;
            validate_envelope(&value)?;
            if value.get("id").and_then(Value::as_u64) != Some(id) || value.get("method").is_some()
            {
                return Err(failed("MCP HTTP mismatched response"));
            }
            return Ok(value);
        }
        if mime != "text/event-stream" {
            return Err(failed("MCP HTTP unsupported response content type"));
        }
        let mut line = Vec::new();
        let mut data = Vec::new();
        let mut count = 0usize;
        let mut total = 0usize;
        let mut previous_cr = false;
        let mut first_line = true;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failed("MCP SSE interrupted; external effect outcome uncertain"))?
        {
            total = total.saturating_add(chunk.len());
            if total > 8 * MAX_FRAME_BYTES {
                return Err(failed("MCP SSE response byte limit exceeded"));
            }
            for byte in chunk {
                if byte == b'\n' && previous_cr {
                    previous_cr = false;
                    continue;
                }
                previous_cr = byte == b'\r';
                if byte != b'\n' && byte != b'\r' {
                    if line.len() >= MAX_FRAME_BYTES {
                        return Err(failed("MCP SSE line limit exceeded"));
                    }
                    line.push(byte);
                    continue;
                }
                if first_line {
                    first_line = false;
                    if line.starts_with(&[0xef, 0xbb, 0xbf]) {
                        line.drain(..3);
                    }
                }
                if line.is_empty() {
                    if data.is_empty() {
                        continue;
                    }
                    data.pop(); // Last data-line newline.
                    let value: Value = serde_json::from_slice(&data)
                        .map_err(|_| failed("MCP SSE malformed JSON"))?;
                    data.clear();
                    validate_envelope(&value)?;
                    if value.get("id").and_then(Value::as_u64) == Some(id)
                        && value.get("method").is_none()
                    {
                        return Ok(value);
                    }
                    count += 1;
                    if count > MAX_UNRELATED_FRAMES {
                        return Err(failed("MCP SSE unrelated-message limit exceeded"));
                    }
                    if let Some(request_id) = value.get("id")
                        && let Some(method) = value.get("method").and_then(Value::as_str)
                    {
                        let reply = if method == "ping" {
                            json!({"jsonrpc":"2.0","id":request_id,"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Method not supported"}})
                        };
                        self.send(&super::encode_frame(&reply)?).await?;
                    }
                } else {
                    // Validate every line as UTF-8, including ignored extension fields.
                    std::str::from_utf8(&line).map_err(|_| failed("MCP SSE invalid UTF-8"))?;
                    if line == b"data" || line.starts_with(b"data:") {
                        let value = if line.len() == 4 {
                            &[][..]
                        } else {
                            line[5..].strip_prefix(b" ").unwrap_or(&line[5..])
                        };
                        if data.len().saturating_add(value.len()).saturating_add(1)
                            > MAX_FRAME_BYTES
                        {
                            return Err(failed("MCP SSE event frame limit exceeded"));
                        }
                        data.extend_from_slice(value);
                        data.push(b'\n');
                    }
                }
                line.clear();
            }
        }
        Err(failed(
            "MCP SSE closed before matching response; external effect outcome uncertain",
        ))
    }
    pub(super) async fn shutdown(&self) -> Result<(), ToolError> {
        let mut done = self.shutdown.lock().await;
        if *done {
            return Ok(());
        }
        if self.session.lock().await.is_some() {
            let result = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                self.request_builder(Method::DELETE, false)
                    .await
                    .send()
                    .await
            })
            .await
            .map_err(|_| failed("MCP HTTP session cleanup timed out"))?
            .map_err(|_| failed("MCP HTTP session cleanup unconfirmed"))?;
            // 404 attests an expired session. 405 is the protocol's explicit absence
            // of remote session termination; release our lease, never claim effects stopped.
            if !result.status().is_success()
                && result.status() != StatusCode::NOT_FOUND
                && result.status() != StatusCode::METHOD_NOT_ALLOWED
            {
                return Err(failed("MCP HTTP session cleanup unconfirmed"));
            }
        }
        *done = true;
        Ok(())
    }
}
