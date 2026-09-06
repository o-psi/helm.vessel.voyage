//! Native ChatGPT subscription authentication and request preparation.
//!
//! This module never launches Codex. Endpoints and storage roots are injectable so
//! authentication, refresh rotation, and backend headers can be tested offline.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

use super::{
    ModelInfo, Provider, ProviderError, ProviderStream, checked_json, checked_stream_response,
    normalize_models,
};
use crate::model::{ModelRequest, ModelResponse};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REFRESH_SKEW_SECS: u64 = 300;

#[derive(Clone, Debug)]
pub struct OAuthEndpoints {
    pub authorize: String,
    pub token: String,
    pub device_user_code: String,
    pub device_token: String,
    pub responses: String,
    pub models: String,
}

#[async_trait]
impl Provider for ChatGptOAuth {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let tokens = self.valid_tokens().await?;
        let response = self
            .client
            .get(&self.endpoints.models)
            // OpenAI's own catalog-refresh workflow uses this sentinel so new
            // models are not hidden behind an unrelated client release number.
            .query(&[("client_version", "99.99.99")])
            .bearer_auth(tokens.access_token)
            .header("ChatGPT-Account-Id", tokens.account_id)
            .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(map_request)?;
        let value = checked_json(response).await?;
        let entries = value
            .get("models")
            .or_else(|| value.get("data"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("ChatGPT models response omitted models".into())
            })?;
        let mut models = entries
            .iter()
            .filter(|entry| {
                entry.get("supported_in_api").and_then(Value::as_bool) != Some(false)
                    && entry.get("visibility").and_then(Value::as_str) != Some("hide")
            })
            .filter_map(|entry| {
                let id = entry
                    .get("slug")
                    .or_else(|| entry.get("id"))
                    .and_then(Value::as_str)?
                    .to_owned();
                let reasoning_efforts = entry
                    .get("supported_reasoning_levels")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|level| level.get("effort").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect();
                Some(ModelInfo {
                    display_name: entry
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_owned(),
                    description: entry
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    is_default: entry
                        .get("is_default")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    reasoning_efforts,
                    input_modalities: vec!["text".into()],
                    id,
                })
            })
            .collect();
        normalize_models(&mut models);
        Ok(models)
    }

    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let mut body = super::openai_responses::request_body(request, false)?;
        body.as_object_mut()
            .map(|value| value.remove("max_output_tokens"));
        let response = self
            .responses_request(&body)
            .await?
            .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(map_request)?;
        super::openai_responses::decode_response(checked_json(response).await?)
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let mut body = super::openai_responses::request_body(request, true)?;
        body.as_object_mut()
            .map(|value| value.remove("max_output_tokens"));
        let response = self
            .responses_request(&body)
            .await?
            .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(map_request)?;
        let response = checked_stream_response(response).await?;
        Ok(Box::pin(super::openai_responses::responses_stream(
            response.bytes_stream(),
        )))
    }
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            authorize: "https://auth.openai.com/oauth/authorize".into(),
            token: "https://auth.openai.com/oauth/token".into(),
            device_user_code: "https://auth.openai.com/api/accounts/deviceauth/usercode".into(),
            device_token: "https://auth.openai.com/api/accounts/deviceauth/token".into(),
            responses: "https://chatgpt.com/backend-api/codex/responses".into(),
            models: "https://chatgpt.com/backend-api/codex/models".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    pub expires_at: u64,
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenStatus {
    pub authenticated: bool,
    pub account_id: Option<String>,
    pub expires_at: Option<u64>,
    pub refreshable: bool,
}

#[derive(Clone, Debug)]
pub struct PkceAuthorization {
    pub url: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub device_auth_id: String,
    pub user_code: String,
    #[serde(default = "device_verification_uri")]
    pub verification_uri: String,
    #[serde(default = "default_poll_interval")]
    pub interval: u64,
}

#[derive(Deserialize)]
struct DeviceToken {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Clone)]
pub struct TokenStore {
    path: PathBuf,
}

/// Public registry-facing name for the native subscription credential store.
pub type ChatGptTokenStore = TokenStore;

