use super::*;

pub(crate) fn execute_profile(
    profile: RequiredProfile,
    extras: &[ExtraTensor],
    declaration: ConfigDeclaration,
    expected_observed: ScalarTypeSet,
    expected_execution: ScalarType,
    expected_final: MemoryFootprint,
    expected_loading: MemoryFootprint,
) -> TestResult {
    let fixture = TinyLlamaFixture::create(profile, extras)?;
    let source = fixture.source(declaration)?;
    let mut loader = CandleLlamaLoader::new(BACKEND);
    let descriptor = loader.inspect(&source).map_err(debug_error)?;
    assert_eq!(
        descriptor.metadata.configuration_declared_scalar_type,
        declaration.recognized()?
    );
    assert_eq!(
        descriptor.metadata.observed_tensor_scalar_types,
        expected_observed
    );
    assert_eq!(descriptor.estimated_footprint, expected_final);
    let expected_cache_rate = match expected_execution {
        ScalarType::F32 => F32_SEQUENCE_CACHE_BYTES_PER_TOKEN,
        ScalarType::F16 | ScalarType::Bf16 => F16_SEQUENCE_CACHE_BYTES_PER_TOKEN,
        _ => return Err("test profile selected a non-floating execution scalar".to_owned()),
    };
    assert_eq!(
        descriptor.sequence_cache_bytes_per_token,
        expected_cache_rate
    );

    let (plan, mut model) = prepare_and_load(&mut loader, &source, load_configuration())?;
    assert_eq!(plan.descriptor, descriptor);
    assert_eq!(plan.execution_scalar_type, expected_execution);
    assert_eq!(plan.final_footprint, expected_final);
    assert_eq!(plan.loading_peak_footprint, expected_loading);
    assert_eq!(plan.loading_peak_footprint.device_working_bytes, 0);
    assert_eq!(model.descriptor(), &descriptor);
    assert_eq!(model.execution_scalar_type(), expected_execution);
    assert_eq!(model.reported_footprint(), expected_final);
    exercise_model(&mut model)?;
    clean_model(&mut model)
}

pub(crate) fn exercise_model(model: &mut CandleLlamaModel) -> TestResult {
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(16).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::new(8).ok_or_else(|| "maximum prefill must be nonzero".to_owned())?,
    );
    let sequence_plan = model.plan_sequence(&configuration).map_err(debug_error)?;
    assert_cpu_sequence_reservation(model, sequence_plan)?;

    let mut first = model
        .create_sequence(SequenceId::new(1), &configuration)
        .map_err(debug_error)?;
    let mut second = model
        .create_sequence(SequenceId::new(2), &configuration)
        .map_err(debug_error)?;
    assert_eq!(first.reservation(), sequence_plan.reservation);
    assert_eq!(second.reservation(), sequence_plan.reservation);
    assert_eq!(first.reported_plan(), sequence_plan);
    assert_eq!(second.reported_plan(), sequence_plan);
    assert_eq!(first.token_staging_capacity(), 8);
    assert_eq!(first.token_staging_logical_bytes(), 32);
    exercise_repeated_prefill(model, &mut first)?;
    exercise_maximum_prefill_and_near_capacity_decode(model, &mut second)?;
    exercise_first_decode(model, &mut first)?;
    exercise_mask_free_prefill_and_decode(model)?;

    model.destroy_sequence(&mut first).map_err(debug_error)?;
    model.destroy_sequence(&mut second).map_err(debug_error)?;
    assert_eq!(first.state(), SequenceState::Finished);
    assert_eq!(second.state(), SequenceState::Finished);
    Ok(())
}

pub(crate) fn assert_cpu_sequence_reservation(
    model: &CandleLlamaModel,
    plan: domain_contracts::SequencePlan,
) -> TestResult {
    let expected = match model.execution_scalar_type() {
        ScalarType::F32 => (
            F32_SEQUENCE_PERSISTENT_BYTES,
            F32_SEQUENCE_TRANSIENT_BYTES,
            F32_SEQUENCE_HOST_WORKING_BYTES,
        ),
        ScalarType::F16 | ScalarType::Bf16 => (
            HALF_SEQUENCE_PERSISTENT_BYTES,
            HALF_SEQUENCE_TRANSIENT_BYTES,
            HALF_SEQUENCE_HOST_WORKING_BYTES,
        ),
        _ => return Err("test model selected a non-floating execution scalar".to_owned()),
    };
    for (footprint, host_working_bytes) in [
        (plan.reservation.persistent_footprint, expected.0),
        (plan.reservation.transient_footprint, expected.1),
        (plan.reservation.total_footprint, expected.2),
    ] {
        assert_eq!(
            footprint,
            MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes,
                device_working_bytes: 0,
            }
        );
    }
    Ok(())
}

