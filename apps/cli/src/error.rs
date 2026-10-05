//! Stable failure categories, redaction and process exits.
use milkdrift_control_client::{ClientError, status_class};
use milkdrift_control_protocol::{ErrorCode, RunRead};
use serde_json::{Value, json};
use std::io::Write as _;

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
    #[error("invocation was refused or did not complete successfully")]
    InvocationFailed(Box<Value>),
    #[error("internal CLI failure: {0}")]
    Internal(String),
    #[error("private output staging cleanup unconfirmed: {operation}; cleanup: {cleanup:?}")]
    OutputCleanup {
        operation: Box<CliError>,
        cleanup: std::io::ErrorKind,
    },
    #[error("output published, but directory durability is unconfirmed: {operation}")]
    OutputPublished { operation: Box<CliError> },
    #[error(
        "command deadline or polling/reconnect bound reached; submitted work may still be running"
    )]
    Deadline,
    #[error("client cancelled; submitted work may still be running")]
    Cancelled,
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(format!("I/O operation failed: {:?}", error.kind()))
    }
}

pub(crate) fn exit_code(error: &CliError) -> u8 {
    match error {
        CliError::Invalid(_) | CliError::Client(ClientError::Configuration(_)) => 2,
        CliError::NotFound(_) => 6,
        CliError::Internal(_)
        | CliError::OutputCleanup { .. }
        | CliError::OutputPublished { .. } => 9,
        CliError::FailedTask(_) | CliError::InvocationFailed(_) => 8,
        CliError::Deadline => 10,
        CliError::Cancelled => 130,
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

pub(crate) fn report(
    json: bool,
    operation: &str,
    command_id: Option<&str>,
    error: &CliError,
) -> std::process::ExitCode {
    // A failed diagnostic cannot be reported through the same failed channel. Preserve a
    // nonzero exit even when neither the human nor the machine envelope can be delivered.
    std::process::ExitCode::from(if emit_error(json, operation, command_id, error).is_ok() {
        exit_code(error)
    } else {
        9
    })
}

fn emit_error(
    json: bool,
    operation: &str,
    command_id: Option<&str>,
    error: &CliError,
) -> Result<(), CliError> {
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
        CliError::InvocationFailed(_) => (
            "invocation_failed",
            "invocation was refused or did not complete successfully; inspect retained evidence",
        ),
        CliError::Deadline => (
            "timeout",
            "command deadline or polling/reconnect bound reached; submitted work may still be running",
        ),
        CliError::Cancelled => (
            "cancelled",
            "client cancelled; submitted work may still be running",
        ),
        CliError::OutputCleanup { .. } => (
            "internal_client",
            "private output staging cleanup could not be confirmed; inspect retained staging before removal",
        ),
        CliError::OutputPublished { .. } => (
            "internal_client",
            "output was published, but directory durability is unconfirmed; inspect the destination before retrying",
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
        let mut stderr = std::io::stderr().lock();
        match error {
            CliError::Invalid(_) => writeln!(stderr, "milkdrift: {error}")?,
            _ => writeln!(stderr, "milkdrift: {detail}")?,
        }
        stderr.flush()?;
        return Ok(());
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
        CliError::FailedTask(run) => {
            serde_json::to_value(run).map_err(|error| CliError::Internal(error.to_string()))?
        }
        CliError::InvocationFailed(value) => (**value).clone(),
        _ => Value::Null,
    };
    let code = daemon_code
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| CliError::Internal(error.to_string()))?
        .unwrap_or_else(|| Value::String(classification.to_owned()));
    let failure = json!({"classification": classification, "code": code, "daemon_code": daemon_code, "retryable": retryable, "detail": detail});
    let encoded = crate::output::encode(operation, command_id, "failure", value, failure, true)?;
    crate::output::line(format_args!("{encoded}"))
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
