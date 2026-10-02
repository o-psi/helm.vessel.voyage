//! Native SuperGrok OAuth. Tokens never leave the executing host account boundary.
use super::{ModelInfo, OpenAiProvider, Provider, ProviderError, ProviderStream};
use crate::model::{ModelRequest, ModelResponse};
use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use voyage_protocol::accounts::AccountBinding;

pub const ENDPOINT: &str = "https://api.x.ai/v1";
pub const VERIFICATION_URI: &str = "https://accounts.x.ai/oauth2/device";
const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: Option<String>,
    pub expires_at: u64,
    // Trusted token-endpoint claims, not independently verified JWT signatures.
    pub identity: String,
}
fn auth_error() -> ProviderError {
    ProviderError::Authentication(
        "SuperGrok sign-in unavailable or changed; review the selected account".into(),
    )
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn claims(token: &str) -> Option<Value> {
    if token.len() > 16_384 {
        return None;
    }
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}
fn safe(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16_384 && value.bytes().all(|b| b.is_ascii_graphic())
}
fn identity(access: &str, id: Option<&str>) -> Result<String, ProviderError> {
    let access = claims(access).ok_or_else(auth_error)?;
    if access.get("iss").and_then(Value::as_str) != Some("https://auth.x.ai") {
        return Err(auth_error());
    }
    if access
        .get("client_id")
        .is_some_and(|v| v.as_str() != Some(CLIENT_ID))
    {
        return Err(auth_error());
    }
    let subject = access
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| safe(s) && s.len() <= 512)
        .ok_or_else(auth_error)?;
    if let Some(id) = id {
        let id = claims(id).ok_or_else(auth_error)?;
        let audience = id.get("aud").is_some_and(|v| {
            v.as_str() == Some(CLIENT_ID)
                || v.as_array()
                    .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(CLIENT_ID)))
        });
        if id.get("iss").and_then(Value::as_str) != Some("https://auth.x.ai")
            || id.get("sub").and_then(Value::as_str) != Some(subject)
            || !audience
        {
            return Err(auth_error());
        }
    }
    let principal = |name| -> Result<Option<&str>, ProviderError> {
        match access.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_str()
                .filter(|s| safe(s) && s.len() <= 512)
                .map(Some)
                .ok_or_else(auth_error),
        }
    };
    serde_json::to_string(&(
        subject,
        principal("principal_type")?,
        principal("principal_id")?,
        principal("team_id")?,
    ))
    .map_err(|_| auth_error())
}
pub(crate) fn validate_tokens(tokens: &Tokens) -> Result<(), ProviderError> {
    if !safe(&tokens.access_token)
        || !safe(&tokens.refresh_token)
        || tokens.expires_at == 0
        || tokens.id_token.as_deref().is_some_and(|s| !safe(s))
        || tokens.identity != identity(&tokens.access_token, tokens.id_token.as_deref())?
    {
        return Err(auth_error());
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct OAuth {
    client: reqwest::Client,
    device_url: String,
    token_url: String,
}
impl Default for OAuth {
    fn default() -> Self {
        Self {
            client: super::native_http_client(),
            device_url: "https://auth.x.ai/oauth2/device/code".into(),
            token_url: "https://auth.x.ai/oauth2/token".into(),
        }
    }
}
impl OAuth {
    #[cfg(test)]
    pub(crate) fn loopback(address: std::net::SocketAddr) -> Self {
        assert!(address.ip().is_loopback());
        Self {
            client: super::native_http_client(),
            device_url: format!("http://{address}/device"),
            token_url: format!("http://{address}/token"),
        }
    }
    async fn post(&self, url: &str, form: &[(&str, &str)]) -> Result<Value, ProviderError> {
        let response = super::endpoint_http_client(&self.client, url)
            .post(url)
            .header(
                reqwest::header::USER_AGENT,
                concat!("voyage/", env!("CARGO_PKG_VERSION")),
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .form(form)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(super::map_transport)?;
        let status = response.status().as_u16();
        // Read a bounded JSON response; upstream diagnostics never enter errors.
        let mut bytes = Vec::new();
        let mut response = response;
        while let Some(chunk) = response.chunk().await.map_err(super::map_transport)? {
            if bytes.len().saturating_add(chunk.len()) > 65_536 {
                return Err(ProviderError::InvalidResponse(
                    "xAI authentication response exceeded its limit".into(),
                )
                .with_http_status(status));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ProviderError::InvalidResponse("invalid xAI authentication response".into())
                .with_http_status(status)
        })?;
        if !(200..300).contains(&status) {
            let error = match value.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => {
                    ProviderError::Unavailable("device authorization pending".into())
                }
                Some("slow_down") => {
                    ProviderError::Unavailable("device authorization slow_down".into())
                }
                Some("access_denied" | "authorization_denied") => {
                    ProviderError::Authentication("device authorization denied".into())
                }
                Some("expired_token") => {
                    ProviderError::Authentication("device authorization expired".into())
                }
                _ => auth_error(),
            };
            return Err(error.with_http_status(status));
        }
        Ok(value)
    }
    pub(crate) async fn begin_device(&self) -> Result<super::DeviceAuthorization, ProviderError> {
        let value = self
            .post(
                &self.device_url,
                &[
                    ("client_id", CLIENT_ID),
                    ("scope", SCOPE),
                    ("referrer", "voyage"),
                ],
            )
            .await?;
        let string = |name| {
            value
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("incomplete xAI device response".into())
                })
        };
        Ok(super::DeviceAuthorization {
            device_auth_id: string("device_code")?,
            user_code: string("user_code")?,
            verification_uri: string("verification_uri")?,
            interval: value.get("interval").and_then(Value::as_u64).unwrap_or(5),
            expires_in: Some(
                value
                    .get("expires_in")
                    .and_then(Value::as_u64)
                    .unwrap_or(300),
            ),
        })
    }
    pub(crate) async fn poll(
        &self,
        device: &super::DeviceAuthorization,
    ) -> Result<Tokens, ProviderError> {
        let value = self
            .post(
                &self.token_url,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("client_id", CLIENT_ID),
                    ("device_code", &device.device_auth_id),
                ],
            )
            .await?;
        tokens(value, None)
    }
    async fn refresh(&self, previous: &Tokens) -> Result<Tokens, ProviderError> {
        let value = self
            .post(
                &self.token_url,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", CLIENT_ID),
                    ("refresh_token", &previous.refresh_token),
                ],
            )
            .await?;
        tokens(value, Some(&previous.refresh_token))
    }
}
fn tokens(value: Value, old_refresh: Option<&str>) -> Result<Tokens, ProviderError> {
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(auth_error)?
        .to_owned();
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(old_refresh)
        .ok_or_else(auth_error)?
        .to_owned();
    let id_token = match value.get("id_token") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(auth_error)?.to_owned()),
    };
    if value
        .get("token_type")
        .is_some_and(|v| !v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("bearer")))
    {
        return Err(auth_error());
    }
    let current = now();
    let exp = claims(&access_token).and_then(|v| v.get("exp").and_then(Value::as_u64));
    let lifetime = match value.get("expires_in") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|s| *s > 0).ok_or_else(auth_error)?),
    };
    let expires_at = match (exp, lifetime) {
        (Some(exp), Some(lifetime)) => exp.min(current.saturating_add(lifetime)),
        (Some(exp), None) => exp,
        (None, Some(lifetime)) => current.saturating_add(lifetime),
        _ => return Err(auth_error()),
    };
    if expires_at <= current {
        return Err(auth_error());
    }
    let result = Tokens {
        identity: identity(&access_token, id_token.as_deref())?,
        access_token,
        refresh_token,
        id_token,
        expires_at,
    };
    validate_tokens(&result)?;
    Ok(result)
}

