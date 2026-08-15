use super::support::*;

#[test]
fn wrong_model_handle_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::WRONG_MODEL_HANDLE, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn wrong_device_id_after_native_load_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_DEVICE_ID);
}

#[test]
fn wrong_device_kind_after_native_load_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_DEVICE_KIND);
}

#[test]
fn correct_execution_scalar_wrong_reported_footprint_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_MODEL_FOOTPRINT);
}

#[test]
fn correct_device_wrong_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::WRONG_EXECUTION_SCALAR);
}

#[test]
fn source_scalar_mistaken_for_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_for_source_is_cleaned(
        Faults::SOURCE_SCALAR_AS_EXECUTION_SCALAR,
        BF16_SOURCE_WITH_F32_EXECUTION,
    );
}

#[test]
fn unsupported_actual_execution_scalar_is_cleaned_without_publication() {
    assert_model_admission_mismatch_is_cleaned(Faults::UNSUPPORTED_ACTUAL_EXECUTION_SCALAR);
}

#[test]
fn planned_execution_scalar_is_published_independently_from_source_scalar() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));

    let loaded = load_source(&mut runtime, BF16_SOURCE_WITH_F32_EXECUTION).map_err(debug_error)?;
    assert_eq!(
        loaded
            .descriptor
            .metadata
            .configuration_declared_scalar_type,
        Some(ScalarType::Bf16)
    );
    assert_eq!(loaded.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded.execution_device,
        ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
    );
    assert_eq!(loaded.reserved_footprint, model_footprint());
    let snapshot = runtime
        .model_snapshot(loaded.handle)
        .ok_or_else(|| "loaded model snapshot missing".to_owned())?;
    assert_eq!(
        snapshot
            .descriptor
            .metadata
            .configuration_declared_scalar_type,
        Some(ScalarType::Bf16)
    );
    assert_eq!(snapshot.execution_scalar_type, ScalarType::F32);
    assert_eq!(snapshot.reserved_footprint, model_footprint());

    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn wrong_execution_scalar_cleanup_failure_retains_accounting_until_successful_retry() -> TestResult
{
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_EXECUTION_SCALAR.union(Faults::FAIL_MODEL_CLEANUP_ONCE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_operation == RuntimeOperation::ModelAdmission
                && state.failure.primary_failure == FailureClass::BackendContract
                && state.failure.primary_detail
                    == FailureDetail::Class(FailureClass::BackendContract)
                && state.failure.cleanup_operation == RuntimeOperation::ModelUnload
                && state.failure.cleanup_failure == FailureClass::Synchronization
                && state.resource
                    == CleanupResource::IncompatibleModel {
                        handle: expected_handle(1),
                    }
                && state.ownership
                    == RetainedOwnership::Unverified {
                        accepted_footprint: loading_peak_footprint(),
                        reported_footprint: model_footprint(),
                        conservative_footprint: ConservativeFootprint::Known(
                            loading_peak_footprint(),
                        ),
                    }
    ));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    let retained = runtime.snapshot();
    assert_eq!(retained.loaded_models, 0);
    assert_eq!(retained.pending_cleanup_models, 1);
    assert_eq!(retained.reserved_footprint, MemoryFootprint::default());
    assert_eq!(
        retained.unverified_ownership,
        Some(inference_runtime::UnverifiedOwnershipSummary {
            owners: 1,
            conservative_footprint: ConservativeFootprint::Known(loading_peak_footprint()),
        })
    );
    assert!(retained.admission_blocked);
    assert!(runtime.model_snapshots().is_empty());
    assert!(matches!(
        runtime.model_cleanup_state(ModelId::new(1)),
        Some(state)
            if state.attempts == 1
                && !state.exhausted()
                && state.resource
                    == CleanupResource::IncompatibleModel {
                        handle: expected_handle(1),
                    }
                && matches!(state.ownership, RetainedOwnership::Unverified { .. })
    ));

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state)
            if state.attempts == 2
                && state.ownership == RetainedOwnership::Released
                && !state.exhausted()
    ));
    assert_eq!(counts.model_cleanups.get(), 2);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn mismatched_metadata_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISMATCHED_METADATA, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn mismatched_loaded_descriptor_is_explicitly_cleaned_without_publication() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISMATCHED_DESCRIPTOR, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.model_cleanups.get(), 1);
    assert_empty(&runtime);
}

#[test]
fn multiple_sequences_requires_the_matching_capability() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::MISSING_MULTIPLE_SEQUENCES, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert_eq!(result, Err(RuntimeError::BackendContractViolation));
    assert_eq!(counts.model_loads.get(), 0);
    assert_eq!(counts.model_cleanups.get(), 0);
    assert_empty(&runtime);
}

