use super::support::*;

#[test]
fn registry_interleaves_sequences_and_reclaims_all_resources() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(4, 8, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(1),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 8,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    assert_eq!(loaded.execution_device, cpu_device());
    assert_eq!(loaded.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded.reserved_footprint,
        descriptor(MockSource {
            model_bytes: 100,
            vocabulary_size: 8,
        })
        .estimated_footprint
    );
    let loaded_snapshot = runtime
        .model_snapshot(loaded.handle)
        .ok_or_else(|| "loaded model snapshot missing".to_owned())?;
    assert_eq!(loaded_snapshot.execution_device, cpu_device());
    assert_eq!(loaded_snapshot.execution_scalar_type, ScalarType::F32);
    assert_eq!(
        loaded_snapshot.reserved_footprint,
        loaded.reserved_footprint
    );
    let configuration = sequence_configuration(16, 8)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(10),
            SequenceId::new(100),
            configuration,
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(11),
            SequenceId::new(101),
            configuration,
        )
        .map_err(debug_error)?;

    let mut logits_a = vec![0.0_f32; 8];
    let mut logits_b = vec![0.0_f32; 8];
    let prompt = [TokenId::new(1), TokenId::new(2)];
    runtime
        .prefill(RequestId::new(10), &prompt, true, &mut logits_a)
        .map_err(debug_error)?;
    runtime
        .prefill(RequestId::new(11), &prompt, true, &mut logits_b)
        .map_err(debug_error)?;
    runtime
        .decode(RequestId::new(10), TokenId::new(3), &mut logits_a)
        .map_err(debug_error)?;
    runtime
        .decode(RequestId::new(11), TokenId::new(4), &mut logits_b)
        .map_err(debug_error)?;

    runtime
        .cancel_request(RequestId::new(10), CancellationReason::UserRequested)
        .map_err(debug_error)?;
    runtime
        .complete_request(RequestId::new(11), FinishReason::TokenLimit)
        .map_err(debug_error)?;
    let unloaded = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    if unloaded.status != UnloadStatus::Unloaded {
        return Err("ready model was not unloaded".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.loaded_models != 0
        || snapshot.active_requests != 0
        || snapshot.reserved_footprint != MemoryFootprint::default()
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("registry retained resources after unload".into());
    }
    Ok(())
}

#[test]
fn failed_sequence_release_preserves_request_for_retry() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(7),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(70),
            SequenceId::new(999),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let first = runtime.cancel_request(RequestId::new(70), CancellationReason::UserRequested);
    if !matches!(
        first,
        Err(inference_runtime::RuntimeError::CleanupFailed(state))
            if state.failure.primary_failure == inference_runtime::FailureClass::Cancellation
                && state.failure.cleanup_failure == inference_runtime::FailureClass::Sequence
    ) {
        return Err(format!("unexpected first release result: {first:?}"));
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.pending_cleanup_sequences != 1
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("failed sequence release did not quarantine ownership".into());
    }

    if !matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        inference_runtime::CleanupPoll::Released(state)
            if state.ownership == RetainedOwnership::Released && !state.exhausted()
    ) {
        return Err("pending cleanup was not retried".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.pending_cleanup_sequences != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("successful release retry retained the sequence".into());
    }
    Ok(())
}

#[test]
fn direct_prefill_preserves_primary_and_exposes_retained_cleanup() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(10),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(100),
            SequenceId::new(998),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let mut logits = [0.0_f32; 4];
    let result = runtime.prefill(RequestId::new(100), &[TokenId::new(1)], true, &mut logits);
    if !matches!(
        result,
        Err(RuntimeError::Sequence(SequenceError::Backend(failure)))
            if failure == mock_failure(3)
    ) {
        return Err(format!(
            "primary prefill failure was not preserved: {result:?}"
        ));
    }

    let snapshot = runtime.snapshot();
    let cleanup = snapshot
        .last_cleanup
        .ok_or("failed cleanup was not exposed by the runtime snapshot")?;
    let resource = CleanupResource::Sequence {
        handle: loaded.handle,
        request_id: RequestId::new(100),
        sequence_id: SequenceId::new(998),
    };
    if snapshot.active_requests != 0
        || snapshot.pending_cleanup_sequences != 1
        || cleanup.resource != resource
        || cleanup.failure.primary_operation != RuntimeOperation::Prefill
        || cleanup.failure.primary_detail
            != FailureDetail::Sequence(SequenceError::Backend(mock_failure(3)))
        || cleanup.failure.cleanup_detail
            != FailureDetail::Sequence(SequenceError::Backend(mock_failure(2)))
        || cleanup.ownership
            != RetainedOwnership::Exact(MemoryFootprint::host_working(ByteCount::from_u64(8)))
    {
        return Err("retained direct-prefill cleanup state lost structured identity".into());
    }

    if !matches!(
        runtime.poll_cleanup().map_err(debug_error)?,
        CleanupPoll::Released(state)
            if state.resource == resource
                && state.ownership == RetainedOwnership::Released
                && !state.exhausted()
    ) {
        return Err("retained direct-prefill sequence was not released exactly once".into());
    }
    let model = runtime
        .model_snapshot(loaded.handle)
        .ok_or("model disappeared after sequence cleanup")?;
    if runtime.snapshot().pending_cleanup_sequences != 0 || model.degraded {
        return Err("successful cleanup did not clear retained sequence state".into());
    }
    runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    Ok(())
}

#[test]
fn undersized_logits_finish_request_without_backend_overwrite() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(5),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 8,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    runtime
        .start_request(
            loaded.handle,
            RequestId::new(50),
            SequenceId::new(500),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;

    let mut logits = [0.0_f32; 2];
    let receipt = runtime
        .prefill(RequestId::new(50), &[TokenId::new(1)], true, &mut logits)
        .map_err(debug_error)?;
    if !matches!(
        receipt.outcome,
        PrefillOutcome::Finished(FinishReason::BufferExhausted(_))
    ) {
        return Err("undersized logits did not finish with BufferExhausted".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("buffer-exhausted request remained active".into());
    }
    Ok(())
}
