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
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum PeerRouteAuthorityMapping {
    Exact(AuthorityOperation),
    QueryDerived,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum PeerRouteResourceMapping {
    Relationship,
    Capability,
    Execution,
    Artifact,
}

#[allow(dead_code)]
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
            let _declaration = AuthorizedPeerRouteDeclaration {
                path: $path,
                authority: $authority,
                resource: $resource,
            };
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
            protocol: milkdrift_peer_protocol::ProtocolVersion::V1_0,
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
        .with_state(AppState { service })
}

async fn handshake(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let peer = authenticate(&state, &headers)?;
    let request: ProtocolEnvelope<HandshakeRequest> = decode(&body)?;
    success(state.service.handshake(&peer, &request.message)?)
}

async fn catalog(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let peer = authenticate(&state, &headers)?;
    success(state.service.catalog(&peer)?)
}

async fn invoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let peer = authenticate(&state, &headers)?;
    let request: ProtocolEnvelope<PeerInvocationRequest> = decode(&body)?;
    success(state.service.invoke(&peer, request.message)?)
}

async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<Response, ApiError> {
    let peer = authenticate(&state, &headers)?;
    let request = PeerRequestId::new(request)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    success(state.service.lookup(&peer, &request)?)
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
    let peer = authenticate(&state, &headers)?;
    let execution = PeerExecutionId::new(execution)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    success(
        state
            .service
            .observations(&peer, &execution, query.after, query.limit)?,
    )
}

async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let peer = authenticate(&state, &headers)?;
    let envelope: ProtocolEnvelope<PeerCancellationRequest> = decode(&body)?;
    if envelope.message.execution.as_str() != execution {
        return Err(ApiError(PeerHttpError::Protocol(
            "cancellation path and body execution identities differ".to_owned(),
        )));
    }
    success(state.service.cancel(&peer, &envelope.message)?)
}

async fn observation_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(execution): Path<String>,
    Query(query): Query<ObservationQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let peer = authenticate(&state, &headers)?;
    let bearer = bearer_value(&headers)?.to_owned();
    let execution = PeerExecutionId::new(execution)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let service = state.service.clone();
    let output = stream! {
        let mut after = query.after;
        loop {
            if !matches!(
                service.authenticate_bearer(bearer.as_bytes()),
                Ok(current) if current == peer
            ) {
                yield Ok(Event::default()
                    .event("authorization_terminated")
                    .data("peer credential or authority was revoked or rotated"));
                break;
            }
            match service.observations(&peer, &execution, after, query.limit) {
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
    let peer = authenticate(&state, &headers)?;
    let envelope: ProtocolEnvelope<ArtifactMetadataOffer> = decode(&body)?;
    success(state.service.negotiate_artifact(&peer, &envelope.message)?)
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
    let peer = authenticate(&state, &headers)?;
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    success(state.service.write_artifact_chunk(
        &peer,
        &ArtifactChunk {
            transfer,
            offset: query.offset,
            bytes: body.to_vec(),
            final_chunk: query.final_chunk,
        },
    )?)
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
    let peer = authenticate(&state, &headers)?;
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    let chunk =
        state
            .service
            .read_artifact_chunk(&peer, &transfer, query.offset, query.maximum_bytes)?;
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
    let peer = authenticate(&state, &headers)?;
    let transfer = TransferId::new(transfer)
        .map_err(|error| ApiError(PeerHttpError::Protocol(error.to_string())))?;
    state.service.abort_artifact(&peer, &transfer)?;
    success(serde_json::json!({"aborted": true, "transfer": transfer.as_str()}))
}

fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<milkdrift_authority::PeerId, ApiError> {
    let value = bearer_value(headers)?;
    state
        .service
        .authenticate_bearer(value.as_bytes())
        .map_err(ApiError)
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
    if value.len() <= maximum {
        return value.to_owned();
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
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
