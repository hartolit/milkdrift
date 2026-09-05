mod response;
use response::{
    authenticate, bearer_header, not_found_response, owner_error, protocol_error, success,
};
mod peers;
use peers::{peer, peer_connect, peer_disconnect, peer_revoke, peers};
mod commands;
use commands::command;
mod reads;
use reads::{
    attempt, authority, capabilities, health, layout, node, proposal, proposals, readiness,
    revision, revision_diff, revisions, run, runs, timeline, version,
};
mod artifacts;
use artifacts::{artifact_content, artifact_metadata};
mod streams;
use reads::ListQuery;
use response::ApiError;
use streams::{CapabilityFeed, capability_stream, health_stream, run_stream};

use std::{collections::BTreeMap, future::Future, sync::Arc, sync::atomic::AtomicU64};

use axum::{Router, extract::DefaultBodyLimit, routing::get, routing::post};
use milkdrift_authority::AuthorityOperation;
use tower::ServiceBuilder;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};
use tracing::{info, warn};

use crate::{DaemonHost, HostError};

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

/// Builds the bounded protocol-2.3 router. CORS is intentionally absent.
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

#[cfg(test)]
mod tests {
    use super::artifacts::parse_range;
    use super::*;
    use axum::http::HeaderValue;

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
