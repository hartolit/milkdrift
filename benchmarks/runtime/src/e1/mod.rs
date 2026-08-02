//! Download-free public `ApplicationRuntime` lifecycle observation and cleanup policy.

mod lifecycle;

use application_runtime::{ApplicationError, ApplicationRuntime};

use crate::error::{BenchmarkError, BenchmarkResult};

const CLEANUP_SHUTDOWN_ATTEMPTS: u32 = 3;

pub(crate) use lifecycle::run_lifecycle_cycles;

#[derive(Debug, PartialEq, Eq)]
enum CleanupShutdownFailure {
    NonRetryable(ApplicationError),
    RetryExhausted {
        attempts: u32,
        last: ApplicationError,
    },
}

const fn is_retryable_shutdown_error(error: &ApplicationError) -> bool {
    matches!(
        error,
        ApplicationError::ShutdownTimeout(_)
            | ApplicationError::RuntimeBusy
            | ApplicationError::HubBusy
    )
}

fn retry_cleanup_shutdown(
    mut shutdown: impl FnMut() -> Result<(), ApplicationError>,
) -> Result<(), CleanupShutdownFailure> {
    for attempt in 1..=CLEANUP_SHUTDOWN_ATTEMPTS {
        match shutdown() {
            Ok(()) => return Ok(()),
            Err(error) if !is_retryable_shutdown_error(&error) => {
                return Err(CleanupShutdownFailure::NonRetryable(error));
            }
            Err(error) if attempt == CLEANUP_SHUTDOWN_ATTEMPTS => {
                return Err(CleanupShutdownFailure::RetryExhausted {
                    attempts: attempt,
                    last: error,
                });
            }
            Err(_) => {}
        }
    }
    unreachable!("the positive cleanup-attempt bound always returns from the loop")
}

fn shutdown_for_cleanup(runtime: &mut ApplicationRuntime) -> BenchmarkResult {
    retry_cleanup_shutdown(|| runtime.shutdown()).map_err(|failure| match failure {
        CleanupShutdownFailure::NonRetryable(error) => BenchmarkError::new(format!(
            "ApplicationRuntime cleanup shutdown returned a non-retryable error: {error}"
        )),
        CleanupShutdownFailure::RetryExhausted { attempts, last } => BenchmarkError::new(format!(
            "ApplicationRuntime cleanup shutdown remained retryable after {attempts} attempts: {last}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use application_runtime::{ApplicationError, ApplicationWorker};

    use super::{
        CLEANUP_SHUTDOWN_ATTEMPTS, CleanupShutdownFailure, is_retryable_shutdown_error,
        retry_cleanup_shutdown,
    };

    #[test]
    fn cleanup_retry_classifier_covers_every_bounded_submission_and_wait_state() {
        for error in [
            ApplicationError::ShutdownTimeout(ApplicationWorker::Inference),
            ApplicationError::ShutdownTimeout(ApplicationWorker::Hub),
            ApplicationError::RuntimeBusy,
            ApplicationError::HubBusy,
        ] {
            assert!(is_retryable_shutdown_error(&error), "{error:?}");
        }
        assert!(!is_retryable_shutdown_error(
            &ApplicationError::RuntimeDisconnected
        ));
    }

    #[test]
    fn cleanup_retries_are_finite_and_preserve_the_last_typed_error() {
        let mut calls = 0;
        let result = retry_cleanup_shutdown(|| {
            calls += 1;
            Err(ApplicationError::HubBusy)
        });
        assert_eq!(calls, CLEANUP_SHUTDOWN_ATTEMPTS);
        assert_eq!(
            result,
            Err(CleanupShutdownFailure::RetryExhausted {
                attempts: CLEANUP_SHUTDOWN_ATTEMPTS,
                last: ApplicationError::HubBusy,
            })
        );
    }

    #[test]
    fn cleanup_can_recover_across_each_retryable_queue_or_wait_failure() {
        let mut outcomes = [
            Err(ApplicationError::RuntimeBusy),
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference,
            )),
            Ok(()),
        ]
        .into_iter();
        assert_eq!(
            retry_cleanup_shutdown(|| outcomes.next().unwrap_or(Ok(()))),
            Ok(())
        );
    }

    #[test]
    fn cleanup_stops_immediately_on_a_terminal_disconnection() {
        let mut calls = 0;
        let result = retry_cleanup_shutdown(|| {
            calls += 1;
            Err(ApplicationError::RuntimeDisconnected)
        });
        assert_eq!(calls, 1);
        assert_eq!(
            result,
            Err(CleanupShutdownFailure::NonRetryable(
                ApplicationError::RuntimeDisconnected
            ))
        );
    }
}
