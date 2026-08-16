//! Loaded Candle model and device-aware sequence implementations.

use candle_core::{DType, Device, Storage, Tensor};
use candle_transformers::models::llama::{Cache, Config, Llama};
use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, CapacityExhausted, CapacityResource,
    DecodeBufferRequirements, DecodeInput, DecodeOutcome, ExecutionDevice, LoadedModel,
    MemoryFootprint, ModelDescriptor, ModelError, ModelHandle, PrefillBufferRequirements,
    PrefillInput, PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers, ScalarType,
    SequenceConfiguration, SequenceError, SequenceId, SequencePlan, SequenceReservation,
    SequenceState, SynchronizationError,
};

use crate::failure::{
    CODE_CACHE_CREATE, CODE_FORWARD, CODE_INPUT_TENSOR, CODE_LOGITS_LAYOUT, CODE_LOGITS_STORAGE,
    CODE_LOGITS_TRANSFER, CODE_NUMERIC_OVERFLOW, CODE_RESERVATION, CODE_SYNCHRONIZE,
    candle_cuda_failure_kind, failure,
};
use crate::sequence_reservation;
pub(crate) struct CandleLlamaModelParameters {
    pub(crate) backend: BackendId,
    pub(crate) handle: ModelHandle,
    pub(crate) execution_device: ExecutionDevice,
    pub(crate) descriptor: ModelDescriptor,
    pub(crate) reported_footprint: MemoryFootprint,
    pub(crate) config: Config,
    pub(crate) dtype: DType,
    pub(crate) execution_scalar_type: ScalarType,
    pub(crate) device: Device,
}

/// Loaded Llama model exclusively owned by the inference runtime.
pub struct CandleLlamaModel {
    backend: BackendId,
    handle: ModelHandle,
    execution_device: ExecutionDevice,
    descriptor: ModelDescriptor,
    reported_footprint: MemoryFootprint,
    vocabulary_size: usize,
    config: Config,
    dtype: DType,
    execution_scalar_type: ScalarType,
    device: Device,
    model: Llama,
    unloading: bool,
}

impl CandleLlamaModel {
    pub(crate) fn new(parameters: CandleLlamaModelParameters, model: Llama) -> Self {
        Self {
            backend: parameters.backend,
            handle: parameters.handle,
            execution_device: parameters.execution_device,
            descriptor: parameters.descriptor,
            reported_footprint: parameters.reported_footprint,
            vocabulary_size: parameters.config.vocab_size,
            config: parameters.config,
            dtype: parameters.dtype,
            execution_scalar_type: parameters.execution_scalar_type,
            device: parameters.device,
            model,
            unloading: false,
        }
    }

    /// Returns the complete inspected descriptor retained by the loaded model.
    #[must_use]
    pub const fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    /// Returns the actual domain scalar retained for backend execution tensors.
    ///
    /// This evidence is selected independently from the source scalar metadata
    /// retained in [`ModelDescriptor`].
    #[must_use]
    pub const fn execution_scalar_type(&self) -> ScalarType {
        self.execution_scalar_type
    }

    fn sequence_reservation(
        &self,
        configuration: SequenceConfiguration,
    ) -> Result<SequenceReservation, ModelError> {
        sequence_reservation::calculate(
            self.backend,
            &self.config,
            self.dtype,
            self.execution_device.kind,
            configuration,
        )
    }

    fn prepare_input(&self, tokens: &[u32]) -> Result<Tensor, SequenceError> {
        Tensor::from_slice(tokens, (1, tokens.len()), &self.device).map_err(|error| {
            sequence_candle_failure(self.backend, &self.device, &error, CODE_INPUT_TENSOR)
        })
    }

    fn forward(
        &self,
        cache: &mut Cache,
        position: usize,
        input: &Tensor,
    ) -> Result<Tensor, SequenceError> {
        self.device
            .with_context(|| self.model.forward(input, position, cache))
            .map_err(|error| {
                sequence_candle_failure(self.backend, &self.device, &error, CODE_FORWARD)
            })
    }

