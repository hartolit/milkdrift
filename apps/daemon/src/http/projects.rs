//! Public project operations use the same authenticated owner queue as tasks.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use crate::host::ProjectRequest;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PageQuery {
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: u16,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryQuery {
    parent: Option<String>,
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: u16,
}
fn default_limit() -> u16 {
    50
}
async fn execute(
    state: &AppState,
    headers: &HeaderMap,
    request: ProjectRequest,
) -> Result<Response, ApiError> {
    let (id, session) = authenticate(state, headers)?;
    let expected = super::tasks::expected_daemon(headers, &id)?;
    let value = state
        .host
        .project_request(session, expected, request)
        .await
        .map_err(|e| owner_error(e, id.clone()))?;
    success(id, value)
}
pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<Response, ApiError> {
    execute(
        &state,
        &headers,
        ProjectRequest::List {
            cursor: q.cursor,
            limit: q.limit,
        },
    )
    .await
}
pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    execute(&state, &headers, ProjectRequest::Read(id)).await
}
pub(super) async fn directories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DirectoryQuery>,
) -> Result<Response, ApiError> {
    execute(
        &state,
        &headers,
        ProjectRequest::Directories {
            parent: q.parent,
            cursor: q.cursor,
            limit: q.limit,
        },
    )
    .await
}
pub(super) async fn environments(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    execute(&state, &headers, ProjectRequest::Environments).await
}
pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request = milkdrift_control_protocol::decode_json(&body)
        .map_err(|e| protocol_error(e, "project-command".into()))?;
    let canonical_request = super::tasks::canonical_request(&body, "project-command")?;
    execute(
        &state,
        &headers,
        ProjectRequest::Command {
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
    execute(&state, &headers, ProjectRequest::Receipt(key)).await
}
pub(super) async fn operations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    execute(&state, &headers, ProjectRequest::Operations).await
}
