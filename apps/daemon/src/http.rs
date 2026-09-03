use std::{
    collections::{BTreeMap, VecDeque},
    convert::Infallible,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

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
use futures_util::Stream;
use milkdrift_authority::AuthorityOperation;
use milkdrift_capability::PeerId;
use milkdrift_control_protocol::{
    CapabilityRead, Command, CommandRequest, Cursor, CursorBinding, ErrorCode, ErrorEnvelope,
    Observation, ObservationEnvelope, PageRequest, ProtocolVersion, ResponseEnvelope,
    VersionRequest, VersionResponse, decode_json,
};
use serde::Deserialize;
use tower::ServiceBuilder;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::{info, warn};

use crate::{
    DaemonHost, HostError,
    auth::ActorSession,
    host::{PublicFailure, StreamAuthority},
};

const MAX_ARTIFACT_HTTP_RANGE: u32 = 1_048_576;
const STREAM_PAGE_ITEMS: u32 = 128;
const CAPABILITY_FEED_ITEMS: usize = 256;

#[derive(Clone, Copy)]
enum RouteAuthorityMapping {
    Exact(AuthorityOperation),
    CommandDerived,
    QueryDerived,
    StreamDerived,
}

impl RouteAuthorityMapping {
    const fn exact_operation(self) -> Option<AuthorityOperation> {
        match self {
            Self::Exact(operation) => Some(operation),
            Self::CommandDerived | Self::QueryDerived | Self::StreamDerived => None,
        }
    }
}

#[derive(Clone, Copy)]
enum RouteResourceMapping {
    Daemon,
    WorkflowRevision,
    Run,
    Capability,
    Peer,
    Artifact,
    Layout,
}

struct AuthorizedRouteDeclaration {
    path: &'static str,
    authority: RouteAuthorityMapping,
    resource: RouteResourceMapping,
}

macro_rules! authorized_routes {
    ($router:expr; $(
        $path:literal => $method:expr, $authority:expr, $resource:expr;
    )+) => {{
        $(
            let declaration = AuthorizedRouteDeclaration {
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

#[derive(Clone)]
struct AppState {
    host: DaemonHost,
    request_sequence: Arc<AtomicU64>,
    capability_feeds: Arc<tokio::sync::Mutex<BTreeMap<String, CapabilityFeed>>>,
}

#[derive(Default)]
struct CapabilityFeed {
    last_snapshot_digest: Option<String>,
    next_position: u64,
    entries: VecDeque<(u64, CapabilityRead)>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    envelope: ErrorEnvelope,
}

impl ApiError {
    fn new(
        status: StatusCode,
        code: ErrorCode,
        message: impl Into<String>,
        retryable: bool,
        request_id: Option<String>,
    ) -> Self {
        let mut envelope = ErrorEnvelope::new(code, message, retryable);
        envelope.request_id = request_id;
        Self { status, envelope }
    }

    fn unauthenticated(request_id: String) -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthenticated,
            "valid bearer authentication is required",
            false,
            Some(request_id),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.envelope).unwrap_or_else(|_| {
            br#"{"protocol":{"major":1,"minor":0},"request_id":null,"code":"internal","message":"error encoding failed","retryable":false,"details":{}}"#.to_vec()
        });
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = self.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response
    }
}

/// Builds the bounded protocol-2.2 router. CORS is intentionally absent.
pub(crate) fn router(host: DaemonHost) -> Router {
    let peer_service = host.peer_service();
    let state = AppState {
        host,
        request_sequence: Arc::new(AtomicU64::new(1)),
        capability_feeds: Arc::new(tokio::sync::Mutex::new(BTreeMap::new())),
    };
    let router = authorized_routes! { Router::new();
        "/v1/version" => post(version), RouteAuthorityMapping::Exact(AuthorityOperation::NegotiateControlProtocol), RouteResourceMapping::Daemon;
        "/v1/health" => get(health), RouteAuthorityMapping::Exact(AuthorityOperation::InspectDaemonHealth), RouteResourceMapping::Daemon;
        "/v1/readiness" => get(readiness), RouteAuthorityMapping::Exact(AuthorityOperation::ReadReadiness), RouteResourceMapping::Daemon;
        "/v1/commands" => post(command), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Run;
        "/v1/revisions" => get(revisions), RouteAuthorityMapping::Exact(AuthorityOperation::InspectRevision), RouteResourceMapping::WorkflowRevision;
        "/v1/revisions/{revision}" => get(revision), RouteAuthorityMapping::Exact(AuthorityOperation::InspectRevision), RouteResourceMapping::WorkflowRevision;
        "/v1/revisions/{from}/diff/{to}" => get(revision_diff), RouteAuthorityMapping::Exact(AuthorityOperation::InspectRevision), RouteResourceMapping::WorkflowRevision;
        "/v1/runs" => get(runs), RouteAuthorityMapping::Exact(AuthorityOperation::InspectRun), RouteResourceMapping::Run;
        "/v1/runs/{run}" => get(run), RouteAuthorityMapping::Exact(AuthorityOperation::InspectRun), RouteResourceMapping::Run;
        "/v1/runs/{run}/nodes/{execution}" => get(node), RouteAuthorityMapping::Exact(AuthorityOperation::InspectNodeExecution), RouteResourceMapping::Run;
        "/v1/runs/{run}/attempts/{attempt}" => get(attempt), RouteAuthorityMapping::Exact(AuthorityOperation::InspectAttempt), RouteResourceMapping::Run;
        "/v1/runs/{run}/timeline" => get(timeline), RouteAuthorityMapping::Exact(AuthorityOperation::InspectTimeline), RouteResourceMapping::Run;
        "/v1/runs/{run}/stream" => get(run_stream), RouteAuthorityMapping::StreamDerived, RouteResourceMapping::Run;
        "/v1/runs/{run}/proposals" => get(proposals), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProposal), RouteResourceMapping::Run;
        "/v1/runs/{run}/proposals/{proposal}" => get(proposal), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProposal), RouteResourceMapping::Run;
        "/v1/capabilities" => get(capabilities), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Capability;
        "/v1/peers" => get(peers), RouteAuthorityMapping::Exact(AuthorityOperation::InspectPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}" => get(peer), RouteAuthorityMapping::Exact(AuthorityOperation::InspectPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/connect" => post(peer_connect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/reload" => post(peer_connect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/disconnect" => post(peer_disconnect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/drain" => post(peer_disconnect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/revoke" => post(peer_revoke), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/authority" => get(authority), RouteAuthorityMapping::Exact(AuthorityOperation::InspectOwnAuthority), RouteResourceMapping::Daemon;
        "/v1/artifacts/{artifact}" => get(artifact_metadata), RouteAuthorityMapping::Exact(AuthorityOperation::ReadArtifactMetadata), RouteResourceMapping::Artifact;
        "/v1/artifacts/{artifact}/content" => get(artifact_content), RouteAuthorityMapping::Exact(AuthorityOperation::ReadArtifactContent), RouteResourceMapping::Artifact;
        "/v1/layouts/{workflow}/{revision}" => get(layout), RouteAuthorityMapping::Exact(AuthorityOperation::ReadLayout), RouteResourceMapping::Layout;
        "/v1/stream/health" => get(health_stream), RouteAuthorityMapping::Exact(AuthorityOperation::InspectDaemonHealth), RouteResourceMapping::Daemon;
        "/v1/stream/capabilities" => get(capability_stream), RouteAuthorityMapping::StreamDerived, RouteResourceMapping::Capability;
    }
        .layer(DefaultBodyLimit::max(
            milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
        ))
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                .layer(TraceLayer::new_for_http()),
        )
        .with_state(state);
    if let Some(service) = peer_service {
        router.merge(milkdrift_peer_http::peer_router(service))
    } else {
        router
    }
}

async fn peers(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let visible = state
        .host
        .visible_peers(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(
        request_id,
        state
            .host
            .peers()
            .into_iter()
            .filter(|peer| visible.contains(&peer.peer_id))
            .collect::<Vec<_>>(),
    )
}

async fn peer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    state
        .host
        .authorize_peer_read(session, peer.clone())
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let status = state
        .host
        .peers()
        .into_iter()
        .find(|status| status.peer_id == peer)
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                ErrorCode::NotFound,
                "configured peer was not found",
                false,
                Some(request_id.clone()),
            )
        })?;
    success(request_id, status)
}

