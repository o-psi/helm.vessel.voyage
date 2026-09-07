//! Authenticated grant gateway. This adapter never upgrades enrollment to execution authority.
use super::*;
use axum::response::sse::{Event, KeepAlive, Sse};
use voyage_protocol::vessel::{
    VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VesselRequest,
    VesselResponse, VoyageCommand, VoyageReply, VoyageRequest,
};

static CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);
static EVENT_CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);

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
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

pub(super) async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VesselEventRequest>,
) -> Response {
    let Some(directory) = state.process_directory else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.protocol != VESSEL_API_VERSION
        || request.subscriptions.len() != 1
        || headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(token) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(grant_id) = headers
        .get("x-voyage-grant")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(permit) = EVENT_CAPACITY.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut subscription = request
        .subscriptions
        .into_iter()
        .next()
        .expect("one subscription");
    let stream = async_stream::stream! {
        let _permit = permit;
        loop {
            let response = vessel::process::exchange(
                &directory,
                &VesselRequest {
                    protocol: VESSEL_API_VERSION,
                    command: VesselCommand::Granted {
                        grant_id,
                        token: token.clone(),
                        command: Box::new(VesselCommand::Voyage(VoyageRequest {
                            session_id: subscription.session_id,
                            incarnation: None,
                            command: VoyageCommand::Events {
                                after: subscription.after,
                                limit: 128,
                                // Re-enter the grant gateway between waits so expiry
                                // and revocation stop publication at a bounded point.
                                wait_ms: 0,
                            },
                        })),
                    },
                },
            )
            .await;
            let (result, error, outcome_unknown) = match response {
                Ok(response) if response.error.is_none() => {
                    match serde_json::from_value::<VoyageReply>(response.result) {
                        Ok(reply) => {
                            let mut result = reply.result;
                            if reply.incarnation != subscription.incarnation {
                                result["owner_changed"] = serde_json::Value::Bool(true);
                                result["replay_gap"] = serde_json::Value::Bool(true);
                                result["recovery"] = serde_json::Value::String("snapshot".into());
                                subscription.incarnation = reply.incarnation;
                            }
                            (result, None, false)
                        },
                        Err(_) => (serde_json::Value::Null, Some("invalid Vessel event response".into()), true),
                    }
                }
                Ok(response) => (response.result, response.error, response.outcome_unknown),
                Err(_) => (serde_json::Value::Null, Some("Vessel routing unavailable; event stream ended".into()), true),
            };
            let terminal = error.is_some();
            if let Some(cursor) = result.get("cursor").and_then(serde_json::Value::as_u64) {
                subscription.after = cursor;
            }
            let changed = terminal
                || result.get("replay_gap") == Some(&serde_json::Value::Bool(true))
                || result.get("events").and_then(serde_json::Value::as_array).is_some_and(|events| !events.is_empty());
            if changed {
                let event = VesselEvent {
                    protocol: VESSEL_API_VERSION,
                    session_id: subscription.session_id,
                    incarnation: subscription.incarnation,
                    result,
                    error,
                    outcome_unknown,
                };
                let Ok(data) = serde_json::to_string(&event) else { break; };
                yield Ok::<Event, std::convert::Infallible>(Event::default().event("update").data(data));
            }
            if terminal {
                break;
            }
            if !changed {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        }
    };
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(5))
                .text("keep-alive"),
        )
        .into_response()
}

pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VesselRequest>,
) -> Response {
    let Some(directory) = state.process_directory else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.protocol != VESSEL_API_VERSION
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
            protocol: VESSEL_API_VERSION,
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
            protocol: VESSEL_API_VERSION,
            result: serde_json::Value::Null,
            error: Some("Vessel routing unavailable; command outcome unknown".into()),
            outcome_unknown: true,
        },
    };
    #[cfg(not(target_os = "linux"))]
    let response = {
        let _ = (directory, token, grant_id);
        VesselResponse {
            protocol: VESSEL_API_VERSION,
            result: serde_json::Value::Null,
            error: Some("process gateway unsupported on this platform".into()),
            outcome_unknown: false,
        }
    };
    Json(response).into_response()
}
