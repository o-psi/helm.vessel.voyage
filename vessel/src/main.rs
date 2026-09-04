use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::{Form, Path, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    middleware,
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Duration as ChronoDuration, Utc};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::RwLock;
use tracing::Instrument;
use uuid::Uuid;
use voyage_protocol::{
    ApiError, FleetSummary, HealthResponse, HeartbeatRequest, HelmStatus, PAIRING_PREFIX,
    PROTOCOL_VERSION, PairingClaimRequest, PairingStartRequest, PairingStartResponse,
    PairingStatus, RegisteredHelm, TaskCompletion, TaskEnvelope, TaskFailure, TaskRecord,
    TaskRequest, TaskState,
};

#[derive(Parser)]
#[command(version, about = "Voyage management plane for outbound Helm workers")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:9480")]
    bind: String,
    #[arg(long, default_value_t = 90)]
    stale_after_secs: u64,
    #[arg(long, default_value = "vessel.db")]
    database: PathBuf,
    #[arg(long, default_value_t = 60)]
    lease_secs: u64,
    #[arg(long, default_value_t = 600)]
    pairing_ttl_secs: u64,
    /// Secret required for the operator web console (or VESSEL_OPERATOR_TOKEN).
    #[arg(long, env = "VESSEL_OPERATOR_TOKEN")]
    operator_token: Option<String>,
    #[arg(long, global = true, value_enum, default_value = "text")]
    log_format: LogFormat,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Claim the one-time string printed by `helm --voyage`.
    Pair {
        connection_string: String,
        #[arg(long, default_value = "http://127.0.0.1:9480")]
        vessel: String,
    },
    /// Show an operations summary for the managed fleet.
    Fleet {
        #[arg(long, default_value = "http://127.0.0.1:9480")]
        vessel: String,
    },
    /// Generate a shell completion script on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff manpage on stdout.
    Manpage,
}

#[derive(Clone, clap::ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<ControlPlane>>,
    database: Arc<Mutex<Connection>>,
    lease_duration: Duration,
    task_available: Arc<tokio::sync::Notify>,
    pairing_ttl: ChronoDuration,
    operator_token_hash: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
struct ControlPlane {
    pairings: HashMap<String, PendingPairing>,
    helms: HashMap<Uuid, RegisteredHelm>,
    worker_tokens: HashMap<String, Uuid>,
    queues: HashMap<Uuid, VecDeque<TaskEnvelope>>,
    tasks: HashMap<Uuid, TaskRecord>,
}
#[derive(Serialize, Deserialize)]
struct PendingPairing {
    helm: voyage_protocol::HelmDescriptor,
    worker_token_hash: String,
    expires_at: chrono::DateTime<Utc>,
    claimed: bool,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "vessel=info".into());
    match cli.log_format {
        LogFormat::Text => tracing_subscriber::fmt().with_env_filter(filter).init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .init(),
    }
    match &cli.command {
        Some(Command::Completions { shell }) => {
            clap_complete::generate(
                *shell,
                &mut Cli::command(),
                "vessel",
                &mut std::io::stdout(),
            );
            return Ok(());
        }
        Some(Command::Manpage) => {
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
            return Ok(());
        }
        _ => {}
    }
    if let Some(Command::Pair {
        connection_string,
        vessel,
    }) = &cli.command
    {
        let paired: RegisteredHelm = reqwest::Client::new()
            .post(format!(
                "{}/v1/pairings/claim",
                vessel.trim_end_matches('/')
            ))
            .json(&PairingClaimRequest {
                connection_string: connection_string.clone(),
            })
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        println!(
            "Paired {} ({})",
            paired.descriptor.name, paired.descriptor.id
        );
        return Ok(());
    }
    if let Some(Command::Fleet { vessel }) = &cli.command {
        let summary: FleetSummary =
            reqwest::get(format!("{}/v1/fleet/summary", vessel.trim_end_matches('/')))
                .await?
                .error_for_status()?
                .json()
                .await?;
        println!(
            "helms: {} total, {} online\ntasks: {} queued, {} running, {} failed",
            summary.total_helms,
            summary.online_helms,
            summary.queued_tasks,
            summary.running_tasks,
            summary.failed_tasks
        );
        return Ok(());
    }
    let database = open_database(&cli.database)?;
    let initial = load_state(&database)?.unwrap_or_default();
    let state = AppState {
        inner: Arc::new(RwLock::new(initial)),
        database: Arc::new(Mutex::new(database)),
        lease_duration: Duration::from_secs(cli.lease_secs),
        task_available: Arc::new(tokio::sync::Notify::new()),
        pairing_ttl: ChronoDuration::seconds(cli.pairing_ttl_secs.max(1) as i64),
        operator_token_hash: cli.operator_token.as_deref().map(token_hash),
    };
    spawn_reaper(state.clone(), Duration::from_secs(cli.stale_after_secs));
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/v1/diagnostics", get(diagnostics))
        .route("/v1/pairings/start", post(start_pairing))
        .route("/v1/pairings/claim", post(claim_pairing))
        .route("/v1/pairings/{code}", get(pairing_status))
        .route("/v1/worker/heartbeat", post(worker_heartbeat))
        .route("/v1/worker/tasks/next", get(next_task))
        .route("/v1/worker/tasks/{id}/result", post(complete_task))
        .route("/v1/worker/tasks/{id}/failure", post(fail_task))
        .route("/v1/helms", get(list_helms))
        .route("/v1/helms/{id}", get(get_helm).delete(remove_helm))
        .route("/v1/helms/{id}/tasks", post(enqueue_task))
        .route("/v1/tasks/{id}", get(get_task))
        .route("/v1/tasks", get(list_tasks))
        .route("/v1/tasks/{id}/cancel", post(cancel_task))
        .route("/v1/fleet/summary", get(fleet_summary))
        .route("/ui", get(operator_dashboard))
        .route("/ui/helms/{id}", get(operator_helm))
        .route("/ui/helms/{id}/tasks", post(operator_create_task))
        .route("/ui/tasks/{id}", get(operator_task))
        .route("/ui/tasks/{id}/cancel", post(operator_cancel_task))
        .route("/ui/tasks/{id}/retry", post(operator_retry_task))
        .layer(middleware::from_fn(correlate))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    tracing::info!(address = %cli.bind, "Vessel ready for outbound Helm workers");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

