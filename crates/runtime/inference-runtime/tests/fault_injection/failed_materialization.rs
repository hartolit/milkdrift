use super::support::*;

#[test]
fn failed_load_immediate_cleanup_returns_exact_primary_and_restores_accounting() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::FAIL_LOAD, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::Load(LoadError::Backend(failure)))
            if failure.failure.code == 5
                && failure.context
                    == Some(domain_contracts::LoadFailureContext::tensor(
                        LoadFailureStage::DeviceTransfer,
                        FAILED_LOAD_LOCATION,
                    ))
    ));
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.failed_load_cleanups.get(), 1);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
}

#[test]
fn failed_load_cleanup_failure_retains_owner_and_full_loading_peak() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_operation == RuntimeOperation::ModelLoad
                && state.failure.primary_failure == FailureClass::Load
                && state.failure.cleanup_operation == RuntimeOperation::FailedLoadCleanup
                && state.failure.cleanup_failure == FailureClass::Synchronization
                && state.failure.primary_detail == FailureDetail::Load(failed_load_error())
                && state.resource
                    == CleanupResource::FailedLoad {
                        handle: expected_handle(1),
                    }
                && state.ownership == RetainedOwnership::Exact(loading_peak_footprint())
    ));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, loading_peak_footprint());
    assert!(runtime.model_snapshots().is_empty());
    assert!(matches!(
        runtime.model_cleanup_state(ModelId::new(1)),
        Some(state)
            if state.attempts == 1
                && !state.exhausted()
                && state.resource
                    == (CleanupResource::FailedLoad {
                        handle: expected_handle(1),
                    })
                && state.ownership == RetainedOwnership::Exact(loading_peak_footprint())
    ));
    assert_eq!(counts.failed_load_cleanups.get(), 1);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 0);
    assert_eq!(
        counts.retained_partial_load_bytes.get(),
        loading_peak_host_bytes()
    );
    assert_eq!(
        load(&mut runtime),
        Err(RuntimeError::ModelAlreadyLoaded(ModelId::new(1)))
    );
}

#[test]
fn failed_load_cleanup_retry_releases_owner_and_accounting_once() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state)
            if state.attempts == 2
                && state.resource
                    == (CleanupResource::FailedLoad {
                        handle: expected_handle(1),
                    })
                && state.ownership == RetainedOwnership::Released
                && state.failure.primary_detail == FailureDetail::Load(failed_load_error())
                && !state.exhausted()
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.failed_load_cleanups.get(), 2);
    assert_eq!(counts.successful_failed_load_cleanups.get(), 1);
    assert_eq!(counts.retained_partial_load_bytes.get(), 0);
    assert_empty(&runtime);
    Ok(())
}
