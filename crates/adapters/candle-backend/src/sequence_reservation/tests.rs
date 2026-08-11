use std::num::NonZeroU32;

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, MemoryFootprint, ModelError, SequenceConfiguration,
};

use super::{ReservationComponents, ReservationInputs, calculate, maximum_mask_cache_bytes};
use crate::failure::CODE_NUMERIC_OVERFLOW;

const BACKEND: BackendId = BackendId::new(77);
type TestResult<T = ()> = Result<T, String>;

#[test]
fn fixture_f32_components_and_cpu_cuda_phases_are_exact() -> TestResult {
    let (inputs, components) = fixture_components(DType::F32, 8, 16)?;
    assert_eq!(inputs.head_dimension, 4);
    assert_eq!(inputs.grouped_kv_width, 8);
    assert_eq!(inputs.half_conversion, 0);
    assert_eq!(inputs.grouped_query_expansion, 0);
    assert_eq!(inputs.batched_prefill, 1);
    assert_eq!(
        components,
        ReservationComponents {
            cache_bytes_per_token: 64,
            kv_cache_bytes: 1_024,
            rope_bytes: 256,
            mask_cache_bytes: 192,
            token_staging_bytes: 32,
            mask_source_bytes: 128,
            cache_creation_device_bytes: 392,
            cache_creation_cuda_host_bytes: 64,
            block_forward_peak_bytes: 11_524,
            model_forward_peak_bytes: 12_292,
            cpu_creation_host_bytes: 424,
            cpu_forward_host_bytes: 13_924,
            cuda_forward_device_bytes: 13_764,
            cuda_creation_host_bytes: 96,
            cuda_forward_host_bytes: 228,
        }
    );
    assert_eq!(
        footprint(components, DeviceKind::Cpu)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 13_924,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        footprint(components, DeviceKind::Cuda)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 228,
            device_working_bytes: 13_764,
        }
    );
    Ok(())
}

#[test]
fn fixture_f16_and_bf16_components_and_cpu_cuda_phases_are_exact() -> TestResult {
    let (inputs, f16) = fixture_components(DType::F16, 8, 16)?;
    let (_, bf16) = fixture_components(DType::BF16, 8, 16)?;
    assert_eq!(inputs.half_conversion, 1);
    assert_eq!(f16, bf16);
    assert_eq!(
        f16,
        ReservationComponents {
            cache_bytes_per_token: 32,
            kv_cache_bytes: 512,
            rope_bytes: 128,
            mask_cache_bytes: 192,
            token_staging_bytes: 32,
            mask_source_bytes: 128,
            cache_creation_device_bytes: 392,
            cache_creation_cuda_host_bytes: 64,
            block_forward_peak_bytes: 9_604,
            model_forward_peak_bytes: 10_132,
            cpu_creation_host_bytes: 424,
            cpu_forward_host_bytes: 11_124,
            cuda_forward_device_bytes: 10_964,
            cuda_creation_host_bytes: 96,
            cuda_forward_host_bytes: 228,
        }
    );
    assert_eq!(
        footprint(f16, DeviceKind::Cpu)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 11_124,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        footprint(f16, DeviceKind::Cuda)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 228,
            device_working_bytes: 10_964,
        }
    );
    Ok(())
}