// xAI's model schema decoder accepts signed 64-bit bounds, but rejects u64::MAX.
// Only the model-facing copy changes. Registry contracts remain authoritative.
fn model_schema(schema: &mut Value) {
    if let Some(nodes) = schema.as_array_mut() {
        for node in nodes {
            model_schema(node);
        }
        return;
    }
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    let mut bounds = Vec::new();
    for name in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if object
            .get(name)
            .and_then(Value::as_u64)
            .is_some_and(|v| v > i64::MAX as u64)
        {
            let value = object.remove(name).unwrap();
            bounds.push(format!("{name}={value}"));
        }
    }
    if !bounds.is_empty() {
        let prior = object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("");
        object.insert("description".into(), Value::String(format!("{prior} Runtime bounds: {}. The tool validates these exact bounds on execution.", bounds.join(", ")).trim().to_owned()));
    }
    // Visit schema positions only; const/default/enum/examples are literal data.
    for name in [
        "properties",
        "patternProperties",
        "$defs",
        "definitions",
        "dependentSchemas",
    ] {
        if let Some(children) = object.get_mut(name).and_then(Value::as_object_mut) {
            for child in children.values_mut() {
                model_schema(child);
            }
        }
    }
    for name in [
        "items",
        "additionalItems",
        "contains",
        "additionalProperties",
        "propertyNames",
        "not",
        "if",
        "then",
        "else",
        "unevaluatedProperties",
        "unevaluatedItems",
        "contentSchema",
        "allOf",
        "anyOf",
        "oneOf",
        "prefixItems",
    ] {
        if let Some(child) = object.get_mut(name) {
            model_schema(child);
        }
    }
}
fn wire_request(mut request: ModelRequest) -> ModelRequest {
    for tool in &mut request.tools {
        model_schema(&mut tool.input_schema);
    }
    request
}

