//! Explicitly enabled, bounded enrollment HTTP boundary. Does not expose session
//! execution. TLS terminates at the operator's reverse proxy; the configured
//! public origin is authoritative, never X-Forwarded-Host supplied by a request.
use crate::enrollment::{EnrollmentError, EnrollmentStore, Invitation, Receipt};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
use uuid::Uuid;
use voyage_protocol::enrollment::{Challenge, MAX_PROOF_BYTES, ProofOperation, SignedChallenge};

#[derive(Clone)]
pub struct EnrollmentApi {
    control_generation: Option<u64>,
    store: Arc<Mutex<EnrollmentStore>>,
    origin: String,
    operator_hash: [u8; 32],
    capacity: Arc<Semaphore>,
    requests: Arc<Semaphore>,
    operator_requests: Arc<Semaphore>,
}
impl EnrollmentApi {
    /// Explicit construction requires an operator credential. No anonymous
    /// enrollment administration and no reuse of legacy machine credentials.
    pub fn new(store: EnrollmentStore, operator_token: &str) -> Result<Self, EnrollmentError> {
        if operator_token.len() < 32
            || operator_token.len() > 1024
            || operator_token.chars().any(char::is_control)
        {
            return Err(EnrollmentError::Invalid);
        }
        Ok(Self {
            control_generation: None,
            origin: store.origin().into(),
            store: Arc::new(Mutex::new(store)),
            operator_hash: Sha256::digest(operator_token.as_bytes()).into(),
            capacity: Arc::new(Semaphore::new(16)),
            requests: Arc::new(Semaphore::new(32)),
            operator_requests: Arc::new(Semaphore::new(4)),
        })
    }
    pub fn router(self) -> Router {
        Router::new()
            .route("/v2/enrollment/invitations", post(invite))
            .route("/v2/enrollment/challenge", post(challenge))
            .route("/v2/enrollment/complete", post(complete))
            .route("/v2/enrollment/revoke", post(revoke))
            .layer(DefaultBodyLimit::max(MAX_PROOF_BYTES))
            .route_layer(middleware::from_fn_with_state(self.clone(), boundary))
            .with_state(self)
    }
    /// Canonical public origin validated by the enrollment store. All transports
    /// use this value rather than independently interpreting configuration.
    pub fn origin(&self) -> &str {
        &self.origin
    }
    pub(crate) async fn attachment_connect(
        &self,
        proof: SignedChallenge,
    ) -> Result<Receipt, EnrollmentError> {
        if !matches!(proof.challenge.operation, ProofOperation::Connect { .. }) {
            return Err(EnrollmentError::Denied);
        }
        self.operation(move |store, now| store.complete(&proof, None, now))
            .await
            .map_err(|e| e.0)
    }
    pub(crate) async fn attachment_current(
        &self,
        machine: Uuid,
        epoch: u64,
    ) -> Result<Receipt, EnrollmentError> {
        self.operation(move |store, _| store.current(machine, epoch))
            .await
            .map_err(|e| e.0)
    }
    async fn operation<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut EnrollmentStore, i64) -> Result<T, EnrollmentError> + Send + 'static,
    ) -> Result<T, ApiError> {
        // Permit follows blocking work, not a disconnected/timed-out HTTP future.
        // A slow disk therefore cannot accumulate an unbounded blocking queue.
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError(EnrollmentError::Busy))?;
        let store = self.store.clone();
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = store.lock().map_err(|_| EnrollmentError::Storage)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| EnrollmentError::Invalid)?
                .as_millis();
            let now = i64::try_from(now).map_err(|_| EnrollmentError::Invalid)?;
            f(&mut store, now)
        });
        // Timeout is an uncertain delivery outcome: the operator/client must
        // inspect/recover the original transaction, never invent a new one.
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .map_err(|_| ApiError(EnrollmentError::Busy))?
            .map_err(|_| ApiError(EnrollmentError::Storage))?
            .map_err(ApiError)
    }
    fn operator(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        if headers.get_all("authorization").iter().count() != 1 {
            return Err(ApiError(EnrollmentError::Denied));
        }
        let value = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() <= 2048)
            .ok_or(ApiError(EnrollmentError::Denied))?;
        let token = if let Some(token) = value.strip_prefix("Bearer ") {
            token.to_owned()
        } else if let Some(encoded) = value.strip_prefix("Basic ") {
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| ApiError(EnrollmentError::Denied))?;
            let value =
                std::str::from_utf8(&bytes).map_err(|_| ApiError(EnrollmentError::Denied))?;
            value
                .split_once(':')
                .ok_or(ApiError(EnrollmentError::Denied))?
                .1
                .to_owned()
        } else {
            return Err(ApiError(EnrollmentError::Denied));
        };
        let supplied: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if self
            .operator_hash
            .iter()
            .zip(&supplied)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            != 0
        {
            return Err(ApiError(EnrollmentError::Denied));
        }
        Ok(())
    }
}
struct ApiError(EnrollmentError);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            EnrollmentError::Invalid => StatusCode::BAD_REQUEST,
            EnrollmentError::Denied => StatusCode::FORBIDDEN,
            EnrollmentError::Conflict => StatusCode::CONFLICT,
            EnrollmentError::Busy | EnrollmentError::Capacity => StatusCode::SERVICE_UNAVAILABLE,
            EnrollmentError::Storage | EnrollmentError::Unsupported => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (
            status,
            Json(serde_json::json!({"error":self.0.to_string()})),
        )
            .into_response()
    }
}
async fn boundary(
    State(api): State<EnrollmentApi>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let administrative = matches!(
        request.uri().path(),
        "/v2/enrollment/invitations" | "/v2/enrollment/revoke"
    );
    let authorized = !administrative || api.operator(request.headers()).is_ok();
    let permit = if administrative {
        api.operator_requests.clone()
    } else {
        api.requests.clone()
    }
    .try_acquire_owned();
    let headers = request.headers();
    let origin = headers.get("origin");
    let valid_origin = origin.is_none_or(|v| v.to_str().ok() == Some(api.origin.as_str()));
    let same_site = headers
        .get("sec-fetch-site")
        .is_none_or(|v| v == "same-origin" || v == "none");
    let json = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"));
    let mut response = if !authorized {
        ApiError(EnrollmentError::Denied).into_response()
    } else if permit.is_err() {
        ApiError(EnrollmentError::Busy).into_response()
    } else if request.uri().query().is_some()
        || headers.get_all("origin").iter().count() > 1
        || !valid_origin
        || !same_site
        || headers.get("x-voyage-request").is_none_or(|v| v != "2")
        || !json
    {
        ApiError(EnrollmentError::Denied).into_response()
    } else {
        match tokio::time::timeout(Duration::from_secs(10), next.run(request)).await {
            Ok(response) => response,
            Err(_) => ApiError(EnrollmentError::Busy).into_response(),
        }
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response
}
fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    if body.len() > MAX_PROOF_BYTES {
        return Err(ApiError(EnrollmentError::Invalid));
    }
    serde_json::from_slice(body).map_err(|_| ApiError(EnrollmentError::Invalid))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteRequest {
    ttl_ms: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeRequest {
    operation: ProofOperation,
    invitation_key: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteRequest {
    proof: SignedChallenge,
    invitation_key: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeRequest {
    machine_id: Uuid,
    expected_epoch: u64,
    transaction_id: Uuid,
}
async fn invite(
    State(api): State<EnrollmentApi>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Invitation>, ApiError> {
    api.operator(&headers)?;
    let request: InviteRequest = decode(&body)?;
    api.operation(move |s, now| s.invite(request.ttl_ms, now))
        .await
        .map(Json)
}
async fn challenge(
    State(api): State<EnrollmentApi>,
    body: Bytes,
) -> Result<Json<Challenge>, ApiError> {
    let request: ChallengeRequest = decode(&body)?;
    api.operation(move |s, now| {
        s.challenge(request.operation, request.invitation_key.as_deref(), now)
    })
    .await
    .map(Json)
}
async fn complete(
    State(api): State<EnrollmentApi>,
    body: Bytes,
) -> Result<Json<Receipt>, ApiError> {
    let request: CompleteRequest = decode(&body)?;
    api.operation(move |s, now| s.complete(&request.proof, request.invitation_key.as_deref(), now))
        .await
        .map(Json)
}
async fn revoke(
    State(api): State<EnrollmentApi>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Receipt>, ApiError> {
    api.operator(&headers)?;
    let request: RevokeRequest = decode(&body)?;
    api.operation(move |s, now| {
        s.revoke(
            request.machine_id,
            request.expected_epoch,
            request.transaction_id,
            now,
        )
    })
    .await
    .map(Json)
}

#[cfg(test)]
#[cfg(unix)]
mod tests;

mod control;
