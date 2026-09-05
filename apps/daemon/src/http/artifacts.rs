//! Artifacts transport adaptation over typed daemon calls.
use super::{
    ApiError, AppState, MAX_ARTIFACT_HTTP_RANGE, authenticate, not_found_response, owner_error,
    success,
};
use axum::{
    body::Body, extract::Path, extract::State, http::HeaderMap, http::HeaderValue,
    http::StatusCode, http::header, response::Response,
};
use milkdrift_control_protocol::ErrorCode;
use std::sync::atomic::Ordering;

pub(super) async fn artifact_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(artifact): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .artifact_metadata(session, artifact)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn artifact_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(artifact): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let (offset, maximum) = parse_range(headers.get(header::RANGE), &request_id)?;
    let evidence = format!(
        "api-read-{}",
        state.request_sequence.fetch_add(1, Ordering::SeqCst)
    );
    let result = state
        .host
        .artifact_range(session, artifact, offset, maximum, evidence)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let crate::host::ArtifactContentRead {
        metadata,
        offset,
        bytes,
        end,
    } = result;
    let returned_end =
        offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(0).saturating_sub(1));
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&metadata.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let content_range = format!("bytes {offset}-{returned_end}/{}", metadata.size);
    if let Ok(value) = HeaderValue::from_str(&content_range) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment"),
    );
    response.headers_mut().insert(
        "x-milkdrift-artifact-complete",
        if end {
            HeaderValue::from_static("true")
        } else {
            HeaderValue::from_static("false")
        },
    );
    Ok(response)
}

pub(super) fn parse_range(
    value: Option<&HeaderValue>,
    request_id: &str,
) -> Result<(u64, u32), ApiError> {
    let Some(value) = value else {
        return Ok((0, MAX_ARTIFACT_HTTP_RANGE));
    };
    let value = value.to_str().map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            "range header is invalid",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let range = value.strip_prefix("bytes=").ok_or_else(|| {
        ApiError::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::InvalidInput,
            "only one explicit byte range is supported",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let (start, end) = range.split_once('-').ok_or_else(|| {
        ApiError::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::InvalidInput,
            "byte range requires start and end",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let start: u64 = start
        .parse()
        .map_err(|_| not_found_response(request_id.to_owned()))?;
    let end: u64 = end
        .parse()
        .map_err(|_| not_found_response(request_id.to_owned()))?;
    let requested = end
        .checked_sub(start)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| not_found_response(request_id.to_owned()))?;
    let maximum = u32::try_from(requested.min(u64::from(MAX_ARTIFACT_HTTP_RANGE)))
        .unwrap_or(MAX_ARTIFACT_HTTP_RANGE);
    Ok((start, maximum))
}
