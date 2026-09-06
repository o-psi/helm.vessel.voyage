mod remote_http;
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
    /// Private enrollment authority directory; enables authenticated attachment presence.
    #[arg(long, requires = "public_origin")]
    attachment_directory: Option<PathBuf>,
    /// Canonical HTTPS origin served by a TLS proxy on this host.
    #[arg(long, requires = "attachment_directory")]
    public_origin: Option<String>,
    /// Enable authenticated relay to explicitly running dedicated Helm remote workers.
    #[arg(long, requires = "attachment_directory")]
    remote_execution: bool,
    /// Allow HTTP only for literal loopback development origins.
    #[arg(long, requires = "attachment_directory")]
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
    database: Arc<Mutex<Connection>>,
    operator_token_hash: Option<String>,
    attachment: Option<vessel::attachment_transport::AttachmentApi>,
    remote: Option<remote_http::RemoteApi>,
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
    let enrollment =
        if let (Some(directory), Some(origin)) = (&cli.attachment_directory, &cli.public_origin) {
            let address: std::net::SocketAddr = cli.bind.parse().map_err(|_| {
                anyhow::anyhow!("enrollment listener requires a literal loopback bind address")
            })?;
            anyhow::ensure!(
                address.ip().is_loopback(),
                "enrollment listener must be loopback behind a local TLS proxy"
            );
            let token = cli
                .operator_token
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("enrollment requires an operator credential"))?;
            anyhow::ensure!(
                token.len() >= 32 && token.len() <= 1024 && !token.chars().any(char::is_control),
                "enrollment requires a 32-1024 byte operator credential without controls"
            );
            let store = vessel::enrollment::EnrollmentStore::open(
                directory,
                origin,
                cli.allow_insecure_loopback,
            )?;
            Some(vessel::enrollment_http::EnrollmentApi::new(store, token)?)
        } else {
            None
        };
    let remote = if cli.remote_execution {
        Some(remote_http::RemoteApi::new(
            enrollment
                .clone()
                .ok_or_else(|| anyhow::anyhow!("enrollment required"))?,
        )?)
    } else {
        None
    };
    let attachment = match &remote {
        Some(remote) => Some(remote.attachment()),
        None => enrollment
            .clone()
            .map(vessel::attachment_transport::AttachmentApi::presence)
            .transpose()?,
    };
    let database = open_database(&cli.database)?;
    let state = AppState {
        database: Arc::new(Mutex::new(database)),
        operator_token_hash: cli.operator_token.as_deref().map(token_hash),
        attachment: attachment.clone(),
        remote,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/v1/diagnostics", get(diagnostics))
        .route("/ui", get(operator_dashboard))
        .route(
            "/v1/remote/{machine}/command",
            axum::routing::post(remote_http::command),
        )
        .route("/v1/remote/{machine}/events", get(remote_http::watch))
        .layer(axum::extract::DefaultBodyLimit::max(
            voyage_protocol::attachment::MAX_FRAME_BYTES,
        ))
        .layer(middleware::from_fn(correlate))
        .with_state(state);
    let app = if let Some(enrollment) = enrollment {
        app.merge(enrollment.router())
    } else {
        app
    };
    let app = if let Some(attachment) = &attachment {
        app.merge(attachment.clone().router())
    } else {
        app
    };
    let listener = tokio::net::TcpListener::bind(&cli.bind).await?;
    tracing::info!(address = %cli.bind, attachment_presence = attachment.is_some(), remote_execution = cli.remote_execution, "Vessel ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            if let Some(attachment) = attachment {
                attachment.shutdown().await;
            }
        })
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
        "# HELP voyage_connectivity_enabled Whether attachment presence is enabled (not remote execution).\n# TYPE voyage_connectivity_enabled gauge\nvoyage_connectivity_enabled {}\n",
        u8::from(state.attachment.is_some())
    )
}

