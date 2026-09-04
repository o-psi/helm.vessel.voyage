use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;
use uuid::Uuid;
use voyage_protocol::{
    ApiError, HealthResponse, HeartbeatRequest, HelmStatus, PAIRING_PREFIX, PairingClaimRequest,
    PairingStartRequest, PairingStartResponse, PairingStatus, RegisteredHelm, TaskEnvelope,
    TaskRecord, TaskRequest, TaskResult, TaskState,
};

#[derive(Parser)]
#[command(version, about = "Voyage management plane for outbound Helm workers")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:9480")]
    bind: String,
    #[arg(long, default_value_t = 90)]
    stale_after_secs: u64,
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
    /// Generate a shell completion script on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff manpage on stdout.
    Manpage,
}

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<ControlPlane>>,
}
#[derive(Default)]
struct ControlPlane {
    pairings: HashMap<String, PendingPairing>,
    helms: HashMap<Uuid, RegisteredHelm>,
    worker_tokens: HashMap<String, Uuid>,
    queues: HashMap<Uuid, VecDeque<TaskEnvelope>>,
    tasks: HashMap<Uuid, TaskRecord>,
}
struct PendingPairing {
    helm: voyage_protocol::HelmDescriptor,
    worker_token: String,
    expires_at: chrono::DateTime<Utc>,
    claimed: bool,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vessel=info".into()),
        )
        .init();
    let cli = Cli::parse();
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
    }) = cli.command
    {
        let paired: RegisteredHelm = reqwest::Client::new()
            .post(format!(
                "{}/v1/pairings/claim",
                vessel.trim_end_matches('/')
            ))
            .json(&PairingClaimRequest { connection_string })
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
    let state = AppState {
        inner: Arc::new(RwLock::new(ControlPlane::default())),
    };
    spawn_reaper(state.clone(), Duration::from_secs(cli.stale_after_secs));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/pairings/start", post(start_pairing))
        .route("/v1/pairings/claim", post(claim_pairing))
        .route("/v1/pairings/{code}", get(pairing_status))
        .route("/v1/worker/heartbeat", post(worker_heartbeat))
        .route("/v1/worker/tasks/next", get(next_task))
        .route("/v1/worker/tasks/{id}/result", post(complete_task))
        .route("/v1/helms", get(list_helms))
        .route("/v1/helms/{id}", get(get_helm).delete(remove_helm))
        .route("/v1/helms/{id}/tasks", post(enqueue_task))
        .route("/v1/tasks/{id}", get(get_task))
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

async fn start_pairing(
    State(state): State<AppState>,
    Json(request): Json<PairingStartRequest>,
) -> Json<PairingStartResponse> {
    let code = Uuid::new_v4().simple().to_string()[..10].to_ascii_uppercase();
    let worker_token = Uuid::new_v4().as_simple().to_string();
    let expires_at = Utc::now() + ChronoDuration::minutes(10);
    state.inner.write().await.pairings.insert(
        code.clone(),
        PendingPairing {
            helm: request.helm,
            worker_token: worker_token.clone(),
            expires_at,
            claimed: false,
        },
    );
    Json(PairingStartResponse {
        code,
        worker_token,
        expires_at,
    })
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
    pending.claimed = true;
    let helm_id = pending.helm.id;
    let token = pending.worker_token.clone();
    let now = Utc::now();
    let registered = RegisteredHelm {
        descriptor: pending.helm.clone(),
        status: HelmStatus::Online,
        paired_at: now,
        last_seen_at: now,
    };
    inner.worker_tokens.insert(token, helm_id);
    inner.helms.insert(helm_id, registered.clone());
    inner.queues.entry(helm_id).or_default();
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
    require_token(&headers, &pending.worker_token)?;
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
    helm.last_seen_at = Utc::now();
    helm.status = request.status;
    Ok(Json(helm.clone()))
}

async fn next_task(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Option<TaskEnvelope>> {
    let mut inner = state.inner.write().await;
    let id = worker_id(&inner, &headers)?;
    let task = inner.queues.entry(id).or_default().pop_front();
    if let Some(task) = &task
        && let Some(record) = inner.tasks.get_mut(&task.id)
    {
        record.state = TaskState::Running;
        record.updated_at = Utc::now();
    }
    Ok(Json(task))
}

async fn complete_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(result): Json<TaskResult>,
) -> ApiResult<TaskRecord> {
    let mut inner = state.inner.write().await;
    let helm_id = worker_id(&inner, &headers)?;
    if result.task_id != id {
        return api_error(StatusCode::BAD_REQUEST, "task id mismatch");
    }
    let record = inner.tasks.get_mut(&id).ok_or_else(not_found)?;
    if record.helm_id != helm_id {
        return api_error(StatusCode::FORBIDDEN, "task belongs to another Helm");
    }
    record.state = TaskState::Completed;
    record.result = Some(result);
    record.updated_at = Utc::now();
    Ok(Json(record.clone()))
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
    };
    let record = TaskRecord {
        id,
        helm_id,
        request,
        state: TaskState::Queued,
        result: None,
        created_at: now,
        updated_at: now,
    };
    inner.queues.entry(helm_id).or_default().push_back(envelope);
    inner.tasks.insert(id, record.clone());
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

fn worker_id(
    inner: &ControlPlane,
    headers: &HeaderMap,
) -> Result<Uuid, (StatusCode, Json<ApiError>)> {
    let token = bearer(headers).ok_or_else(unauthorized)?;
    inner
        .worker_tokens
        .get(token)
        .copied()
        .ok_or_else(unauthorized)
}
fn require_token(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, Json<ApiError>)> {
    if bearer(headers) == Some(expected) {
        Ok(())
    } else {
        Err(unauthorized())
    }
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
        }
    });
}
