//! Checked logical-payload reservation for the reviewed Candle Llama sequence.
//!
//! The formulas in this module are locked to batch-one, non-flash
//! `candle-transformers` Llama 0.11.0. They conservatively bound reviewed live
//! logical tensor payload and source-transfer bytes across sequence creation and
//! every permitted prefill/decode schedule. They do not model physical RSS/VRAM,
//! allocator rounding, fragmentation or pools, CUDA context/driver allocations,
//! native workspaces, or metadata. Caller-owned logits workspace is reserved
//! separately and is not included here.

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, DeviceKind, MemoryFootprint, ModelError, SequenceConfiguration,
};

use crate::failure::{CODE_NUMERIC_OVERFLOW, failure};

const REVIEWED_MAX_HIDDEN_LAYERS: u64 = 256;

pub(super) fn calculate(
    backend: BackendId,
    config: &Config,
    dtype: DType,
    device_kind: DeviceKind,
    configuration: SequenceConfiguration,
    expected_cache_bytes_per_token: u64,
) -> Result<MemoryFootprint, ModelError> {
    if !matches!(device_kind, DeviceKind::Cpu | DeviceKind::Cuda) {
        return Err(ModelError::Unsupported);
    }
    let inputs = ReservationInputs::new(backend, config, dtype, configuration)?;
    let components = inputs.components(backend)?;
    if components.cache_bytes_per_token != expected_cache_bytes_per_token {
        return Err(numeric_error(backend));
    }
    components.footprint(device_kind)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservationInputs {
    hidden_size: u64,
    intermediate_size: u64,
    attention_heads: u64,
    kv_heads: u64,
    head_dimension: u64,
    grouped_kv_width: u64,
    vocabulary_size: u64,
    hidden_layers: u64,
    maximum_positions: u64,
    maximum_prefill: u64,
    maximum_tokens: u64,
    dtype_bytes: u64,
    half_conversion: u64,
    grouped_query_expansion: u64,
    batched_prefill: u64,
}

impl ReservationInputs {
    fn new(
        backend: BackendId,
        config: &Config,
        dtype: DType,
        configuration: SequenceConfiguration,
    ) -> Result<Self, ModelError> {
        if config.use_flash_attn {
            return Err(ModelError::Unsupported);
        }

        let hidden_size = checked_usize_to_u64(backend, config.hidden_size)?;
        let intermediate_size = checked_usize_to_u64(backend, config.intermediate_size)?;
        let vocabulary_size = checked_usize_to_u64(backend, config.vocab_size)?;
        let hidden_layers = checked_usize_to_u64(backend, config.num_hidden_layers)?;
        let attention_heads = checked_usize_to_u64(backend, config.num_attention_heads)?;
        let kv_heads = checked_usize_to_u64(backend, config.num_key_value_heads)?;
        let maximum_positions = checked_usize_to_u64(backend, config.max_position_embeddings)?;
        let maximum_tokens = u64::from(configuration.maximum_tokens.get());
        let maximum_prefill = u64::from(configuration.maximum_prefill_batch.get());
        let dtype_bytes = match dtype {
            DType::F32 => 4,
            DType::F16 | DType::BF16 => 2,
            _ => return Err(ModelError::Unsupported),
        };

        let required_non_zero = [
            hidden_size,
            intermediate_size,
            vocabulary_size,
            hidden_layers,
            attention_heads,
            kv_heads,
            maximum_positions,
        ];
        if required_non_zero.contains(&0)
            || hidden_layers > REVIEWED_MAX_HIDDEN_LAYERS
            || !hidden_size.is_multiple_of(attention_heads)
            || !attention_heads.is_multiple_of(kv_heads)
            || kv_heads > maximum_positions
            || maximum_prefill > maximum_tokens
            || maximum_tokens > maximum_positions
        {
            return Err(ModelError::Unsupported);
        }

        let head_dimension = hidden_size / attention_heads;
        if !head_dimension.is_multiple_of(2) {
            return Err(ModelError::Unsupported);
        }
        let grouped_kv_width = checked_mul(backend, kv_heads, head_dimension)?;

        Ok(Self {
            hidden_size,
            intermediate_size,
            attention_heads,
            kv_heads,
            head_dimension,
            grouped_kv_width,
            vocabulary_size,
            hidden_layers,
            maximum_positions,
            maximum_prefill,
            maximum_tokens,
            dtype_bytes,
            half_conversion: u64::from(dtype_bytes == 2),
            grouped_query_expansion: u64::from(attention_heads > kv_heads),
            batched_prefill: u64::from(maximum_prefill > 1),
        })
    }

    fn components(self, backend: BackendId) -> Result<ReservationComponents, ModelError> {
        let cache_bytes_per_token = cache_bytes_per_token(backend, self)?;
        let kv_cache_bytes = checked_mul(backend, cache_bytes_per_token, self.maximum_tokens)?;
        let rope_bytes = checked_product(
            backend,
            &[
                self.maximum_positions,
                self.head_dimension,
                self.dtype_bytes,
            ],
        )?;
        let mask_cache_bytes =
            maximum_mask_cache_bytes(backend, self.maximum_tokens, self.maximum_prefill)?;
        let token_staging_bytes = checked_mul(backend, 4, self.maximum_prefill)?;
        let mask_source_bytes = checked_product(
            backend,
            &[
                self.batched_prefill,
                self.maximum_prefill,
                self.maximum_tokens,
            ],
        )?;
        let cache_creation_device_bytes = cache_creation_device_bytes(backend, self)?;
        let cache_creation_cuda_host_bytes = cache_creation_cuda_host_bytes(backend, self)?;
        let layer_forward_bytes = layer_forward_bytes(backend, self)?;
        let forward_bytes = complete_forward_bytes(backend, self, layer_forward_bytes)?;

        let cpu_creation_host_bytes =
            checked_sum(backend, &[token_staging_bytes, cache_creation_device_bytes])?;
        let cpu_forward_host_bytes = checked_sum(
            backend,
            &[
                token_staging_bytes,
                rope_bytes,
                kv_cache_bytes,
                mask_cache_bytes,
                forward_bytes,
                mask_source_bytes,
            ],
        )?;
        let cuda_forward_device_bytes = checked_sum(
            backend,
            &[rope_bytes, kv_cache_bytes, mask_cache_bytes, forward_bytes],
        )?;
        let cuda_creation_host_bytes = checked_sum(
            backend,
            &[token_staging_bytes, cache_creation_cuda_host_bytes],
        )?;
        let cuda_forward_host_bytes = checked_sum(
            backend,
            &[
                token_staging_bytes,
                mask_source_bytes,
                checked_product(backend, &[4, self.batched_prefill, self.hidden_layers])?,
                checked_mul(backend, 4, self.vocabulary_size)?,
            ],
        )?;

        Ok(ReservationComponents {
            cache_bytes_per_token,
            kv_cache_bytes,
            rope_bytes,
            mask_cache_bytes,
            token_staging_bytes,
            mask_source_bytes,
            cache_creation_device_bytes,
            cache_creation_cuda_host_bytes,
            layer_forward_bytes,
            forward_bytes,
            cpu_creation_host_bytes,
            cpu_forward_host_bytes,
            cuda_forward_device_bytes,
            cuda_creation_host_bytes,
            cuda_forward_host_bytes,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservationComponents {
    cache_bytes_per_token: u64,
    kv_cache_bytes: u64,
    rope_bytes: u64,
    mask_cache_bytes: u64,
    token_staging_bytes: u64,
    mask_source_bytes: u64,
    cache_creation_device_bytes: u64,
    cache_creation_cuda_host_bytes: u64,
    layer_forward_bytes: u64,
    forward_bytes: u64,
    cpu_creation_host_bytes: u64,
    cpu_forward_host_bytes: u64,
    cuda_forward_device_bytes: u64,
    cuda_creation_host_bytes: u64,
    cuda_forward_host_bytes: u64,
}

impl ReservationComponents {
    const fn footprint(self, device_kind: DeviceKind) -> Result<MemoryFootprint, ModelError> {
        match device_kind {
            DeviceKind::Cpu => Ok(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: if self.cpu_creation_host_bytes >= self.cpu_forward_host_bytes {
                    self.cpu_creation_host_bytes
                } else {
                    self.cpu_forward_host_bytes
                },
                device_working_bytes: 0,
            }),
            DeviceKind::Cuda => Ok(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: if self.cuda_creation_host_bytes >= self.cuda_forward_host_bytes
                {
                    self.cuda_creation_host_bytes
                } else {
                    self.cuda_forward_host_bytes
                },
                device_working_bytes: if self.cache_creation_device_bytes
                    >= self.cuda_forward_device_bytes
                {
                    self.cache_creation_device_bytes
                } else {
                    self.cuda_forward_device_bytes
                },
            }),
            _ => Err(ModelError::Unsupported),
        }
    }
}

fn cache_bytes_per_token(backend: BackendId, inputs: ReservationInputs) -> Result<u64, ModelError> {
    checked_product(
        backend,
        &[
            2,
            inputs.hidden_layers,
            inputs.grouped_kv_width,
            inputs.dtype_bytes,
        ],
    )
}

fn maximum_mask_cache_bytes(
    backend: BackendId,
    maximum_tokens: u64,
    maximum_prefill: u64,
) -> Result<u64, ModelError> {
    if maximum_prefill == 1 {
        return Ok(0);
    }

    // floor((T*T + T*M) / 2) is evaluated in u128 because the numerator
    // can exceed u64 even when the final conservative bound is representable.
    let tokens = u128::from(maximum_tokens);
    let prefill = u128::from(maximum_prefill);
    let token_square = tokens
        .checked_mul(tokens)
        .ok_or_else(|| numeric_error(backend))?;
    let token_prefill = tokens
        .checked_mul(prefill)
        .ok_or_else(|| numeric_error(backend))?;
    let numerator = token_square
        .checked_add(token_prefill)
        .ok_or_else(|| numeric_error(backend))?;
    u64::try_from(numerator / 2).map_err(|_| numeric_error(backend))
}

fn cache_creation_device_bytes(
    backend: BackendId,
    inputs: ReservationInputs,
) -> Result<u64, ModelError> {
    checked_sum(
        backend,
        &[
            checked_mul(backend, 2, inputs.head_dimension)?,
            checked_product(
                backend,
                &[6, inputs.maximum_positions, inputs.head_dimension],
            )?,
        ],
    )
}

fn cache_creation_cuda_host_bytes(
    backend: BackendId,
    inputs: ReservationInputs,
) -> Result<u64, ModelError> {
    let positions = checked_mul(backend, 4, inputs.maximum_positions)?;
    let head_dimension = checked_mul(backend, 4, inputs.head_dimension)?;
    Ok(positions.max(head_dimension))
}

fn layer_forward_bytes(backend: BackendId, inputs: ReservationInputs) -> Result<u64, ModelError> {
    let projected_hidden_coefficient = checked_add(backend, 10, inputs.half_conversion)?;
    let dtype_elements = checked_sum(
        backend,
        &[
            checked_product(
                backend,
                &[
                    projected_hidden_coefficient,
                    inputs.maximum_prefill,
                    inputs.hidden_size,
                ],
            )?,
            checked_product(
                backend,
                &[4, inputs.maximum_prefill, inputs.grouped_kv_width],
            )?,
            checked_product(
                backend,
                &[2, inputs.maximum_tokens, inputs.grouped_kv_width],
            )?,
            checked_product(
                backend,
                &[
                    2,
                    inputs.grouped_query_expansion,
                    inputs.maximum_tokens,
                    inputs.hidden_size,
                ],
            )?,
            checked_product(
                backend,
                &[4, inputs.maximum_prefill, inputs.intermediate_size],
            )?,
        ],
    )?;
    let half_conversion_bytes = if inputs.half_conversion == 0 {
        0
    } else {
        checked_sum(
            backend,
            &[
                checked_product(backend, &[4, inputs.maximum_prefill, inputs.hidden_size])?,
                checked_product(backend, &[8, inputs.maximum_tokens, inputs.hidden_size])?,
            ],
        )?
    };
    let attention_multiplier = checked_add(backend, 3, inputs.batched_prefill)?;

    checked_sum(
        backend,
        &[
            checked_mul(backend, inputs.dtype_bytes, dtype_elements)?,
            half_conversion_bytes,
            checked_product(backend, &[4, inputs.maximum_tokens, inputs.hidden_size])?,
            checked_product(backend, &[4, inputs.maximum_prefill, inputs.hidden_size])?,
            checked_product(
                backend,
                &[
                    4,
                    inputs.attention_heads,
                    inputs.maximum_prefill,
                    inputs.maximum_tokens,
                    attention_multiplier,
                ],
            )?,
            checked_mul(backend, 4, inputs.batched_prefill)?,
        ],
    )
}

fn complete_forward_bytes(
    backend: BackendId,
    inputs: ReservationInputs,
    layer_forward_bytes: u64,
) -> Result<u64, ModelError> {
    let prefill_hidden_bytes = checked_product(
        backend,
        &[
            inputs.dtype_bytes,
            inputs.maximum_prefill,
            inputs.hidden_size,
        ],
    )?;
    checked_sum(
        backend,
        &[
            checked_mul(backend, 4, inputs.maximum_prefill)?,
            prefill_hidden_bytes,
            checked_mul(backend, inputs.hidden_layers, layer_forward_bytes)?,
            prefill_hidden_bytes,
            checked_mul(backend, inputs.dtype_bytes, inputs.hidden_size)?,
            checked_mul(backend, inputs.dtype_bytes, inputs.vocabulary_size)?,
            checked_product(
                backend,
                &[4, inputs.half_conversion, inputs.vocabulary_size],
            )?,
            checked_product(
                backend,
                &[
                    inputs.batched_prefill,
                    inputs.maximum_prefill,
                    inputs.maximum_tokens,
                ],
            )?,
        ],
    )
}

fn checked_usize_to_u64(backend: BackendId, value: usize) -> Result<u64, ModelError> {
    u64::try_from(value).map_err(|_| numeric_error(backend))
}

fn checked_add(backend: BackendId, left: u64, right: u64) -> Result<u64, ModelError> {
    left.checked_add(right)
        .ok_or_else(|| numeric_error(backend))
}

fn checked_mul(backend: BackendId, left: u64, right: u64) -> Result<u64, ModelError> {
    left.checked_mul(right)
        .ok_or_else(|| numeric_error(backend))
}

fn checked_sum(backend: BackendId, terms: &[u64]) -> Result<u64, ModelError> {
    terms
        .iter()
        .try_fold(0_u64, |total, term| checked_add(backend, total, *term))
}

fn checked_product(backend: BackendId, factors: &[u64]) -> Result<u64, ModelError> {
    factors
        .iter()
        .try_fold(1_u64, |total, factor| checked_mul(backend, total, *factor))
}

const fn numeric_error(backend: BackendId) -> ModelError {
    ModelError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_NUMERIC_OVERFLOW,
    ))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use candle_core::DType;
    use candle_transformers::models::llama::Config;
    use domain_contracts::{
        BackendFailureKind, BackendId, DeviceKind, MemoryFootprint, ModelError,
        SequenceConfiguration,
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
                layer_forward_bytes: 11_524,
                forward_bytes: 12_292,
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
                layer_forward_bytes: 9_604,
                forward_bytes: 10_132,
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
                layer_forward_bytes: 2_656,
                forward_bytes: 2_820,
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
                layer_forward_bytes: 2_864,
                forward_bytes: 3_012,
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
                layer_forward_bytes: 6_788,
                forward_bytes: 14_137,
                cpu_creation_host_bytes: 788,
                cpu_forward_host_bytes: 15_613,
                cuda_forward_device_bytes: 15_580,
                cuda_creation_host_bytes: 140,
                cuda_forward_host_bytes: 121,
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
                layer_forward_bytes: 128,
                forward_bytes: 150,
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

        let mut cache_trimming_hazard = fixture_config();
        cache_trimming_hazard.num_key_value_heads = 2;
        cache_trimming_hazard.num_attention_heads = 2;
        cache_trimming_hazard.max_position_embeddings = 1;
        assert_eq!(
            calculate(
                BACKEND,
                &cache_trimming_hazard,
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
}