async fn diagnostics(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> UiResult<Json<serde_json::Value>> {
    operator_auth(&state, &headers)?;
    let connections = match &state.attachment {
        Some(api) => api.connections().await,
        None => Vec::new(),
    };
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "connectivity": if state.remote.is_some() { "outbound_managed_sessions" } else if state.attachment.is_some() { "presence_only" } else { "unavailable" },
        "legacy_state": "not_loaded",
        "attachment": if state.remote.is_some() { "managed_execution" } else if state.attachment.is_some() { "presence_only" } else { "disabled" },
        "remote_execution": if state.remote.is_some() { "enabled" } else { "unavailable" },
        "connections": connections,
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
    let path = request.uri().path().to_owned();
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
    let status = if state.attachment.is_some() {
        if state.remote.is_some() {
            "Dedicated remote workers are available through authenticated /v1/remote operator endpoints. Local approval and cleanup remain on Helm."
        } else {
            "Authenticated Helm attachment presence is enabled. Remote execution is unavailable."
        }
    } else {
        "Helm connectivity is unavailable until enrollment is configured. Remote execution is unavailable."
    };
    Ok(Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Vessel</title></head><body><main><h1>Vessel</h1><p>{status}</p><p><a href=\"/v1/diagnostics\">Connection diagnostics</a></p></main></body></html>"
    )))
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

// Do not deserialize, migrate, or overwrite the retired control_plane snapshot.
// Future attachment storage needs an explicit new schema and migration decision.
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_cli_commands_and_timing_flags_are_rejected() {
        for args in [
            vec!["vessel", "pair", "voyage:v1:TEST"],
            vec!["vessel", "fleet"],
            vec!["vessel", "--lease-secs", "1"],
            vec!["vessel", "--stale-after-secs", "1"],
            vec!["vessel", "--pairing-ttl-secs", "1"],
        ] {
            assert!(Cli::try_parse_from(&args).is_err(), "accepted {args:?}");
        }
        let help = Cli::command().render_long_help().to_string();
        for retired in [
            "pairing-ttl",
            "lease-secs",
            "stale-after",
            "  pair",
            "  fleet",
        ] {
            assert!(!help.contains(retired));
        }
    }

    #[test]
    fn legacy_database_is_preserved_without_deserializing_or_migrating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vessel.db");
        // Even malformed/unknown snapshots must stay inert and unchanged.
        let original = "not JSON: legacy credentials and uncertain running tasks";
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE control_plane(id INTEGER PRIMARY KEY, state TEXT NOT NULL); CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY); INSERT INTO schema_migrations VALUES(1);").unwrap();
            db.execute("INSERT INTO control_plane VALUES(1, ?1)", [original])
                .unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        for _ in 0..2 {
            let db = open_database(&path).unwrap();
            assert_eq!(
                db.query_row("SELECT state FROM control_plane", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                original
            );
            assert_eq!(
                db.query_row("SELECT version FROM schema_migrations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn database_open_failure_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_database(dir.path()).is_err());
    }

    #[test]
    fn credentials_are_stored_as_hashes() {
        assert_ne!(token_hash("secret"), "secret");
        assert_eq!(token_hash("secret"), token_hash("secret"));
    }

    #[test]
    fn constant_time_comparison_rejects_mismatches() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn operator_auth_is_default_deny_and_rejects_malformed_credentials() {
        let mut state = AppState {
            database: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
            operator_token_hash: None,
            attachment: None,
            remote: None,
        };
        let mut headers = HeaderMap::new();
        assert!(operator_auth(&state, &headers).is_err());
        state.operator_token_hash = Some(token_hash("test-operator"));
        assert!(operator_auth(&state, &headers).is_err());
        for value in [
            "Bearer wrong",
            "Basic !invalid!",
            "Basic /w==",
            "Basic bm9jb2xvbg==",
        ] {
            headers.insert("authorization", HeaderValue::from_str(value).unwrap());
            assert!(operator_auth(&state, &headers).is_err());
        }
        for value in [
            "Bearer test-operator".to_owned(),
            format!("Basic {}", STANDARD.encode("operator:test-operator")),
        ] {
            headers.insert("authorization", HeaderValue::from_str(&value).unwrap());
            assert!(operator_auth(&state, &headers).is_ok());
        }
    }

    #[tokio::test]
    async fn poisoned_database_readiness_fails_closed() {
        let database = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let shared = database.clone();
        let _ = std::thread::spawn(move || {
            let _guard = shared.lock().unwrap();
            panic!("injected database lock failure");
        })
        .join();
        let result = readiness(State(AppState {
            database,
            operator_token_hash: None,
            attachment: None,
            remote: None,
        }))
        .await;
        assert_eq!(result.unwrap_err().0, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
