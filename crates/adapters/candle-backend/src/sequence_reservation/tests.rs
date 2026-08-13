use std::num::NonZeroU32;

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, MemoryFootprint, ModelError, SequenceConfiguration,
    SequenceReservation,
};

use super::{
    AttentionPhaseComponents, CreationTransientComponents, ExecutionTransientComponents,
    PersistentComponents, ReservationComponents, ReservationInputs, calculate,
    maximum_mask_cache_bytes,
};
use crate::failure::CODE_NUMERIC_OVERFLOW;

const BACKEND: BackendId = BackendId::new(77);
type TestResult<T = ()> = Result<T, String>;

#[test]
fn fixture_f32_components_and_cpu_cuda_reservations_are_source_named() -> TestResult {
    let (inputs, components) = fixture_components(DType::F32, 8, 16)?;
    assert_eq!(inputs.head_dimension, 4);
    assert_eq!(inputs.grouped_kv_width, 8);
    assert_eq!(inputs.half_conversion, 0);
    assert_eq!(inputs.grouped_query_expansion, 0);
    assert_eq!(inputs.mask_producing_prefill, 1);
    assert_eq!(
        components,
        ReservationComponents {
            cache_bytes_per_token: 64,
            persistent: PersistentComponents {
                token_staging: 32,
                kv_cache: 1_024,
                rope: 256,
                mask_cache: 192,
            },
            creation: CreationTransientComponents {
                cache_device_bytes: 136,
                cuda_host_source_bytes: 64,
            },
            execution: ExecutionTransientComponents {
                mask_source_bytes: 128,
                input_tensor_bytes: 32,
                attention: AttentionPhaseComponents {
                    qkv_projection: 768,
                    qk_layout_copy: 512,
                    qk_rotary_output: 512,
                    cache_replacement: 1_024,
                    grouped_query_expansion: 0,
                    f32_qkv_conversion: 0,
                    attention_score: 3_072,
                    masked_fill_scalar: 4,
                    f32_value_contiguous: 256,
                    f32_attention_value: 256,
                    attention_value_cast: 0,
                    output_projection: 256,
                    cache_replacement_phase: 2_816,
                    first_attention_compute_phase: 4_100,
                    cached_attention_compute_phase: 5_380,
                    attention_compute_phase: 5_380,
                    phase: 5_892,
                },
                residual_add_phase_bytes: 1_024,
                mlp_gate_up_phase_bytes: 3_072,
                mlp_down_projection_phase_bytes: 1_792,
                mlp_phase_bytes: 3_072,
                final_block_add_phase_bytes: 1_536,
                block_peak_bytes: 5_892,
                embedding_phase_bytes: 256,
                final_logits_phase_bytes: 608,
                model_forward_peak_bytes: 5_924,
                cuda_logits_transfer_bytes: 64,
            },
        }
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?,
        SequenceReservation {
            persistent_footprint: host_working(1_504),
            transient_footprint: host_working(6_052),
            total_footprint: host_working(7_556),
        }
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?,
        SequenceReservation {
            persistent_footprint: split_working(32, 1_472),
            transient_footprint: split_working(128, 5_924),
            total_footprint: split_working(160, 7_396),
        }
    );
    Ok(())
}

#[test]
fn fixture_f16_and_bf16_components_and_reservations_are_exactly_equal() -> TestResult {
    let (inputs, f16) = fixture_components(DType::F16, 8, 16)?;
    let (_, bf16) = fixture_components(DType::BF16, 8, 16)?;
    assert_eq!(inputs.half_conversion, 1);
    assert_eq!(f16, bf16);
    assert_eq!(
        f16.persistent,
        PersistentComponents {
            token_staging: 32,
            kv_cache: 512,
            rope: 128,
            mask_cache: 192,
        }
    );
    assert_eq!(f16.creation.cache_device_bytes, 264);
    assert_eq!(f16.execution.attention.phase, 6_020);
    assert_eq!(f16.execution.block_peak_bytes, 6_020);
    assert_eq!(f16.execution.model_forward_peak_bytes, 6_052);
    assert_eq!(
        reservation(&f16, DeviceKind::Cpu)?,
        SequenceReservation {
            persistent_footprint: host_working(864),
            transient_footprint: host_working(6_180),
            total_footprint: host_working(7_044),
        }
    );
    assert_eq!(
        reservation(&f16, DeviceKind::Cuda)?,
        SequenceReservation {
            persistent_footprint: split_working(32, 832),
            transient_footprint: split_working(128, 6_052),
            total_footprint: split_working(160, 6_884),
        }
    );
    Ok(())
}