#[test]
fn single_token_prefill_elides_mask_and_batched_source_terms() -> TestResult {
    let (f32_inputs, f32) = fixture_components(DType::F32, 1, 16)?;
    assert_eq!(f32_inputs.batched_prefill, 0);
    assert_eq!(
        f32,
        ReservationComponents {
            cache_bytes_per_token: 64,
            kv_cache_bytes: 1_024,
            rope_bytes: 256,
            mask_cache_bytes: 0,
            token_staging_bytes: 4,
            mask_source_bytes: 0,
            cache_creation_device_bytes: 392,
            cache_creation_cuda_host_bytes: 64,
            block_forward_peak_bytes: 2_656,
            model_forward_peak_bytes: 2_820,
            cpu_creation_host_bytes: 396,
            cpu_forward_host_bytes: 4_104,
            cuda_forward_device_bytes: 4_100,
            cuda_creation_host_bytes: 68,
            cuda_forward_host_bytes: 68,
        }
    );

    let (half_inputs, half) = fixture_components(DType::F16, 1, 16)?;
    assert_eq!(half_inputs.batched_prefill, 0);
    assert_eq!(
        half,
        ReservationComponents {
            cache_bytes_per_token: 32,
            kv_cache_bytes: 512,
            rope_bytes: 128,
            mask_cache_bytes: 0,
            token_staging_bytes: 4,
            mask_source_bytes: 0,
            cache_creation_device_bytes: 392,
            cache_creation_cuda_host_bytes: 64,
            block_forward_peak_bytes: 2_864,
            model_forward_peak_bytes: 3_012,
            cpu_creation_host_bytes: 396,
            cpu_forward_host_bytes: 3_656,
            cuda_forward_device_bytes: 3_652,
            cuda_creation_host_bytes: 68,
            cuda_forward_host_bytes: 68,
        }
    );
    Ok(())
}

#[test]
fn gqa_and_non_gqa_layer_terms_are_both_exact() -> TestResult {
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
    let configuration = sequence_configuration(7, 3)?;
    let inputs = ReservationInputs::new(BACKEND, &config, DType::F32, configuration)
        .map_err(debug_error)?;
    let components = inputs.components(BACKEND).map_err(debug_error)?;
    assert_eq!(inputs.head_dimension, 4);
    assert_eq!(inputs.grouped_kv_width, 8);
    assert_eq!(inputs.grouped_query_expansion, 1);
    assert_eq!(
        components,
        ReservationComponents {
            cache_bytes_per_token: 128,
            kv_cache_bytes: 896,
            rope_bytes: 512,
            mask_cache_bytes: 35,
            token_staging_bytes: 12,
            mask_source_bytes: 21,
            cache_creation_device_bytes: 776,
            cache_creation_cuda_host_bytes: 128,
            block_forward_peak_bytes: 6_788,
            model_forward_peak_bytes: 7_349,
            cpu_creation_host_bytes: 788,
            cpu_forward_host_bytes: 8_825,
            cuda_forward_device_bytes: 8_792,
            cuda_creation_host_bytes: 140,
            cuda_forward_host_bytes: 121,
        }
    );
    Ok(())
}

