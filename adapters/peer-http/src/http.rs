use std::{convert::Infallible, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{
        IntoResponse, Response, Sse,
        sse::{Event, KeepAlive},
    },
    routing::{get, post},
};
use milkdrift_authority::AuthorityOperation;
use milkdrift_peer_protocol::{
    ArtifactChunk, ArtifactMetadataOffer, HandshakeRequest, PeerCancellationRequest,
    PeerExecutionId, PeerInvocationRequest, PeerRequestId, ProtocolEnvelope, TransferId,
    decode_envelope, encode_envelope,
};
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};

use crate::{PeerHttpError, PeerService};

#[derive(Clone)]
struct AppState {
    service: Arc<PeerService>,
    blocking_calls: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone, Copy)]
enum PeerRouteAuthorityMapping {
    Exact(AuthorityOperation),
    QueryDerived,
}

impl PeerRouteAuthorityMapping {
    const fn exact_operation(self) -> Option<AuthorityOperation> {
        match self {
            Self::Exact(operation) => Some(operation),
            Self::QueryDerived => None,
        }
    }
}

#[derive(Clone, Copy)]
enum PeerRouteResourceMapping {
    Relationship,
    Capability,
    Execution,
    Artifact,
}

struct AuthorizedPeerRouteDeclaration {
    path: &'static str,
    authority: PeerRouteAuthorityMapping,
    resource: PeerRouteResourceMapping,
}

macro_rules! authorized_peer_routes {
    ($router:expr; $(
        $path:literal => $method:expr, $authority:expr, $resource:expr;
    )+) => {{
        $(
            let declaration = AuthorizedPeerRouteDeclaration {
                path: $path,
                authority: $authority,
                resource: $resource,
            };
            let _ = (
                declaration.path,
                declaration.authority.exact_operation(),
                declaration.resource,
            );
        )+
        $router$(.route($path, $method))+
    }};
}

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
struct ErrorBody {
    protocol: milkdrift_peer_protocol::ProtocolVersion,
    code: &'static str,
    message: String,
    retryable: bool,
}

struct ApiError(PeerHttpError);

impl From<PeerHttpError> for ApiError {
    fn from(error: PeerHttpError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, retryable) = match &self.0 {
            PeerHttpError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated", false),
            PeerHttpError::Unauthorized(_) => (StatusCode::FORBIDDEN, "unauthorized", false),
            PeerHttpError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found", false),
            PeerHttpError::Overloaded(_) => (StatusCode::TOO_MANY_REQUESTS, "overloaded", true),
            PeerHttpError::Protocol(_) | PeerHttpError::Configuration(_) => {
                (StatusCode::BAD_REQUEST, "invalid_request", false)
            }
            PeerHttpError::Persistence(_) | PeerHttpError::Unavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "unavailable", true)
            }
            PeerHttpError::Transport(_) => (StatusCode::BAD_GATEWAY, "transport", true),
        };
        let body = serde_json::to_vec(&ErrorBody {
            protocol: milkdrift_peer_protocol::ProtocolVersion::V1_2,
            code,
            message: bounded(&self.0.to_string(), 512),
            retryable,
        })
        .unwrap_or_else(|_| br#"{"code":"internal","message":"encoding failed"}"#.to_vec());
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response
    }
}

/// Builds the distinct `/peer/v1` route and authentication realm. CORS is absent.
pub fn peer_router(service: Arc<PeerService>) -> Router {
    let blocking_call_limit = service.http_connection_limit();
    authorized_peer_routes! { Router::new();
        "/peer/v1/handshake" => post(handshake), PeerRouteAuthorityMapping::Exact(AuthorityOperation::NegotiatePeerSession), PeerRouteResourceMapping::Relationship;
        "/peer/v1/catalog" => get(catalog), PeerRouteAuthorityMapping::QueryDerived, PeerRouteResourceMapping::Capability;
        "/peer/v1/invocations" => post(invoke), PeerRouteAuthorityMapping::Exact(AuthorityOperation::InvokePeerCapability), PeerRouteResourceMapping::Capability;
        "/peer/v1/requests/{request}" => get(lookup), PeerRouteAuthorityMapping::Exact(AuthorityOperation::InspectPeerExecution), PeerRouteResourceMapping::Execution;
        "/peer/v1/executions/{execution}/observations" => get(observations), PeerRouteAuthorityMapping::Exact(AuthorityOperation::InspectPeerExecution), PeerRouteResourceMapping::Execution;
        "/peer/v1/executions/{execution}/stream" => get(observation_stream), PeerRouteAuthorityMapping::Exact(AuthorityOperation::InspectPeerExecution), PeerRouteResourceMapping::Execution;
        "/peer/v1/executions/{execution}/cancel" => post(cancel), PeerRouteAuthorityMapping::Exact(AuthorityOperation::CancelPeerCapability), PeerRouteResourceMapping::Execution;
        "/peer/v1/artifacts/negotiate" => post(artifact_negotiate), PeerRouteAuthorityMapping::QueryDerived, PeerRouteResourceMapping::Artifact;
        "/peer/v1/artifacts/{transfer}/content" => get(artifact_read).post(artifact_write), PeerRouteAuthorityMapping::QueryDerived, PeerRouteResourceMapping::Artifact;
        "/peer/v1/artifacts/{transfer}/abort" => post(artifact_abort), PeerRouteAuthorityMapping::QueryDerived, PeerRouteResourceMapping::Artifact;
    }
        .layer(DefaultBodyLimit::max(
            milkdrift_peer_protocol::MAX_PEER_DOCUMENT_BYTES,
        ))
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                .layer(TraceLayer::new_for_http()),
        )
        .with_state(AppState {
            service,
            blocking_calls: Arc::new(tokio::sync::Semaphore::new(blocking_call_limit)),
        })
}