async fn peer_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    authorize_peer_admin(&state, session, &peer, &request_id).await?;
    let peer = PeerId::new(peer).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            error.to_string(),
            false,
            Some(request_id.clone()),
        )
    })?;
    let status = state.host.connect_peer(&peer).await.map_err(|error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            bounded_http(&error.to_string()),
            true,
            Some(request_id.clone()),
        )
    })?;
    success(request_id, status)
}

async fn peer_disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    authorize_peer_admin(&state, session, &peer, &request_id).await?;
    let peer = PeerId::new(peer).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            error.to_string(),
            false,
            Some(request_id.clone()),
        )
    })?;
    let status = state.host.disconnect_peer(&peer).await.map_err(|error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            bounded_http(&error.to_string()),
            true,
            Some(request_id.clone()),
        )
    })?;
    success(request_id, status)
}

async fn peer_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    authorize_peer_admin(&state, session, &peer, &request_id).await?;
    let peer = PeerId::new(peer).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            error.to_string(),
            false,
            Some(request_id.clone()),
        )
    })?;
    let status = state.host.revoke_peer(&peer).await.map_err(|error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            bounded_http(&error.to_string()),
            false,
            Some(request_id.clone()),
        )
    })?;
    success(request_id, status)
}

async fn authorize_peer_admin(
    state: &AppState,
    session: ActorSession,
    peer: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    state
        .host
        .authorize_peer_administration(session, peer.to_owned())
        .await
        .map_err(|error| owner_error(error, request_id.to_owned()))?;
    Ok(())
}