impl TokenStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> Result<PathBuf, ProviderError> {
        dirs::data_local_dir()
            .map(|root| root.join("helm").join("chatgpt-oauth.json"))
            .ok_or_else(|| {
                ProviderError::Authentication("cannot resolve Helm data directory".into())
            })
    }

    pub async fn load(&self) -> Result<Option<OAuthTokens>, ProviderError> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|error| {
                ProviderError::Authentication(format!("invalid ChatGPT token cache: {error}"))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ProviderError::Authentication(format!(
                "cannot read ChatGPT token cache: {error}"
            ))),
        }
    }

    pub async fn save(&self, tokens: &OAuthTokens) -> Result<(), ProviderError> {
        validate_tokens(tokens)?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| ProviderError::Authentication("token cache has no parent".into()))?;
        tokio::fs::create_dir_all(parent).await.map_err(auth_io)?;
        set_mode(parent, 0o700).await?;
        let temporary = self
            .path
            .with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temporary).await.map_err(auth_io)?;
        set_mode(&temporary, 0o600).await?;
        file.write_all(
            &serde_json::to_vec(tokens)
                .map_err(|error| ProviderError::Authentication(error.to_string()))?,
        )
        .await
        .map_err(auth_io)?;
        file.sync_all().await.map_err(auth_io)?;
        if let Err(error) = tokio::fs::rename(&temporary, &self.path).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(auth_io(error));
        }
        set_mode(&self.path, 0o600).await?;
        Ok(())
    }

    pub async fn clear(&self) -> Result<(), ProviderError> {
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(auth_io(error)),
        }
    }

    pub async fn status(&self) -> Result<TokenStatus, ProviderError> {
        let tokens = self.load().await?;
        Ok(TokenStatus {
            authenticated: tokens.is_some(),
            account_id: tokens.as_ref().map(|value| value.account_id.clone()),
            expires_at: tokens.as_ref().map(|value| value.expires_at),
            refreshable: tokens
                .as_ref()
                .is_some_and(|value| !value.refresh_token.is_empty()),
        })
    }

    /// Imports a Codex login once as migration data; Codex is never invoked.
    pub async fn import_codex(
        &self,
        source: &Path,
        overwrite: bool,
    ) -> Result<OAuthTokens, ProviderError> {
        if !overwrite && self.load().await?.is_some() {
            return Err(ProviderError::Authentication(
                "Helm ChatGPT credentials already exist".into(),
            ));
        }
        let document: Value =
            serde_json::from_slice(&tokio::fs::read(source).await.map_err(auth_io)?).map_err(
                |error| ProviderError::Authentication(format!("invalid Codex auth file: {error}")),
            )?;
        let source = document.get("tokens").unwrap_or(&document);
        let access_token = required_string(source, "access_token")?;
        let refresh_token = required_string(source, "refresh_token")?;
        let id_token = source
            .get("id_token")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let account_id = source
            .get("account_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| id_token.as_deref().and_then(account_id_from_jwt))
            .or_else(|| account_id_from_jwt(&access_token))
            .ok_or_else(|| {
                ProviderError::Authentication("Codex auth omitted ChatGPT account ID".into())
            })?;
        let expires_at = jwt_expiry(&access_token)
            .or_else(|| id_token.as_deref().and_then(jwt_expiry))
            .unwrap_or_else(|| now_secs().saturating_add(3600));
        let tokens = OAuthTokens {
            access_token,
            refresh_token,
            id_token,
            expires_at,
            account_id,
        };
        self.save(&tokens).await?;
        Ok(tokens)
    }

    pub async fn import_default_codex(
        &self,
        overwrite: bool,
    ) -> Result<OAuthTokens, ProviderError> {
        let source = dirs::home_dir()
            .ok_or_else(|| ProviderError::Authentication("cannot resolve home directory".into()))?
            .join(".codex")
            .join("auth.json");
        self.import_codex(&source, overwrite).await
    }
}

pub struct ChatGptOAuth {
    client: reqwest::Client,
    endpoints: OAuthEndpoints,
    store: TokenStore,
    tokens: Arc<Mutex<Option<OAuthTokens>>>,
}

/// Public registry-facing name for the native subscription transport seam.
pub type ChatGptOauthProvider = ChatGptOAuth;

