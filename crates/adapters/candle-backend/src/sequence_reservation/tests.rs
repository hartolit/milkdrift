use std::num::NonZeroU32;

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, MemoryFootprint, ModelError, SequenceConfiguration,
    SequenceReservation,
};

use super::{LlamaMemoryGeometry, ReservationComponents, calculate, maximum_mask_cache_bytes};
use crate::failure::CODE_NUMERIC_OVERFLOW;

const BACKEND: BackendId = BackendId::new(77);
type TestResult<T = ()> = Result<T, String>;

#[test]
fn tiny_f32_geometry_has_hand_derived_cpu_and_cuda_boundary_vectors() -> TestResult {
    let (inputs, components) = fixture_components(DType::F32, 8, 16)?;
    assert_eq!(inputs.head_dimension, 4);
    assert_eq!(inputs.grouped_kv_width, 8);

    // Independent reviewed boundary vector for the synthetic 1-layer fixture.
    // Persistent F32: token ids 16*2 + KV 2*16*2*4*4 + rope 16*4*4
    // + repeated-prefill masks 192 = 1_504. The transient values are reviewed
    // maximum live phase totals, not snapshots of the planner's local fields.
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?,
        expected_reservation(host_working(1_504), host_working(6_052))?
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?,
        expected_reservation(split_working(32, 1_472), split_working(128, 5_924))?
    );
    Ok(())
}

#[test]
fn selected_half_execution_types_have_equal_boundary_reservations() -> TestResult {
    let (inputs, f16) = fixture_components(DType::F16, 8, 16)?;
    let (_, bf16) = fixture_components(DType::BF16, 8, 16)?;
    assert_eq!(inputs.half_conversion, 1);
    assert_eq!(
        reservation(&f16, DeviceKind::Cpu)?,
        reservation(&bf16, DeviceKind::Cpu)?
    );
    assert_eq!(
        reservation(&f16, DeviceKind::Cuda)?,
        reservation(&bf16, DeviceKind::Cuda)?
    );
    assert_eq!(
        reservation(&f16, DeviceKind::Cpu)?,
        expected_reservation(host_working(864), host_working(6_180))?
    );
    assert_eq!(
        reservation(&f16, DeviceKind::Cuda)?,
        expected_reservation(split_working(32, 832), split_working(128, 6_052))?
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
        reservation(&f32, DeviceKind::Cpu)?.total_footprint(),
        host_working(2_600)
    );

    let (half_inputs, half) = fixture_components(DType::F16, 1, 16)?;
    assert_eq!(half_inputs.mask_producing_prefill, 0);
    assert_eq!(half.execution.attention.f32_qkv_conversion, 1_056);
    assert_eq!(half.execution.model_forward_peak_bytes, 1_524);
    assert_eq!(
        reservation(&half, DeviceKind::Cpu)?.total_footprint(),
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
        LlamaMemoryGeometry::new(BACKEND, &config, DType::F32, sequence_configuration(7, 3)?)
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
        expected_reservation(host_working(1_455), host_working(3_669))?
    );
    Ok(())
}