async fn readiness(State(state): State<AppState>) -> ApiResult<HealthResponse> {
    state
        .database
        .lock()
        .map_err(|error| internal_error(anyhow::anyhow!(error.to_string())))?
        .query_row("SELECT 1", [], |_| Ok(()))
        .map_err(|error| internal_error(error.into()))?;
    Ok(Json(HealthResponse {
        status: "ready".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    }))
}

async fn metrics(State(state): State<AppState>) -> String {
    let inner = state.inner.read().await;
    let online = inner
        .helms
        .values()
        .filter(|helm| helm.status == HelmStatus::Online)
        .count();
    let queued = inner
        .tasks
        .values()
        .filter(|task| task.state == TaskState::Queued)
        .count();
    let running = inner
        .tasks
        .values()
        .filter(|task| task.state == TaskState::Running)
        .count();
    let failed = inner
        .tasks
        .values()
        .filter(|task| task.state == TaskState::Failed)
        .count();
    format!(
        "# TYPE voyage_helms gauge\nvoyage_helms{{status=\"online\"}} {online}\nvoyage_helms{{status=\"total\"}} {}\n# TYPE voyage_tasks gauge\nvoyage_tasks{{state=\"queued\"}} {queued}\nvoyage_tasks{{state=\"running\"}} {running}\nvoyage_tasks{{state=\"failed\"}} {failed}\n",
        inner.helms.len()
    )
}

async fn diagnostics(State(state): State<AppState>) -> Json<serde_json::Value> {
    let inner = state.inner.read().await;
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
        "helms": inner.helms.len(),
        "pending_pairings": inner.pairings.values().filter(|pairing| !pairing.claimed).count(),
        "tasks": {
            "total": inner.tasks.len(),
            "queued": inner.tasks.values().filter(|task| task.state == TaskState::Queued).count(),
            "running": inner.tasks.values().filter(|task| task.state == TaskState::Running).count(),
            "failed": inner.tasks.values().filter(|task| task.state == TaskState::Failed).count()
        }
    }))
}

