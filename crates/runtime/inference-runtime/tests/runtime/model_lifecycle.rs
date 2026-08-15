use super::support::*;

#[test]
fn reloading_increments_generation_and_rejects_stale_handles() -> Result<(), String> {
    let mut runtime = InferenceRuntime::new(MockLoader, limits(1, 1, 10_000));
    let source = MockSource {
        model_bytes: 100,
        vocabulary_size: 4,
    };
    let first = runtime
        .load_model(ModelId::new(3), &source, cpu_device())
        .map_err(debug_error)?;
    runtime
        .unload_model(
            first.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .map_err(debug_error)?;
    let second = runtime
        .load_model(ModelId::new(3), &source, cpu_device())
        .map_err(debug_error)?;
    if second.handle.generation.get() != first.handle.generation.get() + 1 {
        return Err("model generation did not advance".into());
    }
    if runtime
        .unload_model(
            first.handle,
            UnloadPolicy::RejectIfBusy,
            MonotonicMillis::new(0),
        )
        .is_ok()
    {
        return Err("stale model handle was accepted".into());
    }
    Ok(())
}

#[test]
fn candle_loader_satisfies_runtime_loader_contract() {
    const fn assert_loader<L: ModelLoader>() {}
    assert_loader::<candle_backend::CandleLlamaLoader>();
}
