//! Error vocabulary for the benchmark observer.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// One actionable benchmark configuration, execution, validation, or cleanup failure.
#[derive(Debug)]
pub struct BenchmarkError {
    message: String,
}

impl BenchmarkError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub(crate) fn with_cleanup(self, cleanup: Result<(), Self>) -> Self {
        match cleanup {
            Ok(()) => self,
            Err(cleanup_error) => Self::new(format!(
                "{}; cleanup also failed: {}",
                self.message, cleanup_error.message
            )),
        }
    }
}

impl Display for BenchmarkError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for BenchmarkError {}

/// Result type shared by the normal runner and Criterion support.
#[doc(hidden)]
pub type BenchmarkResult<T = ()> = Result<T, BenchmarkError>;

#[cfg(test)]
mod tests {
    use super::BenchmarkError;

    #[test]
    fn cleanup_failure_is_appended_without_replacing_the_primary_error() {
        let error = BenchmarkError::new("primary scenario failed")
            .with_cleanup(Err(BenchmarkError::new("bounded cleanup failed")));
        assert_eq!(
            error.to_string(),
            "primary scenario failed; cleanup also failed: bounded cleanup failed"
        );
    }
}