async fn correlate(mut request: Request<Body>, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            value.len() <= 128
                && value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    request.extensions_mut().insert(request_id.clone());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!("http.request", request_id = %request_id, %method, %path);
    let mut response = next.run(request).instrument(span).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

async fn start_pairing(
    State(state): State<AppState>,
    Json(request): Json<PairingStartRequest>,
) -> ApiResult<PairingStartResponse> {
    if request.protocol_version != PROTOCOL_VERSION {
        return api_error(StatusCode::UPGRADE_REQUIRED, "unsupported protocol version");
    }
    let code = Uuid::new_v4().simple().to_string()[..10].to_ascii_uppercase();
    let worker_token = Uuid::new_v4().as_simple().to_string();
    let expires_at = Utc::now() + state.pairing_ttl;
    state.inner.write().await.pairings.insert(
        code.clone(),
        PendingPairing {
            helm: request.helm,
            worker_token_hash: token_hash(&worker_token),
            expires_at,
            claimed: false,
        },
    );
    persist(&state, &*state.inner.read().await).map_err(internal_error)?;
    Ok(Json(PairingStartResponse {
        code,
        worker_token,
        expires_at,
        protocol_version: PROTOCOL_VERSION,
    }))
}

async fn claim_pairing(
    State(state): State<AppState>,
    Json(request): Json<PairingClaimRequest>,
) -> ApiResult<RegisteredHelm> {
    let code = request
        .connection_string
        .strip_prefix(PAIRING_PREFIX)
        .unwrap_or(&request.connection_string)
        .trim()
        .to_ascii_uppercase();
    let mut inner = state.inner.write().await;
    let pending = inner.pairings.get_mut(&code).ok_or_else(not_found)?;
    if pending.expires_at <= Utc::now() {
        return api_error(StatusCode::GONE, "pairing code expired");
    }
    if pending.claimed {
        return api_error(StatusCode::CONFLICT, "pairing code was already claimed");
    }
    pending.claimed = true;
    let helm_id = pending.helm.id;
    let token_hash = pending.worker_token_hash.clone();
    let now = Utc::now();
    let registered = RegisteredHelm {
        descriptor: pending.helm.clone(),
        status: HelmStatus::Online,
        paired_at: now,
        last_seen_at: now,
    };
    inner.worker_tokens.insert(token_hash, helm_id);
    inner.helms.insert(helm_id, registered.clone());
    inner.queues.entry(helm_id).or_default();
    persist(&state, &inner).map_err(internal_error)?;
    Ok(Json(registered))
}

async fn pairing_status(
    State(state): State<AppState>,
    Path(code): Path<String>,
    headers: HeaderMap,
) -> ApiResult<PairingStatus> {
    let inner = state.inner.read().await;
    let pending = inner
        .pairings
        .get(&code.to_ascii_uppercase())
        .ok_or_else(not_found)?;
    require_token(&headers, &pending.worker_token_hash)?;
    Ok(Json(PairingStatus {
        code: code.to_ascii_uppercase(),
        claimed: pending.claimed,
        expires_at: pending.expires_at,
    }))
}

async fn worker_heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<HeartbeatRequest>,
) -> ApiResult<RegisteredHelm> {
    let mut inner = state.inner.write().await;
    let id = worker_id(&inner, &headers)?;
    let helm = inner.helms.get_mut(&id).ok_or_else(not_found)?;
    if request.protocol_version != PROTOCOL_VERSION {
        return api_error(StatusCode::UPGRADE_REQUIRED, "unsupported protocol version");
    }
    helm.last_seen_at = Utc::now();
    helm.status = request.status;
    let response = helm.clone();
    persist(&state, &inner).map_err(internal_error)?;
    state.task_available.notify_waiters();
    Ok(Json(response))
}

async fn next_task(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Option<TaskEnvelope>> {
    let worker = {
        let inner = state.inner.read().await;
        worker_id(&inner, &headers)?
    };
    let notified = state.task_available.notified();
    if !state
        .inner
        .read()
        .await
        .queues
        .get(&worker)
        .is_some_and(|queue| !queue.is_empty())
    {
        let _ = tokio::time::timeout(Duration::from_secs(25), notified).await;
    }
    let mut inner = state.inner.write().await;
    let id = worker_id(&inner, &headers)?;
    let task = inner.queues.entry(id).or_default().pop_front();
    let task = task.map(|mut task| {
        let lease_id = Uuid::new_v4();
        let expires = Utc::now()
            + ChronoDuration::from_std(state.lease_duration)
                .unwrap_or_else(|_| ChronoDuration::seconds(60));
        task.lease_id = lease_id;
        task.lease_expires_at = expires;
        task.attempt += 1;
        if let Some(record) = inner.tasks.get_mut(&task.id) {
            record.state = TaskState::Running;
            record.updated_at = Utc::now();
            record.attempt = task.attempt;
            record.lease_id = Some(lease_id);
            record.lease_expires_at = Some(expires);
        }
        task
    });
    persist(&state, &inner).map_err(internal_error)?;
    Ok(Json(task))
}

async fn complete_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(completion): Json<TaskCompletion>,
) -> ApiResult<TaskRecord> {
    let mut inner = state.inner.write().await;
    let helm_id = worker_id(&inner, &headers)?;
    if completion.result.task_id != id {
        return api_error(StatusCode::BAD_REQUEST, "task id mismatch");
    }
    let record = inner.tasks.get_mut(&id).ok_or_else(not_found)?;
    if record.helm_id != helm_id {
        return api_error(StatusCode::FORBIDDEN, "task belongs to another Helm");
    }
    if record.lease_id != Some(completion.lease_id) {
        return api_error(StatusCode::CONFLICT, "task lease is stale");
    }
    if record.state == TaskState::Cancelled {
        return api_error(StatusCode::CONFLICT, "task was cancelled");
    }
    record.state = TaskState::Completed;
    record.result = Some(completion.result);
    record.lease_id = None;
    record.lease_expires_at = None;
    record.updated_at = Utc::now();
    let response = record.clone();
    persist(&state, &inner).map_err(internal_error)?;
    Ok(Json(response))
}

