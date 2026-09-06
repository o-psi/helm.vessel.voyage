//! Authenticated grant gateway. This adapter never upgrades enrollment to execution authority.
use super::*;
use voyage_protocol::process::{PROCESS_PROTOCOL, VesselCommand, VesselRequest, VesselResponse};

static CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);

pub(super) async fn boundary(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(enrollment) = &state.enrollment else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.headers().get_all("origin").iter().count() > 1
        || request
            .headers()
            .get("origin")
            .is_some_and(|origin| origin.to_str().ok() != Some(enrollment.origin()))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(_permit) = CAPACITY.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VesselRequest>,
) -> Response {
    let Some(directory) = state.process_directory else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.protocol != PROCESS_PROTOCOL
        || headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|token| token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(grant_id) = headers
        .get("x-voyage-grant")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    #[cfg(target_os = "linux")]
    let response = match vessel::process::exchange(
        &directory,
        &VesselRequest {
            protocol: PROCESS_PROTOCOL,
            command: VesselCommand::Granted {
                grant_id,
                token: token.to_owned(),
                command: Box::new(request.command),
            },
        },
    )
    .await
    {
        Ok(response) => response,
        Err(_) => VesselResponse {
            protocol: PROCESS_PROTOCOL,
            result: serde_json::Value::Null,
            error: Some("Vessel routing unavailable; command outcome unknown".into()),
            outcome_unknown: true,
        },
    };
    #[cfg(not(target_os = "linux"))]
    let response = {
        let _ = (directory, token, grant_id);
        VesselResponse {
            protocol: PROCESS_PROTOCOL,
            result: serde_json::Value::Null,
            error: Some("process gateway unsupported on this platform".into()),
            outcome_unknown: false,
        }
    };
    Json(response).into_response()
}
