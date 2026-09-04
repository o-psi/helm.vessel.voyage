use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand};
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
}

#[derive(Clone)]
struct AppState {
    inner: Arc<RwLock<ControlPlane>>,
    database: Arc<Mutex<Connection>>,
    lease_duration: Duration,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vessel=info".into()),
        )
        .init();
    let cli = Cli::parse();
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
        .route("/v1/worker/tasks/{id}/failure", post(fail_task))
        .route("/v1/helms", get(list_helms))
        .route("/v1/helms/{id}", get(get_helm).delete(remove_helm))
        .route("/v1/helms/{id}/tasks", post(enqueue_task))
        .route("/v1/tasks/{id}", get(get_task))
        .route("/v1/tasks", get(list_tasks))
        .route("/v1/tasks/{id}/cancel", post(cancel_task))
        .route("/v1/fleet/summary", get(fleet_summary))
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
) -> ApiResult<PairingStartResponse> {
    if request.protocol_version != PROTOCOL_VERSION {
        return api_error(StatusCode::UPGRADE_REQUIRED, "unsupported protocol version");
    }
    let code = Uuid::new_v4().simple().to_string()[..10].to_ascii_uppercase();
    let worker_token = Uuid::new_v4().as_simple().to_string();
    let expires_at = Utc::now() + ChronoDuration::minutes(10);
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
    Ok(Json(response))
}

async fn next_task(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> ApiResult<Option<TaskEnvelope>> {
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
}