async fn list_helms(State(state): State<AppState>) -> Json<Vec<RegisteredHelm>> {
    let mut values: Vec<_> = state.inner.read().await.helms.values().cloned().collect();
    values.sort_by(|a, b| a.descriptor.name.cmp(&b.descriptor.name));
    Json(values)
}
async fn get_helm(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<RegisteredHelm> {
    state
        .inner
        .read()
        .await
        .helms
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(not_found)
}
async fn remove_helm(State(state): State<AppState>, Path(id): Path<Uuid>) -> StatusCode {
    let mut inner = state.inner.write().await;
    if inner.helms.remove(&id).is_none() {
        return StatusCode::NOT_FOUND;
    }
    inner.worker_tokens.retain(|_, value| *value != id);
    inner.queues.remove(&id);
    persist(&state, &inner).ok();
    StatusCode::NO_CONTENT
}
async fn enqueue_task(
    State(state): State<AppState>,
    Path(helm_id): Path<Uuid>,
    Json(request): Json<TaskRequest>,
) -> ApiResult<TaskRecord> {
    let mut inner = state.inner.write().await;
    if !inner.helms.contains_key(&helm_id) {
        return Err(not_found());
    }
    let now = Utc::now();
    let id = Uuid::new_v4();
    let envelope = TaskEnvelope {
        id,
        request: request.clone(),
        created_at: now,
        attempt: 0,
        lease_id: Uuid::nil(),
        lease_expires_at: now,
    };
    let record = TaskRecord {
        id,
        helm_id,
        request,
        state: TaskState::Queued,
        result: None,
        created_at: now,
        updated_at: now,
        attempt: 0,
        max_attempts: 3,
        lease_id: None,
        lease_expires_at: None,
        last_error: None,
    };
    inner.queues.entry(helm_id).or_default().push_back(envelope);
    inner.tasks.insert(id, record.clone());
    persist(&state, &inner).map_err(internal_error)?;
    state.task_available.notify_waiters();
    Ok(Json(record))
}
async fn get_task(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<TaskRecord> {
    state
        .inner
        .read()
        .await
        .tasks
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(not_found)
}

async fn list_tasks(State(state): State<AppState>) -> Json<Vec<TaskRecord>> {
    let mut tasks: Vec<_> = state.inner.read().await.tasks.values().cloned().collect();
    tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at));
    Json(tasks)
}

async fn fail_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(failure): Json<TaskFailure>,
) -> ApiResult<TaskRecord> {
    let mut inner = state.inner.write().await;
    let helm_id = worker_id(&inner, &headers)?;
    let mut retry = None;
    {
        let record = inner.tasks.get_mut(&id).ok_or_else(not_found)?;
        if failure.task_id != id || record.helm_id != helm_id {
            return api_error(StatusCode::FORBIDDEN, "invalid task ownership");
        }
        if record.lease_id != Some(failure.lease_id) {
            return api_error(StatusCode::CONFLICT, "task lease is stale");
        }
        record.last_error = Some(failure.error);
        record.lease_id = None;
        record.lease_expires_at = None;
        record.updated_at = Utc::now();
        if failure.retryable && record.attempt < record.max_attempts {
            record.state = TaskState::Queued;
            retry = Some((
                record.helm_id,
                TaskEnvelope {
                    id,
                    request: record.request.clone(),
                    created_at: record.created_at,
                    attempt: record.attempt,
                    lease_id: Uuid::nil(),
                    lease_expires_at: Utc::now(),
                },
            ));
        } else {
            record.state = TaskState::Failed;
        }
    }
    if let Some((helm_id, envelope)) = retry {
        inner.queues.entry(helm_id).or_default().push_back(envelope);
    }
    let response = inner.tasks[&id].clone();
    persist(&state, &inner).map_err(internal_error)?;
    state.task_available.notify_waiters();
    Ok(Json(response))
}

