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

#[derive(Debug, thiserror::Error)]
#[error("OAuth refresh pending or uncertain")]
pub(crate) struct RefreshPending;

const REFRESH_PENDING: &str = "OAuth refresh pending or uncertain; wait for the owning exchange or explicitly reauthenticate if it stopped";

pub(crate) mod storage;

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
        super::validate_native_endpoint(&self.endpoints.models)?;
        let tokens = self.valid_tokens().await?;
        let response = super::endpoint_http_client(&self.client, &self.endpoints.models)
            .get(&self.endpoints.models)
            // OpenAI's own catalog-refresh workflow uses this sentinel so new
            // models are not hidden behind an unrelated client release number.
            .query(&[("client_version", "99.99.99")])
            .bearer_auth(&tokens.access_token)
            .header("ChatGPT-Account-Id", &tokens.account_id)
            .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await
            .map_err(super::catalog::transport)?;
        let mut remaining = super::catalog::MAX_BYTES;
        let value = super::catalog::json(response, &mut remaining).await?;
        let entries = value
            .get("models")
            .or_else(|| value.get("data"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("ChatGPT models response omitted models".into())
            })?;
        if entries.len() > super::catalog::MAX_MODELS {
            return Err(ProviderError::InvalidResponse(
                "model list exceeds 1024 entries".into(),
            ));
        }
        let mut models = entries
            .iter()
            .filter(|entry| {
                entry.get("supported_in_api").and_then(Value::as_bool) != Some(false)
                    && entry.get("visibility").and_then(Value::as_str) != Some("hide")
            })
            .map(|entry| {
                let id = entry
                    .get("slug")
                    .or_else(|| entry.get("id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| ProviderError::InvalidResponse("model entry omitted ID".into()))?
                    .to_owned();
                let reasoning_efforts =
                    super::catalog::strings(entry, "supported_reasoning_levels", Some("effort"))?;
                Ok(ModelInfo {
                    display_name: super::catalog::optional_text(entry, "display_name", &id)?
                        .to_owned(),
                    description: super::catalog::optional_text(entry, "description", "")?
                        .to_owned(),
                    is_default: super::catalog::optional_bool(entry, "is_default", false)?,
                    reasoning_efforts,
                    reasoning_support_known: entry
                        .get("supported_reasoning_levels")
                        .is_some_and(Value::is_array),
                    default_reasoning_effort: super::catalog::nullable_text(
                        entry,
                        "default_reasoning_level",
                    )?,
                    service_tiers: super::catalog::strings(entry, "service_tiers", Some("id"))?,
                    service_support_known: entry.get("service_tiers").is_some_and(Value::is_array),
                    default_service_tier: super::catalog::nullable_text(
                        entry,
                        "default_service_tier",
                    )?,
                    observed_at_ms: Some(super::catalog::now_ms()),
                    input_modalities: super::multimodal::discovered_modalities(entry)?,
                    id,
                })
            })
            .collect::<Result<Vec<_>, ProviderError>>()?;
        super::validate_models(
            &models,
            &[
                &tokens.access_token,
                &tokens.refresh_token,
                tokens.id_token.as_deref().unwrap_or(""),
                &tokens.account_id,
            ],
        )?;
        normalize_models(&mut models);
        Ok(models)
    }

    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(self, &crate::config::ProviderKind::ChatGptOauth, &request)
            .await?;
        let result: Result<ModelResponse, ProviderError> =
            super::multimodal::guard(images, async {
                super::inference::validate_request(
                    &crate::config::ProviderKind::ChatGptOauth,
                    &request,
                )?;
                let mut body = super::openai_responses::request_body_for(
                    &crate::ProviderKind::ChatGptOauth,
                    subscription_request(request),
                    false,
                )?;
                body.as_object_mut()
                    .map(|value| value.remove("max_output_tokens"));
                super::multimodal::check_body(&body)?;
                let (request, tokens) = self.responses_request_with_tokens(&body).await?;
                let response = request
                    .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
                    .send()
                    .await
                    .map_err(map_request)?;
                let mut response =
                    super::openai_responses::decode_response(checked_json(response).await?)?;
                filter_response_tier(&mut response, &tokens);
                Ok(response)
            })
            .await;
        result
    }

    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let images = super::multimodal::has_images(&request);
        super::multimodal::preflight(self, &crate::config::ProviderKind::ChatGptOauth, &request)
            .await?;
        let result: Result<ProviderStream, ProviderError> =
            super::multimodal::guard(images, async {
                super::inference::validate_request(
                    &crate::config::ProviderKind::ChatGptOauth,
                    &request,
                )?;
                let mut body = super::openai_responses::request_body_for(
                    &crate::ProviderKind::ChatGptOauth,
                    subscription_request(request),
                    true,
                )?;
                body.as_object_mut()
                    .map(|value| value.remove("max_output_tokens"));
                super::multimodal::check_body(&body)?;
                let (request, tokens) = self.responses_request_with_tokens(&body).await?;
                let response = request
                    .header("User-Agent", format!("helm/{}", env!("CARGO_PKG_VERSION")))
                    .send()
                    .await
                    .map_err(map_request)?;
                let response = checked_stream_response(response).await?;
                use futures_util::StreamExt;
                Ok(Box::pin(
                    super::observed_stream(response, |response| {
                        Box::pin(super::openai_responses::responses_stream(
                            response.bytes_stream(),
                        ))
                    })
                    .map(move |event| {
                        event.map(|mut event| {
                            if let super::ProviderStreamEvent::Completed(response) = &mut event {
                                filter_response_tier(response, &tokens);
                            }
                            event
                        })
                    }),
                ) as ProviderStream)
            })
            .await;
        result.map(|stream| super::multimodal::guard_stream(images, stream))
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

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Clone)]
pub struct PkceAuthorization {
    pub url: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeviceAuthorization {
    pub device_auth_id: String,
    pub user_code: String,
    #[serde(default = "device_verification_uri")]
    pub verification_uri: String,
    #[serde(
        default = "default_poll_interval",
        deserialize_with = "device_poll_interval"
    )]
    pub interval: u64,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

