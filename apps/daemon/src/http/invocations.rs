//! Authenticate client identity, decode bounded transport input, and call the serving owner.
use super::{ApiError, AppState, authenticate, success};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use milkdrift_authority::ActorRef;
use milkdrift_capability_host::{PeerService, ServingError};
use milkdrift_control_protocol::{ErrorCode, decode_json};
use milkdrift_peer_protocol::{
    DirectInvocationRequest, PeerCancellationRequest, PeerExecutionId, PeerRequestId,
};
use serde::{Deserialize, Serialize};

async fn serving<T, F>(state: AppState, headers: HeaderMap, action: F) -> Result<Response, ApiError>
where
    T: Serialize + Send + 'static,
    F: FnOnce(&PeerService, &ActorRef) -> Result<T, ServingError> + Send + 'static,
{
    let (request_id, session) = authenticate(&state, &headers)?;
    let permit = state
        .serving_requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                ServingError::Overloaded("request capacity".to_owned()),
                request_id.clone(),
            )
        })?;
    let service = state.host.peer_service().ok_or_else(|| {
        failure(
            ServingError::Unavailable("execution serving is unavailable".to_owned()),
            request_id.clone(),
        )
    })?;
    let value = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action(&service, &session.actor)
    })
    .await
    .map_err(|_| {
        failure(
            ServingError::Unavailable("serving task failed".to_owned()),
            request_id.clone(),
        )
    })?
    .map_err(|error| failure(error, request_id.clone()))?;
    success(request_id, value)
}

pub(super) async fn catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serving(state, headers, PeerService::client_discovery).await
}

pub(super) async fn invoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let request: DirectInvocationRequest =
            decode_json(&body).map_err(|error| ServingError::Protocol(error.to_string()))?;
        service.invoke_client(actor, &request)
    })
    .await
}

pub(super) async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let request = PeerRequestId::new(request)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        service.client_lookup(actor, &request)
    })
    .await
}

pub(super) async fn inspect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let execution = PeerExecutionId::new(execution)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        service.client_read(actor, &execution)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ObservationQuery {
    #[serde(default)]
    after: u64,
    limit: u32,
}

pub(super) async fn observations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let execution = PeerExecutionId::new(execution)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        service.client_observations(actor, &execution, query.after, query.limit)
    })
    .await
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let request: PeerCancellationRequest =
            decode_json(&body).map_err(|error| ServingError::Protocol(error.to_string()))?;
        if request.execution.as_str() != execution {
            return Err(ServingError::Protocol(
                "cancellation path and body differ".to_owned(),
            ));
        }
        service.cancel_client(actor, &request)
    })
    .await
}

fn failure(error: ServingError, request_id: String) -> ApiError {
    let (status, code, message, retryable) = match error {
        ServingError::Unauthenticated => (
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "valid client authentication is required".to_owned(),
            false,
        ),
        ServingError::Unauthorized(_) => (
            StatusCode::FORBIDDEN,
            ErrorCode::Unauthorized,
            "current authority does not permit this operation".to_owned(),
            false,
        ),
        ServingError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "invocation was not found".to_owned(),
            false,
        ),
        ServingError::Overloaded(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Overload,
            "serving capacity is exhausted".to_owned(),
            true,
        ),
        ServingError::Protocol(message) | ServingError::Configuration(message) => (
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            message.chars().take(512).collect(),
            false,
        ),
        ServingError::Persistence(_) | ServingError::Unavailable(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "execution serving is unavailable".to_owned(),
            true,
        ),
    };
    ApiError::new(status, code, message, retryable, Some(request_id))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OutputQuery {
    offset: u64,
    maximum: u32,
}

pub(super) async fn output(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((execution, artifact)): Path<(String, String)>,
    Query(query): Query<OutputQuery>,
) -> Result<Response, ApiError> {
    serving(state, headers, move |service, actor| {
        let execution = PeerExecutionId::new(execution)
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        service.client_output(actor, &execution, &artifact, query.offset, query.maximum)
    })
    .await
}