async fn cancel_task(State(state): State<AppState>, Path(id): Path<Uuid>) -> ApiResult<TaskRecord> {
    let mut inner = state.inner.write().await;
    let record = inner.tasks.get_mut(&id).ok_or_else(not_found)?;
    if matches!(record.state, TaskState::Completed | TaskState::Failed) {
        return api_error(StatusCode::CONFLICT, "task is terminal");
    }
    record.state = TaskState::Cancelled;
    record.lease_id = None;
    record.lease_expires_at = None;
    record.updated_at = Utc::now();
    let response = record.clone();
    for queue in inner.queues.values_mut() {
        queue.retain(|task| task.id != id);
    }
    persist(&state, &inner).map_err(internal_error)?;
    Ok(Json(response))
}

async fn fleet_summary(State(state): State<AppState>) -> Json<FleetSummary> {
    let inner = state.inner.read().await;
    Json(FleetSummary {
        total_helms: inner.helms.len(),
        online_helms: inner
            .helms
            .values()
            .filter(|h| h.status == HelmStatus::Online)
            .count(),
        queued_tasks: inner
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Queued)
            .count(),
        running_tasks: inner
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Running)
            .count(),
        failed_tasks: inner
            .tasks
            .values()
            .filter(|t| t.state == TaskState::Failed)
            .count(),
    })
}

struct UiError(Box<Response>);
impl IntoResponse for UiError {
    fn into_response(self) -> Response {
        *self.0
    }
}
fn ui_error(value: impl IntoResponse) -> UiError {
    UiError(Box::new(value.into_response()))
}
type UiResult<T> = Result<T, UiError>;

#[derive(Deserialize)]
struct CreateTaskForm {
    prompt: String,
}

fn operator_auth(state: &AppState, headers: &HeaderMap) -> UiResult<()> {
    let Some(expected) = state.operator_token_hash.as_deref() else {
        return Err(ui_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Operator UI is disabled. Set VESSEL_OPERATOR_TOKEN.",
        )));
    };
    let supplied = bearer(headers).map(str::to_owned).or_else(|| {
        let encoded = headers
            .get(axum::http::header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Basic ")?;
        let decoded = STANDARD.decode(encoded).ok()?;
        let credentials = std::str::from_utf8(&decoded).ok()?;
        credentials
            .split_once(':')
            .map(|(_, password)| password.to_owned())
    });
    if supplied
        .is_some_and(|token| constant_time_eq(token_hash(&token).as_bytes(), expected.as_bytes()))
    {
        Ok(())
    } else {
        let mut response =
            (StatusCode::UNAUTHORIZED, "Operator authentication required").into_response();
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Vessel operator\", charset=\"UTF-8\""),
        );
        Err(ui_error(response))
    }
}

fn operator_write_auth(state: &AppState, headers: &HeaderMap) -> UiResult<()> {
    operator_auth(state, headers)?;
    let uses_basic = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Basic "));
    if uses_basic {
        let host = headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok());
        let origin_host = headers
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .and_then(|origin| {
                origin
                    .split_once("://")
                    .map(|(_, rest)| rest.trim_end_matches('/'))
            });
        if host != origin_host {
            return Err(ui_error((
                StatusCode::FORBIDDEN,
                "Cross-origin operator action rejected",
            )));
        }
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn page(title: &str, body: String) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><meta http-equiv="refresh" content="5"><title>{title} · Vessel</title><style>
    :root{{--bg:#0b1020;--panel:#151c30;--text:#e8edf8;--muted:#94a3b8;--line:#28344d;--accent:#67e8f9;--bad:#fda4af;--good:#86efac}}*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--text);font:15px system-ui,sans-serif}}main{{max-width:1180px;margin:auto;padding:28px}}a{{color:var(--accent)}}header{{display:flex;justify-content:space-between;align-items:center}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:14px}}.card,table,form{{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:16px}}table{{width:100%;border-collapse:collapse;margin:16px 0}}th,td{{text-align:left;padding:10px;border-bottom:1px solid var(--line);vertical-align:top}}.muted{{color:var(--muted)}}.state{{font-weight:700}}input,textarea,button{{width:100%;background:#0c1427;color:var(--text);border:1px solid #3a4968;border-radius:6px;padding:10px;margin:5px 0}}button{{cursor:pointer;width:auto}}code{{overflow-wrap:anywhere}}.error{{color:var(--bad)}}.online,.completed{{color:var(--good)}}.offline,.failed,.cancelled{{color:var(--bad)}}
    </style></head><body><main><header><h1><a href="/ui">Vessel</a> · {title}</h1><span class="muted">auto-refresh 5s · protocol v{PROTOCOL_VERSION}</span></header>{body}</main></body></html>"#
    ))
}