#[test]
fn transformer_depth_scales_persistent_cache_not_block_transient_peak() -> TestResult {
    let mut one_layer = tinyllama_config();
    one_layer.num_hidden_layers = 1;
    let one_layer_inputs = ReservationInputs::new(
        BACKEND,
        &one_layer,
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?;
    let one_layer_components = one_layer_inputs.components(BACKEND).map_err(debug_error)?;

    let twenty_two_layer_inputs = ReservationInputs::new(
        BACKEND,
        &tinyllama_config(),
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?;
    let twenty_two_layer_components = twenty_two_layer_inputs
        .components(BACKEND)
        .map_err(debug_error)?;

    assert_eq!(
        twenty_two_layer_components.block_forward_peak_bytes,
        one_layer_components.block_forward_peak_bytes
    );
    assert_eq!(
        twenty_two_layer_components.model_forward_peak_bytes,
        one_layer_components.model_forward_peak_bytes
    );
    assert_eq!(
        twenty_two_layer_components.cache_bytes_per_token,
        one_layer_components
            .cache_bytes_per_token
            .checked_mul(22)
            .ok_or_else(|| "cache scaling overflowed".to_owned())?
    );
    assert_eq!(
        twenty_two_layer_components.kv_cache_bytes,
        one_layer_components
            .kv_cache_bytes
            .checked_mul(22)
            .ok_or_else(|| "KV-cache scaling overflowed".to_owned())?
    );
    Ok(())
}

#[test]
fn tinyllama_full_context_reservation_uses_one_block_peak_and_all_layer_cache() -> TestResult {
    let inputs = ReservationInputs::new(
        BACKEND,
        &tinyllama_config(),
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?;
    let components = inputs.components(BACKEND).map_err(debug_error)?;

    assert_eq!(
        components,
        ReservationComponents {
            cache_bytes_per_token: 22_528,
            kv_cache_bytes: 46_137_344,
            rope_bytes: 262_144,
            mask_cache_bytes: 4_194_304,
            token_staging_bytes: 8_192,
            mask_source_bytes: 4_194_304,
            cache_creation_device_bytes: 786_560,
            cache_creation_cuda_host_bytes: 8_192,
            block_forward_peak_bytes: 2_438_987_780,
            model_forward_peak_bytes: 2_460_163_588,
            cpu_creation_host_bytes: 794_752,
            cpu_forward_host_bytes: 2_514_959_876,
            cuda_forward_device_bytes: 2_510_757_380,
            cuda_creation_host_bytes: 16_384,
            cuda_forward_host_bytes: 4_330_584,
        }
    );
    assert_eq!(
        footprint(components, DeviceKind::Cpu)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 2_514_959_876,
            device_working_bytes: 0,
        }
    );
    assert_eq!(
        footprint(components, DeviceKind::Cuda)?,
        MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 0,
            host_working_bytes: 4_330_584,
            device_working_bytes: 2_510_757_380,
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

    let maximum = u64::from(u32::MAX);
    let expected = maximum
        .checked_mul(maximum)
        .ok_or_else(|| "u32 maximum square unexpectedly overflowed u64".to_owned())?;
    assert_eq!(
        maximum_mask_cache_bytes(BACKEND, maximum, maximum).map_err(debug_error)?,
        expected
    );
    Ok(())
}

#[test]
fn cache_creation_can_dominate_cpu_cuda_device_and_cuda_host_phases() -> TestResult {
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
    let inputs =
        ReservationInputs::new(BACKEND, &config, DType::F16, sequence_configuration(1, 1)?)
            .map_err(debug_error)?;
    let components = inputs.components(BACKEND).map_err(debug_error)?;
    assert_eq!(
        components,
        ReservationComponents {
            cache_bytes_per_token: 8,
            kv_cache_bytes: 8,
            rope_bytes: 400,
            mask_cache_bytes: 0,
            token_staging_bytes: 4,
            mask_source_bytes: 0,
            cache_creation_device_bytes: 1_204,
            cache_creation_cuda_host_bytes: 400,
            block_forward_peak_bytes: 128,
            model_forward_peak_bytes: 150,
            cpu_creation_host_bytes: 1_208,
            cpu_forward_host_bytes: 562,
            cuda_forward_device_bytes: 558,
            cuda_creation_host_bytes: 404,
            cuda_forward_host_bytes: 8,
        }
    );
    assert_eq!(
        footprint(components, DeviceKind::Cpu)?.host_working_bytes,
        1_208
    );
    let cuda = footprint(components, DeviceKind::Cuda)?;
    assert_eq!(cuda.device_working_bytes, 1_204);
    assert_eq!(cuda.host_working_bytes, 404);
    Ok(())
}

#[test]
fn invalid_locked_assumptions_and_numeric_overflow_fail_closed() -> TestResult {
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

    let mut short_context_many_kv_heads = fixture_config();
    short_context_many_kv_heads.max_position_embeddings = 1;
    let reservation = calculate(
        BACKEND,
        &short_context_many_kv_heads,
        DType::F32,
        DeviceKind::Cpu,
        sequence_configuration(1, 1)?,
        64,
    );
    assert!(
        reservation.is_ok(),
        "KV-head count and context length are independent model dimensions"
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
    let maximum =
        NonZeroU32::new(u32::MAX).ok_or_else(|| "u32 maximum must be nonzero".to_owned())?;
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

fn footprint(
    components: ReservationComponents,
    device_kind: DeviceKind,
) -> TestResult<MemoryFootprint> {
    components.footprint(device_kind).map_err(debug_error)
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
