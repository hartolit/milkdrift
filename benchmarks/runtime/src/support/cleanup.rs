//! Shared retained-cleanup diagnostics and bounded application shutdown cleanup.

use application_runtime::{
    ApplicationError, ApplicationFailure, ApplicationModelCleanupDisposition,
    ApplicationRetainedModelResource, ApplicationRuntime, GenerationTerminalKind,
};
use domain_contracts::RequestId;

use crate::error::{BenchmarkError, BenchmarkResult};

const CLEANUP_SHUTDOWN_ATTEMPTS: u32 = 3;

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
    let mut last = match shutdown() {
        Ok(()) => return Ok(()),
        Err(error) if !is_retryable_shutdown_error(&error) => {
            return Err(CleanupShutdownFailure::NonRetryable(error));
        }
        Err(error) => error,
    };

    for _ in 2..=CLEANUP_SHUTDOWN_ATTEMPTS {
        match shutdown() {
            Ok(()) => return Ok(()),
            Err(error) if !is_retryable_shutdown_error(&error) => {
                return Err(CleanupShutdownFailure::NonRetryable(error));
            }
            Err(error) => last = error,
        }
    }

    Err(CleanupShutdownFailure::RetryExhausted {
        attempts: CLEANUP_SHUTDOWN_ATTEMPTS,
        last,
    })
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

pub(crate) fn cleanup_runtime_after_failure(
    mut runtime: ApplicationRuntime,
    primary: BenchmarkError,
) -> BenchmarkError {
    match shutdown_for_cleanup(&mut runtime) {
        Ok(()) => primary,
        Err(cleanup_error) => {
            let cleanup_error = BenchmarkError::new(format!(
                "{cleanup_error}; failed ApplicationRuntime owner retained until process exit"
            ));
            let combined = primary.with_cleanup(Err(cleanup_error));
            std::mem::forget(runtime);
            combined
        }
    }
}

pub(crate) fn retained_model_cleanup_error(
    stage: &'static str,
    resource: ApplicationRetainedModelResource,
    disposition: ApplicationModelCleanupDisposition,
    primary: &ApplicationFailure,
    cleanup: Option<&ApplicationFailure>,
) -> BenchmarkError {
    BenchmarkError::new(format!(
        "{stage} retained model ownership for {resource:?}: primary operation failure={primary}; cleanup disposition={disposition:?}; cleanup failure={cleanup:?}"
    ))
}

pub(crate) fn generation_cleanup_error(
    stage: &'static str,
    expected_request: Option<RequestId>,
    request_id: RequestId,
    exhausted: bool,
    failure: &ApplicationFailure,
) -> BenchmarkError {
    if expected_request.is_some_and(|expected| expected != request_id) {
        return BenchmarkError::new(format!(
            "{stage} observed generation cleanup for request {}, expected {}",
            request_id.get(),
            expected_request.map_or(0, RequestId::get)
        ));
    }
    let disposition = if exhausted { "exhausted" } else { "pending" };
    BenchmarkError::new(format!(
        "{stage} preserved the operation result while generation cleanup remained {disposition} for request {}: {failure}",
        request_id.get()
    ))
}

pub(crate) fn generation_output_cleanup_error(
    stage: &'static str,
    request_id: RequestId,
    exhausted: bool,
    primary_terminal: Option<GenerationTerminalKind>,
) -> BenchmarkError {
    let disposition = if exhausted { "exhausted" } else { "pending" };
    BenchmarkError::new(format!(
        "{stage} preserved prior terminal output {primary_terminal:?} while output cleanup remained {disposition} for request {}",
        request_id.get()
    ))
}

#[cfg(test)]
mod tests {
    use application_runtime::{
        ApplicationError, ApplicationFailure, ApplicationFailureKind,
        ApplicationModelCleanupDisposition, ApplicationRetainedModelResource, ApplicationWorker,
    };
    use domain_contracts::{ModelGeneration, ModelHandle, ModelId};

    use super::{
        CLEANUP_SHUTDOWN_ATTEMPTS, CleanupShutdownFailure, retained_model_cleanup_error,
        retry_cleanup_shutdown,
    };

    #[test]
    fn retained_cleanup_diagnostics_preserve_primary_failure_for_pending_and_exhausted_states() {
        let resource = ApplicationRetainedModelResource::FailedLoad {
            handle: ModelHandle::new(ModelId::new(7), ModelGeneration::new(3)),
        };
        let primary = ApplicationFailure::new(ApplicationFailureKind::ModelLoad, "digest mismatch");
        let cleanup = ApplicationFailure::new(
            ApplicationFailureKind::RetainedCleanup,
            "device synchronization failed",
        );
        for disposition in [
            ApplicationModelCleanupDisposition::Pending,
            ApplicationModelCleanupDisposition::LowerExhausted {
                attempts: 3,
                maximum_attempts: 3,
            },
        ] {
            let error = retained_model_cleanup_error(
                "model load",
                resource,
                disposition,
                &primary,
                Some(&cleanup),
            );
            assert!(error.to_string().contains("digest mismatch"));
            assert!(error.to_string().contains("device synchronization failed"));
            assert!(error.to_string().contains(&format!("{disposition:?}")));
        }
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
            retry_cleanup_shutdown(|| match outcomes.next() {
                Some(outcome) => outcome,
                None => Ok(()),
            }),
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
