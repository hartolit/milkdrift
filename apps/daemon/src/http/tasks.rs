//! Browser, CLI and raw JSON callers share these authenticated task operations.
use super::{ApiError, AppState, authenticate, owner_error, protocol_error, success};
use crate::host::TaskRequest;
use axum::{
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::sse::{Event, KeepAlive},
    response::{IntoResponse, Response, Sse},
};
use milkdrift_control_protocol::{Cursor, ErrorCode, decode_json};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskListQuery {
    #[serde(default)]
    search: String,
    cursor: Option<String>,
    #[serde(default = "default_limit")]
    limit: u16,
}
fn default_limit() -> u16 {
    50
}

pub(super) fn expected_daemon(
    headers: &HeaderMap,
    request_id: &str,
) -> Result<Option<String>, ApiError> {
    headers
        .get("x-milkdrift-daemon")
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidInput,
                "invalid daemon identity header",
                false,
                Some(request_id.to_owned()),
            )
        })
}

async fn execute(
    state: &AppState,
    headers: &HeaderMap,
    operation: TaskRequest,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(state, headers)?;
    let expected = expected_daemon(headers, &request_id)?;
    let value = state
        .host
        .task_request(session, expected, operation)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    execute(&state, &headers, TaskRequest::Session).await
}
pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<TaskListQuery>,
) -> Result<Response, ApiError> {
    execute(
        &state,
        &headers,
        TaskRequest::List {
            search: query.search,
            cursor: query.cursor,
            limit: query.limit,
        },
    )
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkflowQuery {
    cursor: Option<String>,
}

pub(super) async fn workflow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    Query(query): Query<WorkflowQuery>,
) -> Result<Response, ApiError> {
    execute(
        &state,
        &headers,
        TaskRequest::Workflow {
            workflow,
            cursor: query.cursor,
        },
    )
    .await
}
pub(super) async fn task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workflow, task)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    execute(&state, &headers, TaskRequest::Task { workflow, task }).await
}
pub(super) async fn result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workflow, task)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    execute(&state, &headers, TaskRequest::Result { workflow, task }).await
}
pub(super) async fn receipt(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<Response, ApiError> {
    execute(&state, &headers, TaskRequest::Receipt(request)).await
}
// Canonicalize the public request before defaults, aliases or uploads are resolved.
// Receipt identity must distinguish omitted fields and different uploaded identities.
pub(super) fn canonical_request(body: &[u8], operation: &str) -> Result<Vec<u8>, ApiError> {
    let value: serde_json::Value =
        decode_json(body).map_err(|error| protocol_error(error, operation.into()))?;
    milkdrift_control_protocol::encode_json(&value)
        .map_err(|error| protocol_error(error, operation.into()))
}

pub(super) async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "task-create".into()))?;
    let canonical_request = canonical_request(&body, "task-create")?;
    execute(
        &state,
        &headers,
        TaskRequest::Create {
            request,
            canonical_request,
        },
    )
    .await
}
pub(super) async fn change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(workflow): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "task-change".into()))?;
    let canonical_request = canonical_request(&body, "task-change")?;
    execute(
        &state,
        &headers,
        TaskRequest::Change {
            workflow,
            request,
            canonical_request,
        },
    )
    .await
}
pub(super) async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request =
        decode_json(&body).map_err(|error| protocol_error(error, "task-upload".into()))?;
    execute(&state, &headers, TaskRequest::Upload(request)).await
}

pub(super) async fn result_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((workflow, task)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let expected = expected_daemon(&headers, &request_id)?;
    let result = state
        .host
        .task_request(
            session,
            expected,
            TaskRequest::ResultContent { workflow, task },
        )
        .await
        .map_err(|error| owner_error(error, request_id))?;
    let text = result.as_str().unwrap_or_default().to_owned();
    let mut response = Response::new(Body::from(text));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

pub(super) async fn stream(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let bearer = super::bearer_header(&headers)
        .ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let expected = expected_daemon(&headers, &request_id)?;
    let handshake = state
        .host
        .task_request(session.clone(), expected.clone(), TaskRequest::Session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let daemon = handshake
        .get("daemon_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let feed = format!("task-stream:{daemon}");
    let binding = session.cursor_binding(&feed);
    let incarnation = blake3::hash(&state.stream_incarnation).to_string();
    let expired = if let Some(cursor) = headers.get("last-event-id") {
        let cursor: Cursor = serde_json::from_value(serde_json::Value::String(
            cursor
                .to_str()
                .map_err(|_| {
                    ApiError::new(
                        StatusCode::BAD_REQUEST,
                        ErrorCode::InvalidInput,
                        "invalid stream cursor",
                        false,
                        Some(request_id.clone()),
                    )
                })?
                .into(),
        ))
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::InvalidInput,
                "invalid stream cursor",
                false,
                Some(request_id.clone()),
            )
        })?;
        let prior = cursor
            .key_for_bound(&feed, &binding, session.cursor_key())
            .map_err(|error| protocol_error(error, request_id.clone()))?;
        prior != incarnation
    } else {
        false
    };
    let cursor = Cursor::new_bound_key(
        &feed,
        &incarnation,
        binding.clone(),
        &binding.grant_digest,
        session.cursor_key(),
    )
    .map_err(|error| protocol_error(error, request_id.clone()))?;
    let cursor = serde_json::to_value(cursor)
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                "stream cursor encoding failed",
                false,
                None,
            )
        })?
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let permit = state
        .serving_requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::Overload,
                "stream connection limit reached",
                true,
                Some(request_id),
            )
        })?;
    let events = async_stream::stream! {
        let _permit = permit;
        if expired {
            yield Ok::<Event, std::convert::Infallible>(Event::default().event("resync_required").id(cursor).data("{}"));
            return;
        }
        // Each connection has finite observation lifetime; the client reconnects with its cursor.
        for _ in 0..1800 {
            let Some(current_session) = state.host.authenticate_header(Some(&bearer)) else {
                yield Ok(Event::default().event("forbidden").data("{\"message\":\"Authentication expired; reconnect\"}"));
                return;
            };
            if current_session.cursor_binding(&feed) != binding {
                yield Ok(Event::default().event("forbidden").data("{\"message\":\"Authority changed; reconnect\"}"));
                return;
            }
            // This is an invalidation hint, not a durable event log. Refreshing the visible
            // page and selected workflow also observes work outside the first discovery page.
            yield Ok(Event::default().event("workflow_changed").id(cursor.clone()).data("{}"));
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    };
    Ok(Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response())
}
