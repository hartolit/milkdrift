use super::support::*;

#[test]
fn exact_preparation_is_consumed_once_without_replanning() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime(Faults::default(), Rc::clone(&counts));

    let loaded = load(&mut runtime).map_err(debug_error)?;
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.plan_reads.get(), 1);
    assert_eq!(counts.prepared_drops.get(), 1);
    assert_eq!(counts.retained_prepared_drops.get(), 0);
    assert_eq!(counts.model_loads.get(), 1);
    assert_eq!(counts.failed_load_cleanups.get(), 0);
    assert_eq!(runtime.snapshot().reserved_footprint, model_footprint());

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
fn invalid_prepared_plans_are_rejected_before_materialization() {
    for fault in [
        Faults::WRONG_ACCEPTED_CONFIGURATION,
        Faults::EMPTY_OBSERVED_TENSOR_SET,
        Faults::OVERFLOWING_FINAL_FOOTPRINT,
        Faults::OVERFLOWING_LOADING_PEAK,
        Faults::LOADING_PEAK_BELOW_FINAL,
        Faults::RECLASSIFIED_LOADING_PEAK,
    ] {
        let counts = Rc::new(CleanupCounts::default());
        let mut runtime = runtime(fault, Rc::clone(&counts));

        assert_eq!(
            load(&mut runtime),
            Err(RuntimeError::BackendContractViolation)
        );
        assert_eq!(counts.preparations.get(), 1);
        assert_eq!(counts.plan_reads.get(), 1);
        assert_eq!(counts.prepared_drops.get(), 1);
        assert_eq!(counts.retained_prepared_drops.get(), 0);
        assert_eq!(counts.model_loads.get(), 0);
        assert_eq!(counts.failed_load_cleanups.get(), 0);
        assert_eq!(counts.model_cleanups.get(), 0);
        assert_empty(&runtime);
    }
}

#[test]
fn failed_owner_plan_substitution_blocks_admission_until_release() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD
        .union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE)
        .union(Faults::ALTERNATING_PLAN_REPORT);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert_eq!(counts.plan_reads.get(), 4);
    assert_eq!(counts.prepared_drops.get(), 0);
    let state = runtime
        .model_cleanup_state(ModelId::new(1))
        .ok_or_else(|| "failed preparation cleanup state was not retained".to_owned())?;
    assert!(matches!(
        state.ownership,
        RetainedOwnership::Unverified {
            accepted_footprint,
            reported_footprint,
            conservative_footprint: ConservativeFootprint::Known(conservative),
        } if accepted_footprint == loading_peak_footprint()
            && reported_footprint.host_working_bytes
                == loading_peak_footprint().host_working_bytes.saturating_add(1)
            && conservative.host_working_bytes
                == loading_peak_footprint().host_working_bytes.saturating_add(1)
    ));
    assert_eq!(state.failure.primary_failure, FailureClass::BackendContract);
    assert!(runtime.snapshot().admission_blocked);
    assert_eq!(
        runtime.snapshot().reserved_footprint,
        MemoryFootprint::default()
    );
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(released)
            if released.ownership == RetainedOwnership::Released && !released.exhausted()
    ));
    assert_eq!(counts.plan_reads.get(), 6);
    assert_eq!(counts.prepared_drops.get(), 1);
    assert_eq!(counts.retained_prepared_drops.get(), 0);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn failed_owner_plan_mutation_during_cleanup_blocks_admission_until_release() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let faults = Faults::FAIL_LOAD
        .union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE)
        .union(Faults::MUTATE_FAILED_PLAN_ON_CLEANUP_FAILURE);
    let mut runtime = runtime(faults, Rc::clone(&counts));

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert_eq!(counts.plan_reads.get(), 4);
    let state = runtime
        .model_cleanup_state(ModelId::new(1))
        .ok_or_else(|| "mutated failed owner was not retained".to_owned())?;
    assert!(matches!(
        state.ownership,
        RetainedOwnership::Unverified {
            accepted_footprint,
            reported_footprint,
            conservative_footprint: ConservativeFootprint::Known(conservative),
        } if accepted_footprint == loading_peak_footprint()
            && reported_footprint.host_working_bytes
                == loading_peak_footprint().host_working_bytes.saturating_add(7)
            && conservative.host_working_bytes
                == loading_peak_footprint().host_working_bytes.saturating_add(7)
    ));
    assert_eq!(state.failure.primary_failure, FailureClass::BackendContract);
    assert!(runtime.snapshot().admission_blocked);
    assert_eq!(
        runtime.snapshot().reserved_footprint,
        MemoryFootprint::default()
    );

    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(released)
            if released.ownership == RetainedOwnership::Released && !released.exhausted()
    ));
    assert_eq!(counts.plan_reads.get(), 6);
    assert_eq!(counts.prepared_drops.get(), 1);
    assert_eq!(counts.retained_prepared_drops.get(), 0);
    assert_empty(&runtime);
    Ok(())
}

#[test]
fn retained_failed_transaction_release_then_reload_advances_generation() -> TestResult {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime_with_resources(
        Faults::default(),
        Rc::clone(&counts),
        3,
        1,
        1,
        MemoryBudget {
            host_bytes: 1_024,
            device_bytes: 0,
        },
    );
    let failed_source =
        source_with_faults(Faults::FAIL_LOAD.union(Faults::FAIL_FAILED_LOAD_CLEANUP_ONCE));
    assert!(matches!(
        load_model_id(&mut runtime, 1, failed_source),
        Err(RuntimeError::CleanupFailed(_))
    ));
    assert!(matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(_)
    ));
    let loaded = load_model_id(&mut runtime, 1, DEFAULT_SOURCE).map_err(debug_error)?;
    assert_eq!(loaded.handle.generation, ModelGeneration::new(2));
    assert!(matches!(
        runtime.unload_model(
            expected_handle(1),
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        ),
        Err(RuntimeError::StaleModelHandle { current, .. }) if current == loaded.handle
    ));
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
fn aggregate_loading_peak_budget_rejection_precedes_materialization() {
    let counts = Rc::new(CleanupCounts::default());
    let mut runtime = runtime_with_host_budget(
        Faults::default(),
        Rc::clone(&counts),
        loading_peak_host_bytes() - 1,
    );

    assert!(matches!(
        load(&mut runtime),
        Err(RuntimeError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes,
            available_bytes,
        }) if required_bytes == loading_peak_host_bytes()
            && available_bytes == loading_peak_host_bytes() - 1
    ));
    assert_eq!(counts.preparations.get(), 1);
    assert_eq!(counts.model_loads.get(), 0);
    assert_eq!(counts.failed_load_cleanups.get(), 0);
    assert_eq!(counts.model_cleanups.get(), 0);
    assert_empty(&runtime);
}
