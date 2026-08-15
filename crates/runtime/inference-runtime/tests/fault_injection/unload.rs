use super::support::*;

#[test]
fn ordinary_unload_releases_all_runtime_ownership_and_accounting() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    let receipt = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::CancelActive,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(receipt.cancelled_requests, 1);
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn normal_model_unload_failure_uses_the_bounded_cleanup_state_machine() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_MODEL_CLEANUP, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;

    let initial = runtime.unload_model(
        loaded.handle,
        UnloadPolicy::RejectIfBusy,
        MonotonicMillis::new(0),
    );
    assert!(matches!(
        initial,
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_operation == RuntimeOperation::ModelUnload
                && state.failure.primary_failure == FailureClass::Completion
                && state.failure.cleanup_failure == FailureClass::Synchronization
    ));
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state) if state.attempts == 2
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Exhausted(state)
            if state.attempts == 3
                && state.ownership == RetainedOwnership::Exact(model_footprint())
                && matches!(
                    state.resource,
                    CleanupResource::Model { handle } if handle == loaded.handle
                )
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.model_cleanups.get(), 3);

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.exhausted_cleanup_models, 1);
    assert!(matches!(
        runtime.unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(1),
        ),
        Err(RuntimeError::CleanupRetryExhausted(state))
            if state.attempts == 3 && state.exhausted()
    ));
    Ok(())
}