// The native device endpoint encodes interval as a decimal string; retain
// numeric responses as well for compatible providers and saved fixtures.
fn device_poll_interval<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Interval {
        Number(u64),
        Text(String),
    }
    match Interval::deserialize(deserializer)? {
        Interval::Number(value) => Ok(value),
        Interval::Text(value) => value
            .parse()
            .map_err(|_| serde::de::Error::custom("invalid device polling interval")),
    }
}

#[derive(Deserialize)]
pub(crate) struct DeviceToken {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Clone)]
pub struct TokenStore {
    path: PathBuf,
    binding: Option<(
        crate::accounts::Registry,
        voyage_protocol::accounts::AccountBinding,
    )>,
    legacy_default: bool,
    initialization_error: bool,
}

/// Public registry-facing name for the native subscription credential store.
pub type ChatGptTokenStore = TokenStore;

impl TokenStore {
    pub fn new(path: PathBuf) -> Self {
        let legacy_default = Self::default_path().is_ok_and(|default| path == default);
        let resolved = if legacy_default {
            crate::accounts::Registry::legacy_store_binding()
        } else {
            Ok(None)
        };
        let initialization_error = resolved.is_err();
        Self {
            path,
            binding: resolved.ok().flatten(),
            legacy_default,
            initialization_error,
        }
    }

    pub(crate) fn bound(
        registry: crate::accounts::Registry,
        binding: voyage_protocol::accounts::AccountBinding,
    ) -> Self {
        Self {
            path: PathBuf::new(),
            binding: Some((registry, binding)),
            legacy_default: false,
            initialization_error: false,
        }
    }

    // Retain the legacy filename. After explicit registry migration, new stores
    // resolve its stable account UUID; reads never initiate migration or resurrect login.
    pub fn default_path() -> Result<PathBuf, ProviderError> {
        dirs::data_local_dir()
            .map(|root| root.join("helm").join("chatgpt-oauth.json"))
            .ok_or_else(|| {
                ProviderError::Authentication(
                    "cannot resolve local credential data directory".into(),
                )
            })
    }

