//! Response transport adaptation over typed daemon calls.
use super::AppState;
use crate::{auth::ActorSession, host::PublicFailure};
use axum::{
    body::Body, http::HeaderMap, http::HeaderValue, http::StatusCode, http::header,
    response::IntoResponse, response::Response,
};
use milkdrift_control_protocol::{ErrorCode, ErrorEnvelope, ProtocolVersion, ResponseEnvelope};
use std::sync::atomic::Ordering;
use tracing::warn;

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    pub(super) envelope: ErrorEnvelope,
}

impl ApiError {
    pub(super) fn new(
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

    pub(super) fn unauthenticated(request_id: String) -> Self {
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

pub(super) fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(String, ActorSession), ApiError> {
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

pub(super) fn bearer_header(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if !value.starts_with("Bearer ") || value.len() > 4_103 {
        return None;
    }
    Some(value.to_owned())
}

pub(super) fn request_id(state: &AppState, headers: &HeaderMap) -> String {
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

pub(super) fn success<T: serde::Serialize>(
    request_id: String,
    value: T,
) -> Result<Response, ApiError> {
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

pub(super) fn owner_error(error: PublicFailure, request_id: String) -> ApiError {
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

pub(super) fn protocol_error(
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

pub(super) fn internal_response(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::Internal,
        "internal control response mismatch",
        false,
        Some(request_id),
    )
}

pub(super) fn not_found_response(request_id: String) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        ErrorCode::NotFound,
        "requested resource was not found",
        false,
        Some(request_id),
    )
}
