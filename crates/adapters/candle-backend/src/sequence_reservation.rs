//! Source-locked logical-payload reservation for Candle Llama sequences.
//!
//! This module models the exact `candle-transformers` 0.11.0 batch-one,
//! non-flash Llama implementation. Persistent sequence payload and additional
//! creation/execution headroom are calculated separately, then combined with
//! checked arithmetic. The result is a conservative logical-payload bound, not
//! physical RSS/VRAM or an allocator measurement. Caller-owned logits and
//! generation workspaces remain outside this reservation.

use candle_core::DType;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, ByteCount, DeviceKind, MemoryFootprint, ModelError,
    SequenceConfiguration, SequenceReservation,
};

use crate::failure::{CODE_NUMERIC_OVERFLOW, failure};
use crate::loader::math::execution_dtype_bytes;

const REVIEWED_MAX_HIDDEN_LAYERS: u64 = 256;

pub(super) fn calculate(
    backend: BackendId,
    config: &Config,
    dtype: DType,
    device_kind: DeviceKind,
    configuration: SequenceConfiguration,
) -> Result<SequenceReservation, ModelError> {
    if !matches!(device_kind, DeviceKind::Cpu | DeviceKind::Cuda) {
        return Err(ModelError::Unsupported);
    }
    let inputs = LlamaMemoryGeometry::new(backend, config, dtype, configuration)?;
    let components = inputs.components(backend)?;
    components.reservation(backend, device_kind)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LlamaMemoryGeometry {
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
    mask_producing_prefill: u64,
}

impl LlamaMemoryGeometry {
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
        let dtype_bytes = execution_dtype_bytes(dtype).ok_or(ModelError::Unsupported)?;

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
            // Candle 0.11.0 checks concatenated cache `dims()[1]` as though it
            // were sequence length. That dimension is the KV-head count. Fail
            // closed before allocation when the upstream check could enter its
            // erroneous last-dimension narrowing branch.
            || kv_heads > maximum_positions
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
            mask_producing_prefill: u64::from(maximum_prefill > 1),
        })
    }

    fn components(self, backend: BackendId) -> Result<ReservationComponents, ModelError> {
        let cache_bytes_per_token = cache_bytes_per_token(backend, self)?;
        let persistent = PersistentComponents {
            token_staging: checked_mul(backend, 4, self.maximum_prefill)?,
            kv_cache: checked_mul(backend, cache_bytes_per_token, self.maximum_tokens)?,
            rope: checked_product(
                backend,
                &[
                    self.maximum_positions,
                    self.head_dimension,
                    self.dtype_bytes,
                ],
            )?,
            mask_cache: maximum_mask_cache_bytes(
                backend,
                self.maximum_tokens,
                self.maximum_prefill,
            )?,
        };
        let cache_creation_device_peak = cache_creation_device_peak(backend, self)?;
        let creation = CreationTransientComponents {
            cache_device_bytes: checked_sub(backend, cache_creation_device_peak, persistent.rope)?,
            cuda_host_source_bytes: cache_creation_cuda_host_bytes(backend, self)?,
        };
        let execution = execution_transient_components(backend, self)?;

        Ok(ReservationComponents {
            cache_bytes_per_token,
            persistent,
            creation,
            execution,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PersistentComponents {
    token_staging: u64,
    kv_cache: u64,
    rope: u64,
    mask_cache: u64,
}

impl PersistentComponents {
    fn device_tensor_bytes(self, backend: BackendId) -> Result<u64, ModelError> {
        checked_sum(backend, &[self.kv_cache, self.rope, self.mask_cache])
    }

    fn footprint(
        self,
        backend: BackendId,
        device_kind: DeviceKind,
    ) -> Result<MemoryFootprint, ModelError> {
        let device_tensor_bytes = self.device_tensor_bytes(backend)?;
        match device_kind {
            DeviceKind::Cpu => Ok(MemoryFootprint::host_working(ByteCount::from_u64(
                checked_add(backend, self.token_staging, device_tensor_bytes)?,
            ))),
            DeviceKind::Cuda => Ok(MemoryFootprint::host_working(ByteCount::from_u64(
                self.token_staging,
            ))
            .with_device_working_bytes(ByteCount::from_u64(device_tensor_bytes))),
            _ => Err(ModelError::Unsupported),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CreationTransientComponents {
    /// Device/CPU tensor payload live beyond the final retained rope tensors.
    cache_device_bytes: u64,
    /// CUDA-only host vectors consumed by theta/arange device construction.
    cuda_host_source_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AttentionPhaseComponents {
    qkv_projection: u64,
    qk_layout_copy: u64,
    qk_rotary_output: u64,
    cache_replacement: u64,
    grouped_query_expansion: u64,
    f32_qkv_conversion: u64,
    attention_score: u64,
    masked_fill_scalar: u64,
    f32_value_contiguous: u64,
    f32_attention_value: u64,
    attention_value_cast: u64,
    output_projection: u64,
    cache_replacement_phase: u64,
    first_attention_compute_phase: u64,
    cached_attention_compute_phase: u64,
    attention_compute_phase: u64,
    phase: u64,
}

#[derive(Clone, Copy)]
struct AttentionLayoutComponents {
    qkv_projection: u64,
    qk_layout_copy: u64,
    qk_rotary_output: u64,
    cache_replacement: u64,
    grouped_query_expansion: u64,
}

#[derive(Clone, Copy)]
struct AttentionF32Components {
    qkv_conversion: u64,
    score: u64,
    masked_fill_scalar: u64,
    value_contiguous: u64,
    attention_value: u64,
    value_cast: u64,
    output_projection: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExecutionTransientComponents {
    mask_source_bytes: u64,
    input_tensor_bytes: u64,
    attention: AttentionPhaseComponents,
    residual_add_phase_bytes: u64,
    mlp_gate_up_phase_bytes: u64,
    mlp_down_projection_phase_bytes: u64,
    mlp_phase_bytes: u64,
    final_block_add_phase_bytes: u64,
    block_peak_bytes: u64,
    embedding_phase_bytes: u64,
    final_logits_phase_bytes: u64,
    model_forward_peak_bytes: u64,
    cuda_logits_transfer_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReservationComponents {
    cache_bytes_per_token: u64,
    persistent: PersistentComponents,
    creation: CreationTransientComponents,
    execution: ExecutionTransientComponents,
}

impl ReservationComponents {
    fn reservation(
        self,
        backend: BackendId,
        device_kind: DeviceKind,
    ) -> Result<SequenceReservation, ModelError> {
        let persistent_footprint = self.persistent.footprint(backend, device_kind)?;
        let transient_footprint = match device_kind {
            DeviceKind::Cpu => MemoryFootprint::host_working(ByteCount::from_u64(
                self.creation.cache_device_bytes.max(checked_add(
                    backend,
                    self.execution.mask_source_bytes,
                    self.execution.model_forward_peak_bytes,
                )?),
            )),
            DeviceKind::Cuda => MemoryFootprint::host_working(ByteCount::from_u64(
                self.creation
                    .cuda_host_source_bytes
                    .max(self.execution.mask_source_bytes)
                    .max(self.execution.cuda_logits_transfer_bytes),
            ))
            .with_device_working_bytes(ByteCount::from_u64(
                self.creation
                    .cache_device_bytes
                    .max(self.execution.model_forward_peak_bytes),
            )),
            _ => return Err(ModelError::Unsupported),
        };
        SequenceReservation::checked(persistent_footprint, transient_footprint)
            .ok_or_else(|| numeric_error(backend))
    }
}

fn cache_bytes_per_token(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
) -> Result<u64, ModelError> {
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

    // Every distinct `(seq_len, kv_len)` mask is retained. This closed-form
    // bound covers every chunk schedule with `seq_len <= maximum_prefill`.
    // Evaluate the numerator in u128 because the representable quotient can
    // have an unrepresentable u64 intermediate.
    let tokens = u128::from(maximum_tokens);
    let prefill = u128::from(maximum_prefill);
    let numerator = tokens
        .checked_mul(tokens)
        .and_then(|square| {
            tokens
                .checked_mul(prefill)
                .and_then(|product| square.checked_add(product))
        })
        .ok_or_else(|| numeric_error(backend))?;
    u64::try_from(numerator / 2).map_err(|_| numeric_error(backend))
}

fn cache_creation_device_peak(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
) -> Result<u64, ModelError> {
    // `Cache::new`: retained F32 theta plus idx-theta and the retained/temporary
    // cos/sin tensors. Both F32 and half/BF16 paths reach the same byte peak.
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
    inputs: LlamaMemoryGeometry,
) -> Result<u64, ModelError> {
    // Llama3 rope scaling can hold input and output inverse-frequency Vecs at
    // once (4 * head_dimension bytes total). Arange separately owns one u32 Vec.
    let inverse_frequency_vectors = checked_mul(backend, 4, inputs.head_dimension)?;
    let positions = checked_mul(backend, 4, inputs.maximum_positions)?;
    Ok(inverse_frequency_vectors.max(positions))
}

fn execution_transient_components(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
) -> Result<ExecutionTransientComponents, ModelError> {
    let prefill_hidden_elements = checked_mul(backend, inputs.maximum_prefill, inputs.hidden_size)?;
    let prefill_kv_elements =
        checked_mul(backend, inputs.maximum_prefill, inputs.grouped_kv_width)?;
    let context_kv_elements = checked_mul(backend, inputs.maximum_tokens, inputs.grouped_kv_width)?;
    let context_hidden_elements = checked_mul(backend, inputs.maximum_tokens, inputs.hidden_size)?;
    let prefill_intermediate_elements =
        checked_mul(backend, inputs.maximum_prefill, inputs.intermediate_size)?;
    let hidden_payload_bytes = checked_mul(backend, inputs.dtype_bytes, prefill_hidden_elements)?;
    let attention = attention_phase_components(
        backend,
        inputs,
        prefill_hidden_elements,
        prefill_kv_elements,
        context_kv_elements,
        context_hidden_elements,
        hidden_payload_bytes,
    )?;
    let residual_add_phase_bytes = checked_mul(backend, 4, hidden_payload_bytes)?;
    // At the gate/up/product expression peak, four block-hidden tensors are
    // live with four intermediate tensors. Those expression temporaries are
    // gone before the down projection adds its output hidden tensor, so take
    // the larger phase instead of summing mutually exclusive payloads.
    let mlp_gate_up_phase_bytes = checked_sum(
        backend,
        &[
            checked_mul(backend, 4, hidden_payload_bytes)?,
            checked_product(
                backend,
                &[4, inputs.dtype_bytes, prefill_intermediate_elements],
            )?,
        ],
    )?;
    let mlp_down_projection_phase_bytes = checked_sum(
        backend,
        &[
            checked_mul(backend, 5, hidden_payload_bytes)?,
            checked_product(
                backend,
                &[inputs.dtype_bytes, prefill_intermediate_elements],
            )?,
        ],
    )?;
    let mlp_phase_bytes = mlp_gate_up_phase_bytes.max(mlp_down_projection_phase_bytes);
    let final_block_add_phase_bytes = checked_mul(backend, 6, hidden_payload_bytes)?;
    let block_peak_bytes = attention
        .phase
        .max(residual_add_phase_bytes)
        .max(mlp_phase_bytes)
        .max(final_block_add_phase_bytes);

    let embedding_phase_bytes = hidden_payload_bytes;
    let final_logits_phase_bytes = checked_sum(
        backend,
        &[
            checked_mul(backend, 2, hidden_payload_bytes)?,
            checked_product(backend, &[inputs.dtype_bytes, inputs.hidden_size])?,
            checked_product(backend, &[inputs.dtype_bytes, inputs.vocabulary_size])?,
            checked_product(
                backend,
                &[4, inputs.half_conversion, inputs.vocabulary_size],
            )?,
        ],
    )?;
    let input_tensor_bytes = checked_mul(backend, 4, inputs.maximum_prefill)?;
    let model_forward_peak_bytes = checked_add(
        backend,
        input_tensor_bytes,
        embedding_phase_bytes
            .max(block_peak_bytes)
            .max(final_logits_phase_bytes),
    )?;
    let mask_source_bytes = checked_product(
        backend,
        &[
            inputs.mask_producing_prefill,
            inputs.maximum_prefill,
            inputs.maximum_tokens,
        ],
    )?;
    let cuda_logits_transfer_bytes = checked_mul(backend, 4, inputs.vocabulary_size)?;

    Ok(ExecutionTransientComponents {
        mask_source_bytes,
        input_tensor_bytes,
        attention,
        residual_add_phase_bytes,
        mlp_gate_up_phase_bytes,
        mlp_down_projection_phase_bytes,
        mlp_phase_bytes,
        final_block_add_phase_bytes,
        block_peak_bytes,
        embedding_phase_bytes,
        final_logits_phase_bytes,
        model_forward_peak_bytes,
        cuda_logits_transfer_bytes,
    })
}

fn attention_phase_components(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
    prefill_hidden_elements: u64,
    prefill_kv_elements: u64,
    context_kv_elements: u64,
    context_hidden_elements: u64,
    hidden_payload_bytes: u64,
) -> Result<AttentionPhaseComponents, ModelError> {
    let layout = attention_layout_components(
        backend,
        inputs,
        prefill_hidden_elements,
        prefill_kv_elements,
        context_kv_elements,
        context_hidden_elements,
    )?;
    let cached_f32 = attention_f32_components(
        backend,
        inputs,
        prefill_hidden_elements,
        prefill_kv_elements,
        context_hidden_elements,
        inputs.maximum_tokens,
        false,
    )?;
    let first_f32 = attention_f32_components(
        backend,
        inputs,
        prefill_hidden_elements,
        prefill_kv_elements,
        prefill_hidden_elements,
        inputs.maximum_prefill,
        true,
    )?;
    // The previous cache pair is live only while `Tensor::cat` replaces it.
    // Candle overwrites and drops that pair before grouped-query expansion and
    // score computation, so replacement and compute are alternative phases.
    let common_layout = checked_sum(
        backend,
        &[
            layout.qkv_projection,
            layout.qk_layout_copy,
            layout.qk_rotary_output,
        ],
    )?;
    let cache_replacement_phase = checked_add(backend, common_layout, layout.cache_replacement)?;
    let cached_attention_compute_phase = attention_compute_phase(
        backend,
        common_layout,
        layout.grouped_query_expansion,
        cached_f32,
    )?;
    let first_grouped_query_expansion = checked_product(
        backend,
        &[
            2,
            inputs.grouped_query_expansion,
            inputs.dtype_bytes,
            prefill_hidden_elements,
        ],
    )?;
    let first_attention_compute_phase = attention_compute_phase(
        backend,
        common_layout,
        first_grouped_query_expansion,
        first_f32,
    )?;
    let attention_compute_phase = first_attention_compute_phase.max(cached_attention_compute_phase);
    let phase = checked_sum(
        backend,
        &[
            hidden_payload_bytes,
            hidden_payload_bytes,
            cache_replacement_phase.max(attention_compute_phase),
        ],
    )?;
    Ok(AttentionPhaseComponents {
        qkv_projection: layout.qkv_projection,
        qk_layout_copy: layout.qk_layout_copy,
        qk_rotary_output: layout.qk_rotary_output,
        cache_replacement: layout.cache_replacement,
        grouped_query_expansion: layout.grouped_query_expansion,
        f32_qkv_conversion: cached_f32.qkv_conversion,
        attention_score: cached_f32.score,
        masked_fill_scalar: cached_f32.masked_fill_scalar,
        f32_value_contiguous: first_f32.value_contiguous,
        f32_attention_value: cached_f32.attention_value,
        attention_value_cast: cached_f32.value_cast,
        output_projection: cached_f32.output_projection,
        cache_replacement_phase,
        first_attention_compute_phase,
        cached_attention_compute_phase,
        attention_compute_phase,
        phase,
    })
}

fn attention_layout_components(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
    prefill_hidden_elements: u64,
    prefill_kv_elements: u64,
    context_kv_elements: u64,
    context_hidden_elements: u64,
) -> Result<AttentionLayoutComponents, ModelError> {
    let qkv_projection = checked_product(
        backend,
        &[
            inputs.dtype_bytes,
            checked_add(
                backend,
                prefill_hidden_elements,
                checked_mul(backend, 2, prefill_kv_elements)?,
            )?,
        ],
    )?;
    let qk_layout_copy = checked_product(
        backend,
        &[
            inputs.dtype_bytes,
            checked_add(backend, prefill_hidden_elements, prefill_kv_elements)?,
        ],
    )?;
    // The persistent bound covers the new full K/V pair. One current-layer
    // full-pair term conservatively covers the old pair during concatenation.
    let cache_replacement =
        checked_product(backend, &[2, inputs.dtype_bytes, context_kv_elements])?;
    let grouped_query_expansion = checked_product(
        backend,
        &[
            2,
            inputs.grouped_query_expansion,
            inputs.dtype_bytes,
            context_hidden_elements,
        ],
    )?;
    Ok(AttentionLayoutComponents {
        qkv_projection,
        qk_layout_copy,
        qk_rotary_output: qk_layout_copy,
        cache_replacement,
        grouped_query_expansion,
    })
}

fn attention_f32_components(
    backend: BackendId,
    inputs: LlamaMemoryGeometry,
    prefill_hidden_elements: u64,
    prefill_kv_elements: u64,
    context_hidden_elements: u64,
    context_tokens: u64,
    include_value_contiguous: bool,
) -> Result<AttentionF32Components, ModelError> {
    let qkv_conversion = checked_product(
        backend,
        &[
            4,
            inputs.half_conversion,
            checked_add(
                backend,
                prefill_hidden_elements,
                checked_mul(backend, 2, context_hidden_elements)?,
            )?,
        ],
    )?;
    let score_elements = checked_product(
        backend,
        &[
            inputs.attention_heads,
            inputs.maximum_prefill,
            context_tokens,
        ],
    )?;
    let score = checked_product(
        backend,
        &[
            4,
            score_elements,
            checked_add(backend, 2, inputs.mask_producing_prefill)?,
        ],
    )?;
    // `masked_fill` constructs one device-side F32 `on_true` tensor before
    // broadcasting it over the score shape.
    let masked_fill_scalar = checked_mul(backend, 4, inputs.mask_producing_prefill)?;
    let value_contiguous = checked_product(
        backend,
        &[
            4,
            u64::from(include_value_contiguous),
            1_u64.saturating_sub(inputs.half_conversion),
            1_u64.saturating_sub(inputs.grouped_query_expansion),
            prefill_kv_elements,
        ],
    )?;
    let attention_value = checked_mul(backend, 4, prefill_hidden_elements)?;
    let value_cast = checked_product(
        backend,
        &[
            inputs.half_conversion,
            inputs.dtype_bytes,
            prefill_hidden_elements,
        ],
    )?;
    let output_projection = checked_mul(backend, inputs.dtype_bytes, prefill_hidden_elements)?;
    Ok(AttentionF32Components {
        qkv_conversion,
        score,
        masked_fill_scalar,
        value_contiguous,
        attention_value,
        value_cast,
        output_projection,
    })
}

fn attention_compute_phase(
    backend: BackendId,
    common_layout: u64,
    grouped_query_expansion: u64,
    f32: AttentionF32Components,
) -> Result<u64, ModelError> {
    checked_sum(
        backend,
        &[
            common_layout,
            grouped_query_expansion,
            f32.qkv_conversion,
            f32.score,
            f32.masked_fill_scalar,
            f32.value_contiguous,
            f32.attention_value,
            f32.value_cast,
            f32.output_projection,
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

fn checked_sub(backend: BackendId, left: u64, right: u64) -> Result<u64, ModelError> {
    left.checked_sub(right)
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
