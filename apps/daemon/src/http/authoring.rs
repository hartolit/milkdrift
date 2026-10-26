//! Sketch saves use conditional PUT; deliberate Run uses the ordinary exact receipt path.
use super::{ApiError, AppState, protocol_error, tasks};
use crate::host::TaskRequest;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use milkdrift_control_protocol::decode_json;

pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
) -> Result<Response, ApiError> {
    tasks::execute(&state, &headers, TaskRequest::Authoring { workflow }).await
}

pub(super) async fn checkpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "authoring-save".into()))?;
    tasks::execute(
        &state,
        &headers,
        TaskRequest::CheckpointAuthoring { workflow, request },
    )
    .await
}

pub(super) async fn positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
) -> Result<Response, ApiError> {
    tasks::execute(&state, &headers, TaskRequest::Positions { workflow }).await
}

pub(super) async fn checkpoint_positions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "positions-save".into()))?;
    tasks::execute(
        &state,
        &headers,
        TaskRequest::CheckpointPositions { workflow, request },
    )
    .await
}

pub(super) async fn review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "authoring-review".into()))?;
    tasks::execute(
        &state,
        &headers,
        TaskRequest::ReviewAuthoring { workflow, request },
    )
    .await
}

pub(super) async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "authoring-run".into()))?;
    let canonical_request = tasks::canonical_request(&body, "authoring-run")?;
    tasks::execute(
        &state,
        &headers,
        TaskRequest::RunAuthoring {
            workflow,
            request,
            canonical_request,
        },
    )
    .await
}