impl ChatGptOAuth {
    pub fn from_store(store: TokenStore, endpoints: OAuthEndpoints) -> Self {
        Self {
            client: super::native_http_client(),
            endpoints,
            store,
            tokens: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn new(store: TokenStore, endpoints: OAuthEndpoints) -> Result<Self, ProviderError> {
        let tokens = store.load().await?;
        Ok(Self {
            client: super::native_http_client(),
            endpoints,
            store,
            tokens: Arc::new(Mutex::new(tokens)),
        })
    }

    pub fn begin_pkce(&self, redirect_uri: impl Into<String>) -> PkceAuthorization {
        let verifier = format!(
            "{}{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let state = uuid::Uuid::new_v4().to_string();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect_uri = redirect_uri.into();
        let query = form_urlencoded(&[
            ("response_type", "code"),
            ("client_id", CLIENT_ID),
            ("redirect_uri", &redirect_uri),
            ("scope", "openid profile email offline_access"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "helm"),
        ]);
        PkceAuthorization {
            url: format!("{}?{query}", self.endpoints.authorize),
            verifier,
            state,
            redirect_uri,
        }
    }

    /// Completes browser PKCE login through an owner-local callback.
    ///
    /// `present` should open or print the authorization URL. It is deliberately
    /// caller-owned so this module never shells out to a browser helper.
    pub async fn login_browser<F>(
        &self,
        timeout: Duration,
        present: F,
    ) -> Result<OAuthTokens, ProviderError>
    where
        F: FnOnce(&str),
    {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 1455)).await {
            Ok(listener) => listener,
            Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 1457))
                .await
                .map_err(|error| {
                    ProviderError::Authentication(format!(
                        "cannot bind OAuth callback ports 1455 or 1457: {error}"
                    ))
                })?,
        };
        let port = listener.local_addr().map_err(auth_io)?.port();
        let flow = self.begin_pkce(format!("http://127.0.0.1:{port}/auth/callback"));
        present(&flow.url);
        let (mut stream, _) = tokio::time::timeout(timeout, listener.accept())
            .await
            .map_err(|_| ProviderError::Timeout("ChatGPT browser login timed out".into()))?
            .map_err(auth_io)?;
        let callback = read_callback(&mut stream, timeout).await;
        let result = match callback {
            Ok(parameters) => {
                if parameters.get("state") != Some(&flow.state) {
                    Err(ProviderError::Authentication(
                        "OAuth callback state did not match".into(),
                    ))
                } else if let Some(error) = parameters.get("error") {
                    Err(ProviderError::Authentication(format!(
                        "ChatGPT authorization was rejected: {error}"
                    )))
                } else {
                    let code = parameters.get("code").ok_or_else(|| {
                        ProviderError::Authentication("OAuth callback omitted code".into())
                    })?;
                    self.exchange_code(code, &flow).await
                }
            }
            Err(error) => Err(error),
        };
        let (status, message) = if result.is_ok() {
            ("200 OK", "Helm is signed in. You may close this window.")
        } else {
            (
                "400 Bad Request",
                "Helm could not complete sign-in. Return to the terminal.",
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{message}",
            message.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        result
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        flow: &PkceAuthorization,
    ) -> Result<OAuthTokens, ProviderError> {
        let tokens = self
            .exchange(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", code),
                ("code_verifier", &flow.verifier),
                ("redirect_uri", &flow.redirect_uri),
            ])
            .await?;
        self.replace(tokens.clone()).await?;
        Ok(tokens)
    }