    fn copy_logits(
        &self,
        tensor: &Tensor,
        output: &mut [f32],
        required: usize,
    ) -> Result<usize, SequenceError> {
        if tensor.elem_count() != required || !tensor.is_contiguous() {
            return Err(sequence_failure(self.backend, CODE_LOGITS_LAYOUT));
        }
        let available = output.len();
        let Some(destination) = output.get_mut(..required) else {
            return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::Logits,
                usize_to_u64(required),
                usize_to_u64(available),
            )));
        };
        let logits = tensor.to_dtype(DType::F32).map_err(|error| {
            sequence_candle_failure(self.backend, &self.device, &error, CODE_LOGITS_STORAGE)
        })?;
        let host_logits = if self.device.is_cuda() {
            // Candle's safe device-to-host boundary allocates a temporary
            // upstream CPU tensor. The project-owned destination remains fixed
            // and reusable; this CUDA path is not claimed allocation-free.
            let transferred = logits.to_device(&Device::Cpu).map_err(|error| {
                sequence_candle_failure(self.backend, &self.device, &error, CODE_LOGITS_TRANSFER)
            })?;
            self.device.synchronize().map_err(|_| {
                synchronization_sequence_failure(self.backend, CODE_LOGITS_TRANSFER)
            })?;
            transferred
        } else {
            logits
        };
        copy_cpu_storage(self.backend, &host_logits, destination, required)?;
        Ok(required)
    }
}

impl LoadedModel for CandleLlamaModel {
    type Sequence = CandleLlamaSequence;

    fn handle(&self) -> ModelHandle {
        self.handle
    }

    fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }

    fn execution_scalar_type(&self) -> ScalarType {
        self.execution_scalar_type
    }

    fn execution_device(&self) -> ExecutionDevice {
        self.execution_device
    }

    fn reported_footprint(&self) -> MemoryFootprint {
        self.reported_footprint
    }

    fn plan_sequence(
        &self,
        configuration: &SequenceConfiguration,
    ) -> Result<SequencePlan, ModelError> {
        if self.unloading {
            return Err(ModelError::InvalidState);
        }
        if configuration.maximum_tokens.get() > self.descriptor.metadata.context_length
            || configuration.maximum_prefill_batch.get() > configuration.maximum_tokens.get()
        {
            return Err(ModelError::Unsupported);
        }

        Ok(SequencePlan {
            configuration: *configuration,
            reservation: self.sequence_reservation(*configuration)?,
            logits_capacity: self.vocabulary_size,
        })
    }

    fn create_sequence(
        &mut self,
        sequence_id: SequenceId,
        configuration: &SequenceConfiguration,
    ) -> Result<Self::Sequence, ModelError> {
        let plan = self.plan_sequence(configuration)?;
        let maximum_prefill = usize::try_from(configuration.maximum_prefill_batch.get())
            .map_err(|_| numeric_model_error(self.backend))?;
        let token_capacity = usize::try_from(configuration.maximum_tokens.get())
            .map_err(|_| numeric_model_error(self.backend))?;
        let mut token_staging = Vec::new();
        token_staging
            .try_reserve_exact(maximum_prefill)
            .map_err(|_| reservation_model_error(self.backend))?;
        if token_staging.capacity() != maximum_prefill {
            return Err(reservation_model_error(self.backend));
        }
        let cache = Cache::new(true, self.dtype, &self.config, &self.device).map_err(|error| {
            model_candle_failure(self.backend, &self.device, &error, CODE_CACHE_CREATE)
        })?;

        Ok(CandleLlamaSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            position: 0,
            token_capacity,
            maximum_prefill,
            cache,
            token_staging,
            plan,
        })
    }

    fn prefill_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        input: &PrefillInput<'_>,
    ) -> PrefillBufferRequirements {
        PrefillBufferRequirements {
            logits: if input.emit_logits {
                self.vocabulary_size
            } else {
                0
            },
        }
    }

    fn decode_buffer_requirements(
        &self,
        _sequence: &Self::Sequence,
        _input: DecodeInput,
    ) -> DecodeBufferRequirements {
        DecodeBufferRequirements {
            logits: self.vocabulary_size,
        }
    }

    fn prefill_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: PrefillInput<'_>,
        mut buffers: PreparedPrefillBuffers<'_>,
    ) -> Result<PrefillOutcome, SequenceError> {
        if self.unloading || sequence.state == SequenceState::Finished || input.tokens.is_empty() {
            return Err(SequenceError::InvalidState);
        }
        if input.tokens.len() > sequence.maximum_prefill {
            return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
                CapacityResource::PrefillBatch,
                usize_to_u64(input.tokens.len()),
                usize_to_u64(sequence.maximum_prefill),
            )));
        }

        let next_position = checked_next_position(
            self.backend,
            sequence.position,
            input.tokens.len(),
            sequence.token_capacity,
        )?;
        sequence.token_staging.clear();
        for token in input.tokens {
            sequence.token_staging.push(token.get());
        }
        let position = sequence.position;
        let input_tensor = self.prepare_input(sequence.token_staging.as_slice())?;
        let forward = self.forward(&mut sequence.cache, position, &input_tensor);
        let logits = finish_after_cache_boundary(&mut sequence.state, forward)?;
        let logits_written = if input.emit_logits {
            let required_logits = buffers.required_logits();
            let copy = self.copy_logits(&logits, buffers.logits_mut(), required_logits);
            finish_after_cache_boundary(&mut sequence.state, copy)?
        } else {
            0
        };
        sequence.position = next_position;
        sequence.state = SequenceState::Ready;

        Ok(PrefillOutcome::Ready {
            consumed_tokens: input.tokens.len(),
            position: sequence.position,
            logits_written,
        })
    }

    fn decode_prepared(
        &mut self,
        sequence: &mut Self::Sequence,
        input: DecodeInput,
        mut buffers: PreparedDecodeBuffers<'_>,
    ) -> Result<DecodeOutcome, SequenceError> {
        if self.unloading || sequence.state != SequenceState::Ready {
            return Err(SequenceError::InvalidState);
        }

        let next_position =
            checked_next_position(self.backend, sequence.position, 1, sequence.token_capacity)?;
        let token = [input.token.get()];
        let position = sequence.position;
        let input_tensor = self.prepare_input(&token)?;
        let forward = self.forward(&mut sequence.cache, position, &input_tensor);
        let logits = finish_after_cache_boundary(&mut sequence.state, forward)?;
        let required_logits = buffers.required_logits();
        let copy = self.copy_logits(&logits, buffers.logits_mut(), required_logits);
        let logits_written = finish_after_cache_boundary(&mut sequence.state, copy)?;
        sequence.position = next_position;

        Ok(DecodeOutcome::Ready {
            position: sequence.position,
            logits_written,
        })
    }

    fn destroy_sequence(&mut self, sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        self.device
            .synchronize()
            .map_err(|_| synchronization_sequence_failure(self.backend, CODE_SYNCHRONIZE))?;
        sequence.state = SequenceState::Finished;
        Ok(())
    }

    fn reset_sequence(&mut self, _sequence: &mut Self::Sequence) -> Result<(), SequenceError> {
        if self.unloading {
            return Err(SequenceError::InvalidState);
        }
        Err(SequenceError::Unsupported)
    }

    fn synchronize(&mut self) -> Result<(), SynchronizationError> {
        self.device.synchronize().map_err(|_| {
            SynchronizationError::Backend(failure(
                self.backend,
                BackendFailureKind::Synchronization,
                CODE_SYNCHRONIZE,
            ))
        })
    }

    fn prepare_unload(&mut self) -> Result<(), SynchronizationError> {
        if self.unloading {
            return Err(SynchronizationError::InvalidState);
        }
        self.synchronize()?;
        self.unloading = true;
        Ok(())
    }
}

/// Sequence-local Candle cache, position, and prepared token staging.
pub struct CandleLlamaSequence {
    id: SequenceId,
    state: SequenceState,
    position: usize,
    token_capacity: usize,
    maximum_prefill: usize,
    cache: Cache,
    token_staging: Vec<u32>,
    plan: SequencePlan,
}

impl CandleLlamaSequence {
    /// Returns the retained source-reviewed sequence reservation.
    #[must_use]
    pub const fn reservation(&self) -> SequenceReservation {
        self.plan.reservation
    }

    /// Returns the maximum prompt tokens accepted by one prefill call.
    #[must_use]
    pub const fn maximum_prefill_batch(&self) -> usize {
        self.maximum_prefill
    }

    /// Returns the actual fixed token-staging allocation capacity.
    #[must_use]
    pub fn token_staging_capacity(&self) -> usize {
        self.token_staging.capacity()
    }

