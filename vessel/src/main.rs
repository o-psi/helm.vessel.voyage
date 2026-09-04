use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::Utc;
use clap::Parser;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use uuid::Uuid;
use voyage_protocol::{
    ApiError, HealthResponse, HeartbeatRequest, HelmStatus, RegisteredHelm, RegistrationRequest,
    TaskRequest, TaskResponse,
};

#[derive(Parser)]
#[command(version, about = "Voyage management plane for Helm fleets")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:9480")]
    bind: String,
    #[arg(long, default_value_t = 90)]
    stale_after_secs: u64,
    /// Bearer token used when dispatching to protected Helms.
    #[arg(long, env = "VESSEL_HELM_TOKEN")]
    helm_token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    helms: Arc<RwLock<HashMap<Uuid, RegisteredHelm>>>,
    client: reqwest::Client,
    helm_token: Option<String>,
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
    let state = AppState {
        helms: Arc::new(RwLock::new(HashMap::new())),
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?,
        helm_token: cli.helm_token,
    };
    spawn_reaper(state.clone(), Duration::from_secs(cli.stale_after_secs));
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/helms", get(list_helms))
        .route("/v1/helms/register", post(register))
        .route("/v1/helms/heartbeat", post(heartbeat))
        .route("/v1/helms/{id}", get(get_helm).delete(remove_helm))
        .route("/v1/helms/{id}/tasks", post(dispatch_task))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    tracing::info!(address = %cli.bind, "Vessel ready");
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
async fn list_helms(State(state): State<AppState>) -> Json<Vec<RegisteredHelm>> {
    let mut helms: Vec<_> = state.helms.read().await.values().cloned().collect();
    helms.sort_by(|a, b| a.descriptor.name.cmp(&b.descriptor.name));
    Json(helms)
}
async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegistrationRequest>,
) -> ApiResult<RegisteredHelm> {
    if !(request.helm.endpoint.starts_with("http://")
        || request.helm.endpoint.starts_with("https://"))
    {
        return api_error(StatusCode::BAD_REQUEST, "endpoint must use http or https");
    }
    let now = Utc::now();
    let registered = RegisteredHelm {
        descriptor: request.helm,
        status: HelmStatus::Online,
        registered_at: now,
        last_seen_at: now,
    };
    state
        .helms
        .write()
        .await
        .insert(registered.descriptor.id, registered.clone());
    Ok(Json(registered))
}
async fn heartbeat(
    State(state): State<AppState>,
    Json(request): Json<HeartbeatRequest>,
) -> ApiResult<RegisteredHelm> {
    let mut helms = state.helms.write().await;
    let helm = helms.get_mut(&request.id).ok_or_else(not_found)?;
    helm.last_seen_at = Utc::now();
    helm.status = HelmStatus::Online;
    Ok(Json(helm.clone()))
}
async fn get_helm(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<RegisteredHelm> {
    state
        .helms
        .read()
        .await
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(not_found)
}
async fn remove_helm(State(state): State<AppState>, Path(id): Path<Uuid>) -> StatusCode {
    if state.helms.write().await.remove(&id).is_some() {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}
async fn dispatch_task(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<TaskRequest>,
) -> ApiResult<TaskResponse> {
    let helm = state
        .helms
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(not_found)?;
    if helm.status != HelmStatus::Online {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "Helm is not online");
    }
    let mut outbound = state.client.post(format!(
        "{}/v1/tasks",
        helm.descriptor.endpoint.trim_end_matches('/')
    ));
    if let Some(token) = &state.helm_token {
        outbound = outbound.bearer_auth(token);
    }
    let response = outbound
        .json(&request)
        .send()
        .await
        .map_err(|e| gateway(e.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return api_error(StatusCode::BAD_GATEWAY, format!("Helm returned {status}"));
    }
    response
        .json::<TaskResponse>()
        .await
        .map(Json)
        .map_err(|e| gateway(e.to_string()))
}

fn spawn_reaper(state: AppState, stale_after: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(stale_after / 2);
        loop {
            interval.tick().await;
            let now = Utc::now();
            for helm in state.helms.write().await.values_mut() {
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
fn not_found() -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            error: "Helm not found".into(),
        }),
    )
}
fn gateway(message: String) -> (StatusCode, Json<ApiError>) {
    (StatusCode::BAD_GATEWAY, Json(ApiError { error: message }))
}
fn api_error<T>(status: StatusCode, message: impl Into<String>) -> ApiResult<T> {
    Err((
        status,
        Json(ApiError {
            error: message.into(),
        }),
    ))
}