    pub async fn begin_device(&self) -> Result<DeviceAuthorization, ProviderError> {
        let response = self
            .client
            .post(&self.endpoints.device_user_code)
            .json(&serde_json::json!({"client_id": CLIENT_ID}))
            .send()
            .await
            .map_err(map_request)?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            return Err(ProviderError::Authentication(format!(
                "device authorization failed with HTTP {}",
                response.status()
            )));
        }
        response.json().await.map_err(|error| {
            ProviderError::InvalidResponse(format!(
                "invalid device authorization response: {error}"
            ))
        })
    }

    /// Performs one device-flow poll; callers retry pending responses after `interval`.
    pub async fn poll_device(
        &self,
        device: &DeviceAuthorization,
    ) -> Result<OAuthTokens, ProviderError> {
        let response = self.client.post(&self.endpoints.device_token).json(&serde_json::json!({"device_auth_id":device.device_auth_id,"user_code":device.user_code})).send().await.map_err(map_request)?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            return match response.status().as_u16() {
                403 | 404 => Err(ProviderError::Unavailable(
                    "device authorization pending".into(),
                )),
                status => Err(ProviderError::Authentication(format!(
                    "device authorization poll failed with HTTP {status}"
                ))),
            };
        }
        let grant: DeviceToken = response.json().await.map_err(|error| {
            ProviderError::InvalidResponse(format!("invalid device token response: {error}"))
        })?;
        let tokens = self
            .exchange(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT_ID),
                ("code", &grant.authorization_code),
                ("code_verifier", &grant.code_verifier),
                (
                    "redirect_uri",
                    "https://auth.openai.com/deviceauth/callback",
                ),
            ])
            .await?;
        self.replace(tokens.clone()).await?;
        Ok(tokens)
    }

    pub async fn logout(&self) -> Result<(), ProviderError> {
        self.store.clear().await?;
        *self.tokens.lock().await = None;
        Ok(())
    }

    pub async fn status(&self) -> TokenStatus {
        let mut tokens = self.tokens.lock().await;
        if tokens.is_none() {
            *tokens = self.store.load().await.ok().flatten();
        }
        TokenStatus {
            authenticated: tokens.is_some(),
            account_id: tokens.as_ref().map(|value| value.account_id.clone()),
            expires_at: tokens.as_ref().map(|value| value.expires_at),
            refreshable: tokens
                .as_ref()
                .is_some_and(|value| !value.refresh_token.is_empty()),
        }
    }

    pub async fn responses_request(
        &self,
        body: &Value,
    ) -> Result<reqwest::RequestBuilder, ProviderError> {
        let tokens = self.valid_tokens().await?;
        Ok(self
            .client
            .post(&self.endpoints.responses)
            .bearer_auth(tokens.access_token)
            .header("ChatGPT-Account-Id", tokens.account_id)
            .header("originator", "helm")
            .header("OpenAI-Beta", "responses=experimental")
            .json(body))
    }

    async fn valid_tokens(&self) -> Result<OAuthTokens, ProviderError> {
        let mut guard = self.tokens.lock().await;
        if guard.is_none() {
            *guard = self.store.load().await?;
        }
        let current = guard
            .clone()
            .ok_or_else(|| ProviderError::Authentication("ChatGPT login required".into()))?;
        if current.expires_at > now_secs().saturating_add(REFRESH_SKEW_SECS) {
            return Ok(current);
        }
        let mut refreshed = self
            .exchange(&[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", &current.refresh_token),
            ])
            .await?;
        if refreshed.refresh_token.is_empty() {
            refreshed.refresh_token = current.refresh_token;
        }
        self.store.save(&refreshed).await?;
        *guard = Some(refreshed.clone());
        Ok(refreshed)
    }

    async fn replace(&self, tokens: OAuthTokens) -> Result<(), ProviderError> {
        self.store.save(&tokens).await?;
        *self.tokens.lock().await = Some(tokens);
        Ok(())
    }

    async fn exchange(&self, form: &[(&str, &str)]) -> Result<OAuthTokens, ProviderError> {
        let response = self
            .client
            .post(&self.endpoints.token)
            .form(form)
            .send()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            return Err(ProviderError::Authentication(format!(
                "ChatGPT OAuth exchange failed with HTTP {}",
                response.status()
            )));
        }
        let raw: TokenResponse = response.json().await.map_err(|error| {
            ProviderError::InvalidResponse(format!("invalid OAuth response: {error}"))
        })?;
        let account_id = raw
            .account_id
            .or_else(|| raw.id_token.as_deref().and_then(account_id_from_jwt))
            .or_else(|| account_id_from_jwt(&raw.access_token))
            .ok_or_else(|| {
                ProviderError::Authentication("OAuth tokens omitted ChatGPT account ID".into())
            })?;
        let refresh_token = raw.refresh_token.unwrap_or_default();
        let tokens = OAuthTokens {
            access_token: raw.access_token,
            refresh_token,
            id_token: raw.id_token,
            expires_at: now_secs().saturating_add(raw.expires_in),
            account_id,
        };
        Ok(tokens)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    #[serde(default = "default_expiry")]
    expires_in: u64,
    account_id: Option<String>,
}
fn default_expiry() -> u64 {
    3600
}
fn default_poll_interval() -> u64 {
    5
}
fn device_verification_uri() -> String {
    "https://auth.openai.com/codex/device".into()
}
fn map_request(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError::Timeout(error.to_string())
    } else {
        ProviderError::Request(error.to_string())
    }
}

