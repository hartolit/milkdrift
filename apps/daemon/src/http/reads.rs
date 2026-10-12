//! Translate HTTP identities and page queries into authorized owner reads.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use axum::{body::Bytes, extract::State, http::HeaderMap, http::StatusCode, response::Response};
use milkdrift_control_protocol::{Cursor, ErrorCode, VersionRequest, VersionResponse, decode_json};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ListQuery {
    pub(super) cursor: Option<Cursor>,
}

pub(super) async fn version(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    state
        .host
        .authorize_version(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let request: VersionRequest =
        decode_json(&body).map_err(|error| protocol_error(error, request_id.clone()))?;
    let protocol = request
        .protocol
        .negotiate()
        .map_err(|error| protocol_error(error, request_id.clone()))?;
    success(
        request_id,
        VersionResponse {
            protocol,
            service: "milkdrift-daemon".to_owned(),
        },
    )
}

pub(super) async fn health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    state
        .host
        .authorize_health(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, state.host.health())
}

pub(super) async fn readiness(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    state
        .host
        .authorize_readiness(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let health = state.host.health();
    if !health.ready {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "daemon recovery or adapter initialization is incomplete",
            true,
            Some(request_id),
        ));
    }
    success(
        request_id,
        milkdrift_control_protocol::HealthRead {
            role: health.role,
            state: health.state,
            live: health.live,
            ready: health.ready,
            draining: health.draining,
            queued_requests: 0,
            request_queue_capacity: 0,
            active_effects: 0,
            last_failure: None,
            peer_executions: milkdrift_control_protocol::PeerExecutionHealthRead {
                enabled: health.peer_executions.enabled,
                active_count: 0,
                active_bound: 0,
                dispatch_queued: 0,
                dispatch_bound: 0,
                hot_terminal_count: 0,
                hot_terminal_bound: 0,
                tombstone_count: 0,
                archive_batch_size: 0,
                observation_hot_retention_ms: 0,
                archive_generation: 0,
                last_archived_at_unix_ms: None,
                archival_degraded: health.peer_executions.archival_degraded,
                last_archival_failure: None,
            },
        },
    )
}

pub(super) async fn capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .capabilities(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn authority(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .own_authority(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}
