//! Stable CLI failure classification and process exit mapping.

use milkdrift_control_client::{ClientError, status_class};

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
        CliError::Internal(_) => 7,
        CliError::FailedTask(_) => 8,
        CliError::Client(client) => match status_class(client).map(|status| status.as_u16()) {
            Some(401 | 403) => 3,
            Some(409) => 4,
            Some(429 | 502 | 503 | 504) | None if client.retryable() => 5,
            Some(404) => 6,
            _ => 7,
        },
    }
}

#[cfg(test)]
mod tests {
    use milkdrift_control_client::ClientError;
    use milkdrift_control_protocol::{ErrorCode, ErrorEnvelope};

    use super::{CliError, exit_code};

    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(exit_code(&CliError::Invalid("fixture".to_owned())), 2);
        assert_eq!(exit_code(&CliError::FailedTask("fixture".to_owned())), 8);
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
}