pub(crate) fn exercise_repeated_prefill(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    for (tokens, expected_position) in [
        ([TokenId::new(1), TokenId::new(2)], 2),
        ([TokenId::new(3), TokenId::new(4)], 4),
    ] {
        let outcome = prefill_checked(
            model,
            sequence,
            PrefillInput::new(&tokens, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?;
        assert_eq!(
            outcome,
            PrefillOutcome::Ready {
                consumed_tokens: 2,
                position: expected_position,
                logits_written: VOCABULARY_SIZE,
            }
        );
        assert_eq!(maximum_logit_token(&logits)?, tokens[1]);
    }
    assert_eq!(sequence.state(), SequenceState::Ready);
    Ok(())
}

pub(crate) fn exercise_maximum_prefill_and_near_capacity_decode(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    let prompt = [TokenId::new(6); 8];
    assert_eq!(
        prefill_checked(
            model,
            sequence,
            PrefillInput::new(&prompt, true),
            PrefillBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?,
        PrefillOutcome::Ready {
            consumed_tokens: 8,
            position: 8,
            logits_written: VOCABULARY_SIZE,
        }
    );
    for expected_position in 9..=16 {
        assert_eq!(
            decode_checked(
                model,
                sequence,
                DecodeInput::new(TokenId::new(7)),
                DecodeBuffers::new(&mut logits),
                CancellationStatus::Running,
            )
            .map_err(debug_error)?,
            DecodeOutcome::Ready {
                position: expected_position,
                logits_written: VOCABULARY_SIZE,
            }
        );
    }
    Ok(())
}

pub(crate) fn exercise_first_decode(
    model: &mut CandleLlamaModel,
    sequence: &mut CandleLlamaSequence,
) -> TestResult {
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    assert_eq!(
        decode_checked(
            model,
            sequence,
            DecodeInput::new(TokenId::new(5)),
            DecodeBuffers::new(&mut logits),
            CancellationStatus::Running,
        )
        .map_err(debug_error)?,
        DecodeOutcome::Ready {
            position: 5,
            logits_written: VOCABULARY_SIZE,
        }
    );
    assert_eq!(maximum_logit_token(&logits)?, TokenId::new(5));
    Ok(())
}

pub(crate) fn exercise_mask_free_prefill_and_decode(model: &mut CandleLlamaModel) -> TestResult {
    let configuration = SequenceConfiguration::new(
        NonZeroU32::new(4).ok_or_else(|| "maximum tokens must be nonzero".to_owned())?,
        NonZeroU32::MIN,
    );
    let plan = model.plan_sequence(&configuration).map_err(debug_error)?;
    let mut sequence = model
        .create_sequence(SequenceId::new(3), &configuration)
        .map_err(debug_error)?;
    assert_eq!(sequence.reported_plan(), plan);
    assert_eq!(sequence.token_staging_capacity(), 1);
    assert_eq!(sequence.token_staging_logical_bytes(), 4);
    prefill_checked(
        model,
        &mut sequence,
        PrefillInput::new(&[TokenId::new(8)], false),
        PrefillBuffers::new(&mut []),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    let mut logits = [0.0_f32; VOCABULARY_SIZE];
    decode_checked(
        model,
        &mut sequence,
        DecodeInput::new(TokenId::new(9)),
        DecodeBuffers::new(&mut logits),
        CancellationStatus::Running,
    )
    .map_err(debug_error)?;
    model.destroy_sequence(&mut sequence).map_err(debug_error)?;
    assert_eq!(sequence.state(), SequenceState::Finished);
    Ok(())
}

pub(crate) fn clean_model(model: &mut CandleLlamaModel) -> TestResult {
    model.synchronize().map_err(debug_error)?;
    model.prepare_unload().map_err(debug_error)
}

pub(crate) fn prepare_and_load(
    loader: &mut CandleLlamaLoader,
    source: &CandleLlamaSource,
    configuration: LoadConfiguration,
) -> TestResult<(LoadPlan, CandleLlamaModel)> {
    let prepared = loader
        .prepare_load(source, &configuration)
        .map_err(debug_error)?;
    let plan = *prepared.plan();
    let model = load_exact_preparation(loader, prepared)?;
    Ok((plan, model))
}

pub(crate) fn load_exact_preparation(
    loader: &mut CandleLlamaLoader,
    prepared: CandleLlamaPreparedLoad,
) -> TestResult<CandleLlamaModel> {
    match loader.load_prepared(prepared) {
        Ok(model) => Ok(model),
        Err(mut failed) => {
            let primary = failed.primary();
            let cleanup = failed.cleanup();
            Err(format!(
                "prepared load failed: {primary:?}; cleanup: {cleanup:?}"
            ))
        }
    }
}