fn validate_tokens(value: &OAuthTokens) -> Result<(), ProviderError> {
    if value.access_token.is_empty()
        || value.refresh_token.is_empty()
        || value.account_id.is_empty()
        || value.expires_at == 0
    {
        return Err(ProviderError::Authentication(
            "ChatGPT token set is incomplete".into(),
        ));
    }
    Ok(())
}
fn required_string(value: &Value, key: &str) -> Result<String, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Authentication(format!("Codex auth omitted {key}")))
}
fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}
fn jwt_expiry(token: &str) -> Option<u64> {
    jwt_payload(token)?.get("exp")?.as_u64()
}
pub fn account_id_from_jwt(token: &str) -> Option<String> {
    let claims = jwt_payload(token)?;
    claims
        .get("chatgpt_account_id")
        .or_else(|| claims.get("account_id"))
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
        })
        .or_else(|| claims.pointer("/organizations/0/id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}
fn form_urlencoded(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{}={}", percent(key), percent(value)))
        .collect::<Vec<_>>()
        .join("&")
}
fn percent(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
fn auth_io(error: std::io::Error) -> ProviderError {
    ProviderError::Authentication(error.to_string())
}

async fn read_callback(
    stream: &mut tokio::net::TcpStream,
    timeout: Duration,
) -> Result<BTreeMap<String, String>, ProviderError> {
    let mut bytes = vec![0_u8; 16 * 1024];
    let read = tokio::time::timeout(timeout, stream.read(&mut bytes))
        .await
        .map_err(|_| ProviderError::Timeout("OAuth callback read timed out".into()))?
        .map_err(auth_io)?;
    let line = std::str::from_utf8(&bytes[..read])
        .map_err(|_| ProviderError::Authentication("OAuth callback was not UTF-8".into()))?
        .lines()
        .next()
        .ok_or_else(|| ProviderError::Authentication("OAuth callback was empty".into()))?;
    let target = line
        .strip_prefix("GET ")
        .and_then(|line| line.split_once(' '))
        .map(|(target, _)| target)
        .ok_or_else(|| {
            ProviderError::Authentication("OAuth callback was not a GET request".into())
        })?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" {
        return Err(ProviderError::Authentication(
            "OAuth callback path was invalid".into(),
        ));
    }
    query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Ok((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_decode(value: &str) -> Result<String, ProviderError> {
    let value = value.replace('+', " ");
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(ProviderError::Authentication(
                    "invalid OAuth callback encoding".into(),
                ));
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).map_err(|_| {
                ProviderError::Authentication("invalid OAuth callback encoding".into())
            })?;
            output.push(u8::from_str_radix(hex, 16).map_err(|_| {
                ProviderError::Authentication("invalid OAuth callback encoding".into())
            })?);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output)
        .map_err(|_| ProviderError::Authentication("invalid OAuth callback encoding".into()))
}
#[cfg(unix)]
async fn set_mode(path: &Path, mode: u32) -> Result<(), ProviderError> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .await
        .map_err(auth_io)
}
#[cfg(not(unix))]
async fn set_mode(_: &Path, _: u32) -> Result<(), ProviderError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn jwt(claims: Value) -> String {
        format!(
            "e30.{}.sig",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    async fn fixture(response: Value, requests: Arc<StdMutex<Vec<String>>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let response = response.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    let mut bytes = vec![0; 8192];
                    let read = stream.read(&mut bytes).await.unwrap();
                    requests
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&bytes[..read]).into());
                    let body = serde_json::to_vec(&response).unwrap();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    stream.write_all(&body).await.unwrap();
                });
            }
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn cache_is_private_and_codex_import_is_one_time() {
        let directory = tempfile::tempdir().unwrap();
        let store = TokenStore::new(directory.path().join("helm/auth.json"));
        let token = jwt(
            serde_json::json!({"exp": 9999999999_u64, "https://api.openai.com/auth": {"chatgpt_account_id":"acct-1"}}),
        );
        let legacy = directory.path().join("codex.json");
        tokio::fs::write(&legacy, serde_json::to_vec(&serde_json::json!({"tokens":{"access_token":token,"refresh_token":"refresh","id_token":null}})).unwrap()).await.unwrap();
        let imported = store.import_codex(&legacy, false).await.unwrap();
        assert_eq!(imported.account_id, "acct-1");
        assert!(store.import_codex(&legacy, false).await.is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                tokio::fs::metadata(directory.path().join("helm/auth.json"))
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        store.clear().await.unwrap();
        assert!(!store.status().await.unwrap().authenticated);
    }

    #[tokio::test]
    async fn extracts_supported_account_claims_and_builds_pkce() {
        let token = jwt(serde_json::json!({"chatgpt_account_id":"acct-direct"}));
        assert_eq!(account_id_from_jwt(&token).as_deref(), Some("acct-direct"));
        let directory = tempfile::tempdir().unwrap();
        let auth = ChatGptOAuth::new(
            TokenStore::new(directory.path().join("auth.json")),
            OAuthEndpoints::default(),
        )
        .await
        .unwrap();
        let flow = auth.begin_pkce("http://127.0.0.1:1455/auth/callback");
        assert!(flow.url.contains("code_challenge_method=S256"));
        assert!(
            flow.url
                .contains("scope=openid%20profile%20email%20offline_access")
        );
        assert!(flow.verifier.len() >= 64);
    }

    #[tokio::test]
    async fn refresh_rotates_cache_and_request_has_account_header() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let access = jwt(serde_json::json!({"chatgpt_account_id":"acct-new","exp":9999999999_u64}));
        let endpoint = fixture(
            serde_json::json!({"access_token":access,"refresh_token":"rotated","expires_in":3600}),
            requests.clone(),
        )
        .await;
        let directory = tempfile::tempdir().unwrap();
        let store = TokenStore::new(directory.path().join("auth.json"));
        store
            .save(&OAuthTokens {
                access_token: "expired".into(),
                refresh_token: "old-refresh".into(),
                id_token: None,
                expires_at: 1,
                account_id: "acct-old".into(),
            })
            .await
            .unwrap();
        let auth = ChatGptOAuth::new(
            store.clone(),
            OAuthEndpoints {
                authorize: format!("{endpoint}/authorize"),
                token: format!("{endpoint}/token"),
                device_user_code: format!("{endpoint}/device/usercode"),
                device_token: format!("{endpoint}/device/token"),
                responses: format!("{endpoint}/responses"),
                models: format!("{endpoint}/models"),
            },
        )
        .await
        .unwrap();
        let request = auth
            .responses_request(&serde_json::json!({"model":"gpt"}))
            .await
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.headers()["ChatGPT-Account-Id"], "acct-new");
        assert_eq!(request.headers()["originator"], "helm");
        assert_eq!(
            store.load().await.unwrap().unwrap().refresh_token,
            "rotated"
        );
        let sent = requests.lock().unwrap().join("\n");
        assert!(sent.contains("old-refresh"));
        assert!(sent.contains("grant_type=refresh_token"));
    }

    #[tokio::test]
    async fn browser_callback_validates_state_and_exchanges_without_exposing_code() {
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let access =
            jwt(serde_json::json!({"chatgpt_account_id":"acct-browser","exp":9999999999_u64}));
        let endpoint = fixture(
            serde_json::json!({"access_token":access,"refresh_token":"refresh","expires_in":3600}),
            requests,
        )
        .await;
        let directory = tempfile::tempdir().unwrap();
        let auth = ChatGptOAuth::new(
            TokenStore::new(directory.path().join("auth.json")),
            OAuthEndpoints {
                authorize: format!("{endpoint}/authorize"),
                token: format!("{endpoint}/token"),
                device_user_code: format!("{endpoint}/device/usercode"),
                device_token: format!("{endpoint}/device/token"),
                responses: format!("{endpoint}/responses"),
                models: format!("{endpoint}/models"),
            },
        )
        .await
        .unwrap();
        let tokens = auth.login_browser(Duration::from_secs(3), |url| {
            let state = url.split('&').find_map(|part| part.strip_prefix("state=")).unwrap().to_owned();
            tokio::spawn(async move {
                let mut stream = match tokio::net::TcpStream::connect(("127.0.0.1", 1455)).await {
                    Ok(stream) => stream,
                    Err(_) => tokio::net::TcpStream::connect(("127.0.0.1", 1457)).await.unwrap(),
                };
                stream.write_all(format!("GET /auth/callback?code=private-code&state={state} HTTP/1.1\r\nhost: localhost\r\n\r\n").as_bytes()).await.unwrap();
                let mut response = Vec::new(); stream.read_to_end(&mut response).await.unwrap();
                let response = String::from_utf8(response).unwrap();
                assert!(response.contains("200 OK")); assert!(!response.contains("private-code"));
            });
        }).await.unwrap();
        assert_eq!(tokens.account_id, "acct-browser");
    }
}
