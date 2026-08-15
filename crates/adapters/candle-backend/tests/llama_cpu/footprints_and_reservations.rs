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
fn huge_ignored_extra_does_not_change_exact_cpu_footprints_or_working_bytes() -> TestResult {
    assert_eq!(CPU_F32_FINAL.host_weight_bytes, REQUIRED_ELEMENTS * 4);
    assert_eq!(CPU_F16_FINAL.host_weight_bytes, REQUIRED_ELEMENTS * 2);
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
    assert_eq!(
        descriptor.sequence_cache_bytes_per_token,
        F32_SEQUENCE_CACHE_BYTES_PER_TOKEN
    );
    assert_eq!(huge_plan.loading_peak_footprint.device_working_bytes, 0);
    assert_eq!(model.reported_footprint(), CPU_F32_FINAL);
    clean_model(&mut model)
}

#[test]
fn host_budget_rejects_exact_required_only_loading_peak() -> TestResult {
    let fixture = TinyLlamaFixture::create(RequiredProfile::F32, &[])?;
    let source = fixture.source(ConfigDeclaration::F32)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    assert_eq!(CPU_F32_LOADING_PEAK.checked_host_bytes(), Some(69_443));

    let mut constrained = load_configuration();
    constrained.memory_budget.host_bytes = 69_442;
    assert!(matches!(
        loader.prepare_load(&source, &constrained),
        Err(LoadError::InsufficientMemory {
            kind: domain_contracts::MemoryKind::Host,
            required_bytes: 69_443,
            available_bytes: 69_442,
        })
    ));
    Ok(())
}
