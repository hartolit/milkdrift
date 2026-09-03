//! Stable CLI failure classification and process exit mapping.

use milkdrift_control_client::{ClientError, status_class};
use milkdrift_control_protocol::{ErrorCode, MAX_REASON_BYTES};
use serde::Serialize;

const JSON_OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("task failed: {0}")]
    FailedTask(String),
    #[error("internal error: {0}")]
    Internal(String),
}

pub(crate) fn exit_code(error: &CliError) -> u8 {
    match error {
        CliError::Invalid(_) => 2,
        CliError::NotFound(_) => 6,
        CliError::Internal(_) => 9,
        CliError::FailedTask(_) => 8,
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

#[derive(Serialize)]
struct JsonFailure<'a> {
    schema_version: u32,
    #[serde(rename = "type")]
    kind: &'static str,
    value: JsonFailureValue<'a>,
}

#[derive(Serialize)]
struct JsonFailureValue<'a> {
    classification: &'static str,
    daemon_code: Option<ErrorCode>,
    retryable: bool,
    detail: &'a str,
}

pub(crate) fn emit_error(json: bool, error: &CliError) {
    if !json {
        eprintln!("milkdrift: {error}");
        return;
    }
    eprintln!("{}", json_error(error));
}

fn json_error(error: &CliError) -> String {
    let detail = safe_detail(error);
    let boundary = milkdrift_contracts::truncate_utf8(&detail, MAX_REASON_BYTES).len();
    let document = JsonFailure {
        schema_version: JSON_OUTPUT_SCHEMA_VERSION,
        kind: "error",
        value: JsonFailureValue {
            classification: classification(error),
            daemon_code: daemon_code(error),
            retryable: retryable(error),
            detail: &detail[..boundary],
        },
    };
    serde_json::to_string(&document).unwrap_or_else(|_| {
        r#"{"schema_version":1,"type":"error","value":{"classification":"internal_client","daemon_code":null,"retryable":false,"detail":"error output encoding failed"}}"#.to_owned()
    })
}

fn classification(error: &CliError) -> &'static str {
    match error {
        CliError::Invalid(_) | CliError::Client(ClientError::Configuration(_)) => "invalid_input",
        CliError::NotFound(_) => "not_found",
        CliError::FailedTask(_) => "failed_terminal",
        CliError::Internal(_) => "internal_client",
        CliError::Client(ClientError::Transport(_) | ClientError::Timeout) => "unavailable",
        CliError::Client(ClientError::Protocol(_) | ClientError::Stream(_)) => "internal_client",
        CliError::Client(ClientError::Api(api)) => match api.code {
            ErrorCode::Unauthenticated | ErrorCode::Unauthorized => "authorization",
            ErrorCode::Conflict => "conflict",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Overload | ErrorCode::Unavailable | ErrorCode::Timeout => "unavailable",
            ErrorCode::InvalidInput
            | ErrorCode::Corruption
            | ErrorCode::Uncertain
            | ErrorCode::UnsupportedVersion
            | ErrorCode::Internal => "daemon_api",
        },
    }
}

fn daemon_code(error: &CliError) -> Option<ErrorCode> {
    match error {
        CliError::Client(ClientError::Api(api)) => Some(api.code),
        _ => None,
    }
}

fn retryable(error: &CliError) -> bool {
    match error {
        CliError::Client(client) => client.retryable(),
        CliError::Invalid(_)
        | CliError::NotFound(_)
        | CliError::FailedTask(_)
        | CliError::Internal(_) => false,
    }
}

fn safe_detail(error: &CliError) -> String {
    match error {
        CliError::Invalid(detail) | CliError::NotFound(detail) | CliError::FailedTask(detail) => {
            detail.clone()
        }
        CliError::Internal(_) => "internal CLI failure".to_owned(),
        CliError::Client(ClientError::Api(api)) => api.message.clone(),
        CliError::Client(ClientError::Configuration(detail)) => detail.clone(),
        CliError::Client(ClientError::Transport(_)) => "control transport failed".to_owned(),
        CliError::Client(ClientError::Protocol(_)) => {
            "daemon response violated the control protocol".to_owned()
        }
        CliError::Client(ClientError::Timeout) => "control request timed out".to_owned(),
        CliError::Client(ClientError::Stream(_)) => "control stream failed".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use milkdrift_control_client::ClientError;
    use milkdrift_control_protocol::{ErrorCode, ErrorEnvelope};

    use super::{CliError, exit_code, json_error};

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&CliError::Invalid("fixture".to_owned())), 2);
        assert_eq!(exit_code(&CliError::FailedTask("fixture".to_owned())), 8);
        assert_eq!(exit_code(&CliError::Internal("fixture".to_owned())), 9);
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(ErrorEnvelope::new(
                ErrorCode::Unauthorized,
                "fixture",
                false,
            )))),
            3
        );
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(ErrorEnvelope::new(
                ErrorCode::Conflict,
                "fixture",
                false,
            )))),
            4
        );
        assert_eq!(
            exit_code(&CliError::Client(ClientError::Api(ErrorEnvelope::new(
                ErrorCode::Overload,
                "fixture",
                true,
            )))),
            5
        );
    }

    #[test]
    fn json_failures_are_single_bounded_machine_documents() -> Result<(), Box<dyn std::error::Error>>
    {
        let document: serde_json::Value =
            serde_json::from_str(&json_error(&CliError::Invalid("fixture".to_owned())))?;
        assert_eq!(document["schema_version"], 1);
        assert_eq!(document["type"], "error");
        assert_eq!(document["value"]["classification"], "invalid_input");
        assert_eq!(document["value"]["daemon_code"], serde_json::Value::Null);
        assert_eq!(document["value"]["retryable"], false);
        assert!(!document.to_string().contains('\u{1b}'));
        Ok(())
    }
}