pub struct XaiOauthProvider {
    registry: crate::accounts::Registry,
    binding: AccountBinding,
    oauth: OAuth,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    redactor: Option<Arc<crate::tools::Redactor>>,
    // Only the test binary can select a fixture; runtime endpoints are fixed.
    endpoint: String,
}
impl XaiOauthProvider {
    pub(crate) fn new(registry: crate::accounts::Registry, binding: AccountBinding) -> Self {
        Self {
            registry,
            binding,
            oauth: OAuth::default(),
            authority: None,
            redactor: None,
            endpoint: ENDPOINT.into(),
        }
    }
    pub(crate) fn with_context(
        mut self,
        config: &crate::Config,
        redactor: Option<Arc<crate::tools::Redactor>>,
    ) -> Self {
        self.authority = config.provider_authority.clone();
        self.redactor = redactor;
        self
    }
    pub fn with_authority(
        mut self,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> Self {
        self.authority = authority;
        self
    }
    /// Refresh only the selected account, without inference or a quota claim.
    pub async fn refresh_sign_in(&self) -> Result<(), ProviderError> {
        self.resolve(None).await.map(|_| ())
    }
    fn remember(&self, tokens: &Tokens) -> Result<(), ProviderError> {
        if let Some(redactor) = &self.redactor {
            for value in [
                &tokens.access_token,
                &tokens.refresh_token,
                &tokens.identity,
            ]
            .into_iter()
            .chain(tokens.id_token.iter())
            {
                redactor
                    .remember_credential(value)
                    .map_err(|_| auth_error())?;
            }
        }
        Ok(())
    }
    async fn resolve(&self, rejected: Option<&str>) -> Result<Tokens, ProviderError> {
        super::check_provider_authority(&self.authority)?;
        let current = self
            .registry
            .xai_load(&self.binding)
            .map_err(|_| auth_error())?;
        validate_tokens(&current)?;
        self.remember(&current)?;
        if rejected != Some(current.access_token.as_str())
            && current.expires_at > now().saturating_add(120)
        {
            return Ok(current);
        }
        let fence = match self.registry.xai_refresh_begin(&self.binding, &current) {
            Ok(fence) => fence,
            Err(_) => {
                let latest = self
                    .registry
                    .xai_load(&self.binding)
                    .map_err(|_| auth_error())?;
                self.remember(&latest)?;
                if latest != current && latest.expires_at > now().saturating_add(120) {
                    return Ok(latest);
                }
                return Err(auth_error());
            }
        };
        super::check_provider_authority(&self.authority)?;
        let refreshed = self.oauth.refresh(&current).await?;
        self.remember(&refreshed)?;
        super::check_provider_authority(&self.authority)?;
        self.registry
            .xai_save(&self.binding, &refreshed, fence)
            .map_err(|_| auth_error())?;
        Ok(refreshed)
    }
    async fn adapter(&self) -> Result<(OpenAiProvider, Tokens), ProviderError> {
        let tokens = self.resolve(None).await?;
        super::check_provider_authority(&self.authority)?;
        Ok((
            OpenAiProvider::new(String::new(), Some(self.endpoint.clone())).with_credential(
                super::api_credential::ApiCredential::XaiAccount {
                    registry: self.registry.clone(),
                    binding: self.binding.clone(),
                    authority: self.authority.clone(),
                    redactor: self.redactor.clone(),
                },
            ),
            tokens,
        ))
    }
    async fn failure<T>(
        &self,
        result: Result<T, ProviderError>,
        used: &Tokens,
    ) -> Result<T, ProviderError> {
        match result {
            Err(error) if error.http_status() == Some(401) => {
                self.resolve(Some(&used.access_token)).await?;
                Err(ProviderError::AuthenticationRefreshed)
            }
            other => other,
        }
    }
}
#[async_trait]
impl Provider for XaiOauthProvider {
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let (provider, used) = self.adapter().await?;
        let models = self.failure(provider.models().await, &used).await?;
        super::validate_models(
            &models,
            &[
                &used.access_token,
                &used.refresh_token,
                &used.identity,
                used.id_token.as_deref().unwrap_or(""),
            ],
        )?;
        Ok(models)
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        super::inference::validate_request(&crate::ProviderKind::XaiOauth, &request)?;
        let (provider, used) = self.adapter().await?;
        self.failure(provider.complete(wire_request(request)).await, &used)
            .await
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        super::inference::validate_request(&crate::ProviderKind::XaiOauth, &request)?;
        let (provider, used) = self.adapter().await?;
        self.failure(provider.stream(wire_request(request)).await, &used)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Registry, device::DeviceService};
    use crate::provider::native_http_tests::server;
    use futures_util::StreamExt;
    use serde_json::json;
    use voyage_protocol::accounts::*;
    fn jwt(subject: &str, expires: u64) -> String {
        format!("e30.{}.fixture", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json!({"iss":"https://auth.x.ai", "sub":subject,"client_id":CLIENT_ID,"exp":expires}).to_string()))
    }
    fn token_response(subject: &str, expires: u64) -> Value {
        json!({"access_token":jwt(subject,expires),"refresh_token":"fixture-refresh","expires_in":7200,"token_type":"bearer"})
    }
    fn fixture_tokens() -> Tokens {
        tokens(token_response("fixture-owner", now() + 7200), None).unwrap()
    }
    fn address(url: &str) -> std::net::SocketAddr {
        url.trim_start_matches("http://").parse().unwrap()
    }
    fn request() -> ModelRequest {
        ModelRequest {
            model: "grok-fixture".into(),
            messages: vec![crate::model::Message::new(
                crate::model::Role::User,
                "hello",
            )],
            tools: vec![],
            temperature: None,
            reasoning_effort: None,
            service_tier: None,
            max_tokens: Some(128),
        }
    }
    fn provider(registry: Registry, binding: AccountBinding, url: &str) -> XaiOauthProvider {
        let mut p = XaiOauthProvider::new(registry, binding);
        p.oauth = OAuth::loopback(address(url));
        p.endpoint = url.into();
        p
    }
    #[test]
    fn identity_and_expiry_validation_are_provider_specific() {
        let response = token_response("fixture-owner", now() + 7200);
        let original = tokens(response.clone(), None).unwrap();
        assert!(validate_tokens(&original).is_ok());
        let mut missing = response.clone();
        missing["refresh_token"] = Value::Null;
        assert!(tokens(missing.clone(), None).is_err());
        assert_eq!(
            tokens(missing, Some("prior-refresh"))
                .unwrap()
                .refresh_token,
            "prior-refresh"
        );
        let mut expired = response.clone();
        expired["access_token"] = json!(jwt("fixture-owner", 1));
        assert!(tokens(expired, None).is_err());
        let mut altered = original.clone();
        altered.identity = "other-owner".into();
        assert!(validate_tokens(&altered).is_err());
        let mut other = response;
        other["id_token"] = json!(jwt("other-owner", now() + 7200));
        assert!(tokens(other, None).is_err());
        let opaque =
            json!({"access_token":"opaque","refresh_token":"fixture-refresh","expires_in":3600});
        assert!(tokens(opaque, None).is_err());
    }
    #[tokio::test]
    async fn enrollment_is_durable_bounded_and_deduplicated() {
        let (url,task)=server(vec![(200,json!({"device_code":"private-device","user_code":"ABCD-1234","verification_uri":VERIFICATION_URI,"interval":1,"expires_in":300}).to_string()),(400,json!({"error":"authorization_pending"}).to_string()),(400,json!({"error":"slow_down"}).to_string()),(200,token_response("fixture-owner",now()+7200).to_string())]).await;
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path().join("accounts"));
        let connection = registry.ensure_xai_connection().unwrap();
        let service = DeviceService::new(registry.clone(), Arc::new(|_, _| true))
            .xai_loopback_fixture(address(&url));
        let actor = EnrollmentActor {
            principal: "fixture".into(),
            workspace: "/fixture".into(),
        };
        let request = EnrollmentRequest {
            command_id: uuid::Uuid::new_v4(),
            enrollment_id: uuid::Uuid::new_v4(),
            connection_id: connection.id,
            alias: "personal".into(),
            label: "Personal Grok".into(),
            actor: actor.clone(),
        };
        let first = service.start(request.clone()).await.unwrap();
        assert_eq!(first.state, EnrollmentState::Pending);
        assert_eq!(service.start(request.clone()).await.unwrap(), first);
        let private = service.status(request.enrollment_id, &actor).unwrap();
        assert_eq!(private.verification_uri.as_deref(), Some(VERIFICATION_URI));
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("private-device")
        );
        for interval in [1, 6] {
            service.fixture_due(request.enrollment_id, false).unwrap();
            assert_eq!(
                service
                    .drive(request.enrollment_id, &actor)
                    .await
                    .unwrap()
                    .state,
                EnrollmentState::Pending
            );
            assert_eq!(
                service.fixture_due(request.enrollment_id, false).unwrap(),
                interval
            );
        }
        let success = service.drive(request.enrollment_id, &actor).await.unwrap();
        assert_eq!(success.state, EnrollmentState::Succeeded);
        let binding = registry
            .freeze(success.account_id.unwrap(), Transport::XaiOauth)
            .unwrap();
        assert_eq!(
            registry.xai_load(&binding).unwrap().identity,
            fixture_tokens().identity
        );
        assert!(
            registry
                .add_api(
                    connection.id,
                    "api".into(),
                    "API".into(),
                    crate::accounts::ApiKeyInput::Stored("secret".into())
                )
                .is_err()
        );
        assert!(registry.token_store(&binding).is_err());
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].contains(CLIENT_ID));
        assert!(requests[0].contains("referrer=voyage"));
        assert!(
            requests[1]
                .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code")
        );
    }
    #[tokio::test]
    async fn refresh_fences_rotation_identity_and_revocation() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path().join("accounts"));
        let mut initial = fixture_tokens();
        initial.expires_at = 1;
        let binding = registry.xai_fixture(initial.clone()).unwrap();
        let mut rotated = token_response("fixture-owner", now() + 9000);
        rotated["refresh_token"] = json!("rotated-refresh");
        let (url, task) = server(vec![
            (200, rotated.to_string()),
            (200, json!({"data":[{"id":"grok-fixture"}]}).to_string()),
        ])
        .await;
        let p = provider(registry.clone(), binding.clone(), &url);
        assert_eq!(p.models().await.unwrap()[0].id, "grok-fixture");
        let current = registry.xai_load(&binding).unwrap();
        assert_eq!(current.refresh_token, "rotated-refresh");
        assert_eq!(
            registry
                .validate_binding(&binding)
                .unwrap()
                .capability_revision,
            1
        );
        let requests = task.await.unwrap();
        assert!(requests[0].contains("refresh_token=fixture-refresh"));
        assert!(requests[1].contains(&current.access_token));
        let fence = registry.xai_refresh_begin(&binding, &current).unwrap();
        assert!(registry.xai_load(&binding).is_err());
        assert!(registry.xai_refresh_begin(&binding, &current).is_err());
        let other = tokens(token_response("other-owner", now() + 7200), None).unwrap();
        assert!(registry.xai_save(&binding, &other, fence).is_err());
        assert!(
            registry
                .xai_save(&binding, &current, uuid::Uuid::new_v4())
                .is_err()
        );
        registry.xai_save(&binding, &current, fence).unwrap();
        registry.logout(binding.account_id, false).unwrap();
        assert!(p.models().await.is_err());
        assert!(registry.xai_save(&binding, &current, fence).is_err());
    }
    #[tokio::test]
    async fn native_stream_tools_and_expired_bearer_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path().join("accounts"));
        let initial = fixture_tokens();
        let binding = registry.xai_fixture(initial).unwrap();
        let sse = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"Hello","tool_calls":[{"index":0,"id":"call-1","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}})
        );
        let (url, task) = server(vec![(200, sse)]).await;
        let p = provider(registry.clone(), binding.clone(), &url);
        let mut stream = p.stream(request()).await.unwrap();
        let mut completed = None;
        while let Some(event) = stream.next().await {
            if let super::super::ProviderStreamEvent::Completed(result) = event.unwrap() {
                completed = Some(result);
            }
        }
        let result = completed.unwrap();
        assert_eq!(result.message.tool_calls.len(), 1);
        assert_eq!(result.message.tool_calls[0].name, "read_file");
        task.await.unwrap();
        let (url, task) = server(vec![
            (401, json!({"error":{"code":"token_expired"}}).to_string()),
            (
                200,
                token_response("fixture-owner", now() + 9000).to_string(),
            ),
        ])
        .await;
        let p = provider(registry.clone(), binding.clone(), &url);
        assert!(matches!(
            p.models().await,
            Err(ProviderError::AuthenticationRefreshed)
        ));
        assert_eq!(task.await.unwrap().len(), 2);
        let (url, task) = server(vec![(
            403,
            json!({"error":{"message":"private-server-diagnostic"}}).to_string(),
        )])
        .await;
        let p = provider(registry, binding, &url);
        let error = p.models().await.err().unwrap();
        assert!(!error.is_retryable());
        assert!(!error.to_string().contains("private-server-diagnostic"));
        assert_eq!(task.await.unwrap().len(), 1);
    }
    #[tokio::test]
    async fn foreign_verification_site_and_cancelled_grants_never_publish_an_account() {
        for uri in [
            VERIFICATION_URI,
            "https://accounts.x.ai.attacker.invalid/oauth2/device",
        ] {
            let (url, task) = server(vec![(200,json!({"device_code":"private-device", "user_code":"ABCD-1234", "verification_uri":uri, "interval":1,"expires_in":300}).to_string())]).await;
            let directory = tempfile::tempdir().unwrap();
            let registry = Registry::new(directory.path().join("accounts"));
            let connection = registry.ensure_xai_connection().unwrap();
            let service = DeviceService::new(registry.clone(), Arc::new(|_, _| true))
                .xai_loopback_fixture(address(&url));
            let actor = EnrollmentActor {
                principal: "fixture".into(),
                workspace: "/fixture".into(),
            };
            let request = EnrollmentRequest {
                command_id: uuid::Uuid::new_v4(),
                enrollment_id: uuid::Uuid::new_v4(),
                connection_id: connection.id,
                alias: "cancel-fixture".into(),
                label: "Fixture".into(),
                actor: actor.clone(),
            };
            let status = service.start(request.clone()).await.unwrap();
            if uri == VERIFICATION_URI {
                assert_eq!(status.state, EnrollmentState::Pending);
                service
                    .cancel(uuid::Uuid::new_v4(), request.enrollment_id, &actor)
                    .unwrap();
                assert_eq!(
                    service
                        .drive(request.enrollment_id, &actor)
                        .await
                        .unwrap()
                        .state,
                    EnrollmentState::Cancelled
                );
            } else {
                assert_eq!(status.state, EnrollmentState::Uncertain);
            }
            assert!(
                service
                    .status(request.enrollment_id, &actor)
                    .unwrap()
                    .user_code
                    .is_none()
            );
            assert_eq!(
                service.start(request).await.unwrap().state,
                if uri == VERIFICATION_URI {
                    EnrollmentState::Cancelled
                } else {
                    EnrollmentState::Uncertain
                }
            );
            assert!(registry.list(|_| true).unwrap().1.is_empty());
            assert_eq!(task.await.unwrap().len(), 1);
        }
    }
    #[test]
    fn unsigned_bounds_remain_exact_in_runtime_contracts_and_literal_metadata() {
        let original = json!({"type":"object","properties":{"count":{"type":["integer","null"],"minimum":0,"maximum":u64::MAX,"description":"Read offset"},"signed":{"type":"integer","maximum":i64::MAX},"data":{"const":{"maximum":u64::MAX},"default":{"maximum":u64::MAX}}},"oneOf":[{"properties":{"offset":{"type":"integer","maximum":u64::MAX}}}]});
        let mut schema = original.clone();
        model_schema(&mut schema);
        assert!(schema["properties"]["count"].get("maximum").is_none());
        assert!(
            schema["properties"]["count"]["description"]
                .as_str()
                .unwrap()
                .contains(&u64::MAX.to_string())
        );
        assert_eq!(
            schema["properties"]["signed"],
            original["properties"]["signed"]
        );
        assert_eq!(schema["properties"]["data"], original["properties"]["data"]);
        assert!(
            schema["oneOf"][0]["properties"]["offset"]
                .get("maximum")
                .is_none()
        );
        let original_contract = jsonschema::validator_for(&original).unwrap();
        assert!(original_contract.is_valid(&json!({"count":u64::MAX})));
        assert!(!original_contract.is_valid(&json!({"count":1e20})));
        assert_eq!(
            original["properties"]["count"]["maximum"].as_u64(),
            Some(u64::MAX)
        );
    }
}