fn bounded_http(value: &str) -> String {
    milkdrift_contracts::truncate_utf8(value, 1_024).to_owned()
}

/// Serves until `shutdown` resolves, then closes admission, drains the host, and joins it.
pub async fn serve<F>(
    listener: tokio::net::TcpListener,
    host: DaemonHost,
    shutdown: F,
) -> Result<(), HostError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let address = listener
        .local_addr()
        .map_err(|error| HostError::Startup(error.to_string()))?;
    info!(%address, phase = "listening", "daemon control listener ready");
    let shutdown_host = host.clone();
    let (shutdown_result, shutdown_observed) = tokio::sync::oneshot::channel();
    let graceful = async move {
        shutdown.await;
        let result = shutdown_host.shutdown().await;
        if let Err(error) = &result {
            warn!(phase = "shutdown", outcome = "error", "{error}");
        }
        let _ = shutdown_result.send(result);
    };
    axum::serve(listener, router(host))
        .with_graceful_shutdown(graceful)
        .await
        .map_err(|error| HostError::Startup(error.to_string()))?;
    shutdown_observed
        .await
        .map_err(|_| HostError::Shutdown("shutdown result channel closed".to_owned()))?
}

async fn version(
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

async fn health(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    state
        .host
        .authorize_health(session)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, state.host.health())
}

async fn readiness(
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

async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    if !state.host.accepting_mutations() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Unavailable,
            "daemon is draining and refuses new mutations",
            true,
            Some(request_id),
        ));
    }
    let request: CommandRequest =
        decode_json(&body).map_err(|error| protocol_error(error, request_id.clone()))?;
    request
        .validate()
        .map_err(|error| protocol_error(error, request_id.clone()))?;
    let actor = session.actor.as_str().to_owned();
    let command_id = request.command_id.clone();
    let trace = command_trace(&request.command);
    let started = Instant::now();
    let result = state.host.command(session, request).await;
    match result {
        Ok(value) => {
            info!(
                request_id,
                command_id,
                actor,
                operation = trace.operation,
                run = ?trace.run,
                revision = ?trace.revision,
                attempt = ?trace.attempt,
                proposal = ?trace.proposal,
                latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                outcome = "accepted",
                result_type = %value.result_type,
                replayed = value.replayed,
                "control command completed"
            );
            success(request_id, value)
        }
        Err(error) => {
            warn!(
                request_id,
                command_id,
                actor,
                operation = trace.operation,
                run = ?trace.run,
                revision = ?trace.revision,
                attempt = ?trace.attempt,
                proposal = ?trace.proposal,
                latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                outcome = "rejected",
                code = ?error.code,
                overload = error.code == ErrorCode::Overload,
                "control command failed"
            );
            Err(owner_error(error, request_id))
        }
    }
}

struct CommandTrace {
    operation: &'static str,
    run: Option<String>,
    revision: Option<String>,
    attempt: Option<String>,
    proposal: Option<String>,
}

