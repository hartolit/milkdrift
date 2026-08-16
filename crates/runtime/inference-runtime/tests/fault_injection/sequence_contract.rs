use super::support::*;

#[test]
fn wrong_sequence_identity_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_SEQUENCE_ID)
}

#[test]
fn wrong_sequence_capacity_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_SEQUENCE_CAPACITY)
}

#[test]
fn nonempty_sequence_state_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_INITIAL_SEQUENCE_STATE)
}

#[test]
fn nonzero_sequence_position_is_destroyed_without_registry_mutation() -> TestResult {
    assert_sequence_contract_rollback(Faults::WRONG_INITIAL_SEQUENCE_POSITION)
}

#[test]
fn failed_sequence_rollback_is_reported_without_registry_mutation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_SEQUENCE_ID.union(Faults::FAIL_SEQUENCE_DESTRUCTION);
    let mut runtime = runtime(faults, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;

    let result = start(&mut runtime, loaded.handle, 10, 100);
    assert!(matches!(
        result,
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == inference_runtime::FailureClass::BackendContract
                && state.failure.cleanup_failure == inference_runtime::FailureClass::Sequence
    ));
    assert_eq!(counts.sequence_creations.get(), 1);
    assert_eq!(counts.sequence_destructions.get(), 1);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(snapshot.pending_cleanup_sequences, 1);
    assert_eq!(snapshot.reserved_footprint, model_footprint());
    assert!(snapshot.admission_blocked);
    assert!(matches!(
        runtime.request_cleanup_state(RequestId::new(10)),
        Some(state)
            if state.ownership
                == RetainedOwnership::Unverified {
                    accepted_footprint: sequence_footprint(),
                    reported_footprint: sequence_footprint(),
                    conservative_footprint: ConservativeFootprint::Known(sequence_footprint()),
                }
    ));
    assert!(
        runtime
            .model_snapshots()
            .first()
            .is_some_and(|model| model.degraded)
    );
    Ok(())
}

#[test]
fn sequence_report_mismatch_matrix_rolls_back_without_publication() -> TestResult {
    for fault in [
        Faults::UNDERREPORTED_SEQUENCE_REPORT,
        Faults::OVERREPORTED_SEQUENCE_REPORT,
        Faults::RECLASSIFIED_SEQUENCE_REPORT,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(fault, Rc::clone(&counts));
        let loaded = load(&mut runtime).map_err(debug_error)?;

        assert_eq!(
            start(&mut runtime, loaded.handle, 10, 100),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.sequence_creations.get(), 1);
        assert_eq!(counts.sequence_destructions.get(), 1);
        assert_only_model_reserved(&runtime);
    }
    Ok(())
}

#[test]
fn mismatched_sequence_reports_with_failed_cleanup_are_unverified() -> TestResult {
    for (report_fault, reported_footprint) in [
        (Faults::UNDERREPORTED_SEQUENCE_REPORT, footprint(0, 0, 4, 0)),
        (Faults::OVERREPORTED_SEQUENCE_REPORT, footprint(0, 0, 16, 0)),
        (Faults::RECLASSIFIED_SEQUENCE_REPORT, footprint(0, 0, 0, 8)),
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let faults = report_fault.union(Faults::FAIL_SEQUENCE_DESTRUCTION);
        let mut runtime = runtime(faults, Rc::clone(&counts));
        let loaded = load(&mut runtime).map_err(debug_error)?;

        assert!(matches!(
            start(&mut runtime, loaded.handle, 10, 100),
            Err(RuntimeError::CleanupFailed(state))
                if state.failure.primary_failure == FailureClass::BackendContract
                    && matches!(
                        state.ownership,
                        RetainedOwnership::Unverified {
                            accepted_footprint,
                            reported_footprint: actual_report,
                            conservative_footprint: ConservativeFootprint::Known(_),
                        } if accepted_footprint == sequence_footprint()
                            && actual_report == reported_footprint
                    )
        ));
        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.reserved_footprint, model_footprint());
        assert!(snapshot.admission_blocked);
        assert_eq!(snapshot.pending_cleanup_sequences, 1);
        assert_eq!(counts.sequence_creations.get(), 1);
        assert_eq!(counts.sequence_destructions.get(), 1);
    }
    Ok(())
}

#[test]
fn sequence_plan_mutation_after_prefill_is_a_contract_violation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(
        Faults::MUTATE_SEQUENCE_REPORT_AFTER_PREFILL,
        Rc::clone(&counts),
    );
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    let mut no_logits = [];
    assert_eq!(
        runtime.prefill(
            RequestId::new(10),
            &[domain_contracts::TokenId::new(1)],
            false,
            &mut no_logits,
        ),
        Err(RuntimeError::BackendContractViolation)
    );
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_only_model_reserved(&runtime);
    Ok(())
}

