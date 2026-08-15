use super::support::*;

#[test]
fn f32_required_ignores_f16_bf16_and_combined_extras_with_declared_and_absent_configs() -> TestResult
{
    let cases = [
        (
            vec![ExtraTensor::new("unused.f16", "F16", 3, 2)],
            scalar_set(&[ScalarType::F32, ScalarType::F16]),
        ),
        (
            vec![ExtraTensor::new("unused.bf16", "BF16", 5, 2)],
            scalar_set(&[ScalarType::F32, ScalarType::Bf16]),
        ),
        (
            vec![
                ExtraTensor::new("unused.f16", "F16", 3, 2),
                ExtraTensor::new("unused.bf16", "BF16", 5, 2),
            ],
            scalar_set(&[ScalarType::F32, ScalarType::F16, ScalarType::Bf16]),
        ),
    ];

    for (extras, observed) in cases {
        for declaration in [ConfigDeclaration::F32, ConfigDeclaration::Absent] {
            execute_profile(
                RequiredProfile::F32,
                extras.as_slice(),
                declaration,
                observed,
                ScalarType::F32,
                CPU_F32_FINAL,
                CPU_F32_LOADING_PEAK,
            )?;
        }
    }
    Ok(())
}

#[test]
fn mixed_required_f16_f32_ignores_bf16_u8_bool_and_other_extras() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.bf16", "BF16", 2, 2),
        ExtraTensor::new("unused.u8", "U8", 3, 1),
        ExtraTensor::new("unused.bool", "BOOL", 4, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::MixedF16F32,
        &extras,
        ConfigDeclaration::F16,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::F16,
            ScalarType::Bf16,
            ScalarType::U8,
            ScalarType::Other(1),
        ]),
        ScalarType::F16,
        CPU_F16_FINAL,
        CPU_MIXED_F16_F32_LOADING_PEAK,
    )
}

#[test]
fn mixed_required_bf16_f32_ignores_f16_u8_bool_and_other_extras() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.f16", "F16", 2, 2),
        ExtraTensor::new("unused.u8", "U8", 3, 1),
        ExtraTensor::new("unused.bool", "BOOL", 4, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::MixedBf16F32,
        &extras,
        ConfigDeclaration::Bf16,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::F16,
            ScalarType::Bf16,
            ScalarType::U8,
            ScalarType::Other(1),
        ]),
        ScalarType::F32,
        CPU_F32_FINAL,
        CPU_MIXED_BF16_F32_LOADING_PEAK,
    )
}

#[test]
fn complete_observed_set_includes_i8_u8_and_other_while_unused_extras_load() -> TestResult {
    let extras = [
        ExtraTensor::new("unused.i8", "I8", 2, 1),
        ExtraTensor::new("unused.u8", "U8", 2, 1),
        ExtraTensor::new("unused.bool", "BOOL", 2, 1),
        ExtraTensor::new("unused.f64", "F64", 1, 8),
    ];
    execute_profile(
        RequiredProfile::F32,
        &extras,
        ConfigDeclaration::F32,
        scalar_set(&[
            ScalarType::F32,
            ScalarType::I8,
            ScalarType::U8,
            ScalarType::Other(17),
        ]),
        ScalarType::F32,
        CPU_F32_FINAL,
        CPU_F32_LOADING_PEAK,
    )
}

#[test]
fn shard_reorder_sorts_complete_expected_content_pairs_and_loads() -> TestResult {
    let fixture = TinyLlamaFixture::create_sharded(RequiredProfile::MixedF16F32, false)?;
    let (z_length, z_sha256) = file_identity(&fixture.weight_path)?;
    let (a_length, a_sha256) = file_identity(&fixture.second_weight_path)?;
    let source = fixture.source_with_shards(
        ConfigDeclaration::F16,
        vec![
            CandleWeightShard::with_expected_content(
                fixture.weight_path.clone(),
                CandleExpectedContentIdentity::new(z_length, z_sha256),
            ),
            CandleWeightShard::with_expected_content(
                fixture.second_weight_path.clone(),
                CandleExpectedContentIdentity::new(a_length, a_sha256),
            ),
        ],
    )?;

    let [first, second] = source.weight_shards() else {
        return Err("sharded source did not retain exactly two content expectations".to_owned());
    };
    assert_eq!(first.path(), fixture.second_weight_path);
    assert_eq!(
        first.expected_content(),
        Some(CandleExpectedContentIdentity::new(a_length, a_sha256))
    );
    assert_eq!(second.path(), fixture.weight_path);
    assert_eq!(
        second.expected_content(),
        Some(CandleExpectedContentIdentity::new(z_length, z_sha256))
    );

    let mut loader = CandleLlamaLoader::new(BACKEND);
    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.final_footprint, CPU_F16_FINAL);
    assert_eq!(plan.loading_peak_footprint, CPU_MIXED_F16_F32_LOADING_PEAK);
    clean_model(&mut model)
}
