use super::support::*;

#[test]
fn prepared_load_consumes_retained_file_after_path_deletion() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let prepared = loader
        .prepare_load(&source, &load_configuration())
        .map_err(debug_error)?;
    let accepted_plan = *prepared.plan();
    fs::remove_file(&fixture.weight_path)
        .map_err(|error| format!("remove prepared path: {error}"))?;

    let mut model = load_exact_preparation(&mut loader, prepared)?;
    assert_eq!(model.reported_footprint(), accepted_plan.final_footprint);
    clean_model(&mut model)
}

#[test]
fn rejects_invalid_cpu_identity_and_unsupported_devices() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);
    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::InvalidConfiguration)
    ));

    for kind in [DeviceKind::Metal, DeviceKind::Accelerator(1)] {
        configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), kind);
        assert!(matches!(
            loader.prepare_load(&source, &configuration),
            Err(LoadError::Backend(failure))
                if failure.failure.kind == BackendFailureKind::Unsupported
        ));
    }
    Ok(())
}

#[cfg(not(feature = "cuda"))]
#[test]
fn cuda_request_fails_explicitly_when_support_is_not_compiled() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cuda);
    configuration.memory_budget = configuration
        .memory_budget
        .with_device_bytes(ByteCount::MAX);

    assert!(matches!(
        loader.prepare_load(&source, &configuration),
        Err(LoadError::Backend(failure))
            if failure.failure.kind == BackendFailureKind::Unsupported
    ));
    Ok(())
}