#[test]
fn single_token_prefill_elides_mask_and_uses_mask_free_attention_phase() -> TestResult {
    let (f32_inputs, f32) = fixture_components(DType::F32, 1, 16)?;
    assert_eq!(f32_inputs.mask_producing_prefill, 0);
    assert_eq!(f32.persistent.mask_cache, 0);
    assert_eq!(f32.execution.mask_source_bytes, 0);
    assert_eq!(f32.execution.attention.attention_score, 256);
    assert_eq!(f32.execution.attention.masked_fill_scalar, 0);
    assert_eq!(f32.execution.model_forward_peak_bytes, 1_316);
    assert_eq!(
        reservation(&f32, DeviceKind::Cpu)?.total_footprint,
        host_working(2_600)
    );

    let (half_inputs, half) = fixture_components(DType::F16, 1, 16)?;
    assert_eq!(half_inputs.mask_producing_prefill, 0);
    assert_eq!(half.execution.attention.f32_qkv_conversion, 1_056);
    assert_eq!(half.execution.model_forward_peak_bytes, 1_524);
    assert_eq!(
        reservation(&half, DeviceKind::Cpu)?.total_footprint,
        host_working(2_168)
    );
    Ok(())
}

#[test]
fn grouped_and_non_grouped_query_components_are_distinct() -> TestResult {
    let (non_gqa, _) = fixture_components(DType::F32, 8, 16)?;
    assert_eq!(non_gqa.grouped_query_expansion, 0);

    let config = Config {
        hidden_size: 16,
        intermediate_size: 24,
        vocab_size: 20,
        num_hidden_layers: 2,
        num_attention_heads: 4,
        num_key_value_heads: 2,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 32,
        tie_word_embeddings: false,
    };
    let inputs =
        ReservationInputs::new(BACKEND, &config, DType::F32, sequence_configuration(7, 3)?)
            .map_err(debug_error)?;
    let components = inputs.components(BACKEND).map_err(debug_error)?;
    assert_eq!(inputs.grouped_query_expansion, 1);
    assert_eq!(components.cache_bytes_per_token, 128);
    assert_eq!(components.execution.attention.cache_replacement, 448);
    assert_eq!(components.execution.attention.grouped_query_expansion, 896);
    assert_eq!(components.execution.attention.f32_value_contiguous, 0);
    assert_eq!(components.execution.attention.phase, 3_636);
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?,
        SequenceReservation {
            persistent_footprint: host_working(1_455),
            transient_footprint: host_working(3_669),
            total_footprint: host_working(5_124),
        }
    );
    Ok(())
}

