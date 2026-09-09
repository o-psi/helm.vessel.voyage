//! Bound untrusted HTTP body collection before dispatch. A receive timeout never
//! cancels an admitted command or implies that a command can safely be replayed.
use axum::{
    body::{Body, to_bytes},
    extract::{MatchedPath, Request},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::time::Duration;
use tokio::sync::Semaphore;
use voyage_protocol::vessel::{COMMAND_PATH, EVENTS_PATH, MAX_VESSEL_BODY, PAIR_PATH};

static REQUESTS: Semaphore = Semaphore::const_new(64);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(10);

fn body_limit(path: &str) -> usize {
    match path {
        COMMAND_PATH | EVENTS_PATH => MAX_VESSEL_BODY,
        PAIR_PATH => 4096,
        _ => MAX_VESSEL_BODY,
    }
}

pub(super) async fn boundary(request: Request, next: Next) -> Response {
    let mut response = receive(request, next).await;
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn receive(request: Request, next: Next) -> Response {
    let Some(path) = request.extensions().get::<MatchedPath>() else {
        // Unknown paths have no handler and should not occupy body capacity.
        return StatusCode::NOT_FOUND.into_response();
    };
    let limit = body_limit(path.as_str());
    let Ok(_permit) = REQUESTS.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(RECEIVE_TIMEOUT, to_bytes(body, limit)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    // Only receiving the request has a deadline here. Inner routing retains its
    // own admission, deduplication and uncertain-delivery semantics. In particular,
    // the response body (SSE) outlives this middleware and its request permit.
    next.run(Request::from_parts(parts, Body::from(bytes)))
        .await
}