async fn handshake(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: ProtocolEnvelope<HandshakeRequest> = decode(&body)?;
    let response = authenticated_service_call(state, &headers, move |service, peer| {
        service.handshake(&peer, &request.message)
    })
    .await?;
    success(response)
}

async fn catalog(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let catalog =
        authenticated_service_call(state, &headers, move |service, peer| service.catalog(&peer))
            .await?;
    success(catalog)
}

async fn invoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: ProtocolEnvelope<PeerInvocationRequest> = decode(&body)?;
    let accepted = authenticated_service_call(state, &headers, move |service, peer| {
        service.invoke(&peer, request.message)
    })
    .await?;
    success(accepted)
}

async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<Response, ApiError> {
    let request = PeerRequestId::new(request)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let execution = authenticated_service_call(state, &headers, move |service, peer| {
        service.lookup(&peer, &request)
    })
    .await?;
    success(execution)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationQuery {
    #[serde(default)]
    after: u64,
    #[serde(default = "default_observation_limit")]
    limit: usize,
}

const fn default_observation_limit() -> usize {
    128
}

async fn observations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Response, ApiError> {
    let execution = PeerExecutionId::new(execution)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let page = authenticated_service_call(state, &headers, move |service, peer| {
        service.observations(&peer, &execution, query.after, query.limit)
    })
    .await?;
    success(page)
}

async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let envelope: ProtocolEnvelope<PeerCancellationRequest> = decode(&body)?;
    if envelope.message.execution.as_str() != execution {
        return Err(ApiError(PeerHttpError::Protocol(
            "cancellation path and body execution identities differ".to_owned(),
        )));
    }
    let acknowledgement = authenticated_service_call(state, &headers, move |service, peer| {
        service.cancel(&peer, &envelope.message)
    })
    .await?;
    success(acknowledgement)
}

async fn observation_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let bearer = bearer_value(&headers)?.to_owned();
    let peer =
        authenticated_service_call(state.clone(), &headers, |_service, peer| Ok(peer)).await?;
    let execution = PeerExecutionId::new(execution)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let service = state.service.clone();
    let blocking_calls = state.blocking_calls.clone();
    let output = stream! {
        let mut after = query.after;
        loop {
            let observation_bearer = bearer.clone();
            let observation_peer = peer.clone();
            let observation_execution = execution.clone();
            let page = service_call(service.clone(), blocking_calls.clone(), move |service| {
                let current = service.authenticate_bearer(observation_bearer.as_bytes())?;
                if current != observation_peer {
                    return Err(PeerHttpError::Unauthenticated);
                }
                service.observations(
                    &observation_peer,
                    &observation_execution,
                    after,
                    query.limit,
                )
            })
            .await;
            match page {
                Ok(page) => {
                    let closed = page.closed;
                    for observation in &page.observations {
                        if let Ok(bytes) = encode_envelope(&ProtocolEnvelope::v1(observation.clone()))
                            && let Ok(data) = String::from_utf8(bytes)
                        {
                            yield Ok(Event::default()
                                .id(observation.sequence.to_string())
                                .event("observation")
                                .data(data));
                        }
                    }
                    after = page.next_sequence;
                    if closed {
                        yield Ok(Event::default().event("closed").data("terminal"));
                        break;
                    }
                }
                Err(PeerHttpError::Unauthenticated) => {
                    yield Ok(Event::default()
                        .event("authorization_terminated")
                        .data("peer credential or authority was revoked or rotated"));
                    break;
                }
                Err(error) => {
                    yield Ok(Event::default().event("error").data(bounded(&error.to_string(), 512)));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(5))
            .text("peer-heartbeat"),
    ))
}