fn command_trace(command: &Command) -> CommandTrace {
    let (operation, run, revision, attempt, proposal) = match command {
        Command::ImportBlueprint { .. } => ("import_blueprint", None, None, None, None),
        Command::ValidateBlueprint { .. } => ("validate_blueprint", None, None, None, None),
        Command::ImportPromptSequence { .. } => ("import_prompt_sequence", None, None, None, None),
        Command::ValidatePromptSequence { .. } => {
            ("validate_prompt_sequence", None, None, None, None)
        }
        Command::StartRun {
            run_id,
            revision_id,
            ..
        } => (
            "start_run",
            Some(run_id.as_str()),
            Some(revision_id.as_str()),
            None,
            None,
        ),
        Command::PauseRun { run_id } => ("pause_run", Some(run_id.as_str()), None, None, None),
        Command::ResumeRun { run_id } => ("resume_run", Some(run_id.as_str()), None, None, None),
        Command::CancelRun { run_id } => ("cancel_run", Some(run_id.as_str()), None, None, None),
        Command::SignalRun { run_id, .. } => {
            ("signal_run", Some(run_id.as_str()), None, None, None)
        }
        Command::ResolveWork {
            run_id, attempt_id, ..
        } => (
            "resolve_work",
            Some(run_id.as_str()),
            None,
            Some(attempt_id.as_str()),
            None,
        ),
        Command::InspectController { run_id, .. } => (
            "inspect_controller",
            Some(run_id.as_str()),
            None,
            None,
            None,
        ),
        Command::ContinueController { run_id, .. } => (
            "continue_controller",
            Some(run_id.as_str()),
            None,
            None,
            None,
        ),
        Command::SubmitProposal { .. } => ("submit_proposal", None, None, None, None),
        Command::DecideProposal {
            run_id,
            proposal_id,
            proposed_revision,
            ..
        } => (
            "decide_proposal",
            Some(run_id.as_str()),
            Some(proposed_revision.as_str()),
            None,
            Some(proposal_id.as_str()),
        ),
        Command::ApplyProposal {
            run_id,
            proposal_id,
            proposed_revision,
            ..
        } => (
            "apply_proposal",
            Some(run_id.as_str()),
            Some(proposed_revision.as_str()),
            None,
            Some(proposal_id.as_str()),
        ),
        Command::PutLayout { layout } => (
            "put_layout",
            None,
            Some(layout.revision_id.as_str()),
            None,
            None,
        ),
    };
    CommandTrace {
        operation,
        run: run.map(str::to_owned),
        revision: revision.map(str::to_owned),
        attempt: attempt.map(str::to_owned),
        proposal: proposal.map(str::to_owned),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: u32,
    cursor: Option<Cursor>,
    workflow: Option<String>,
    state: Option<String>,
}

const fn default_limit() -> u32 {
    100
}

async fn revisions(
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

async fn revision(
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

async fn revision_diff(
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

async fn runs(
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

async fn run(
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

async fn node(
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

async fn attempt(
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

async fn timeline(
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

async fn capabilities(
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

async fn proposals(
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

#[derive(Deserialize)]
struct ProposalQuery {
    revision: String,
}

async fn proposal(
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

async fn authority(
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

async fn artifact_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(artifact): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let value = state
        .host
        .artifact_metadata(session, artifact)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    success(request_id, value)
}

async fn artifact_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(artifact): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let (offset, maximum) = parse_range(headers.get(header::RANGE), &request_id)?;
    let evidence = format!(
        "api-read-{}",
        state.request_sequence.fetch_add(1, Ordering::SeqCst)
    );
    let result = state
        .host
        .artifact_range(session, artifact, offset, maximum, evidence)
        .await
        .map_err(|error| owner_error(error, request_id.clone()))?;
    let crate::host::ArtifactContentRead {
        metadata,
        offset,
        bytes,
        end,
    } = result;
    let returned_end =
        offset.saturating_add(u64::try_from(bytes.len()).unwrap_or(0).saturating_sub(1));
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&metadata.content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let content_range = format!("bytes {offset}-{returned_end}/{}", metadata.size);
    if let Ok(value) = HeaderValue::from_str(&content_range) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment"),
    );
    response.headers_mut().insert(
        "x-milkdrift-artifact-complete",
        if end {
            HeaderValue::from_static("true")
        } else {
            HeaderValue::from_static("false")
        },
    );
    Ok(response)
}

async fn layout(
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

async fn run_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = format!("run:{run}");
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Run(run.clone()),
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut stream_position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        request_id,
        feed,
        resume = query.cursor.is_some(),
        "run stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(current_session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let session = current_session;
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Run(run.clone()),
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            if state.host.health().draining {
                if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            let last_sequence = stream_position / 2;
            if stream_position != 0 && stream_position % 2 == 0 {
                match state.host.run(session.clone(), run.clone()).await {
                    Ok(status) => {
                        stream_position = stream_position.saturating_add(1);
                        let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
                            break 'events;
                        };
                        yield Ok(event);
                    }
                    _ => break,
                }
            }
            let timeline_cursor = if last_sequence == 0 {
                None
            } else {
                let timeline_feed = format!("timeline:{run}");
                Some(stream_cursor_binding(&session, &timeline_feed))
                    .and_then(|binding| Cursor::new_bound(
                        &timeline_feed,
                        last_sequence,
                        binding,
                        &decision,
                        session.cursor_key(),
                    ).ok())
            };
            match state.host.timeline(
                session.clone(),
                run.clone(),
                timeline_cursor,
                STREAM_PAGE_ITEMS,
            ).await {
                Ok(page) if !page.items.is_empty() => {
                    for entry in page.items {
                        stream_position = entry.sequence.saturating_mul(2);
                        let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::Timeline(entry), &session, &decision).await else {
                            break 'events;
                        };
                        yield Ok(event);
                    }
                    match state.host.run(session.clone(), run.clone()).await {
                        Ok(status) => {
                            stream_position = stream_position.saturating_add(1);
                            let Ok(event) = observation_event(&state.host, &feed, stream_position, Observation::RunStatus(status), &session, &decision).await else {
                                break 'events;
                            };
                            yield Ok(event);
                        }
                        _ => break,
                    }
                }
                Ok(_) => tokio::time::sleep(Duration::from_millis(250)).await,
                Err(error) => {
                    let reason = if error.code == ErrorCode::Unauthorized { "authorization changed" } else { "timeline cursor must be resynchronized" };
                    if let Ok(event) = observation_event(&state.host, &feed, stream_position.saturating_add(1), Observation::ResyncRequired { reason: reason.to_owned() }, &session, &decision).await {
                        yield Ok(event);
                    }
                    break;
                }
            }
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

async fn capability_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = "capability-health".to_owned();
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Capabilities,
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "capability stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Capabilities,
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            let binding = stream_cursor_binding(&session, &feed);
            if state.host.health().draining {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "daemon is draining".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            let capabilities = match state.host.capabilities(session.clone()).await {
                Ok(values) => values,
                _ => break,
            };
            let (resync, entries) = {
                let mut capability_feeds = state.capability_feeds.lock().await;
                let capability_feed = capability_feeds
                    .entry(binding.scope_digest.clone())
                    .or_insert_with(|| CapabilityFeed {
                        next_position: 1,
                        ..CapabilityFeed::default()
                    });
                record_capability_snapshot(capability_feed, &capabilities);
                let latest = capability_feed.next_position.saturating_sub(1);
                let oldest = capability_feed
                    .entries
                    .front()
                    .map_or(capability_feed.next_position, |(entry, _)| *entry);
                let resync = position > latest
                    || (position != 0 && position.saturating_add(1) < oldest);
                let entries = capability_feed
                    .entries
                    .iter()
                    .filter(|(entry, _)| *entry > position)
                    .cloned()
                    .collect::<Vec<_>>();
                (resync, entries)
            };
            if resync {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::ResyncRequired { reason: "capability cursor is outside the retained health window".to_owned() }, &session, &decision).await {
                    yield Ok(event);
                }
                break;
            }
            for (entry_position, capability) in entries {
                position = entry_position;
                let Ok(event) = observation_event(&state.host, &feed, position, Observation::Capability(capability), &session, &decision).await else {
                    break 'events;
                };
                yield Ok(event);
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

fn record_capability_snapshot(feed: &mut CapabilityFeed, capabilities: &[CapabilityRead]) {
    let Ok(bytes) = milkdrift_control_protocol::encode_json(&capabilities) else {
        return;
    };
    let digest = blake3::hash(&bytes).to_hex().to_string();
    if feed.last_snapshot_digest.as_deref() == Some(&digest) {
        return;
    }
    feed.last_snapshot_digest = Some(digest);
    for capability in capabilities {
        let position = feed.next_position;
        feed.next_position = feed.next_position.saturating_add(1);
        feed.entries.push_back((position, capability.clone()));
        while feed.entries.len() > CAPABILITY_FEED_ITEMS {
            feed.entries.pop_front();
        }
    }
}

async fn health_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let (request_id, initial_session) = authenticate(&state, &headers)?;
    let bearer =
        bearer_header(&headers).ok_or_else(|| ApiError::unauthenticated(request_id.clone()))?;
    let feed = "daemon-health".to_owned();
    let initial_decision = stream_authority(
        &state,
        initial_session.clone(),
        StreamAuthority::Health,
        &request_id,
    )
    .await?;
    let initial_binding = stream_cursor_binding(&initial_session, &feed);
    let mut position = query
        .cursor
        .as_ref()
        .map(|cursor| {
            cursor.position_for_bound(&feed, &initial_binding, initial_session.cursor_key())
        })
        .transpose()
        .map_err(|error| protocol_error(error, request_id.clone()))?
        .unwrap_or(0);
    info!(
        feed,
        resume = query.cursor.is_some(),
        "health stream subscription established"
    );
    let output = stream! {
        'events: loop {
            let Some(session) = state.host.authenticate_header(Some(&bearer)) else {
                if let Ok(event) = observation_event(&state.host, &feed, position.saturating_add(1), Observation::StreamClosing { reason: "authorization was revoked or rotated".to_owned() }, &initial_session, &initial_decision).await {
                    yield Ok(event);
                }
                break;
            };
            let decision = match stream_authority(
                &state,
                session.clone(),
                StreamAuthority::Health,
                &request_id,
            ).await {
                Ok(decision) => decision,
                Err(_) => break,
            };
            let (generation, health) = state.host.health_snapshot();
            if generation > position {
                position = generation;
                let Ok(event) = observation_event(&state.host, &feed, position, Observation::DaemonHealth(health.clone()), &session, &decision).await else {
                    break 'events;
                };
                yield Ok(event);
            }
            if health.draining {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    };
    Ok(Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    ))
}

async fn stream_authority(
    state: &AppState,
    session: ActorSession,
    stream: StreamAuthority,
    request_id: &str,
) -> Result<String, ApiError> {
    let decision = state
        .host
        .authorize_stream(session, stream)
        .await
        .map_err(|error| owner_error(error, request_id.to_owned()))?;
    Ok(decision)
}

fn stream_cursor_binding(session: &ActorSession, exact_resource_and_filter: &str) -> CursorBinding {
    let claim = session.context.authority();
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.continuation-scope.v1\0");
    hasher.update(exact_resource_and_filter.as_bytes());
    hasher.update(format!("{:?}", session.grant.resources()).as_bytes());
    CursorBinding {
        actor: session.actor.as_str().to_owned(),
        grant_id: claim.grant().as_str().to_owned(),
        grant_revision: claim.grant_revision(),
        grant_digest: claim.grant_digest().as_str().to_owned(),
        scope_digest: format!("b3_{}", hasher.finalize()),
    }
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<(String, ActorSession), ApiError> {
    let request_id = request_id(state, headers);
    let Some(bearer) = bearer_header(headers) else {
        warn!(
            request_id,
            outcome = "unauthenticated",
            "control authentication failed"
        );
        return Err(ApiError::unauthenticated(request_id));
    };
    let Some(session) = state.host.authenticate_header(Some(&bearer)) else {
        warn!(
            request_id,
            outcome = "unauthenticated",
            "control authentication failed"
        );
        return Err(ApiError::unauthenticated(request_id));
    };
    Ok((request_id, session))
}

fn bearer_header(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if !value.starts_with("Bearer ") || value.len() > 4_103 {
        return None;
    }
    Some(value.to_owned())
}

fn request_id(state: &AppState, headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value.is_ascii()
                && !value.bytes().any(|byte| byte.is_ascii_control())
        })
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "req-{}",
                state.request_sequence.fetch_add(1, Ordering::SeqCst)
            )
        })
}

fn success<T: serde::Serialize>(request_id: String, value: T) -> Result<Response, ApiError> {
    let envelope = ResponseEnvelope {
        protocol: ProtocolVersion::CURRENT,
        request_id,
        value,
    };
    let bytes = milkdrift_control_protocol::encode_json(&envelope)
        .map_err(|_| internal_response("response-encoding".to_owned()))?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    Ok(response)
}

fn owner_error(error: PublicFailure, request_id: String) -> ApiError {
    let status = match error.code {
        ErrorCode::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthorized => StatusCode::FORBIDDEN,
        ErrorCode::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorCode::Conflict | ErrorCode::Uncertain => StatusCode::CONFLICT,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Overload => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::UnsupportedVersion => StatusCode::UPGRADE_REQUIRED,
        ErrorCode::Timeout => StatusCode::GATEWAY_TIMEOUT,
        ErrorCode::Corruption | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let mut envelope = ErrorEnvelope::new(error.code, error.message, error.retryable);
    envelope.request_id = Some(request_id);
    envelope.details = error.details;
    ApiError { status, envelope }
}

fn protocol_error(
    error: milkdrift_control_protocol::ProtocolError,
    request_id: String,
) -> ApiError {
    let (status, code) = match error {
        milkdrift_control_protocol::ProtocolError::UnsupportedMajor { .. } => {
            (StatusCode::UPGRADE_REQUIRED, ErrorCode::UnsupportedVersion)
        }
        milkdrift_control_protocol::ProtocolError::Bounds(_) => {
            (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::Overload)
        }
        _ => (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
    };
    ApiError::new(status, code, error.to_string(), false, Some(request_id))
}

fn internal_response(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::Internal,
        "internal control response mismatch",
        false,
        Some(request_id),
    )
}

fn not_found_response(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "requested resource was not found",
        false,
        Some(request_id),
    )
}

fn parse_range(value: Option<&HeaderValue>, request_id: &str) -> Result<(u64, u32), ApiError> {
    let Some(value) = value else {
        return Ok((0, MAX_ARTIFACT_HTTP_RANGE));
    };
    let value = value.to_str().map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            "range header is invalid",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let range = value.strip_prefix("bytes=").ok_or_else(|| {
        ApiError::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::InvalidInput,
            "only one explicit byte range is supported",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let (start, end) = range.split_once('-').ok_or_else(|| {
        ApiError::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::InvalidInput,
            "byte range requires start and end",
            false,
            Some(request_id.to_owned()),
        )
    })?;
    let start: u64 = start
        .parse()
        .map_err(|_| not_found_response(request_id.to_owned()))?;
    let end: u64 = end
        .parse()
        .map_err(|_| not_found_response(request_id.to_owned()))?;
    let requested = end
        .checked_sub(start)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| not_found_response(request_id.to_owned()))?;
    let maximum = u32::try_from(requested.min(u64::from(MAX_ARTIFACT_HTTP_RANGE)))
        .unwrap_or(MAX_ARTIFACT_HTTP_RANGE);
    Ok((start, maximum))
}

async fn observation_event(
    host: &DaemonHost,
    feed: &str,
    position: u64,
    observation: Observation,
    session: &ActorSession,
    decision_digest: &str,
) -> Result<Event, ()> {
    let observed_at_ms = host.now().await.map_err(|error| {
        warn!(
            outcome = "closed",
            code = "observation_clock_unavailable",
            "observation stream closed because its timestamp boundary failed: {}",
            error.message
        );
    })?;
    let cursor = Cursor::new_bound(
        feed,
        position,
        stream_cursor_binding(session, feed),
        decision_digest,
        session.cursor_key(),
    )
    .map_err(|_| ())?;
    let envelope = ObservationEnvelope {
        protocol: ProtocolVersion::CURRENT,
        cursor: cursor.clone(),
        observed_at_ms,
        feed: feed.to_owned(),
        observation,
    };
    let data =
        String::from_utf8(milkdrift_control_protocol::encode_json(&envelope).map_err(|_| ())?)
            .map_err(|_| ())?;
    Ok(Event::default()
        .id(cursor.as_str())
        .event("observation")
        .data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ranges_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let header = HeaderValue::from_static("bytes=100-9999999");
        let (start, maximum) =
            parse_range(Some(&header), "request").map_err(|error| error.envelope.message)?;
        assert_eq!(start, 100);
        assert_eq!(maximum, MAX_ARTIFACT_HTTP_RANGE);
        Ok(())
    }

    #[test]
    fn every_local_external_route_declares_typed_authority_and_resource_mapping() {
        let source = include_str!("http.rs");
        let raw_route_marker = [".", "route("].concat();
        assert_eq!(
            source.match_indices(&raw_route_marker).count(),
            1,
            "add external routes through authorized_routes! with a typed authority mapping"
        );
    }
}
