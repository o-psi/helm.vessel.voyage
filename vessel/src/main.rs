mod auth;
mod http_boundary;
mod process_http;
use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, StatusCode},
    middleware,
    middleware::Next,
    response::{Html, IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tracing::Instrument;
use uuid::Uuid;
use voyage_protocol::{ApiError, HealthResponse};

#[derive(Parser)]
#[command(version, about = "Voyage management plane")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1:9480")]
    bind: String,
    #[arg(long, default_value = "vessel.db")]
    database: PathBuf,
    /// Expose the authenticated process gateway to a private local supervisor.
    #[arg(long, requires = "public_origin")]
    process_directory: Option<PathBuf>,
    /// Canonical HTTPS origin served by a TLS proxy on this host.
    #[arg(long, requires = "process_directory")]
    public_origin: Option<String>,
    /// Allow HTTP only for literal loopback development origins.
    #[arg(long, requires = "public_origin")]
    allow_insecure_loopback: bool,
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
    /// Manage provider credentials for this local executing account.
    Auth {
        #[command(subcommand)]
        command: auth::AuthCommand,
    },
    /// Issue an explicit scoped credential through the local supervisor.
    #[cfg(target_os = "linux")]
    ProcessGrant(vessel::process::grant_cli::GrantArgs),
    /// Approve a principal-bound workspace invitation, written only to a private file.
    #[cfg(target_os = "linux")]
    PairInvite(vessel::process::pair_cli::PairInviteArgs),
    /// List workspace connection scope and revocation IDs without credentials.
    #[cfg(target_os = "linux")]
    ListConnections(vessel::process::pair_cli::ListConnectionsArgs),
    /// Revoke a workspace connection and its derived runtime authority.
    #[cfg(target_os = "linux")]
    RevokeConnection(vessel::process::pair_cli::RevokeConnectionArgs),
    /// Revoke a scoped credential; current dispatch checks fail closed immediately.
    #[cfg(target_os = "linux")]
    ProcessRevoke {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        grant: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Serve authenticated loopback duplex sockets and the compatibility HTTP API.
    LocalServe {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        voyage_binary: Option<PathBuf>,
        /// Deprecated compatibility option; voyage count is no longer capped.
        #[arg(long, hide = true)]
        capacity: Option<usize>,
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
    process_directory: Option<PathBuf>,
    database: Arc<Mutex<Connection>>,
    operator_token_hash: Option<String>,
    public_origin: Option<String>,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "vessel=info".into());
    match cli.log_format {
        LogFormat::Text => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init(),
        LogFormat::Json => tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .json()
            .with_env_filter(filter)
            .init(),
    }
    match &cli.command {
        Some(Command::Auth { command }) => return auth::auth(command).await,
        #[cfg(target_os = "linux")]
        Some(Command::ProcessGrant(_)) => {}
        #[cfg(target_os = "linux")]
        Some(Command::PairInvite(_)) | Some(Command::RevokeConnection(_)) => {}
        #[cfg(target_os = "linux")]
        Some(Command::ProcessRevoke {
            directory,
            grant,
            expected_revision,
            command_id,
        }) => {
            return vessel::process::grant_cli::revoke(
                directory.clone(),
                *grant,
                *expected_revision,
                *command_id,
            )
            .await;
        }
        Some(Command::LocalServe {
            directory,
            voyage_binary,
            capacity: _,
        }) => {
            #[cfg(target_os = "linux")]
            {
                let binary = voyage_binary
                    .clone()
                    .unwrap_or(std::env::current_exe()?.with_file_name("voyage"));
                return vessel::process::serve(directory.clone(), binary).await;
            }
            #[cfg(not(target_os = "linux"))]
            anyhow::bail!(
                "local process supervision currently requires Linux private Unix sockets"
            );
        }
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
    #[cfg(target_os = "linux")]
    match cli.command {
        Some(Command::ProcessGrant(args)) => return vessel::process::grant_cli::issue(args).await,
        Some(Command::PairInvite(args)) => return vessel::process::pair_cli::invite(args),
        Some(Command::ListConnections(args)) => return vessel::process::pair_cli::list(args),
        Some(Command::RevokeConnection(args)) => return vessel::process::pair_cli::revoke(args),
        _ => {}
    }
    let public_origin = if let Some(origin) = &cli.public_origin {
        let address: std::net::SocketAddr = cli.bind.parse().map_err(|_| {
            anyhow::anyhow!("process gateway requires a literal loopback bind address")
        })?;
        anyhow::ensure!(
            address.ip().is_loopback(),
            "process gateway must be loopback behind a local TLS proxy"
        );
        Some(vessel::origin::validate_origin(
            origin,
            cli.allow_insecure_loopback,
        )?)
    } else {
        None
    };
    let database = open_database(&cli.database)?;
    let state = AppState {
        process_directory: cli.process_directory.clone(),
        database: Arc::new(Mutex::new(database)),
        operator_token_hash: cli.operator_token.as_deref().map(token_hash),
        public_origin,
    };
    let app = Router::new()
        .route(
            voyage_protocol::vessel::PAIR_PATH,
            axum::routing::post(process_http::pair)
                .layer(axum::extract::DefaultBodyLimit::max(4096))
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    process_http::boundary,
                )),
        )
        .route(
            voyage_protocol::vessel::PAIR_CAPABILITIES_PATH,
            get(process_http::pair_capabilities).layer(middleware::from_fn_with_state(
                state.clone(),
                process_http::boundary,
            )),
        )
        .route(
            voyage_protocol::vessel::COMMAND_PATH,
            axum::routing::post(process_http::command)
                .layer(axum::extract::DefaultBodyLimit::max(
                    voyage_protocol::vessel::MAX_VESSEL_BODY,
                ))
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    process_http::boundary,
                )),
        )
        .route(
            voyage_protocol::vessel::EVENTS_PATH,
            axum::routing::post(process_http::events)
                .layer(axum::extract::DefaultBodyLimit::max(
                    voyage_protocol::vessel::MAX_VESSEL_BODY,
                ))
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    process_http::boundary,
                )),
        )
        .route(
            voyage_protocol::duplex::SOCKET_PATH,
            get(process_http::socket).layer(middleware::from_fn_with_state(
                state.clone(),
                process_http::boundary,
            )),
        )
        .route("/health", get(health))
        .route("/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/v1/diagnostics", get(diagnostics))
        .route("/ui", get(operator_dashboard))
        .layer(axum::extract::DefaultBodyLimit::max(
            voyage_protocol::vessel::MAX_VESSEL_BODY,
        ))
        .layer(middleware::from_fn(correlate))
        .with_state(state);
    let app = app.layer(middleware::from_fn(http_boundary::boundary));
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    tracing::info!(address = %cli.bind, process_gateway = cli.process_directory.is_some(), "Vessel ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
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
    format!(
        "# HELP voyage_connectivity_enabled Whether the scoped process gateway is configured.\n# TYPE voyage_connectivity_enabled gauge\nvoyage_connectivity_enabled {}\n",
        u8::from(state.process_directory.is_some())
    )
}