#[test]
fn transformer_depth_scales_only_persistent_cache() -> TestResult {
    let mut one_layer = tinyllama_config();
    one_layer.num_hidden_layers = 1;
    let one = ReservationInputs::new(
        BACKEND,
        &one_layer,
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?
    .components(BACKEND)
    .map_err(debug_error)?;
    let twenty_two = ReservationInputs::new(
        BACKEND,
        &tinyllama_config(),
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?
    .components(BACKEND)
    .map_err(debug_error)?;

    assert_eq!(twenty_two.execution, one.execution);
    assert_eq!(twenty_two.creation, one.creation);
    assert_eq!(
        twenty_two.cache_bytes_per_token,
        one.cache_bytes_per_token * 22
    );
    assert_eq!(twenty_two.persistent.kv_cache, one.persistent.kv_cache * 22);
    Ok(())
}

#[test]
fn tinyllama_full_context_reservation_locks_named_phase_totals() -> TestResult {
    let components = ReservationInputs::new(
        BACKEND,
        &tinyllama_config(),
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?
    .components(BACKEND)
    .map_err(debug_error)?;

    assert_eq!(components.cache_bytes_per_token, 22_528);
    assert_eq!(components.persistent.kv_cache, 46_137_344);
    assert_eq!(components.persistent.rope, 262_144);
    assert_eq!(components.persistent.mask_cache, 4_194_304);
    assert_eq!(
        components.execution.attention.attention_score,
        1_610_612_736
    );
    assert_eq!(components.execution.attention.phase, 1_757_413_380);
    assert_eq!(components.execution.block_peak_bytes, 1_757_413_380);
    assert_eq!(components.execution.model_forward_peak_bytes, 1_757_421_572);
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?,
        SequenceReservation {
            persistent_footprint: host_working(50_601_984),
            transient_footprint: host_working(1_761_615_876),
            total_footprint: host_working(1_812_217_860),
        }
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?,
        SequenceReservation {
            persistent_footprint: split_working(8_192, 50_593_792),
            transient_footprint: split_working(4_194_304, 1_757_421_572),
            total_footprint: split_working(4_202_496, 1_808_015_364),
        }
    );
    Ok(())
}

#[test]
fn mask_bound_covers_every_small_repeated_prefill_schedule() -> TestResult {
    for maximum_tokens in 1_u64..=8 {
        for maximum_prefill in 1_u64..=maximum_tokens {
            let bound = maximum_mask_cache_bytes(BACKEND, maximum_tokens, maximum_prefill)
                .map_err(debug_error)?;
            let mut observed_maximum = 0_u64;
            enumerate_schedules(
                maximum_tokens,
                maximum_prefill,
                0,
                0,
                bound,
                &mut observed_maximum,
            );
            assert!(observed_maximum <= bound);
            if maximum_prefill == 1 {
                assert_eq!(bound, 0);
            }
        }
    }
    assert_eq!(
        maximum_mask_cache_bytes(BACKEND, 4, 2).map_err(debug_error)?,
        12
    );
    assert_eq!(
        maximum_mask_cache_bytes(BACKEND, 5, 3).map_err(debug_error)?,
        20
    );
    Ok(())
}

#[test]
fn creation_transient_can_dominate_tiny_execution() -> TestResult {
    let config = Config {
        hidden_size: 2,
        intermediate_size: 1,
        vocab_size: 1,
        num_hidden_layers: 1,
        num_attention_heads: 1,
        num_key_value_heads: 1,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 100,
        tie_word_embeddings: false,
    };
    let components =
        ReservationInputs::new(BACKEND, &config, DType::F16, sequence_configuration(1, 1)?)
            .map_err(debug_error)?
            .components(BACKEND)
            .map_err(debug_error)?;
    assert_eq!(components.creation.cache_device_bytes, 804);
    assert_eq!(components.execution.model_forward_peak_bytes, 88);
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?.total_footprint,
        host_working(1_216)
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?.total_footprint,
        split_working(404, 1_212)
    );
    Ok(())
}

#[test]
fn invalid_upstream_assumptions_and_numeric_overflow_fail_closed() -> TestResult {
    let mut flash = fixture_config();
    flash.use_flash_attn = true;
    assert_eq!(
        calculate(
            BACKEND,
            &flash,
            DType::F32,
            DeviceKind::Cpu,
            sequence_configuration(16, 8)?,
            64,
        ),
        Err(ModelError::Unsupported)
    );

    let mut odd_head_dimension = fixture_config();
    odd_head_dimension.hidden_size = 6;
    assert_eq!(
        calculate(
            BACKEND,
            &odd_head_dimension,
            DType::F32,
            DeviceKind::Cpu,
            sequence_configuration(16, 8)?,
            64,
        ),
        Err(ModelError::Unsupported)
    );

    let mut upstream_cache_narrowing_bug = fixture_config();
    upstream_cache_narrowing_bug.max_position_embeddings = 1;
    assert_eq!(
        calculate(
            BACKEND,
            &upstream_cache_narrowing_bug,
            DType::F32,
            DeviceKind::Cpu,
            sequence_configuration(1, 1)?,
            64,
        ),
        Err(ModelError::Unsupported)
    );

    assert!(matches!(
        calculate(
            BACKEND,
            &fixture_config(),
            DType::F32,
            DeviceKind::Cpu,
            sequence_configuration(16, 8)?,
            63,
        ),
        Err(ModelError::Backend(failure))
            if failure.kind == BackendFailureKind::InvalidModel
                && failure.code == CODE_NUMERIC_OVERFLOW
    ));

    let mut overflowing = fixture_config();
    overflowing.max_position_embeddings = u32::MAX as usize;
    let maximum = NonZeroU32::new(u32::MAX).ok_or_else(|| "u32 max is zero".to_owned())?;
    let error = calculate(
        BACKEND,
        &overflowing,
        DType::F32,
        DeviceKind::Cpu,
        SequenceConfiguration::new(maximum, maximum),
        64,
    )
    .err()
    .ok_or_else(|| "overflowing reservation was accepted".to_owned())?;
    assert!(matches!(
        error,
        ModelError::Backend(failure)
            if failure.kind == BackendFailureKind::InvalidModel
                && failure.code == CODE_NUMERIC_OVERFLOW
    ));
    Ok(())
}

fn fixture_components(
    dtype: DType,
    maximum_prefill: u32,
    maximum_tokens: u32,
) -> TestResult<(ReservationInputs, ReservationComponents)> {
    let inputs = ReservationInputs::new(
        BACKEND,
        &fixture_config(),
        dtype,
        sequence_configuration(maximum_tokens, maximum_prefill)?,
    )
    .map_err(debug_error)?;
    let components = inputs.components(BACKEND).map_err(debug_error)?;
    Ok((inputs, components))
}

fn tinyllama_config() -> Config {
    Config {
        hidden_size: 2_048,
        intermediate_size: 5_632,
        vocab_size: 32_000,
        num_hidden_layers: 22,
        num_attention_heads: 32,
        num_key_value_heads: 4,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 2_048,
        tie_word_embeddings: false,
    }
}

fn fixture_config() -> Config {
    Config {
        hidden_size: 8,
        intermediate_size: 16,
        vocab_size: 16,
        num_hidden_layers: 1,
        num_attention_heads: 2,
        num_key_value_heads: 2,
        use_flash_attn: false,
        rms_norm_eps: 1e-5,
        rope_theta: 10_000.0,
        bos_token_id: None,
        eos_token_id: None,
        rope_scaling: None,
        max_position_embeddings: 16,
        tie_word_embeddings: false,
    }
}

fn sequence_configuration(
    maximum_tokens: u32,
    maximum_prefill: u32,
) -> TestResult<SequenceConfiguration> {
    let maximum_tokens = NonZeroU32::new(maximum_tokens)
        .ok_or_else(|| "maximum tokens must be nonzero".to_owned())?;
    let maximum_prefill = NonZeroU32::new(maximum_prefill)
        .ok_or_else(|| "maximum prefill must be nonzero".to_owned())?;
    Ok(SequenceConfiguration::new(maximum_tokens, maximum_prefill))
}

fn reservation(
    components: &ReservationComponents,
    device_kind: DeviceKind,
) -> TestResult<SequenceReservation> {
    (*components)
        .reservation(BACKEND, device_kind)
        .map_err(debug_error)
}

const fn host_working(bytes: u64) -> MemoryFootprint {
    split_working(bytes, 0)
}

const fn split_working(host: u64, device: u64) -> MemoryFootprint {
    MemoryFootprint {
        host_weight_bytes: 0,
        device_weight_bytes: 0,
        host_working_bytes: host,
        device_working_bytes: device,
    }
}

fn enumerate_schedules(
    remaining: u64,
    maximum_prefill: u64,
    position: u64,
    retained_mask_bytes: u64,
    bound: u64,
    observed_maximum: &mut u64,
) {
    if remaining == 0 {
        *observed_maximum = (*observed_maximum).max(retained_mask_bytes);
        assert!(retained_mask_bytes <= bound);
        return;
    }

    for batch in 1..=remaining.min(maximum_prefill) {
        let next_position = position + batch;
        let additional = if batch == 1 { 0 } else { batch * next_position };
        enumerate_schedules(
            remaining - batch,
            maximum_prefill,
            next_position,
            retained_mask_bytes + additional,
            bound,
            observed_maximum,
        );
    }
}

fn debug_error(error: ModelError) -> String {
    format!("{error:?}")
}
