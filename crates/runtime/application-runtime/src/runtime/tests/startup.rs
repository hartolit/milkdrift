use std::cell::Cell;
use std::time::Duration;

use super::support::*;
use crate::runtime::startup::{reap_startup_cleanup_quarantine, startup_cleanup_quarantine_state};
use crate::support::MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
use crate::{
    ApplicationConfigurationField, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationRuntime, ApplicationRuntimeConfiguration, ApplicationWorker,
};

#[test]
fn shutdown_and_join_deadline_boundaries_are_validated_before_worker_start() -> TestResult {
    let mut maximum = ApplicationRuntimeConfiguration::desktop("unused.redb");
    default_test_configuration(&mut maximum);
    maximum.timing.runtime_shutdown_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    maximum.timing.runtime_join_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    maximum.timing.hub_shutdown_timeout = MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT;
    crate::support::validate_configuration(&maximum).map_err(application_error)?;

    assert_startup_deadline_duration_rejected(Duration::ZERO)?;
    assert_startup_deadline_duration_rejected(
        MAXIMUM_SHUTDOWN_OR_JOIN_TIMEOUT + Duration::from_nanos(1),
    )?;
    assert_startup_deadline_duration_rejected(Duration::MAX)
}

#[test]
fn forced_hub_start_failure_stops_and_joins_started_inference_worker() -> TestResult {
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    let primary = ApplicationError::Failure(ApplicationFailure::new(
        ApplicationFailureKind::Hub,
        "forced Hub startup failure",
    ));

    let start_result =
        ApplicationRuntime::start_transaction(configuration, |_| Err(primary.clone()));
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(failure.primary, primary);
            assert_eq!(failure.inference_rollback, Some(Ok(())));
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err("forced Hub startup failure unexpectedly succeeded".to_owned())
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

#[test]
fn failed_startup_rollback_quarantines_and_later_reaps_inference_worker() -> TestResult {
    assert_eq!(startup_cleanup_quarantine_state(), (0, 0));
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    let primary = ApplicationError::Failure(ApplicationFailure::new(
        ApplicationFailureKind::Hub,
        "forced Hub startup failure with rollback timeout",
    ));
    let rollback_failure = ApplicationError::ShutdownTimeout(ApplicationWorker::Inference);

    let start_result = ApplicationRuntime::start_transaction_with_rollback(
        configuration,
        |_| Err(primary.clone()),
        |_local, _timing| {
            Err(ApplicationError::ShutdownTimeout(
                ApplicationWorker::Inference,
            ))
        },
        crate::local::probe_application_device,
    );
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(failure.primary, primary);
            assert_eq!(failure.inference_rollback, Some(Err(rollback_failure)));
            assert_eq!(startup_cleanup_quarantine_state(), (1, 1));

            let reap_result = reap_startup_cleanup_quarantine()
                .ok_or_else(|| "startup cleanup quarantine was unexpectedly empty".to_owned())?;
            reap_result.map_err(application_error)?;
            assert_eq!(startup_cleanup_quarantine_state(), (0, 0));
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err("forced Hub startup rollback failure unexpectedly succeeded".to_owned())
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}

fn assert_startup_deadline_duration_rejected(duration: Duration) -> TestResult {
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::RuntimeShutdownTimeout,
        |configuration| configuration.timing.runtime_shutdown_timeout = duration,
    )?;
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::RuntimeJoinTimeout,
        |configuration| configuration.timing.runtime_join_timeout = duration,
    )?;
    assert_startup_deadline_rejected(
        ApplicationConfigurationField::HubShutdownTimeout,
        |configuration| configuration.timing.hub_shutdown_timeout = duration,
    )
}

fn assert_startup_deadline_rejected<F>(
    field: ApplicationConfigurationField,
    configure: F,
) -> TestResult
where
    F: FnOnce(&mut ApplicationRuntimeConfiguration),
{
    let database_path = unique_database_path();
    let mut configuration = ApplicationRuntimeConfiguration::desktop(&database_path);
    default_test_configuration(&mut configuration);
    configure(&mut configuration);
    let hub_started = Cell::new(false);

    let start_result = ApplicationRuntime::start_transaction(configuration, |_| {
        hub_started.set(true);
        Err(ApplicationError::HubDisconnected)
    });
    let test_result = match start_result {
        Err(failure) => {
            assert_eq!(
                failure.primary,
                ApplicationError::InvalidConfiguration(field)
            );
            assert!(failure.inference_rollback.is_none());
            assert!(!hub_started.get());
            Ok(())
        }
        Ok(mut runtime) => {
            runtime.shutdown().map_err(application_error)?;
            Err(format!(
                "overflowing startup deadline was accepted for {field:?}"
            ))
        }
    };

    let cleanup_result = remove_database(&database_path);
    test_result.and(cleanup_result)
}
