mod browser;
mod metadata;
mod projects;
mod response;
mod tasks;
use response::{
    authenticate, bearer_header, not_found_response, owner_error, protocol_error, success,
};
mod peers;
use peers::{peer, peer_connect, peer_disconnect, peer_revoke, peers};
mod reads;
use reads::{authority, capabilities, health, readiness, version};
mod artifacts;
mod invocations;
mod resources;
use artifacts::{artifact_content, artifact_metadata};
mod streams;
use reads::ListQuery;
use response::ApiError;
use streams::{CapabilityFeeds, capability_stream, health_stream};

use std::{future::Future, sync::Arc, sync::atomic::AtomicU64};

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use milkdrift_authority::AuthorityOperation;
use tower::ServiceBuilder;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::{info, warn};

use crate::{DaemonHost, HostError};

const MAX_ARTIFACT_HTTP_RANGE: u32 = 1_048_576;
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
    Run,
    Capability,
    Peer,
    Artifact,
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
    capability_feeds: Arc<tokio::sync::Mutex<CapabilityFeeds>>,
    stream_incarnation: [u8; 32],
    serving_requests: Arc<tokio::sync::Semaphore>,
}

/// Builds the bounded control router with exact configured browser origins.
pub(crate) fn router(host: DaemonHost) -> Result<Router, HostError> {
    let browser_origins = host.browser_origins();
    let mut stream_incarnation = [0; 32];
    getrandom::fill(&mut stream_incarnation)
        .map_err(|error| HostError::Startup(format!("stream incarnation unavailable: {error}")))?;
    let peer_service = host.peer_service();
    let state = AppState {
        stream_incarnation,
        host,
        request_sequence: Arc::new(AtomicU64::new(1)),
        capability_feeds: Arc::new(tokio::sync::Mutex::new(CapabilityFeeds::default())),
        serving_requests: Arc::new(tokio::sync::Semaphore::new(256)),
    };
    let router = authorized_routes! { Router::new();
        "/v1/session" => get(tasks::session), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Daemon;
        "/v1/daemon/metadata" => get(metadata::read), RouteAuthorityMapping::Exact(AuthorityOperation::NegotiateControlProtocol), RouteResourceMapping::Daemon;
        "/v1/projects" => get(projects::list), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProject), RouteResourceMapping::Daemon;
        "/v1/projects/{project}" => get(projects::read), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProject), RouteResourceMapping::Daemon;
        "/v1/project-directories" => get(projects::directories), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProject), RouteResourceMapping::Daemon;
        "/v1/project-environments" => get(projects::environments), RouteAuthorityMapping::Exact(AuthorityOperation::ListCapabilities), RouteResourceMapping::Capability;
        "/v1/project-commands" => post(projects::command), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Daemon;
        "/v1/project-receipts/{request}" => get(projects::receipt), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProject), RouteResourceMapping::Daemon;
        "/v1/project-operations" => get(projects::operations), RouteAuthorityMapping::Exact(AuthorityOperation::InspectProject), RouteResourceMapping::Daemon;
        "/v1/daemon/rename" => post(metadata::rename), RouteAuthorityMapping::Exact(AuthorityOperation::RenameDaemon), RouteResourceMapping::Daemon;
        "/v1/daemon/rename-requests/{request}" => get(metadata::receipt), RouteAuthorityMapping::Exact(AuthorityOperation::NegotiateControlProtocol), RouteResourceMapping::Daemon;
        "/v1/workflows" => get(tasks::list).post(tasks::create), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Run;
        "/v1/workflows/stream" => get(tasks::stream), RouteAuthorityMapping::StreamDerived, RouteResourceMapping::Run;
        "/v1/workflows/{workflow}" => get(tasks::workflow), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Run;
        "/v1/workflows/{workflow}/changes" => post(tasks::change), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Run;
        "/v1/workflows/{workflow}/tasks/{task}" => get(tasks::task), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Run;
        "/v1/workflows/{workflow}/tasks/{task}/result" => get(tasks::result), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Run;
        "/v1/workflows/{workflow}/tasks/{task}/result/content" => get(tasks::result_content), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Run;
        "/v1/workflow-requests/{request}" => get(tasks::receipt), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Run;
        "/v1/task-inputs" => post(tasks::upload), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Artifact;
        "/v1/version" => post(version), RouteAuthorityMapping::Exact(AuthorityOperation::NegotiateControlProtocol), RouteResourceMapping::Daemon;
        "/v1/health" => get(health), RouteAuthorityMapping::Exact(AuthorityOperation::InspectDaemonHealth), RouteResourceMapping::Daemon;
        "/v1/readiness" => get(readiness), RouteAuthorityMapping::Exact(AuthorityOperation::ReadReadiness), RouteResourceMapping::Daemon;
        "/v1/resources" => post(resources::manage), RouteAuthorityMapping::CommandDerived, RouteResourceMapping::Capability;
        "/v1/capabilities" => get(capabilities), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Capability;
        "/v1/execution/catalog" => get(invocations::catalog), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Capability;
        "/v1/invocations/prepare" => post(invocations::prepare), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Capability;
        "/v1/invocations" => post(invocations::invoke), RouteAuthorityMapping::Exact(AuthorityOperation::InvokeCapability), RouteResourceMapping::Capability;
        "/v1/invocation-requests/{request}" => get(invocations::lookup), RouteAuthorityMapping::Exact(AuthorityOperation::Inspect), RouteResourceMapping::Capability;
        "/v1/invocations/{execution}" => get(invocations::inspect), RouteAuthorityMapping::Exact(AuthorityOperation::Inspect), RouteResourceMapping::Capability;
        "/v1/invocations/{execution}/observations" => get(invocations::observations), RouteAuthorityMapping::QueryDerived, RouteResourceMapping::Capability;
        "/v1/invocations/{execution}/outputs/{artifact}" => get(invocations::output), RouteAuthorityMapping::Exact(AuthorityOperation::ReadCapabilityOutput), RouteResourceMapping::Capability;
        "/v1/invocations/{execution}/cancel" => post(invocations::cancel), RouteAuthorityMapping::Exact(AuthorityOperation::CancelCapability), RouteResourceMapping::Capability;
        "/v1/peers" => get(peers), RouteAuthorityMapping::Exact(AuthorityOperation::InspectPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}" => get(peer), RouteAuthorityMapping::Exact(AuthorityOperation::InspectPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/connect" => post(peer_connect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/reload" => post(peer_connect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/disconnect" => post(peer_disconnect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/drain" => post(peer_disconnect), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/peers/{peer}/revoke" => post(peer_revoke), RouteAuthorityMapping::Exact(AuthorityOperation::AdministerPeer), RouteResourceMapping::Peer;
        "/v1/authority" => get(authority), RouteAuthorityMapping::Exact(AuthorityOperation::InspectOwnAuthority), RouteResourceMapping::Daemon;
        "/v1/artifacts/{artifact}" => get(artifact_metadata), RouteAuthorityMapping::Exact(AuthorityOperation::ReadArtifactMetadata), RouteResourceMapping::Artifact;
        "/v1/artifact-inputs" => post(artifacts::upload_input), RouteAuthorityMapping::Exact(AuthorityOperation::PublishArtifact), RouteResourceMapping::Artifact;
        "/v1/artifacts/{artifact}/content" => get(artifact_content), RouteAuthorityMapping::Exact(AuthorityOperation::ReadArtifactContent), RouteResourceMapping::Artifact;
        "/v1/stream/health" => get(health_stream), RouteAuthorityMapping::Exact(AuthorityOperation::InspectDaemonHealth), RouteResourceMapping::Daemon;
        "/v1/stream/capabilities" => get(capability_stream), RouteAuthorityMapping::StreamDerived, RouteResourceMapping::Capability;
    }
        .layer(DefaultBodyLimit::max(
            milkdrift_control_protocol::MAX_DOCUMENT_BYTES,
        ))
        .layer(axum::middleware::from_fn_with_state(browser_origins, browser::allow_origin))
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                .layer(TraceLayer::new_for_http()),
        )
        .with_state(state);
    Ok(if let Some(service) = peer_service {
        router.merge(milkdrift_peer_http::peer_router(service))
    } else {
        router
    })
}