#[test]
fn sequence_report_mutation_during_failed_cleanup_extends_unverified_evidence() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults =
        Faults::FAIL_SEQUENCE_DESTRUCTION.union(Faults::MUTATE_SEQUENCE_REPORT_ON_CLEANUP_FAILURE);
    let mut runtime = runtime(faults, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    assert!(matches!(
        runtime.cancel_request(RequestId::new(10), CancellationReason::UserRequested),
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == FailureClass::BackendContract
                && matches!(
                    state.ownership,
                    RetainedOwnership::Unverified {
                        accepted_footprint,
                        reported_footprint,
                        conservative_footprint: ConservativeFootprint::Known(conservative),
                    } if accepted_footprint == sequence_footprint()
                        && reported_footprint.host_working_bytes() == ByteCount::from_u64(16)
                        && conservative.host_working_bytes() == ByteCount::from_u64(16)
                )
    ));
    assert_eq!(runtime.snapshot().reserved_footprint, model_footprint());
    assert!(runtime.snapshot().admission_blocked);
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state)
            if matches!(state.ownership, RetainedOwnership::Unverified { .. })
    ));
    assert_eq!(counts.sequence_destructions.get(), 2);
    Ok(())
}

#[test]
fn sequence_identity_or_capacity_mutation_during_failed_cleanup_is_unverified() -> TestResult {
    for contradiction in [
        Faults::MUTATE_SEQUENCE_ID_ON_CLEANUP_FAILURE,
        Faults::MUTATE_SEQUENCE_CAPACITY_ON_CLEANUP_FAILURE,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let faults = Faults::FAIL_SEQUENCE_DESTRUCTION.union(contradiction);
        let mut runtime = runtime(faults, Rc::clone(&counts));
        let loaded = load(&mut runtime).map_err(debug_error)?;
        start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

        assert!(matches!(
            runtime.cancel_request(RequestId::new(10), CancellationReason::UserRequested),
            Err(RuntimeError::CleanupFailed(state))
                if state.failure.primary_failure == FailureClass::BackendContract
                    && state.ownership
                        == RetainedOwnership::Unverified {
                            accepted_footprint: sequence_footprint(),
                            reported_footprint: sequence_footprint(),
                            conservative_footprint: ConservativeFootprint::Known(
                                sequence_footprint(),
                            ),
                        }
        ));
        assert_eq!(runtime.snapshot().reserved_footprint, model_footprint());
        assert!(runtime.snapshot().admission_blocked);
        assert_eq!(counts.sequence_destructions.get(), 1);
    }
    Ok(())
}

#[test]
fn over_advertised_sequence_plan_is_rejected_before_native_creation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::CONTRADICTORY_SEQUENCE_PLAN, Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;

    assert_eq!(
        start(&mut runtime, loaded.handle, 10, 100),
        Err(RuntimeError::BackendContractViolation)
    );
    assert_eq!(counts.sequence_creations.get(), 0);
    assert_eq!(counts.sequence_destructions.get(), 0);
    assert_only_model_reserved(&runtime);
    Ok(())
}

#[test]
fn direct_sequence_configuration_respects_advertised_limits() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    let configurations = [
        SequenceConfiguration::new(
            NonZeroU32::new(17).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        ),
        SequenceConfiguration::new(
            NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(5).unwrap_or(NonZeroU32::MIN),
        ),
        SequenceConfiguration::new(
            NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN),
            NonZeroU32::new(4).unwrap_or(NonZeroU32::MIN),
        ),
    ];

    for (offset, configuration) in configurations.into_iter().enumerate() {
        let offset = u64::try_from(offset).map_err(debug_error)?;
        assert_eq!(
            runtime.start_request(
                loaded.handle,
                RequestId::new(20_u64.saturating_add(offset)),
                SequenceId::new(200_u64.saturating_add(offset)),
                configuration,
            ),
            Err(RuntimeError::Model(ModelError::Unsupported))
        );
    }
    assert_eq!(counts.sequence_creations.get(), 0);
    assert_eq!(counts.sequence_destructions.get(), 0);
    assert_only_model_reserved(&runtime);
    Ok(())
}

#[test]
fn occupied_request_and_sequence_indexes_fail_before_native_creation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));
    let loaded = load(&mut runtime).map_err(debug_error)?;
    start(&mut runtime, loaded.handle, 10, 100).map_err(debug_error)?;

    assert_eq!(
        start(&mut runtime, loaded.handle, 10, 101),
        Err(RuntimeError::RequestAlreadyActive(RequestId::new(10)))
    );
    assert_eq!(
        start(&mut runtime, loaded.handle, 11, 100),
        Err(RuntimeError::SequenceAlreadyActive(SequenceId::new(100)))
    );
    assert_eq!(counts.sequence_creations.get(), 1);
    assert_eq!(counts.sequence_destructions.get(), 0);
    assert_eq!(runtime.snapshot().active_requests, 1);

    runtime
        .cancel_request(RequestId::new(10), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(counts.sequence_destructions.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
    Ok(())
}
