//! Stable failure categories, redaction and process exits.
use milkdrift_control_client::{ClientError, status_class};
use milkdrift_control_protocol::{ErrorCode, RunRead};
use serde_json::{Value, json};

#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("run terminal outcome did not satisfy the requested success condition")]
    FailedTask(Box<RunRead>),
    #[error("internal CLI failure: {0}")]
    Internal(String),
    #[error(
        "command deadline or polling/reconnect bound reached; submitted work may still be running"
    )]
    Deadline,
    #[error("client cancelled; submitted work may still be running")]
    Cancelled,
}

pub(crate) fn exit_code(error: &CliError) -> u8 {
    match error {
        CliError::Invalid(_) => 2,
        CliError::NotFound(_) => 6,
        CliError::Internal(_) => 9,
        CliError::FailedTask(_) => 8,
        CliError::Deadline => 10,
        CliError::Cancelled => 130,
        CliError::Client(ClientError::Configuration(_)) => 2,
        CliError::Client(client) => match status_class(client).map(|status| status.as_u16()) {
            Some(401 | 403) => 3,
            Some(409) => 4,
            Some(429 | 502 | 503 | 504) | None if client.retryable() => 5,
            Some(404) => 6,
            Some(_) => 7,
            None => 9,
        },
    }
}

pub(crate) fn emit_error(json: bool, operation: &str, command_id: Option<&str>, error: &CliError) {
    let (classification, detail) = match error {
        CliError::Invalid(_) | CliError::Client(ClientError::Configuration(_)) => (
            "invalid_input",
            "input rejected; check command help and the document contract",
        ),
        CliError::NotFound(_) => ("not_found", "resource not found"),
        CliError::FailedTask(_) => (
            "failed_terminal",
            "run terminal outcome did not satisfy the requested success condition",
        ),
        CliError::Deadline => (
            "timeout",
            "command deadline or polling/reconnect bound reached; submitted work may still be running",
        ),
        CliError::Cancelled => (
            "cancelled",
            "client cancelled; submitted work may still be running",
        ),
        CliError::Internal(_)
        | CliError::Client(ClientError::Protocol(_) | ClientError::Stream(_)) => {
            ("internal_client", "internal client or protocol failure")
        }
        CliError::Client(ClientError::Transport(_) | ClientError::Timeout) => (
            "unavailable",
            "control endpoint unavailable; submitted work may still be running",
        ),
        CliError::Client(ClientError::Api(api)) => match api.code {
            ErrorCode::Unauthenticated | ErrorCode::Unauthorized => {
                ("authorization", "authentication or authority refused")
            }
            ErrorCode::Conflict => (
                "conflict",
                "exact command, revision, sequence or runtime policy conflict",
            ),
            ErrorCode::NotFound => ("not_found", "resource not found"),
            ErrorCode::Overload | ErrorCode::Unavailable | ErrorCode::Timeout => (
                "unavailable",
                "control endpoint unavailable; submitted work may still be running",
            ),
            _ => ("daemon_api", "daemon refused the operation"),
        },
    };
    if !json {
        match error {
            CliError::Invalid(_) => eprintln!("milkdrift: {error}"),
            _ => eprintln!("milkdrift: {detail}"),
        }
        return;
    }
    let daemon_code = match error {
        CliError::Client(ClientError::Api(api)) => Some(api.code),
        _ => None,
    };
    let retryable = match error {
        CliError::Client(client) => Some(client.retryable()),
        CliError::Deadline | CliError::Cancelled => None,
        _ => Some(false),
    };
    let value = match error {
        CliError::FailedTask(run) => serde_json::to_value(run).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let failure = json!({"classification": classification, "code": daemon_code.map_or_else(|| classification.to_owned(), |code| serde_json::to_value(code).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| classification.to_owned())), "daemon_code": daemon_code, "retryable": retryable, "detail": detail});
    if let Ok(encoded) =
        crate::output::encode(operation, command_id, "failure", value, failure, true)
    {
        println!("{encoded}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use milkdrift_control_protocol::ErrorEnvelope;

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&CliError::Invalid("fixture".to_owned())), 2);
        assert_eq!(exit_code(&CliError::NotFound("fixture".to_owned())), 6);
        assert_eq!(exit_code(&CliError::Internal("fixture".to_owned())), 9);
        assert_eq!(exit_code(&CliError::Deadline), 10);
        assert_eq!(exit_code(&CliError::Cancelled), 130);
        for (code, retry, exit) in [
            (ErrorCode::Unauthorized, false, 3),
            (ErrorCode::Conflict, false, 4),
            (ErrorCode::Overload, true, 5),
            (ErrorCode::InvalidInput, false, 7),
        ] {
            assert_eq!(
                exit_code(&CliError::Client(ClientError::Api(ErrorEnvelope::new(
                    code, "fixture", retry
                )))),
                exit
            );
        }
    }
}
