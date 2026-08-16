use super::support::*;

#[test]
fn repeated_sequence_cleanup_failure_exhausts_without_releasing_accounting() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime =
        runtime_with_cleanup_attempts(Faults::FAIL_SEQUENCE_DESTRUCTION, Rc::clone(&counts), 3);
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    let initial = runtime.cancel_request(RequestId::new(10), CancellationReason::UserRequested);
    assert!(matches!(
        initial,
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == FailureClass::Cancellation
                && state.failure.cleanup_failure == FailureClass::Sequence
    ));
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(
        start(&mut runtime, loaded.handle, 11, 101),
        Err(RuntimeError::ModelDegraded(loaded.handle.id))
    );
    assert_eq!(counts.sequence_creations.get(), 1);

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state)
            if state.attempts == 2 && !state.exhausted()
    ));
    let exhausted = runtime.poll_cleanup().map_err(debug_error)?;
    assert!(matches!(
        exhausted,
        CleanupPoll::Exhausted(state)
            if state.attempts == 3
                && state.exhausted()
                && matches!(
                    state.resource,
                    CleanupResource::Sequence {
                        handle,
                        request_id,
                        sequence_id,
                    } if handle == loaded.handle
                        && request_id == RequestId::new(10)
                        && sequence_id == SequenceId::new(100)
                )
    ));
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );
    assert_eq!(counts.sequence_destructions.get(), 3);

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 1);
    assert_eq!(snapshot.exhausted_cleanup_sequences, 1);
    assert_eq!(snapshot.reserved_footprint, checked_total_footprint());
    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::TerminalCleanupRetention { first: state, summary })
            if state.attempts == 3
                && state.exhausted()
                && summary.sequences == 1
                && summary.failed_preparations == 0
                && summary.verified_models == 1
                && summary.incompatible_models == 0
    ));
    assert_eq!(counts.sequence_destructions.get(), 3);
    Ok(())
}

#[test]
fn cleanup_selection_rotates_across_classes_and_owners() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime_with_resources(
        Faults::default(),
        Rc::clone(&counts),
        2,
        6,
        4,
        MemoryBudget {
            host_bytes: 10_000,
            device_bytes: 10_000,
        },
    );

    let sequence_model = load_model_id(
        &mut runtime,
        1,
        source_with_faults(Faults::FAIL_SEQUENCE_DESTRUCTION),
    )
    .map_err(debug_error)?;
    start(&mut runtime, sequence_model.handle, 10, 100).map_err(debug_error)?;
    assert!(matches!(
        runtime.cancel_request(RequestId::new(10), CancellationReason::UserRequested),
        Err(RuntimeError::CleanupFailed(_))
    ));
    let second_sequence_model = load_model_id(
        &mut runtime,
        5,
        source_with_faults(Faults::FAIL_SEQUENCE_DESTRUCTION),
    )
    .map_err(debug_error)?;
    start(&mut runtime, second_sequence_model.handle, 11, 101).map_err(debug_error)?;
    assert!(matches!(
        runtime.cancel_request(RequestId::new(11), CancellationReason::UserRequested),
        Err(RuntimeError::CleanupFailed(_))
    ));

    assert!(matches!(
        load_model_id(
            &mut runtime,
            2,
            source_with_faults(Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP),),
        ),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        load_model_id(
            &mut runtime,
            6,
            source_with_faults(Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP),),
        ),
        Err(RuntimeError::CleanupFailed(_))
    ));

    let verified = load_model_id(
        &mut runtime,
        3,
        source_with_faults(Faults::FAIL_MODEL_CLEANUP),
    )
    .map_err(debug_error)?;
    assert!(matches!(
        runtime.unload_model(
            verified.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        ),
        Err(RuntimeError::CleanupFailed(_))
    ));

    assert!(matches!(
        load_model_id(
            &mut runtime,
            4,
            source_with_faults(Faults::WRONG_EXECUTION_SCALAR.union(Faults::FAIL_MODEL_CLEANUP),),
        ),
        Err(RuntimeError::CleanupFailed(_))
    ));

    let expected = [
        CleanupResource::Sequence {
            handle: sequence_model.handle,
            request_id: RequestId::new(10),
            sequence_id: SequenceId::new(100),
        },
        CleanupResource::FailedLoad {
            handle: expected_handle(2),
        },
        CleanupResource::Model {
            handle: verified.handle,
        },
        CleanupResource::Sequence {
            handle: second_sequence_model.handle,
            request_id: RequestId::new(11),
            sequence_id: SequenceId::new(101),
        },
        CleanupResource::FailedLoad {
            handle: expected_handle(6),
        },
        CleanupResource::IncompatibleModel {
            handle: expected_handle(4),
        },
    ];
    for resource in expected {
        assert!(matches!(
            runtime.poll_cleanup().map_err(debug_error)?,
            CleanupPoll::Exhausted(state)
                if state.resource == resource && state.attempts == 2
        ));
    }
    assert_eq!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Idle
    );

    assert!(matches!(
        runtime.shutdown(),
        Err(RuntimeError::TerminalCleanupRetention { summary, .. })
            if summary.failed_preparations == 2
                && summary.verified_models == 3
                && summary.incompatible_models == 1
                && summary.sequences == 2
    ));
    drop(runtime);
    assert_eq!(counts.retained_prepared_drops.get(), 0);
    assert_eq!(counts.model_drops_while_owned.get(), 0);
    Ok(())
}
