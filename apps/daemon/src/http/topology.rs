//! Authenticated topology reads use the same bounded owner queue as other workflow reads.

use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use serde::Deserialize;

use super::{ApiError, AppState, authenticate, owner_error, success};
use crate::host::TaskRequest;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TopologyQuery {
    cursor: Option<String>,
}

pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    Query(query): Query<TopologyQuery>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let expected = super::tasks::expected_daemon(&headers, &request_id)?;
    let value = state
        .host
        .task_request(
            session,
            expected,
            TaskRequest::Topology {
                workflow,
                cursor: query.cursor,
            },
        )
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}