fn h(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn operator_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> UiResult<Html<String>> {
    operator_auth(&state, &headers)?;
    let inner = state.inner.read().await;
    let mut helms: Vec<_> = inner.helms.values().collect();
    helms.sort_by_key(|helm| &helm.descriptor.name);
    let mut tasks: Vec<_> = inner.tasks.values().collect();
    tasks.sort_by_key(|task| std::cmp::Reverse(task.created_at));
    let helm_rows = helms.iter().map(|helm| format!("<tr><td><a href='/ui/helms/{0}'>{1}</a><br><code>{0}</code></td><td class='state'>{2:?}</td><td>{3}</td><td>{4}</td><td>{5}</td></tr>", helm.descriptor.id, h(&helm.descriptor.name), helm.status, h(&helm.descriptor.version), h(&helm.descriptor.model), helm.last_seen_at.to_rfc3339())).collect::<String>();
    let task_rows = tasks.iter().take(100).map(|task| format!("<tr><td><a href='/ui/tasks/{0}'><code>{0}</code></a></td><td><code>{1}</code></td><td class='state'>{2:?}</td><td>{3}/{4}</td><td>{5}</td></tr>", task.id, task.helm_id, task.state, task.attempt, task.max_attempts, task.updated_at.to_rfc3339())).collect::<String>();
    let summary = FleetSummary {
        total_helms: inner.helms.len(),
        online_helms: inner
            .helms
            .values()
            .filter(|helm| helm.status == HelmStatus::Online)
            .count(),
        queued_tasks: inner
            .tasks
            .values()
            .filter(|task| task.state == TaskState::Queued)
            .count(),
        running_tasks: inner
            .tasks
            .values()
            .filter(|task| task.state == TaskState::Running)
            .count(),
        failed_tasks: inner
            .tasks
            .values()
            .filter(|task| task.state == TaskState::Failed)
            .count(),
    };
    Ok(page(
        "Fleet",
        format!(
            "<section class='grid'><div class='card'><b>{}</b><div class='muted'>Helms ({} online)</div></div><div class='card'><b>{}</b><div class='muted'>Queued tasks</div></div><div class='card'><b>{}</b><div class='muted'>Running tasks</div></div><div class='card'><b>{}</b><div class='muted'>Failed tasks</div></div></section><h2>Workers</h2>{}<table><tr><th>Helm</th><th>Status</th><th>Version</th><th>Model</th><th>Last seen (UTC)</th></tr>{}</table><h2>Recent tasks</h2>{}<table><tr><th>Correlation / task ID</th><th>Helm</th><th>State</th><th>Attempt</th><th>Updated (UTC)</th></tr>{}</table>",
            summary.total_helms,
            summary.online_helms,
            summary.queued_tasks,
            summary.running_tasks,
            summary.failed_tasks,
            if helms.is_empty() {
                "<p class='muted'>No Helms paired. Run <code>helm --voyage</code> and claim its pairing string.</p>"
            } else {
                ""
            },
            helm_rows,
            if tasks.is_empty() {
                "<p class='muted'>No tasks have been created.</p>"
            } else {
                ""
            },
            task_rows
        ),
    ))
}

async fn operator_helm(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> UiResult<Html<String>> {
    operator_auth(&state, &headers)?;
    let inner = state.inner.read().await;
    let helm = inner
        .helms
        .get(&id)
        .ok_or_else(|| ui_error((StatusCode::NOT_FOUND, "Helm not found")))?;
    let caps = helm
        .descriptor
        .capabilities
        .iter()
        .map(|cap| format!("<code>{}</code> ", h(cap)))
        .collect::<String>();
    Ok(page(
        &h(&helm.descriptor.name),
        format!(
            "<div class='grid'><div class='card'><b class='state'>{0:?}</b><div class='muted'>status</div></div><div class='card'><b>{1}</b><div class='muted'>version</div></div><div class='card'><b>{2}</b><div class='muted'>model</div></div></div><p><b>ID:</b> <code>{3}</code><br><b>Paired:</b> {4}<br><b>Last seen:</b> {5}<br><b>Capabilities:</b> {6}</p><h2>Create task</h2><form method='post' action='/ui/helms/{3}/tasks'><label>Prompt<textarea name='prompt' required minlength='1' maxlength='100000' rows='8'></textarea></label><button>Create task</button></form>",
            helm.status,
            h(&helm.descriptor.version),
            h(&helm.descriptor.model),
            helm.descriptor.id,
            helm.paired_at.to_rfc3339(),
            helm.last_seen_at.to_rfc3339(),
            caps
        ),
    ))
}

async fn operator_create_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Form(form): Form<CreateTaskForm>,
) -> UiResult<Redirect> {
    operator_write_auth(&state, &headers)?;
    let prompt = form.prompt.trim();
    if prompt.is_empty() {
        return Err(ui_error((
            StatusCode::BAD_REQUEST,
            "Prompt cannot be empty",
        )));
    }
    let record = enqueue_task(
        State(state),
        Path(id),
        Json(TaskRequest {
            prompt: prompt.to_owned(),
            session_id: None,
        }),
    )
    .await
    .map_err(|(status, Json(error))| ui_error((status, error.error)))?;
    Ok(Redirect::to(&format!("/ui/tasks/{}", record.0.id)))
}

async fn operator_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> UiResult<Html<String>> {
    operator_auth(&state, &headers)?;
    let inner = state.inner.read().await;
    let task = inner
        .tasks
        .get(&id)
        .ok_or_else(|| ui_error((StatusCode::NOT_FOUND, "Task not found")))?;
    let result=task.result.as_ref().map(|r|format!("<h2>Result</h2><div class='card'><pre>{}</pre><p>{} input / {} output tokens · session <code>{}</code></p></div>",h(&r.answer),r.input_tokens,r.output_tokens,r.session_id)).unwrap_or_default();
    let actions = if matches!(task.state, TaskState::Queued | TaskState::Running) {
        format!(
            "<form method='post' action='/ui/tasks/{id}/cancel'><button>Cancel task</button></form>"
        )
    } else if matches!(task.state, TaskState::Failed | TaskState::Cancelled) {
        format!(
            "<form method='post' action='/ui/tasks/{id}/retry'><button>Retry task</button></form>"
        )
    } else {
        String::new()
    };
    Ok(page(
        "Task",
        format!(
            "<div class='grid'><div class='card'><b class='state'>{0:?}</b><div class='muted'>state</div></div><div class='card'><b>{1}/{2}</b><div class='muted'>attempts</div></div></div><p><b>Correlation / task ID:</b> <code>{3}</code><br><b>Helm:</b> <a href='/ui/helms/{4}'><code>{4}</code></a><br><b>Created:</b> {5}<br><b>Updated:</b> {6}<br><b>Lease:</b> <code>{7}</code><br><b>Lease expires:</b> {8}</p><h2>Prompt</h2><div class='card'><pre>{9}</pre></div>{10}{11}<p class='error'>{12}</p>",
            task.state,
            task.attempt,
            task.max_attempts,
            task.id,
            task.helm_id,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
            task.lease_id.map_or_else(|| "—".into(), |v| v.to_string()),
            task.lease_expires_at
                .map_or_else(|| "—".into(), |v| v.to_rfc3339()),
            h(&task.request.prompt),
            result,
            actions,
            h(task.last_error.as_deref().unwrap_or(""))
        ),
    ))
}

async fn operator_cancel_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> UiResult<Redirect> {
    operator_write_auth(&state, &headers)?;
    let _ = cancel_task(State(state), Path(id))
        .await
        .map_err(|(s, Json(e))| ui_error((s, e.error)))?;
    Ok(Redirect::to(&format!("/ui/tasks/{id}")))
}

async fn operator_retry_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> UiResult<Redirect> {
    operator_write_auth(&state, &headers)?;
    let mut inner = state.inner.write().await;
    let record = inner
        .tasks
        .get_mut(&id)
        .ok_or_else(|| ui_error((StatusCode::NOT_FOUND, "Task not found")))?;
    if !matches!(record.state, TaskState::Failed | TaskState::Cancelled) {
        return Err(ui_error((
            StatusCode::CONFLICT,
            "Only failed or cancelled tasks can be retried",
        )));
    }
    record.state = TaskState::Queued;
    record.attempt = 0;
    record.last_error = None;
    record.result = None;
    record.updated_at = Utc::now();
    let helm_id = record.helm_id;
    let envelope = TaskEnvelope {
        id,
        request: record.request.clone(),
        created_at: record.created_at,
        attempt: 0,
        lease_id: Uuid::nil(),
        lease_expires_at: Utc::now(),
    };
    inner.queues.entry(helm_id).or_default().push_back(envelope);
    persist(&state, &inner).map_err(|error| ui_error(internal_error(error)))?;
    state.task_available.notify_waiters();
    Ok(Redirect::to(&format!("/ui/tasks/{id}")))
}

fn worker_id(
    inner: &ControlPlane,
    headers: &HeaderMap,
) -> Result<Uuid, (StatusCode, Json<ApiError>)> {
    let token = bearer(headers).ok_or_else(unauthorized)?;
    inner
        .worker_tokens
        .get(&token_hash(token))
        .copied()
        .ok_or_else(unauthorized)
}
fn require_token(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, Json<ApiError>)> {
    if bearer(headers).is_some_and(|token| token_hash(token) == expected) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn open_database(path: &std::path::Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY); CREATE TABLE IF NOT EXISTS control_plane(id INTEGER PRIMARY KEY CHECK(id=1), state TEXT NOT NULL); INSERT OR IGNORE INTO schema_migrations(version) VALUES(1);")?;
    Ok(connection)
}
fn load_state(connection: &Connection) -> Result<Option<ControlPlane>> {
    let value: Option<String> = connection
        .query_row("SELECT state FROM control_plane WHERE id=1", [], |row| {
            row.get(0)
        })
        .optional()?;
    value
        .map(|json| serde_json::from_str(&json).map_err(Into::into))
        .transpose()
}
fn persist(state: &AppState, inner: &ControlPlane) -> Result<()> {
    let json = serde_json::to_string(inner)?;
    let database = state
        .database
        .lock()
        .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
    database.execute("INSERT INTO control_plane(id,state) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET state=excluded.state", params![json])?;
    Ok(())
}
fn internal_error(error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::error!(%error, "persistent state operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            error: "persistent state operation failed".into(),
        }),
    )
}
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}
fn unauthorized() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiError {
            error: "invalid worker credential".into(),
        }),
    )
}
fn not_found() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: "resource not found".into(),
        }),
    )
}
fn api_error<T>(status: StatusCode, message: impl Into<String>) -> ApiResult<T> {
    Err((
        status,
        Json(ApiError {
            error: message.into(),
        }),
    ))
}