#[test]
fn descriptor_numeric_fields_must_be_nonzero_and_consistent() {
    for fault in [
        Faults::ZERO_VOCABULARY,
        Faults::ZERO_CONTEXT_LENGTH,
        Faults::ZERO_MAXIMUM_CONTEXT,
        Faults::ZERO_MAXIMUM_SEQUENCES,
        Faults::ZERO_MAXIMUM_PREFILL,
        Faults::CONTEXT_EXCEEDS_METADATA,
        Faults::PREFILL_EXCEEDS_CONTEXT,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(fault, Rc::clone(&counts));

        assert_eq!(
            load(&mut runtime),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.model_loads.get(), 0);
        assert_eq!(counts.model_cleanups.get(), 0);
        assert_empty(&runtime);
    }
}

#[test]
fn device_mismatch_cleanup_failure_preserves_primary_error_ownership_and_accounting() {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_DEVICE_ID.union(Faults::FAIL_MODEL_CLEANUP);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    let result = load(&mut runtime);
    assert!(matches!(
        result,
        Err(RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == inference_runtime::FailureClass::BackendContract
                && state.failure.cleanup_failure == inference_runtime::FailureClass::Synchronization
    ));
    assert_eq!(counts.model_cleanups.get(), 1);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.loaded_models, 0);
    assert_eq!(snapshot.pending_cleanup_models, 1);
    assert_eq!(snapshot.reserved_footprint, MemoryFootprint::default());
    assert!(snapshot.unverified_ownership.is_some());
    assert!(snapshot.admission_blocked);
    assert!(matches!(
        runtime.model_cleanup_state(ModelId::new(1)),
        Some(state)
            if state.resource
                == CleanupResource::IncompatibleModel {
                    handle: expected_handle(1),
                }
                && matches!(state.ownership, RetainedOwnership::Unverified { .. })
    ));
}

#[test]
fn incompatible_complete_model_matrix_retains_unverified_evidence_and_unlocks_on_release()
-> TestResult {
    for (report_fault, reported_footprint, conservative_footprint) in complete_report_cases() {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime_with_resources(
            Faults::default(),
            Rc::clone(&counts),
            3,
            2,
            2,
            MemoryBudget {
                host_bytes: 10_000,
                device_bytes: 10_000,
            },
        );
        let faults = report_fault
            .union(Faults::FAIL_MODEL_CLEANUP_ONCE)
            .union(Faults::MUTATE_MODEL_REPORT_ON_CLEANUP_FAILURE);
        let result = load_model_id(&mut runtime, 1, source_with_faults(faults));
        assert!(matches!(
            result,
            Err(RuntimeError::CleanupFailed(state))
                if state.failure.primary_detail
                    == FailureDetail::Class(FailureClass::BackendContract)
                    && state.failure.cleanup_detail
                        == FailureDetail::Synchronization(SynchronizationError::Backend(
                            backend_failure(3),
                        ))
        ));
        let expected_ownership = RetainedOwnership::Unverified {
            accepted_footprint: loading_peak_footprint(),
            reported_footprint,
            conservative_footprint,
        };
        let state = runtime
            .model_cleanup_state(ModelId::new(1))
            .ok_or_else(|| "incompatible owner was not retained".to_owned())?;
        assert_eq!(
            state.resource,
            CleanupResource::IncompatibleModel {
                handle: expected_handle(1),
            }
        );
        assert_eq!(state.ownership, expected_ownership);
        let retained = runtime.snapshot();
        assert_eq!(retained.reserved_footprint, MemoryFootprint::default());
        assert_eq!(
            retained.unverified_ownership,
            Some(inference_runtime::UnverifiedOwnershipSummary {
                owners: 1,
                conservative_footprint,
            })
        );
        assert!(retained.admission_blocked);
        assert_eq!(runtime.retained_model_snapshots().len(), 1);

        let preparations = counts.preparations.get();
        assert!(matches!(
            load_model_id(&mut runtime, 2, DEFAULT_SOURCE),
            Err(RuntimeError::AdmissionBlockedByUnverifiedOwnership { owners: 1 })
        ));
        assert_eq!(counts.preparations.get(), preparations);

        assert!(matches!(
            runtime.poll_cleanup().map_err(debug_error)?,
            CleanupPoll::Released(released)
                if released.attempts == 2
                    && released.ownership == RetainedOwnership::Released
                    && !released.exhausted()
        ));
        assert_eq!(counts.successful_model_cleanups.get(), 1);
        assert_eq!(counts.model_drops_while_owned.get(), 0);
        let released = runtime.snapshot();
        assert_eq!(released.reserved_footprint, MemoryFootprint::default());
        assert_eq!(released.unverified_ownership, None);
        assert!(!released.admission_blocked);
        assert!(matches!(
            released.last_cleanup,
            Some(state)
                if state.ownership == RetainedOwnership::Released && !state.exhausted()
        ));
        assert!(runtime.retained_model_snapshots().is_empty());

        let loaded = load_model_id(&mut runtime, 2, DEFAULT_SOURCE).map_err(debug_error)?;
        runtime
            .unload_model(
                loaded.handle,
                UnloadPolicy::RejectIfBusy,
                MonotonicMillis::new(0),
            )
            .map_err(debug_error)?;
        assert_eq!(counts.successful_model_cleanups.get(), 2);
        assert_eq!(
            runtime.poll_cleanup().map_err(debug_error)?,
            CleanupPoll::Idle
        );
        assert_empty(&runtime);
    }
    Ok(())
}

