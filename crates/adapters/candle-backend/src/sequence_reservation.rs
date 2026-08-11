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
        let block_forward_peak_bytes = block_forward_peak_bytes(backend, self)?;
        let model_forward_peak_bytes =
            model_forward_peak_bytes(backend, self, block_forward_peak_bytes)?;

        let cpu_creation_host_bytes =
            checked_sum(backend, &[token_staging_bytes, cache_creation_device_bytes])?;
        let cpu_forward_host_bytes = checked_sum(
            backend,
            &[
                token_staging_bytes,
                rope_bytes,
                kv_cache_bytes,
                mask_cache_bytes,
                model_forward_peak_bytes,
                mask_source_bytes,
            ],
        )?;
        let cuda_forward_device_bytes = checked_sum(
            backend,
            &[rope_bytes, kv_cache_bytes, mask_cache_bytes, model_forward_peak_bytes],
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
            block_forward_peak_bytes,
            model_forward_peak_bytes,
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
    block_forward_peak_bytes: u64,
    model_forward_peak_bytes: u64,
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

fn block_forward_peak_bytes(
    backend: BackendId,
    inputs: ReservationInputs,
) -> Result<u64, ModelError> {
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

fn model_forward_peak_bytes(
    backend: BackendId,
    inputs: ReservationInputs,
    block_forward_peak_bytes: u64,
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
            // Candle executes transformer blocks sequentially and replaces the current
            // hidden-state tensor after each block. Persistent per-layer KV ownership is
            // accounted above in `kv_cache_bytes`; only one block's transient working
            // set is live at the model-forward peak.
            block_forward_peak_bytes,
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
mod tests;