    /// Returns the logical byte extent of the fixed `u32` token staging buffer.
    #[must_use]
    pub fn token_staging_logical_bytes(&self) -> usize {
        self.token_staging
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>())
    }
}

impl BackendSequence for CandleLlamaSequence {
    fn id(&self) -> SequenceId {
        self.id
    }

    fn state(&self) -> SequenceState {
        self.state
    }

    fn position(&self) -> usize {
        self.position
    }

    fn token_capacity(&self) -> usize {
        self.token_capacity
    }

    fn reported_plan(&self) -> SequencePlan {
        self.plan
    }
}

fn copy_cpu_storage(
    backend: BackendId,
    tensor: &Tensor,
    destination: &mut [f32],
    required: usize,
) -> Result<(), SequenceError> {
    if !tensor.is_contiguous() {
        return Err(sequence_failure(backend, CODE_LOGITS_LAYOUT));
    }
    let (storage, layout) = tensor.storage_and_layout();
    let Storage::Cpu(cpu) = &*storage else {
        return Err(sequence_failure(backend, CODE_LOGITS_STORAGE));
    };
    let values = cpu
        .as_slice::<f32>()
        .map_err(|_| sequence_failure(backend, CODE_LOGITS_STORAGE))?;
    let Some((start, end)) = layout.contiguous_offsets() else {
        return Err(sequence_failure(backend, CODE_LOGITS_LAYOUT));
    };
    let Some(source) = values.get(start..end) else {
        return Err(sequence_failure(backend, CODE_LOGITS_LAYOUT));
    };
    if source.len() != required || destination.len() != required {
        return Err(sequence_failure(backend, CODE_LOGITS_LAYOUT));
    }
    destination.copy_from_slice(source);
    Ok(())
}

fn checked_next_position(
    backend: BackendId,
    position: usize,
    input_length: usize,
    token_capacity: usize,
) -> Result<usize, SequenceError> {
    let next_position = position
        .checked_add(input_length)
        .ok_or_else(|| numeric_sequence_error(backend))?;
    if next_position > token_capacity {
        return Err(SequenceError::CapacityExhausted(CapacityExhausted::new(
            CapacityResource::Tokens,
            usize_to_u64(input_length),
            usize_to_u64(token_capacity.saturating_sub(position)),
        )));
    }
    Ok(next_position)
}

fn finish_after_cache_boundary<T>(
    state: &mut SequenceState,
    result: Result<T, SequenceError>,
) -> Result<T, SequenceError> {
    result.inspect_err(|_| {
        *state = SequenceState::Finished;
    })
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

const fn numeric_model_error(backend: BackendId) -> ModelError {
    ModelError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_NUMERIC_OVERFLOW,
    ))
}

const fn reservation_model_error(backend: BackendId) -> ModelError {
    ModelError::Backend(failure(
        backend,
        BackendFailureKind::HostMemory,
        CODE_RESERVATION,
    ))
}

const fn numeric_sequence_error(backend: BackendId) -> SequenceError {
    sequence_failure(backend, CODE_NUMERIC_OVERFLOW)
}

const fn sequence_failure(backend: BackendId, code: u32) -> SequenceError {
    SequenceError::Backend(failure(backend, BackendFailureKind::DeviceExecution, code))
}

const fn synchronization_sequence_failure(backend: BackendId, code: u32) -> SequenceError {
    SequenceError::Backend(failure(backend, BackendFailureKind::Synchronization, code))
}

fn sequence_candle_failure(
    backend: BackendId,
    device: &Device,
    error: &candle_core::Error,
    code: u32,
) -> SequenceError {
    let kind = if device.is_cuda() {
        candle_cuda_failure_kind(error).unwrap_or(BackendFailureKind::DeviceExecution)
    } else {
        BackendFailureKind::DeviceExecution
    };
    SequenceError::Backend(failure(backend, kind, code))
}

