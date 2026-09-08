//! Translate HTTP identities and page queries into authorized owner reads.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use crate::auth::ActorSession;
use axum::{
    body::Bytes, extract::Path, extract::Query, extract::State, http::HeaderMap, http::StatusCode,
    response::Response,
};
use milkdrift_control_protocol::{
    Cursor, ErrorCode, PageRequest, VersionRequest, VersionResponse, decode_json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct ListQuery {
    #[serde(default = "default_limit")]
    pub(super) limit: u32,
    pub(super) cursor: Option<Cursor>,
    pub(super) workflow: Option<String>,
    pub(super) state: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ProposalQuery {
    revision: String,
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
            state: health.state,
            live: health.live,
            ready: health.ready,
            draining: health.draining,
            queued_requests: 0,
            request_queue_capacity: 0,
            active_effects: 0,
            last_failure: None,
            application_receipts: health.application_receipts,
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

pub(super) async fn revisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    PageRequest {
        cursor: query.cursor.clone(),
        limit: query.limit,
    }
    .validate()
    .map_err(|error| protocol_error(error, request_id.clone()))?;
    let value = state
        .host
        .revisions(session, query.workflow, query.cursor, query.limit)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn revision(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(revision): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .revision(session, revision)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn revision_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((from, to)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .revision_diff(session, from, to)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    PageRequest {
        cursor: query.cursor.clone(),
        limit: query.limit,
    }
    .validate()
    .map_err(|error| protocol_error(error, request_id.clone()))?;
    let value = state
        .host
        .runs(
            session,
            query.state,
            query.workflow,
            query.cursor,
            query.limit,
        )
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    run_response(&state, request_id, session, run).await
}

async fn run_response(
    state: &AppState,
    request_id: String,
    session: ActorSession,
    run: String,
) -> Result<Response, ApiError> {
    let value = state
        .host
        .run(session, run)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn node(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((run, execution)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .node(session, run, execution)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn attempt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((run, attempt)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .attempt(session, run, attempt)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn timeline(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    PageRequest {
        cursor: query.cursor.clone(),
        limit: query.limit,
    }
    .validate()
    .map_err(|error| protocol_error(error, request_id.clone()))?;
    let value = state
        .host
        .timeline(session, run, query.cursor, query.limit)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
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

pub(super) async fn proposals(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    PageRequest {
        cursor: query.cursor.clone(),
        limit: query.limit,
    }
    .validate()
    .map_err(|error| protocol_error(error, request_id.clone()))?;
    let value = state
        .host
        .proposals(session, run, query.cursor, query.limit)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((run, proposal)): Path<(String, String)>,
    Query(query): Query<ProposalQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .proposal(session, run, proposal, query.revision)
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

pub(super) async fn layout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workflow, revision)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .layout(session, workflow, revision)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

const fn default_limit() -> u32 {
    100
}
