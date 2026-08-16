use super::support::*;

#[test]
fn homogeneous_f32_f16_and_bf16_preserve_execution_behavior() -> TestResult {
    for (profile, declaration, observed, expected_execution, expected_final, expected_loading) in [
        (
            RequiredProfile::F32,
            ConfigDeclaration::F32,
            scalar_set(&[ScalarType::F32]),
            ScalarType::F32,
            CPU_F32_FINAL,
            CPU_F32_LOADING_PEAK,
        ),
        (
            RequiredProfile::F16,
            ConfigDeclaration::F16,
            scalar_set(&[ScalarType::F16]),
            ScalarType::F16,
            CPU_F16_FINAL,
            CPU_F16_LOADING_PEAK,
        ),
        (
            RequiredProfile::Bf16,
            ConfigDeclaration::Bf16,
            scalar_set(&[ScalarType::Bf16]),
            ScalarType::F32,
            CPU_F32_FINAL,
            CPU_BF16_TO_F32_LOADING_PEAK,
        ),
    ] {
        execute_profile(
            profile,
            &[],
            declaration,
            observed,
            expected_execution,
            expected_final,
            expected_loading,
        )?;
    }
    Ok(())
}

#[test]
fn source_layout_does_not_change_sequence_bytes_when_execution_scalar_is_equal() -> TestResult {
    let homogeneous = TinyLlamaFixture::create(RequiredProfile::F16, &[])?;
    let mixed = TinyLlamaFixture::create(RequiredProfile::MixedF16F32, &[])?;
    let homogeneous_source = homogeneous.source(ConfigDeclaration::F16)?;
    let mixed_source = mixed.source(ConfigDeclaration::F16)?;
    let mut homogeneous_loader = CandleLlamaLoader::new(BACKEND);
    let mut mixed_loader = CandleLlamaLoader::new(BACKEND);
    let (_, mut homogeneous_model) = prepare_and_load(
        &mut homogeneous_loader,
        &homogeneous_source,
        load_configuration(),
    )?;
    let (_, mut mixed_model) =
        prepare_and_load(&mut mixed_loader, &mixed_source, load_configuration())?;
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );

    assert_eq!(homogeneous_model.execution_scalar_type(), ScalarType::F16);
    assert_eq!(mixed_model.execution_scalar_type(), ScalarType::F16);
    assert_eq!(
        homogeneous_model
            .plan_sequence(&configuration)
            .map_err(debug_error)?
            .reservation,
        mixed_model
            .plan_sequence(&configuration)
            .map_err(debug_error)?
            .reservation
    );

    clean_model(&mut homogeneous_model)?;
    clean_model(&mut mixed_model)
}

#[test]
fn huge_ignored_extra_does_not_change_exact_cpu_footprints_or_working_bytes() -> TestResult {
    assert_eq!(
        CPU_F32_FINAL.host_weight_bytes().as_u64(),
        REQUIRED_ELEMENTS * 4
    );
    assert_eq!(
        CPU_F16_FINAL.host_weight_bytes().as_u64(),
        REQUIRED_ELEMENTS * 2
    );
    let base = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let base_source = base.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let base_prepared = loader
        .prepare_load(&base_source, &load_configuration())
        .map_err(debug_error)?;
    let base_plan = *base_prepared.plan();
    drop(base_prepared);

    let extras = [
        ExtraTensor::new("unused.huge.f16", "F16", 1_048_576, 2),
        ExtraTensor::new("unused.non_executable.u8", "U8", 32, 1),
    ];
    let huge = TinyLlamaFixture::create(RequiredProfile::F32, &extras)?;
    let huge_source = huge.source(ConfigDeclaration::F32)?;
    let descriptor = loader.inspect(&huge_source).map_err(debug_error)?;
    let (huge_plan, mut model) = prepare_and_load(&mut loader, &huge_source, load_configuration())?;

    assert_eq!(base_plan.final_footprint, CPU_F32_FINAL);
    assert_eq!(base_plan.loading_peak_footprint, CPU_F32_LOADING_PEAK);
    assert_eq!(huge_plan.final_footprint, base_plan.final_footprint);
    assert_eq!(
        huge_plan.loading_peak_footprint,
        base_plan.loading_peak_footprint
    );
    assert_eq!(descriptor.estimated_footprint, CPU_F32_FINAL);
    assert!(
        huge_plan
            .loading_peak_footprint
            .device_working_bytes()
            .is_zero()
    );
    assert_eq!(model.reported_footprint(), CPU_F32_FINAL);
    clean_model(&mut model)
}

#[test]
fn host_budget_rejects_exact_required_only_loading_peak() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    assert_eq!(
        CPU_F32_LOADING_PEAK.checked_host_bytes(),
        Some(ByteCount::from_u64(69_443))
    );

    let mut constrained = load_configuration();
    constrained.memory_budget = constrained
        .memory_budget
        .with_host_bytes(ByteCount::from_u64(69_442));
    assert!(matches!(
        loader.prepare_load(&source, &constrained),
        Err(LoadError::InsufficientMemory {
            kind: domain_contracts::MemoryKind::Host,
            required_bytes,
            available_bytes,
        }) if required_bytes == ByteCount::from_u64(69_443)
            && available_bytes == ByteCount::from_u64(69_442)
    ));
    Ok(())
}