fn model_candle_failure(
    backend: BackendId,
    device: &Device,
    error: &candle_core::Error,
    code: u32,
) -> ModelError {
    let kind = if device.is_cuda() {
        candle_cuda_failure_kind(error).unwrap_or(BackendFailureKind::DeviceExecution)
    } else {
        BackendFailureKind::HostMemory
    };
    ModelError::Backend(failure(backend, kind, code))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::num::NonZeroU32;

    use candle_core::{DType, Device, Tensor};
    use candle_nn::VarBuilder;
    use candle_transformers::models::llama::{Cache, Config, Llama};
    use domain_contracts::{
        BackendFailureKind, BackendId, CapacityResource, DeviceKind, SequenceConfiguration,
        SequenceError, SequenceState,
    };

    use super::{
        checked_next_position, finish_after_cache_boundary, numeric_sequence_error,
        sequence_failure,
    };
    use crate::failure::{CODE_FORWARD, CODE_NUMERIC_OVERFLOW};

    const BACKEND: BackendId = BackendId::new(91);

    #[test]
    fn capacity_is_rejected_before_the_cache_mutation_boundary() {
        assert_eq!(checked_next_position(BACKEND, 8, 8, 16), Ok(16));
        assert!(matches!(
            checked_next_position(BACKEND, 15, 2, 16),
            Err(SequenceError::CapacityExhausted(capacity))
                if capacity.resource == CapacityResource::Tokens
                    && capacity.required == 2
                    && capacity.available == 1
        ));
        assert_eq!(
            checked_next_position(BACKEND, usize::MAX, 1, usize::MAX),
            Err(numeric_sequence_error(BACKEND))
        );
        assert!(matches!(
            numeric_sequence_error(BACKEND),
            SequenceError::Backend(failure)
                if failure.kind == BackendFailureKind::DeviceExecution
                    && failure.code == CODE_NUMERIC_OVERFLOW
        ));
    }

    #[test]
    fn errors_after_crossing_the_cache_boundary_finish_the_sequence() {
        let mut state = SequenceState::Ready;
        let error = sequence_failure(BACKEND, CODE_FORWARD);
        assert_eq!(
            finish_after_cache_boundary::<()>(&mut state, Err(error)),
            Err(error)
        );
        assert_eq!(state, SequenceState::Finished);

        let mut successful_state = SequenceState::Ready;
        assert_eq!(
            finish_after_cache_boundary(&mut successful_state, Ok(7_u32)),
            Ok(7)
        );
        assert_eq!(successful_state, SequenceState::Ready);
    }

    #[test]
    fn pinned_candle_cache_state_matches_planned_non_gqa_and_gqa_layouts() -> Result<(), String> {
        for (attention_heads, kv_heads, rope_shape, kv_shape, cache_bytes_per_token) in [
            (
                2,
                2,
                "Tensor[dims 16, 2; f32]",
                "Tensor[dims 1, 2, 2, 4; f32]",
                64,
            ),
            (
                4,
                2,
                "Tensor[dims 16, 1; f32]",
                "Tensor[dims 1, 2, 2, 2; f32]",
                32,
            ),
        ] {
            let config = conformance_config(attention_heads, kv_heads);
            let reservation = crate::sequence_reservation::calculate(
                BACKEND,
                &config,
                DType::F32,
                DeviceKind::Cpu,
                SequenceConfiguration::new(
                    NonZeroU32::new(16).unwrap_or(NonZeroU32::MIN),
                    NonZeroU32::new(8).unwrap_or(NonZeroU32::MIN),
                ),
            )
            .map_err(|error| format!("calculate sequence reservation: {error:?}"))?;
            let summed = reservation
                .persistent_footprint()
                .checked_add(reservation.transient_footprint())
                .ok_or_else(|| "reservation sum overflowed".to_owned())?;
            assert_eq!(reservation.total_footprint(), summed);

            let model = conformance_model(&config, DType::F32)?;
            let mut cache = Cache::new(true, DType::F32, &config, &Device::Cpu)
                .map_err(|error| error.to_string())?;
            let created = format!("{cache:?}");
            assert_eq!(
                created.matches(rope_shape).count(),
                2,
                "created cache: {created}"
            );
            assert!(created.contains("kvs: [None]"));
            assert!(created.contains("masks: {}"));

            let input = Tensor::from_slice(&[1_u32, 2], (1, 2), &Device::Cpu)
                .map_err(|error| error.to_string())?;
            let logits = model
                .forward(&input, 0, &mut cache)
                .map_err(|error| error.to_string())?;
            assert_eq!(logits.dims(), &[1, 16]);
            assert_eq!(logits.dtype(), DType::F32);

            let populated = format!("{cache:?}");
            assert_eq!(
                populated.matches(kv_shape).count(),
                2,
                "populated cache: {populated}"
            );
            assert!(populated.contains("(2, 2): Tensor[dims 2, 2; u8]"));
            let observed_cache_bytes = match (attention_heads, kv_heads) {
                (2, 2) => 2 * 2 * 2 * 4 * 4,
                (4, 2) => 2 * 2 * 2 * 2 * 4,
                _ => return Err("unexpected conformance dimensions".to_owned()),
            };
            assert_eq!(observed_cache_bytes / 2, cache_bytes_per_token);
        }
        Ok(())
    }

    #[test]
    fn pinned_cache_creation_retains_reviewed_execution_dtype() -> Result<(), String> {
        let config = conformance_config(2, 2);
        for (dtype, marker) in [
            (DType::F32, "Tensor[dims 16, 2; f32]"),
            (DType::F16, "Tensor[dims 16, 2; f16]"),
            (DType::BF16, "Tensor[dims 16, 2; bf16]"),
        ] {
            let cache = Cache::new(true, dtype, &config, &Device::Cpu)
                .map_err(|error| error.to_string())?;
            assert_eq!(format!("{cache:?}").matches(marker).count(), 2);
        }
        Ok(())
    }

    fn conformance_config(attention_heads: usize, kv_heads: usize) -> Config {
        Config {
            hidden_size: 8,
            intermediate_size: 16,
            vocab_size: 16,
            num_hidden_layers: 1,
            num_attention_heads: attention_heads,
            num_key_value_heads: kv_heads,
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

    fn conformance_model(config: &Config, dtype: DType) -> Result<Llama, String> {
        let mut tensors = HashMap::new();
        insert_zeros(
            &mut tensors,
            "model.embed_tokens.weight",
            (config.vocab_size, config.hidden_size),
            dtype,
        )?;
        insert_zeros(
            &mut tensors,
            "lm_head.weight",
            (config.vocab_size, config.hidden_size),
            dtype,
        )?;
        insert_ones(&mut tensors, "model.norm.weight", config.hidden_size, dtype)?;
        let head_dimension = config.hidden_size / config.num_attention_heads;
        let kv_width = head_dimension * config.num_key_value_heads;
        for (projection, output) in [
            ("q_proj", config.hidden_size),
            ("k_proj", kv_width),
            ("v_proj", kv_width),
            ("o_proj", config.hidden_size),
        ] {
            insert_zeros(
                &mut tensors,
                &format!("model.layers.0.self_attn.{projection}.weight"),
                (output, config.hidden_size),
                dtype,
            )?;
        }
        for normalization in ["input_layernorm", "post_attention_layernorm"] {
            insert_ones(
                &mut tensors,
                &format!("model.layers.0.{normalization}.weight"),
                config.hidden_size,
                dtype,
            )?;
        }
        for projection in ["gate_proj", "up_proj"] {
            insert_zeros(
                &mut tensors,
                &format!("model.layers.0.mlp.{projection}.weight"),
                (config.intermediate_size, config.hidden_size),
                dtype,
            )?;
        }
        insert_zeros(
            &mut tensors,
            "model.layers.0.mlp.down_proj.weight",
            (config.hidden_size, config.intermediate_size),
            dtype,
        )?;

        Llama::load(
            VarBuilder::from_tensors(tensors, dtype, &Device::Cpu),
            config,
        )
        .map_err(|error| error.to_string())
    }

    fn insert_zeros<S: Into<candle_core::Shape>>(
        tensors: &mut HashMap<String, Tensor>,
        name: &str,
        shape: S,
        dtype: DType,
    ) -> Result<(), String> {
        let tensor =
            Tensor::zeros(shape, dtype, &Device::Cpu).map_err(|error| error.to_string())?;
        tensors.insert(name.to_owned(), tensor);
        Ok(())
    }

    fn insert_ones(
        tensors: &mut HashMap<String, Tensor>,
        name: &str,
        length: usize,
        dtype: DType,
    ) -> Result<(), String> {
        let tensor =
            Tensor::ones(length, dtype, &Device::Cpu).map_err(|error| error.to_string())?;
        tensors.insert(name.to_owned(), tensor);
        Ok(())
    }
}