#[test]
fn cleanup_success_on_final_attempt_is_released_not_exhausted() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::WRONG_EXECUTION_SCALAR.union(Faults::FAIL_MODEL_CLEANUP_TWICE);
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));

    assert!(matches!(
        load_model_id(&mut runtime, 1, source_with_faults(faults)),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::RetryFailed(state)
            if state.attempts == 2
                && !state.exhausted()
                && matches!(state.ownership, RetainedOwnership::Unverified { .. })
    ));
    let released = match runtime.poll_cleanup().map_err(debug_error)? {
        CleanupPoll::Released(state) => state,
        other => {
            return Err(format!(
                "final cleanup attempt did not release ownership: {other:?}"
            ));
        }
    };
    assert_eq!(released.attempts, 3);
    assert_eq!(released.ownership, RetainedOwnership::Released);
    assert!(!released.exhausted());
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.last_cleanup, Some(released));
    assert_eq!(snapshot.unverified_ownership, None);
    assert!(!snapshot.admission_blocked);
    assert!(runtime.retained_model_snapshots().is_empty());
    assert_eq!(counts.model_cleanups.get(), 3);

    let loaded = load_model_id(&mut runtime, 2, DEFAULT_SOURCE).map_err(debug_error)?;
    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn unverified_owner_blocks_new_resources_but_existing_healthy_work_progresses() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime_with_resources(
        Faults::default(),
        Rc::clone(&counts),
        3,
        2,
        3,
        MemoryBudget {
            host_bytes: 10_000,
            device_bytes: 10_000,
        },
    );
    let healthy = load_model_id(&mut runtime, 1, DEFAULT_SOURCE).map_err(debug_error)?;
    start(&mut runtime, healthy.handle, 10, 100).map_err(debug_error)?;

    let incompatible =
        source_with_faults(Faults::WRONG_EXECUTION_SCALAR.union(Faults::FAIL_MODEL_CLEANUP_ONCE));
    assert!(matches!(
        load_model_id(&mut runtime, 2, incompatible),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        start(&mut runtime, healthy.handle, 11, 101),
        Err(RuntimeError::AdmissionBlockedByUnverifiedOwnership { owners: 1 })
    ));
    let mut no_logits = [];
    let prefill = runtime
        .prefill(
            RequestId::new(10),
            &[domain_contracts::TokenId::new(1)],
            false,
            &mut no_logits,
        )
        .map_err(debug_error)?;
    assert!(matches!(
        prefill.outcome,
        PrefillOutcome::Ready {
            consumed_tokens: 1,
            position: 1,
            logits_written: 0,
        }
    ));
    runtime
        .cancel_request(RequestId::new(10), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(_)
    ));
    assert!(!runtime.snapshot().admission_blocked);

    start(&mut runtime, healthy.handle, 11, 101).map_err(debug_error)?;
    runtime
        .cancel_request(RequestId::new(11), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    runtime
        .unload_model(
            healthy.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    assert_empty(&runtime);
    assert_eq!(counts.model_drops_while_owned.get(), 0);
    Ok(())
}

#[test]
fn all_complete_model_report_mismatches_cleanup_without_publication() {
    for (report_fault, _, _) in complete_report_cases() {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(report_fault, Rc::clone(&counts));
        assert_eq!(
            load(&mut runtime),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.model_cleanups.get(), 1);
        assert_eq!(counts.successful_model_cleanups.get(), 1);
        assert_eq!(counts.model_drops_while_owned.get(), 0);
        assert_empty(&runtime);
    }
}