    pub async fn load(&self) -> Result<Option<OAuthTokens>, ProviderError> {
        if self.initialization_error {
            return Err(private_cache_error());
        }
        // Instances created before migration must stop rather than read the retained old cache.
        if self.legacy_default
            && self.binding.is_none()
            && crate::accounts::Registry::legacy_store_binding()
                .map_err(account_error)?
                .is_some()
        {
            return Err(ProviderError::Authentication(
                "legacy credentials migrated; recreate provider with the account binding".into(),
            ));
        }
        // Re-read after bounded waits so independent voyages reuse completed rotation.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
        loop {
            let result = if let Some((registry, binding)) = &self.binding {
                if self.legacy_default
                    && registry.legacy_logged_out(binding).map_err(account_error)?
                {
                    Ok(None)
                } else {
                    registry
                        .oauth_load(binding)
                        .map(Some)
                        .map_err(account_error)
                }
            } else {
                let path = self.path.clone();
                let bytes = tokio::task::spawn_blocking(move || storage::read_tokens(&path))
                    .await
                    .map_err(|_| private_cache_error())?
                    .map_err(account_error);
                bytes.and_then(|bytes| match bytes {
                    None => Ok(None),
                    Some(bytes) => serde_json::from_slice::<Option<OAuthTokens>>(&bytes)
                        .map_err(|_| private_cache_error()),
                })
            };
            match result {
                Ok(tokens) => {
                    if let Some(t) = &tokens {
                        validate_tokens(t)?;
                    }
                    return Ok(tokens);
                }
                Err(ProviderError::Unavailable(message))
                    if message == REFRESH_PENDING && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    async fn private_bytes(&self) -> Result<Option<Vec<u8>>, ProviderError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || storage::read(&path))
            .await
            .map_err(|_| private_cache_error())?
            .map_err(|_| private_cache_error())
    }

    async fn publish(&self, bytes: Option<Vec<u8>>) -> Result<(), ProviderError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || storage::write(&path, bytes.as_deref()))
            .await
            .map_err(|_| private_cache_error())?
            .map_err(|_| private_cache_error())
    }

    pub async fn save(&self, tokens: &OAuthTokens) -> Result<(), ProviderError> {
        validate_tokens(tokens)?;
        if self.initialization_error {
            return Err(private_cache_error());
        }
        if let Some((registry, binding)) = &self.binding {
            if self.legacy_default {
                return registry
                    .reauthenticate_oauth(
                        binding.account_id,
                        binding.identity_generation,
                        tokens.clone(),
                        true,
                    )
                    .map(|_| ())
                    .map_err(account_error);
            }
            return registry
                .oauth_save(binding, tokens, None)
                .map_err(account_error);
        }
        if self.legacy_default
            && crate::accounts::Registry::legacy_store_binding()
                .map_err(account_error)?
                .is_some()
        {
            return Err(ProviderError::Authentication(
                "legacy credentials migrated; restart authentication command".into(),
            ));
        }
        let bytes = serde_json::to_vec(tokens).map_err(|_| private_cache_error())?;
        if bytes.len() > storage::LIMIT {
            return Err(private_cache_error());
        }
        self.publish(Some(bytes)).await
    }

    pub async fn clear(&self) -> Result<(), ProviderError> {
        if self.initialization_error {
            return Err(private_cache_error());
        }
        if self.legacy_default
            && self.binding.is_none()
            && crate::accounts::Registry::legacy_store_binding()
                .map_err(account_error)?
                .is_some()
        {
            return Err(ProviderError::Authentication(
                "legacy credentials migrated; restart authentication command".into(),
            ));
        }
        if let Some((registry, binding)) = &self.binding {
            return registry
                .logout(binding.account_id, false)
                .map_err(account_error);
        }
        self.publish(None).await
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
                "Local ChatGPT credentials already exist".into(),
            ));
        }
        let tokens = Self::read_import_tokens(source).await?;
        self.save(&tokens).await?;
        Ok(tokens)
    }

    /// Private explicit import input. Does not execute Codex or publish credentials.
    pub async fn read_import_tokens(source: &Path) -> Result<OAuthTokens, ProviderError> {
        let bytes = Self::new(source.to_owned())
            .private_bytes()
            .await?
            .ok_or_else(private_cache_error)?;
        let document: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::Authentication("invalid Codex auth file".into()))?;
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
        validate_tokens(&tokens)?;
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
    redactor: Option<Arc<crate::tools::Redactor>>,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
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
            authority: None,
            redactor: None,
            client: super::native_http_client(),
            endpoints,
            store,
            tokens: Arc::new(Mutex::new(None)),
        }
    }
    pub(crate) fn with_redactor(mut self, redactor: Option<Arc<crate::tools::Redactor>>) -> Self {
        self.redactor = redactor;
        self
    }
    fn remember_tokens(&self, tokens: &OAuthTokens) -> Result<(), ProviderError> {
        if let Some(redactor) = &self.redactor {
            for secret in [
                &tokens.access_token,
                &tokens.refresh_token,
                tokens.id_token.as_deref().unwrap_or(""),
                &tokens.account_id,
            ] {
                redactor.remember_credential(secret).map_err(|_| {
                    ProviderError::Authentication("credential redaction unavailable".into())
                })?;
            }
        }
        Ok(())
    }
    pub(crate) fn with_authority(
        mut self,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> Self {
        self.authority = authority;
        self
    }

    pub async fn new(store: TokenStore, endpoints: OAuthEndpoints) -> Result<Self, ProviderError> {
        let tokens = store.load().await?;
        Ok(Self {
            authority: None,
            redactor: None,
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
        super::validate_native_endpoint(&self.endpoints.authorize)?;
        super::validate_native_endpoint(&self.endpoints.token)?;
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
        // OAuth redirect matching distinguishes localhost from its loopback IP.
        // Advertise the upstream callback hostname while binding only loopback.
        let flow = self.begin_pkce(format!("http://localhost:{port}/auth/callback"));
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
            ("200 OK", "Vessel is signed in. You may close this window.")
        } else {
            (
                "400 Bad Request",
                "Vessel could not complete sign-in. Return to the terminal.",
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
        super::validate_native_endpoint(&self.endpoints.device_user_code)?;
        let response = super::endpoint_http_client(&self.client, &self.endpoints.device_user_code)
            .post(&self.endpoints.device_user_code)
            .timeout(Duration::from_secs(30))
            .json(&serde_json::json!({"client_id": CLIENT_ID}))
            .send()
            .await
            .map_err(map_request)?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            return Err(ProviderError::Authentication(format!(
                "device authorization failed with HTTP {}",
                response.status()
            ))
            .with_http_status(response.status().as_u16()));
        }
        auth_json(response).await
    }

    /// Performs one device-flow poll; callers retry pending responses after `interval`.
    pub async fn poll_device(
        &self,
        device: &DeviceAuthorization,
    ) -> Result<OAuthTokens, ProviderError> {
        let tokens = self.poll_device_unpublished(device).await?;
        self.replace(tokens.clone()).await?;
        Ok(tokens)
    }

    /// Host enrollment uses this seam: publication belongs to its durable transaction.
    pub async fn poll_device_unpublished(
        &self,
        device: &DeviceAuthorization,
    ) -> Result<OAuthTokens, ProviderError> {
        let grant = self.poll_device_grant(device).await?;
        self.exchange_device_grant(grant).await
    }

    /// Poll only. Host services must recheck cancellation and authority before
    /// claiming and dispatching the separate token exchange effect.
    pub(crate) async fn poll_device_grant(
        &self,
        device: &DeviceAuthorization,
    ) -> Result<DeviceToken, ProviderError> {
        super::validate_native_endpoint(&self.endpoints.device_token)?;
        let response = super::endpoint_http_client(&self.client, &self.endpoints.device_token).post(&self.endpoints.device_token).timeout(Duration::from_secs(30)).json(&serde_json::json!({"device_auth_id":device.device_auth_id,"user_code":device.user_code})).send().await.map_err(map_request)?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            // This device endpoint uses 403/404 for a code that has not yet
            // been approved. Its pending body need not be OAuth error JSON.
            // A complete non-JSON pending response is not a lost token exchange.
            let body: Value = match auth_json(response).await {
                Ok(body) => body,
                Err(error)
                    if matches!(status, 403 | 404) && error.category() == "invalid_response" =>
                {
                    Value::Null
                }
                Err(error) => return Err(error),
            };
            let code = body
                .get("error")
                .and_then(Value::as_str)
                .or_else(|| body.pointer("/error/code").and_then(Value::as_str))
                .or_else(|| body.get("code").and_then(Value::as_str));
            return match code {
                Some("authorization_pending") => Err(ProviderError::Unavailable(
                    "device authorization pending".into(),
                )),
                Some("slow_down") => Err(ProviderError::Unavailable(
                    "device authorization slow_down".into(),
                )),
                Some("access_denied") => Err(ProviderError::Authentication(
                    "device authorization denied".into(),
                )),
                Some("expired_token") => Err(ProviderError::Authentication(
                    "device authorization expired".into(),
                )),
                _ if status == 403 || status == 404 => Err(ProviderError::Unavailable(
                    "device authorization pending".into(),
                )),
                _ => Err(ProviderError::Authentication(
                    "device authorization poll failed".into(),
                )),
            }
            .map_err(|error| error.with_http_status(status));
        }
        auth_json(response).await
    }

    pub(crate) async fn exchange_device_grant(
        &self,
        grant: DeviceToken,
    ) -> Result<OAuthTokens, ProviderError> {
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
        validate_tokens(&tokens)?;
        Ok(tokens)
    }

    pub async fn logout(&self) -> Result<(), ProviderError> {
        self.store.clear().await?;
        *self.tokens.lock().await = None;
        Ok(())
    }

    pub async fn status(&self) -> TokenStatus {
        let mut tokens = self.tokens.lock().await;
        *tokens = self.store.load().await.ok().flatten();
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
        self.responses_request_with_tokens(body)
            .await
            .map(|(request, _)| request)
    }
    async fn responses_request_with_tokens(
        &self,
        body: &Value,
    ) -> Result<(reqwest::RequestBuilder, OAuthTokens), ProviderError> {
        super::validate_native_endpoint(&self.endpoints.responses)?;
        let tokens = self.valid_tokens().await?;
        let request = super::endpoint_http_client(&self.client, &self.endpoints.responses)
            .post(&self.endpoints.responses)
            .bearer_auth(&tokens.access_token)
            .header("ChatGPT-Account-Id", &tokens.account_id)
            .header("originator", "helm")
            .header("OpenAI-Beta", "responses=experimental")
            .json(body);
        Ok((request, tokens))
    }

    async fn valid_tokens(&self) -> Result<OAuthTokens, ProviderError> {
        super::check_provider_authority(&self.authority)?;
        let mut guard = self.tokens.lock().await;
        *guard = self.store.load().await?;
        super::check_provider_authority(&self.authority)?;
        let current = guard
            .clone()
            .ok_or_else(|| ProviderError::Authentication("ChatGPT login required".into()))?;
        self.remember_tokens(&current)?;
        if current.expires_at > now_secs().saturating_add(REFRESH_SKEW_SECS) {
            return Ok(current);
        }
        let claim = if let Some((registry, binding)) = &self.store.binding {
            registry
                .refresh_begin(binding, &current)
                .map_err(account_error)
        } else {
            let path = self.store.path.clone();
            let expected = current.clone();
            tokio::task::spawn_blocking(move || storage::begin_refresh(&path, &expected))
                .await
                .map_err(|_| private_cache_error())?
                .map_err(|_| private_cache_error())
        };
        let fence = match claim {
            Ok(fence) => fence,
            Err(error) => {
                // Another process may have rotated since our read. Never replay its effect.
                if let Some(latest) = self.store.load().await? {
                    super::check_provider_authority(&self.authority)?;
                    if latest.expires_at > now_secs().saturating_add(REFRESH_SKEW_SECS) {
                        self.remember_tokens(&latest)?;
                        *guard = Some(latest.clone());
                        return Ok(latest);
                    }
                }
                return Err(error);
            }
        };
        super::check_provider_authority(&self.authority)?;
        let mut refreshed = self
            .exchange(&[
                ("grant_type", "refresh_token"),
                ("client_id", CLIENT_ID),
                ("refresh_token", &current.refresh_token),
            ])
            .await?;
        if refreshed.refresh_token.is_empty() {
            refreshed.refresh_token = current.refresh_token.clone();
        }
        validate_tokens(&refreshed)?;
        self.remember_tokens(&refreshed)?;
        let identity = login_identity(&current)?;
        if identity.is_none() || identity != login_identity(&refreshed)? {
            return Err(ProviderError::Authentication("OAuth identity changed or cannot be established; explicit reauthentication required".into()));
        }
        if let Some((registry, binding)) = &self.store.binding {
            registry
                .oauth_save(binding, &refreshed, Some(fence))
                .map_err(account_error)?;
        } else {
            let path = self.store.path.clone();
            let previous = current.clone();
            let next = refreshed.clone();
            tokio::task::spawn_blocking(move || {
                storage::commit_refresh(&path, &previous, &next, fence)
            })
            .await
            .map_err(|_| private_cache_error())?
            .map_err(|_| private_cache_error())?;
        }
        *guard = Some(refreshed.clone());
        super::check_provider_authority(&self.authority)?;
        Ok(refreshed)
    }

    async fn replace(&self, tokens: OAuthTokens) -> Result<(), ProviderError> {
        self.store.save(&tokens).await?;
        *self.tokens.lock().await = Some(tokens);
        Ok(())
    }

    async fn exchange(&self, form: &[(&str, &str)]) -> Result<OAuthTokens, ProviderError> {
        super::validate_native_endpoint(&self.endpoints.token)?;
        let response = super::endpoint_http_client(&self.client, &self.endpoints.token)
            .post(&self.endpoints.token)
            .timeout(Duration::from_secs(30))
            .form(form)
            .send()
            .await
            .map_err(|error| {
                // Token rotation may already have happened, including on timeout.
                if error.is_builder() {
                    super::map_transport(error)
                } else {
                    ProviderError::Transport(
                        "OAuth exchange failed; outcome may be uncertain".into(),
                    )
                }
            })?;
        super::reject_redirect(&response)?;
        if !response.status().is_success() {
            return Err(ProviderError::Authentication(format!(
                "ChatGPT OAuth exchange failed with HTTP {}",
                response.status()
            ))
            .with_http_status(response.status().as_u16()));
        }
        let raw: TokenResponse = auth_json(response).await?;
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
    super::map_transport(error)
}

pub(crate) fn validate_tokens(value: &OAuthTokens) -> Result<(), ProviderError> {
    if value.access_token.is_empty()
        || value.refresh_token.is_empty()
        || value.account_id.is_empty()
        || value.expires_at == 0
        || value.access_token.len() > 16_384
        || value.refresh_token.len() > 16_384
        || value.account_id.len() > 512
        || value.id_token.as_ref().is_some_and(|v| v.len() > 16_384)
        || value.access_token.chars().any(char::is_control)
        || value.refresh_token.chars().any(char::is_control)
        || value.account_id.chars().any(char::is_control)
    {
        return Err(ProviderError::Authentication(
            "ChatGPT token set is incomplete".into(),
        ));
    }
    login_identity(value)?;
    Ok(())
}

/// Identity asserted by the trusted OAuth response (or explicitly owner-imported
/// token set), not an independent JWT signature/entitlement verification. Never
/// expose this tuple in account descriptors. Legacy opaque tokens have no proven
/// subject and cannot support an identity-preserving replacement or refresh.
pub(crate) fn login_identity(tokens: &OAuthTokens) -> Result<Option<String>, ProviderError> {
    let invalid = || {
        ProviderError::Authentication(
            "OAuth identity claims are missing or inconsistent; explicit reauthentication required"
                .into(),
        )
    };
    let mut subject: Option<String> = None;
    for token in tokens
        .id_token
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(tokens.access_token.as_str()))
    {
        let Some(claims) = jwt_payload(token) else {
            continue;
        };
        for path in [
            "/chatgpt_account_id",
            "/account_id",
            "/https:~1~1api.openai.com~1auth/chatgpt_account_id",
        ] {
            if let Some(value) = claims.pointer(path)
                && value.as_str() != Some(tokens.account_id.as_str())
            {
                return Err(invalid());
            }
        }
        if let Some(value) = claims.get("sub") {
            let value = value
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 512 && !s.chars().any(char::is_control))
                .ok_or_else(invalid)?;
            if subject.as_deref().is_some_and(|old| old != value) {
                return Err(invalid());
            }
            subject = Some(value.to_owned());
        }
    }
    subject
        .map(|subject| {
            serde_json::to_string(&(tokens.account_id.as_str(), subject)).map_err(|_| invalid())
        })
        .transpose()
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
fn private_cache_error() -> ProviderError {
    ProviderError::Authentication(
        "ChatGPT credential storage must be owned, private, bounded and free of symlinks".into(),
    )
}

// ChatGPT's catalog convention uses default to suppress client tier selection.
// Public OpenAI API adapters intentionally do not apply this normalization.
fn subscription_request(mut request: ModelRequest) -> ModelRequest {
    if request.service_tier.as_deref() == Some("default") {
        request.service_tier = None;
    }
    request
}

fn filter_response_tier(response: &mut ModelResponse, tokens: &OAuthTokens) {
    if response.service_tier.as_ref().is_some_and(|tier| {
        super::catalog::validate_text(
            tier,
            64,
            true,
            &[
                &tokens.access_token,
                &tokens.refresh_token,
                tokens.id_token.as_deref().unwrap_or(""),
                &tokens.account_id,
            ],
        )
        .is_err()
    }) {
        response.service_tier = None;
    }
}

fn account_error(error: anyhow::Error) -> ProviderError {
    if error.downcast_ref::<RefreshPending>().is_some() {
        return ProviderError::Unavailable(REFRESH_PENDING.into());
    }
    ProviderError::Authentication(
        "account revoked, changed or unavailable; inspect host account status".into(),
    )
}

async fn auth_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, ProviderError> {
    let status = response.status().as_u16();
    let read = async {
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(map_request)? {
            if bytes.len().saturating_add(chunk.len()) > storage::LIMIT {
                return Err(ProviderError::InvalidResponse(
                    "OAuth response exceeds bound".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| ProviderError::InvalidResponse("invalid OAuth response".into()))
    };
    tokio::time::timeout(Duration::from_secs(30), read)
        .await
        .map_err(|_| ProviderError::Timeout("OAuth response timed out".into()))
        .and_then(|result| result)
        .map_err(|error| error.with_http_status(status))
}

#[cfg(test)]
mod failure_classification_tests {
    use super::*;

    #[tokio::test]
    async fn oauth_body_transport_is_not_authentication_failure() {
        let response = super::super::failure_tests::http_response(200, "0", "{", 100).await;
        let error = auth_json::<Value>(response).await.unwrap_err();
        assert_eq!(error.category(), "transport");
        assert_eq!(error.http_status(), Some(200));
        assert!(!error.is_retryable());
        let response = super::super::failure_tests::http_response(200, "0", "{", 1).await;
        let error = auth_json::<Value>(response).await.unwrap_err();
        assert_eq!(error.category(), "invalid_response");
        assert_eq!(error.http_status(), Some(200));
    }
}
