use super::support::*;

#[test]
fn aggregate_sequence_memory_is_admitted_before_allocation() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 115));
    let loaded = runtime
        .load_model(
            ModelId::new(6),
            &MockSource {
                model_bytes: 100,
                vocabulary_size: 4,
            },
            cpu_device(),
        )
        .map_err(debug_error)?;
    let error = runtime
        .start_request(
            loaded.handle,
            RequestId::new(60),
            SequenceId::new(600),
            sequence_configuration(8, 4)?,
        )
        .err()
        .ok_or("sequence admission unexpectedly succeeded")?;
    if !matches!(
        error,
        inference_runtime::RuntimeError::InsufficientMemory {
            kind: inference_runtime::MemoryKind::Host,
            ..
        }
    ) {
        return Err(format!("unexpected sequence admission error: {error:?}"));
    }
    let snapshot = runtime.snapshot();
    if snapshot.active_requests != 0
        || snapshot.unverified_ownership.is_some()
        || snapshot.admission_blocked
        || !runtime.retained_model_snapshots().is_empty()
    {
        return Err("failed admission changed active-request state".into());
    }
    Ok(())
}