/// Serves until `shutdown` resolves, then closes admission, drains the host, and joins it.
///
/// # Errors
/// Returns listener/serving errors and propagates incomplete or failed host shutdown.
/// A closed shutdown-result channel also fails rather than implying successful cleanup.
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
    let router = match router(host.clone()) {
        Ok(router) => router,
        Err(error) => {
            host.shutdown()
                .await
                .map_err(|cleanup| HostError::Startup(format!("{error}; {cleanup}")))?;
            return Err(error);
        }
    };
    info!(%address, phase = "listening", "daemon control listener ready");
    let shutdown_host = host.clone();
    let (shutdown_result, shutdown_observed) = tokio::sync::oneshot::channel();
    let graceful = async move {
        shutdown.await;
        let result = shutdown_host.shutdown().await;
        if let Err(error) = &result {
            warn!(phase = "shutdown", outcome = "error", "{error}");
        }
        if shutdown_result.send(result).is_err() {
            tracing::debug!("host shutdown finished after the HTTP caller stopped waiting");
        }
    };
    axum::serve(listener, router)
        .with_graceful_shutdown(graceful)
        .await
        .map_err(|error| HostError::Startup(error.to_string()))?;
    shutdown_observed
        .await
        .map_err(|_| HostError::Shutdown("shutdown result channel closed".to_owned()))?
}

#[cfg(test)]
mod tests;
