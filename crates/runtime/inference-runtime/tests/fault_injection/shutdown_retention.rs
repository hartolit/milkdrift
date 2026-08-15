use super::support::*;

#[test]
fn incompatible_complete_model_matrix_exhausts_to_process_retention() {
    for (report_fault, reported_footprint, conservative_footprint) in complete_report_cases() {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime_with_resources(
            Faults::default(),
            Rc::clone(&counts),
            3,
            1,
            1,
            MemoryBudget {
                host_bytes: 10_000,
                device_bytes: 10_000,
            },
        );
        let faults = report_fault.union(Faults::FAIL_MODEL_CLEANUP);
        assert!(matches!(
            load_model_id(&mut runtime, 1, source_with_faults(faults)),
            Err(RuntimeError::CleanupFailed(_))
        ));
        let terminal = runtime.shutdown();
        assert!(matches!(
            terminal,
            Err(RuntimeError::TerminalCleanupRetention { first, summary })
                if first.attempts == 3
                    && first.exhausted()
                    && first.resource
                        == CleanupResource::IncompatibleModel {
                            handle: expected_handle(1),
                        }
                    && first.ownership
                        == RetainedOwnership::Unverified {
                            accepted_footprint: loading_peak_footprint(),
                            reported_footprint,
                            conservative_footprint,
                        }
                    && summary.failed_preparations == 0
                    && summary.verified_models == 0
                    && summary.incompatible_models == 1
                    && summary.sequences == 0
                    && summary.unverified_conservative_footprint == conservative_footprint
        ));
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.reserved_footprint, MemoryFootprint::default());
        assert!(snapshot.admission_blocked);
        assert_eq!(snapshot.exhausted_cleanup_models, 1);
        assert_eq!(counts.model_drops_while_owned.get(), 0);
        drop(runtime);
        assert_eq!(counts.model_drops_while_owned.get(), 0);
    }
}

#[test]
fn shutdown_releases_retryable_failed_load_without_counting_an_unloaded_model() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    let receipt = runtime.shutdown().map_err(debug_error)?;
    assert_eq!(receipt.unloaded_models, 0);
    assert_eq!(receipt.cancelled_requests, 0);
    assert_eq!(counts.failed_load_cleanups.get(), 2);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn failed_load_cleanup_exhaustion_survives_shutdown_accounted_and_owned() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP);
    let mut runtime = runtime_with_cleanup_attempts(faults, Rc::clone(&counts), 3);

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::TerminalCleanupRetention { first: state, summary })
            if state.attempts == 3
                && state.exhausted()
                && state.resource
                    == (CleanupResource::FailedLoad {
                        handle: expected_handle(1),
                    })
                && state.ownership == RetainedOwnership::Exact(loading_peak_footprint())
                && state.failure.primary_operation == RuntimeOperation::ModelLoad
                && state.failure.primary_failure == FailureClass::Load
                && state.failure.primary_detail
                    == FailureDetail::Load(failed_load_error())
                && state.failure.cleanup_operation == RuntimeOperation::FailedLoadCleanup
                && summary.failed_preparations == 1
                && summary.verified_models == 0
                && summary.incompatible_models == 0
                && summary.sequences == 0
    ));
    assert_eq!(counts.failed_load_cleanups.get(), 3);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 0);
    assert_eq!(
        counts.retained_partial_load_bytes.get(),
        loading_peak_host_bytes()
    );
    let snapshot = runtime.snapshot();
    assert!(snapshot.shutting_down);
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, loading_peak_footprint());
}

#[test]
fn shutdown_reports_model_cleanup_exhaustion_with_shutdown_as_primary() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_MODEL_CLEANUP, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;

    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::TerminalCleanupRetention { first: state, summary })
            if state.attempts == 3
                && state.failure.primary_operation == RuntimeOperation::Shutdown
                && state.failure.primary_failure == FailureClass::Shutdown
                && state.ownership == RetainedOwnership::Exact(model_footprint())
                && matches!(
                    state.resource,
                    CleanupResource::Model { handle } if handle == loaded.handle
                )
                && summary.verified_models == 1
                && summary.failed_preparations == 0
                && summary.incompatible_models == 0
                && summary.sequences == 0
    ));
    assert_eq!(counts.model_cleanups.get(), 3);
    let snapshot = runtime.snapshot();
    assert!(snapshot.shutting_down);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());
    Ok(())
}
