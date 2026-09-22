//! Authenticated bounded lifecycle transport; semantic authority and state checks remain below it.
use super::{ApiError, AppState, authenticate, success};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use milkdrift_capability::managed::ManagedRequest;
use milkdrift_capability_host::managed::ManagedError;
use milkdrift_control_protocol::{ErrorCode, decode_json};

pub(super) async fn manage(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (id, session) = authenticate(&state, &headers)?;
    let request: ManagedRequest =
        decode_json(&body).map_err(|e| failure(ManagedError::Rejected(e.to_string()), &id))?;
    let permit = state
        .serving_requests
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            failure(
                ManagedError::Conflict("bounded management request capacity reached".to_owned()),
                &id,
            )
        })?;
    let value = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.host.manage_resources(&session, &request)
    })
    .await
    .map_err(|_| {
        failure(
            ManagedError::Platform("management task interrupted".to_owned()),
            &id,
        )
    })?
    .map_err(|e| failure(e, &id))?;
    success(id, value)
}

fn failure(error: ManagedError, id: &str) -> ApiError {
    let (status, code) = match error {
        ManagedError::Unauthorized => (StatusCode::FORBIDDEN, ErrorCode::Unauthorized),
        ManagedError::Rejected(_) => (StatusCode::BAD_REQUEST, ErrorCode::InvalidInput),
        ManagedError::Conflict(_) => (StatusCode::CONFLICT, ErrorCode::Conflict),
        ManagedError::Persistence(
            milkdrift_persistence::PersistenceError::ImmutableConflict { .. }
            | milkdrift_persistence::PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::OwnerBusy,
                ..
            },
        ) => (StatusCode::CONFLICT, ErrorCode::Conflict),
        ManagedError::Persistence(_) | ManagedError::Platform(_) => {
            (StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Unavailable)
        }
    };
    ApiError::new(
        status,
        code,
        "resource request refused; inspect the authorized installation for state and recovery diagnostics",
        false,
        Some(id.to_owned()),
    )
}