async fn diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> UiResult<Json<serde_json::Value>> {
    operator_auth(&state, &headers)?;
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "process_gateway": if state.process_directory.is_some() { "configured" } else { "unavailable" },
    })))
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
    // Never log attacker-controlled URL paths (which may contain pasted secrets).
    let path = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or("<unmatched>", axum::extract::MatchedPath::as_str)
        .to_owned();
    let span = tracing::info_span!("http.request", request_id = %request_id, %method, %path);
    let mut response = next.run(request).instrument(span).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
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

fn operator_auth(state: &AppState, headers: &HeaderMap) -> UiResult<()> {
    let Some(expected) = state.operator_token_hash.as_deref() else {
        return Err(ui_error((
            StatusCode::SERVICE_UNAVAILABLE,
            "Operator UI is disabled. Set VESSEL_OPERATOR_TOKEN.",
        )));
    };
    if headers
        .get_all(axum::http::header::AUTHORIZATION)
        .iter()
        .count()
        > 1
    {
        return Err(ui_error(StatusCode::UNAUTHORIZED));
    }
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

async fn operator_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> UiResult<Html<String>> {
    operator_auth(&state, &headers)?;
    let status = if state.process_directory.is_some() {
        "The scoped process gateway is configured. Voyage execution and approvals remain on the executing machine."
    } else {
        "The scoped process gateway is unavailable until a process directory and public origin are configured."
    };
    Ok(Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Vessel</title></head><body><main><h1>Vessel</h1><p>{status}</p><p><a href=\"/v1/diagnostics\">Connection diagnostics</a></p></main></body></html>"
    )))
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

// Do not deserialize, migrate, or overwrite the retired control_plane snapshot.
fn open_database(path: &std::path::Path) -> Result<Connection> {
    Ok(Connection::open(path)?)
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
