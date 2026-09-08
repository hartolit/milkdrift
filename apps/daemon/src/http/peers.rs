//! Expose configured peer observations and explicit administration under the local actor's grant.
use super::{ApiError, AppState, authenticate, owner_error, success};
use crate::auth::ActorSession;
use axum::{extract::Path, extract::State, http::HeaderMap, http::StatusCode, response::Response};
use milkdrift_capability::PeerId;
use milkdrift_control_protocol::ErrorCode;

pub(super) async fn peers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
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

pub(super) async fn peer(
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

pub(super) async fn peer_connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let peer = authorize_peer_admin(&state, session, peer, &request_id).await?;
    let status = state
        .host
        .connect_peer(&peer)
        .await
        .map_err(|error| peer_failure(&error.to_string(), true, &request_id))?;
    success(request_id, status)
}

pub(super) async fn peer_disconnect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let peer = authorize_peer_admin(&state, session, peer, &request_id).await?;
    let status = state
        .host
        .disconnect_peer(&peer)
        .await
        .map_err(|error| peer_failure(&error.to_string(), true, &request_id))?;
    success(request_id, status)
}

pub(super) async fn peer_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(peer): Path<String>,
) -> Result<Response, ApiError> {
    let (request_id, session) = authenticate(&state, &headers)?;
    let peer = authorize_peer_admin(&state, session, peer, &request_id).await?;
    let status = state
        .host
        .revoke_peer(&peer)
        .await
        .map_err(|error| peer_failure(&error.to_string(), false, &request_id))?;
    success(request_id, status)
}

async fn authorize_peer_admin(
    state: &AppState,
    session: ActorSession,
    peer: String,
    request_id: &str,
) -> Result<PeerId, ApiError> {
    state
        .host
        .authorize_peer_administration(session, peer.clone())
        .await
        .map_err(|error| owner_error(error, request_id.to_owned()))?;
    PeerId::new(peer).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::InvalidInput,
            error.to_string(),
            false,
            Some(request_id.to_owned()),
        )
    })
}

fn peer_failure(message: &str, retryable: bool, request_id: &str) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Unavailable,
        milkdrift_contracts::truncate_utf8(message, 1_024).to_owned(),
        retryable,
        Some(request_id.to_owned()),
    )
}
