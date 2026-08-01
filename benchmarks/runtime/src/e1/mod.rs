//! Public application-runtime lifecycle measurements.

mod real;
mod startup;

use application_runtime::{ApplicationError, ApplicationRuntime};

use crate::error::{BenchmarkError, BenchmarkResult};

const CLEANUP_SHUTDOWN_ATTEMPTS: u32 = 3;

pub(crate) use real::{
    REAL_GENERATION_TOKEN_COUNT, REAL_POST_FIRST_TOKEN_WINDOW, REAL_PRODUCT_REPOSITORY,
    REAL_PRODUCT_REVISION, RealCycles, run_real_cycles,
};
pub(crate) use startup::run_startup_cycles;

fn shutdown_for_cleanup(runtime: &mut ApplicationRuntime) -> BenchmarkResult {
    let mut last_timeout = String::new();
    for _ in 0..CLEANUP_SHUTDOWN_ATTEMPTS {
        match runtime.shutdown() {
            Ok(()) => return Ok(()),
            Err(error @ ApplicationError::ShutdownTimeout(_)) => {
                last_timeout = error.to_string();
            }
            Err(error) => {
                return Err(BenchmarkError::new(format!(
                    "ApplicationRuntime cleanup shutdown failed terminally: {error}"
                )));
            }
        }
    }
    Err(BenchmarkError::new(format!(
        "ApplicationRuntime cleanup shutdown remained retryable after {CLEANUP_SHUTDOWN_ATTEMPTS} attempts: {last_timeout}"
    )))
}
