//! Error vocabulary for the benchmark harness.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// One actionable benchmark configuration, execution, or validation failure.
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

pub(crate) type BenchmarkResult<T = ()> = Result<T, BenchmarkError>;
