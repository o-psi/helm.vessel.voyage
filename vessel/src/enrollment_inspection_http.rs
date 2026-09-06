//! Operator observations do not grant machine or execution authority.
use super::*;
use voyage_protocol::enrollment::inspection::Request as InspectionRequest;
fn authorize<'a>(
    state: &'a AppState,
    headers: &HeaderMap,
) -> UiResult<&'a vessel::enrollment_http::EnrollmentApi> {
    if headers.get_all("authorization").iter().count() != 1 {
        return Err(ui_error(StatusCode::UNAUTHORIZED));
    }
    operator_auth(state, headers)?;
    let api = state
        .enrollment
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
        || headers.get_all("x-voyage-request").iter().count() != 1
        || headers.get("x-voyage-request").is_none_or(|v| v != "2")
        || headers.get_all("content-type").iter().count() != 1
        || headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.split(';').next() != Some("application/json"))
    {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    Ok(api)
}
fn parse(body: &[u8]) -> UiResult<InspectionRequest> {
    let request: InspectionRequest =
        serde_json::from_slice(body).map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    request
        .validate()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    Ok(request)
}
fn error(error: vessel::enrollment::EnrollmentError) -> UiError {
    use vessel::enrollment::EnrollmentError as E;
    let code = match error {
        E::Invalid => StatusCode::BAD_REQUEST,
        E::Denied => StatusCode::FORBIDDEN,
        E::Conflict => StatusCode::CONFLICT,
        E::Busy | E::Capacity => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ui_error((
        code,
        "Enrollment inspection unavailable; a changed or expired snapshot requires a fresh page",
    ))
}
fn private(value: impl serde::Serialize) -> Response {
    let mut result = Json(value).into_response();
    result
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    result.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    result
}
pub(super) async fn machines(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::extract::OriginalUri,
    body: axum::body::Bytes,
) -> UiResult<Response> {
    if uri.0.query().is_some() {
        return Err(ui_error(StatusCode::BAD_REQUEST));
    }
    let api = authorize(&state, &headers)?;
    Ok(private(
        api.inspect_machines(parse(&body)?).await.map_err(error)?,
    ))
}
pub(super) async fn audit(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::extract::OriginalUri,
    body: axum::body::Bytes,
) -> UiResult<Response> {
    if uri.0.query().is_some() {
        return Err(ui_error(StatusCode::BAD_REQUEST));
    }
    let api = authorize(&state, &headers)?;
    Ok(private(
        api.inspect_audit(parse(&body)?).await.map_err(error)?,
    ))
}

pub(super) async fn private_response(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let admitted =
        authorize(&state, request.headers()).and_then(|api| api.inspection_permit().map_err(error));
    let mut response = match admitted {
        Err(error) => error.into_response(),
        Ok(_permit) => {
            match tokio::time::timeout(std::time::Duration::from_secs(10), next.run(request)).await
            {
                Ok(response) => response,
                Err(_) => error(vessel::enrollment::EnrollmentError::Busy).into_response(),
            }
        }
    };
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}