fn spawn_reaper(state: AppState, stale_after: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(stale_after / 2);
        loop {
            interval.tick().await;
            let now = Utc::now();
            let mut inner = state.inner.write().await;
            inner.pairings.retain(|_, p| p.expires_at > now);
            for helm in inner.helms.values_mut() {
                let age = now
                    .signed_duration_since(helm.last_seen_at)
                    .to_std()
                    .unwrap_or_default();
                helm.status = if age > stale_after * 2 {
                    HelmStatus::Offline
                } else if age > stale_after {
                    HelmStatus::Stale
                } else {
                    HelmStatus::Online
                };
            }
            let expired: Vec<_> = inner
                .tasks
                .values()
                .filter(|task| {
                    task.state == TaskState::Running
                        && task.lease_expires_at.is_some_and(|expiry| expiry <= now)
                })
                .map(|task| task.id)
                .collect();
            for id in expired {
                let mut retry = None;
                if let Some(record) = inner.tasks.get_mut(&id) {
                    record.lease_id = None;
                    record.lease_expires_at = None;
                    record.last_error = Some("worker lease expired".into());
                    record.updated_at = now;
                    if record.attempt < record.max_attempts {
                        record.state = TaskState::Queued;
                        retry = Some((
                            record.helm_id,
                            TaskEnvelope {
                                id,
                                request: record.request.clone(),
                                created_at: record.created_at,
                                attempt: record.attempt,
                                lease_id: Uuid::nil(),
                                lease_expires_at: now,
                            },
                        ));
                    } else {
                        record.state = TaskState::Failed;
                    }
                }
                if let Some((helm_id, envelope)) = retry {
                    inner.queues.entry(helm_id).or_default().push_back(envelope);
                }
            }
            if let Err(error) = persist(&state, &inner) {
                tracing::error!(%error, "could not persist reaper state");
            }
            state.task_available.notify_waiters();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_round_trip_and_migration() {
        let dir = tempfile::tempdir().unwrap();
        let database = open_database(&dir.path().join("vessel.db")).unwrap();
        assert!(load_state(&database).unwrap().is_none());
        let state = AppState {
            inner: Arc::new(RwLock::new(ControlPlane::default())),
            database: Arc::new(Mutex::new(database)),
            lease_duration: Duration::from_secs(10),
            task_available: Arc::new(tokio::sync::Notify::new()),
            pairing_ttl: ChronoDuration::seconds(10),
            operator_token_hash: None,
        };
        persist(&state, &ControlPlane::default()).unwrap();
        assert!(
            load_state(&state.database.lock().unwrap())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn credentials_are_stored_as_hashes() {
        assert_ne!(token_hash("secret"), "secret");
        assert_eq!(token_hash("secret"), token_hash("secret"));
    }

    #[test]
    fn operator_output_escapes_untrusted_content() {
        assert_eq!(
            h("<script>alert('x') & \"y\"</script>"),
            "&lt;script&gt;alert(&#39;x&#39;) &amp; &quot;y&quot;&lt;/script&gt;"
        );
    }

    #[test]
    fn constant_time_comparison_rejects_mismatches() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }
}
