use super::support::*;

#[test]
fn drain_timeout_force_cancels_and_unloads() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 2, 10_000));
    let loaded = runtime
        .load_model(
            ModelId::new(2),
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
            RequestId::new(20),
            SequenceId::new(200),
            sequence_configuration(8, 4)?,
        )
        .map_err(debug_error)?;
    let timeout = DrainTimeout::from_millis(10).map_err(debug_error)?;
    let receipt = runtime
        .unload_model(
            loaded.handle,
            UnloadPolicy::Drain { timeout },
            MonotonicMillis::new(100),
        )
        .map_err(debug_error)?;
    if receipt.status != UnloadStatus::Draining {
        return Err("active model did not enter draining".into());
    }
    if runtime
        .poll(MonotonicMillis::new(109))
        .map_err(debug_error)?
    {
        return Err("drain escalated before deadline".into());
    }
    if !runtime
        .poll(MonotonicMillis::new(110))
        .map_err(debug_error)?
    {
        return Err("drain did not escalate at deadline".into());
    }
    let snapshot = runtime.snapshot();
    if snapshot.loaded_models != 0
        || snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("timeout escalation did not reclaim registry state".into());
    }
    Ok(())
}