#[test]
fn transformer_depth_scales_only_persistent_cache() -> TestResult {
    let mut one_layer = tinyllama_config();
    one_layer.num_hidden_layers = 1;
    let one = LlamaMemoryGeometry::new(
        BACKEND,
        &one_layer,
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?
    .components(BACKEND)
    .map_err(debug_error)?;
    let twenty_two = LlamaMemoryGeometry::new(
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
fn reservation_is_monotonic_in_token_and_prefill_capacity() -> TestResult {
    for device_kind in [DeviceKind::Cpu, DeviceKind::Cuda] {
        let mut previous = MemoryFootprint::ZERO;
        for maximum_tokens in 1..=16 {
            let current = calculate(
                BACKEND,
                &fixture_config(),
                DType::F32,
                device_kind,
                sequence_configuration(maximum_tokens, 1)?,
            )
            .map_err(debug_error)?
            .total_footprint();
            assert!(current.contains_components(previous));
            previous = current;
        }

        let mut previous = MemoryFootprint::ZERO;
        for maximum_prefill in 1..=16 {
            let current = calculate(
                BACKEND,
                &fixture_config(),
                DType::F32,
                device_kind,
                sequence_configuration(16, maximum_prefill)?,
            )
            .map_err(debug_error)?
            .total_footprint();
            assert!(current.contains_components(previous));
            previous = current;
        }
    }
    Ok(())
}

#[test]
fn planning_is_deterministic_and_aggregate_is_the_checked_component_sum() -> TestResult {
    for dtype in [DType::F32, DType::F16, DType::BF16] {
        for device_kind in [DeviceKind::Cpu, DeviceKind::Cuda] {
            let configuration = sequence_configuration(16, 8)?;
            let first = calculate(
                BACKEND,
                &fixture_config(),
                dtype,
                device_kind,
                configuration,
            )
            .map_err(debug_error)?;
            let second = calculate(
                BACKEND,
                &fixture_config(),
                dtype,
                device_kind,
                configuration,
            )
            .map_err(debug_error)?;
            assert_eq!(first, second);
            assert_eq!(
                first
                    .persistent_footprint()
                    .checked_add(first.transient_footprint()),
                Some(first.total_footprint())
            );
        }
    }
    Ok(())
}

#[test]
fn execution_device_policy_places_working_payload_in_expected_domains() -> TestResult {
    let configuration = sequence_configuration(16, 8)?;
    let cpu = calculate(
        BACKEND,
        &fixture_config(),
        DType::F32,
        DeviceKind::Cpu,
        configuration,
    )
    .map_err(debug_error)?;
    let cuda = calculate(
        BACKEND,
        &fixture_config(),
        DType::F32,
        DeviceKind::Cuda,
        configuration,
    )
    .map_err(debug_error)?;

    assert!(
        cpu.total_footprint()
            .checked_device_bytes()
            .is_some_and(|bytes| bytes.is_zero())
    );
    assert!(
        cuda.total_footprint()
            .device_working_bytes()
            .contains(domain_contracts::ByteCount::from_u64(1))
    );
    assert!(
        cpu.total_footprint()
            .host_working_bytes()
            .contains(cuda.total_footprint().host_working_bytes())
    );
    Ok(())
}

#[test]
fn tinyllama_full_context_has_reviewed_public_boundary_vectors() -> TestResult {
    let components = LlamaMemoryGeometry::new(
        BACKEND,
        &tinyllama_config(),
        DType::F16,
        sequence_configuration(2_048, 2_048)?,
    )
    .map_err(debug_error)?
    .components(BACKEND)
    .map_err(debug_error)?;

    // Provenance: TinyLlama/TinyLlama-1.1B-Chat-v1.0 configuration at the
    // repository-pinned revision. These are final reservation boundaries only;
    // planner-local intermediate values are intentionally not snapshotted.
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?,
        expected_reservation(host_working(50_601_984), host_working(1_761_615_876),)?
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?,
        expected_reservation(
            split_working(8_192, 50_593_792),
            split_working(4_194_304, 1_757_421_572),
        )?
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
fn creation_transient_is_included_when_it_dominates_tiny_execution() -> TestResult {
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
        LlamaMemoryGeometry::new(BACKEND, &config, DType::F16, sequence_configuration(1, 1)?)
            .map_err(debug_error)?
            .components(BACKEND)
            .map_err(debug_error)?;
    // This edge vector is deliberately shaped so cache construction, not a
    // forward pass, owns the public transient maximum.
    assert_eq!(
        reservation(&components, DeviceKind::Cpu)?.total_footprint(),
        host_working(1_216)
    );
    assert_eq!(
        reservation(&components, DeviceKind::Cuda)?.total_footprint(),
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
        ),
        Err(ModelError::Unsupported)
    );

    let mut overflowing = fixture_config();
    overflowing.max_position_embeddings = u32::MAX as usize;
    let maximum = NonZeroU32::new(u32::MAX).ok_or_else(|| "u32 max is zero".to_owned())?;
    let error = calculate(
        BACKEND,
        &overflowing,
        DType::F32,
        DeviceKind::Cpu,
        SequenceConfiguration::new(maximum, maximum),
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
) -> TestResult<(LlamaMemoryGeometry, ReservationComponents)> {
    let inputs = LlamaMemoryGeometry::new(
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

fn expected_reservation(
    persistent: MemoryFootprint,
    transient: MemoryFootprint,
) -> TestResult<SequenceReservation> {
    SequenceReservation::checked(persistent, transient)
        .ok_or_else(|| "independent expected reservation overflowed".to_owned())
}

const fn host_working(bytes: u64) -> MemoryFootprint {
    split_working(bytes, 0)
}

const fn split_working(host: u64, device: u64) -> MemoryFootprint {
    MemoryFootprint::host_working(domain_contracts::ByteCount::from_u64(host))
        .with_device_working_bytes(domain_contracts::ByteCount::from_u64(device))
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
