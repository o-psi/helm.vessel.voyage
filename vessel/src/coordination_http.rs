//! Operator control configuration/inspection; never execution or history consent.
use super::*;
use axum::extract::Path;
use voyage_protocol::control::{Address, Mutation};
fn authorize<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> UiResult<&'a vessel::enrollment_http::EnrollmentApi> {
    if headers.get_all("authorization").iter().count() != 1 {
        return Err(ui_error(StatusCode::UNAUTHORIZED));
    }
    operator_auth(state, headers)?;
    let api = state
        .control
        .as_ref()
        .ok_or_else(|| ui_error(StatusCode::NOT_FOUND))?;
    if headers.get_all("origin").iter().count() > 1
        || headers
            .get("origin")
            .is_some_and(|v| v.to_str().ok() != Some(api.origin()))
        || headers.get_all("sec-fetch-site").iter().count() > 1
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    Ok(api)
}
fn error(error: vessel::enrollment::EnrollmentError) -> UiError {
    use vessel::enrollment::EnrollmentError as E;
    let status = match error {
        E::Invalid => StatusCode::BAD_REQUEST,
        E::Denied => StatusCode::FORBIDDEN,
        E::Conflict => StatusCode::CONFLICT,
        E::Busy => StatusCode::SERVICE_UNAVAILABLE,
        E::Capacity => StatusCode::INSUFFICIENT_STORAGE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ui_error((
        status,
        "Coordination control unavailable; inspect state or retry the exact operation",
    ))
}
fn private(value: impl serde::Serialize) -> Response {
    let mut response = Json(value).into_response();
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}
pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    if headers.get_all("x-voyage-request").iter().count() != 1
        || headers.get("x-voyage-request").is_none_or(|v| v != "2")
    {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    let request: Mutation =
        serde_json::from_slice(&body).map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    request
        .validate()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    Ok(private(api.control_mutate(request).await.map_err(error)?))
}
pub(super) async fn inspect(
    State(state): State<AppState>,
    Path((installation_id, session_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    let (record, candidates) = api
        .control_inspect(Address {
            installation_id,
            session_id,
        })
        .await
        .map_err(error)?;
    let mut leases = Vec::new();
    if let Some(attachment) = &state.attachment {
        for lease in candidates {
            if attachment
                .is_current(lease.machine.machine_id, lease.connection_id)
                .await
            {
                leases.push(lease)
            }
        }
    }
    Ok(private(
        serde_json::json!({"record":record,"control_leases":leases,"execution_authority":"none"}),
    ))
}
