//! A narrow public command path for installation metadata, shared by GUI and CLI.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use crate::host::DaemonMetadataRequest;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};

async fn execute(
    state: &AppState,
    headers: &HeaderMap,
    operation: DaemonMetadataRequest,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(state, headers)?;
    let expected = super::tasks::expected_daemon(headers, &request_id)?;
    let value = state
        .host
        .daemon_metadata_request(session, expected, operation)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    execute(&state, &headers, DaemonMetadataRequest::Read).await
}

pub(super) async fn rename(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request = milkdrift_control_protocol::decode_json(&body)
        .map_err(|error| protocol_error(error, "daemon-rename".into()))?;
    let canonical_request = super::tasks::canonical_request(&body, "daemon-rename")?;
    execute(
        &state,
        &headers,
        DaemonMetadataRequest::Rename {
            request,
            canonical_request,
        },
    )
    .await
}

pub(super) async fn receipt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    execute(&state, &headers, DaemonMetadataRequest::Receipt(key)).await
}
