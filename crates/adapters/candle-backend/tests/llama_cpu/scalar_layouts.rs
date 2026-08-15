use super::support::*;

#[test]
fn genuine_required_f16_bf16_mixture_rejects_before_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::MixedF16Bf16, &[])?;
    let source = fixture.source(ConfigDeclaration::Absent)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);

    assert_unsupported(loader.inspect(&source));
    assert_unsupported(loader.prepare_load(&source, &configuration));
    Ok(())
}

#[test]
fn required_unsupported_dtype_rejects_before_device_initialization() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::UnsupportedU8, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let mut configuration = load_configuration();
    configuration.execution_device = ExecutionDevice::new(DeviceId::new(1), DeviceKind::Cpu);

    let expected = TensorFailureLocation::new(0, 11, MODEL_NORM_NAME_HASH, Some(ScalarType::U8));
    assert_exact_tensor_load_failure(
        loader.inspect(&source),
        BackendFailureKind::Unsupported,
        UNSUPPORTED_SCALAR_CODE,
        LoadFailureStage::ScalarConversion,
        expected,
    )?;
    assert_exact_tensor_load_failure(
        loader.prepare_load(&source, &configuration),
        BackendFailureKind::Unsupported,
        UNSUPPORTED_SCALAR_CODE,
        LoadFailureStage::ScalarConversion,
        expected,
    )?;
    Ok(())
}

#[test]
fn incompatible_existing_required_shape_has_exact_tensor_context() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::InvalidNormShape, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let loader = CandleLlamaLoader::new(BACKEND);
    assert_exact_tensor_load_failure(
        loader.inspect(&source),
        BackendFailureKind::InvalidModel,
        REQUIRED_TENSOR_CODE,
        LoadFailureStage::CompatibilityValidation,
        TensorFailureLocation::new(0, 11, MODEL_NORM_NAME_HASH, Some(ScalarType::F32)),
    )
}