async fn artifact_negotiate(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let envelope: ProtocolEnvelope<ArtifactMetadataOffer> = decode(&body)?;
    let decision = authenticated_service_call(state, &headers, move |service, peer| {
        service.negotiate_artifact(&peer, &envelope.message)
    })
    .await?;
    success(decision)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactWriteQuery {
    offset: u64,
    #[serde(default)]
    final_chunk: bool,
}

async fn artifact_write(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer): Path<String>,
    Query(query): Query<ArtifactWriteQuery>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let chunk = ArtifactChunk {
        transfer,
        offset: query.offset,
        bytes: body.to_vec(),
        final_chunk: query.final_chunk,
    };
    let decision = authenticated_service_call(state, &headers, move |service, peer| {
        service.write_artifact_chunk(&peer, &chunk)
    })
    .await?;
    success(decision)
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactReadQuery {
    offset: u64,
    #[serde(default = "default_artifact_read_limit")]
    maximum_bytes: u32,
}

const fn default_artifact_read_limit() -> u32 {
    262_144
}

async fn artifact_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer): Path<String>,
    Query(query): Query<ArtifactReadQuery>,
) -> Result<Response, ApiError> {
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let chunk = authenticated_service_call(state, &headers, move |service, peer| {
        service.read_artifact_chunk(&peer, &transfer, query.offset, query.maximum_bytes)
    })
    .await?;
    let mut response = Response::new(Body::from(chunk.bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        "x-milkdrift-artifact-offset",
        HeaderValue::from_str(&chunk.offset.to_string()).map_err(|_| {
            ApiError(PeerHttpError::Protocol(
                "invalid artifact offset".to_owned(),
            ))
        })?,
    );
    response.headers_mut().insert(
        "x-milkdrift-artifact-final",
        HeaderValue::from_static(if chunk.final_chunk { "true" } else { "false" }),
    );
    Ok(response)
}

async fn artifact_abort(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer): Path<String>,
) -> Result<Response, ApiError> {
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let response_transfer = transfer.clone();
    authenticated_service_call(state, &headers, move |service, peer| {
        service.abort_artifact(&peer, &transfer)
    })
    .await?;
    success(serde_json::json!({"aborted": true, "transfer": response_transfer.as_str()}))
}

async fn authenticated_service_call<T>(
    state: AppState,
    headers: &HeaderMap,
    operation: impl FnOnce(Arc<PeerService>, milkdrift_capability::PeerId) -> Result<T, PeerHttpError>
    + Send
    + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    let bearer = bearer_value(headers)?.as_bytes().to_vec();
    service_call(state.service, state.blocking_calls, move |service| {
        let peer = service.authenticate_bearer(&bearer)?;
        operation(service, peer)
    })
    .await
    .map_err(ApiError)
}

async fn service_call<T>(
    service: Arc<PeerService>,
    blocking_calls: Arc<tokio::sync::Semaphore>,
    operation: impl FnOnce(Arc<PeerService>) -> Result<T, PeerHttpError> + Send + 'static,
) -> Result<T, PeerHttpError>
where
    T: Send + 'static,
{
    let permit = blocking_permit(blocking_calls)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        operation(service)
    })
    .await
    .map_err(|_| PeerHttpError::Unavailable("peer service task failed".to_owned()))?
}

fn blocking_permit(
    blocking_calls: Arc<tokio::sync::Semaphore>,
) -> Result<tokio::sync::OwnedSemaphorePermit, PeerHttpError> {
    blocking_calls.try_acquire_owned().map_err(|_| {
        PeerHttpError::Overloaded("bounded peer HTTP service capacity is exhausted".to_owned())
    })
}

fn bearer_value(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty() && value.len() <= 8_192)
        .ok_or(ApiError(PeerHttpError::Unauthenticated))
}

fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<ProtocolEnvelope<T>, ApiError> {
    decode_envelope(body, milkdrift_peer_protocol::DecodeLimits::default())
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))
}

fn success<T: Serialize>(message: T) -> Result<Response, ApiError> {
    let bytes = encode_envelope(&ProtocolEnvelope::v1(message))
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

fn bounded(value: &str, maximum: usize) -> String {
    milkdrift_contracts::truncate_utf8(value, maximum).to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::PeerHttpError;

    use super::blocking_permit;

    #[test]
    fn blocking_service_admission_sheds_excess_work() -> Result<(), Box<dyn std::error::Error>> {
        let calls = Arc::new(tokio::sync::Semaphore::new(1));
        let _admitted = blocking_permit(calls.clone())?;
        assert!(matches!(
            blocking_permit(calls),
            Err(PeerHttpError::Overloaded(_))
        ));
        Ok(())
    }

    #[test]
    fn every_peer_external_route_declares_typed_authority_and_resource_mapping() {
        let source = include_str!("http.rs");
        let raw_route_marker = [".", "route("].concat();
        assert_eq!(
            source.match_indices(&raw_route_marker).count(),
            1,
            "add peer routes through authorized_peer_routes! with a typed authority mapping"
        );
    }
}
