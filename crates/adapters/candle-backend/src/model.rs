//! Loaded Candle model and device-aware sequence implementations.

use candle_core::{DType, Device, Storage, Tensor};
use candle_transformers::models::llama::{Cache, Config, Llama};
use domain_contracts::{
    BackendFailureKind, BackendId, BackendSequence, CapacityExhausted, CapacityResource,
    DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceKind, ExecutionDevice, LoadedModel,
    MemoryFootprint, ModelDescriptor, ModelError, ModelHandle, PrefillBufferRequirements,
    PrefillInput, PrefillOutcome, PreparedDecodeBuffers, PreparedPrefillBuffers, ScalarType,
    SequenceConfiguration, SequenceError, SequenceId, SequencePlan, SequenceState,
    SynchronizationError,
};

use crate::failure::{
    CODE_CACHE_CREATE, CODE_FORWARD, CODE_INPUT_TENSOR, CODE_LOGITS_LAYOUT, CODE_LOGITS_STORAGE,
    CODE_LOGITS_TRANSFER, CODE_NUMERIC_OVERFLOW, CODE_RESERVATION, CODE_SYNCHRONIZE,
    candle_cuda_failure_kind, failure,
};
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

    fn sequence_footprint(
        &self,
        configuration: SequenceConfiguration,
    ) -> Result<MemoryFootprint, ModelError> {
        let maximum_tokens = u64::from(configuration.maximum_tokens.get());
        let cache_bytes = self
            .descriptor
            .sequence_cache_bytes_per_token
            .checked_mul(maximum_tokens)
            .ok_or_else(|| numeric_model_error(self.backend))?;
        let head_dimension = self
            .config
            .hidden_size
            .checked_div(self.config.num_attention_heads)
            .ok_or_else(|| numeric_model_error(self.backend))?;
        let scalar_bytes =
            dtype_bytes(self.dtype).ok_or_else(|| numeric_model_error(self.backend))?;
        let rope_bytes = u64::try_from(self.config.max_position_embeddings)
            .ok()
            .and_then(|positions| positions.checked_mul(u64::try_from(head_dimension).ok()?))
            .and_then(|elements| elements.checked_mul(scalar_bytes))
            .ok_or_else(|| numeric_model_error(self.backend))?;
        let working_bytes = cache_bytes
            .checked_add(rope_bytes)
            .ok_or_else(|| numeric_model_error(self.backend))?;

        match self.execution_device.kind {
            DeviceKind::Cpu => Ok(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: working_bytes,
                device_working_bytes: 0,
            }),
            DeviceKind::Cuda => Ok(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 0,
                host_working_bytes: 0,
                device_working_bytes: working_bytes,
            }),
            _ => Err(ModelError::Unsupported),
        }
    }

    fn execute(
        &self,
        cache: &mut Cache,
        position: usize,
        tokens: &[u32],
    ) -> Result<Tensor, SequenceError> {
        let input =
            Tensor::from_slice(tokens, (1, tokens.len()), &self.device).map_err(|error| {
                sequence_candle_failure(self.backend, &self.device, &error, CODE_INPUT_TENSOR)
            })?;
        self.device
            .with_context(|| self.model.forward(&input, position, cache))
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
            expected_footprint: self.sequence_footprint(*configuration)?,
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
        let mut token_staging = Vec::new();
        token_staging
            .try_reserve_exact(maximum_prefill)
            .map_err(|_| {
                ModelError::Backend(failure(
                    self.backend,
                    BackendFailureKind::HostMemory,
                    CODE_RESERVATION,
                ))
            })?;
        let cache = Cache::new(true, self.dtype, &self.config, &self.device).map_err(|error| {
            model_candle_failure(self.backend, &self.device, &error, CODE_CACHE_CREATE)
        })?;

        Ok(CandleLlamaSequence {
            id: sequence_id,
            state: SequenceState::Empty,
            position: 0,
            token_capacity: usize::try_from(configuration.maximum_tokens.get())
                .map_err(|_| numeric_model_error(self.backend))?,
            maximum_prefill,
            cache,
            token_staging,
            expected_footprint: plan.expected_footprint,
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

        sequence.token_staging.clear();
        for token in input.tokens {
            sequence.token_staging.push(token.get());
        }
        let position = sequence.position;
        let cache = &mut sequence.cache;
        let staging = sequence.token_staging.as_slice();
        let logits = self.execute(cache, position, staging)?;
        let logits_written = if input.emit_logits {
            let required_logits = buffers.required_logits();
            self.copy_logits(&logits, buffers.logits_mut(), required_logits)?
        } else {
            0
        };
        sequence.position = sequence
            .position
            .checked_add(input.tokens.len())
            .ok_or_else(|| numeric_sequence_error(self.backend))?;
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

        let token = [input.token.get()];
        let position = sequence.position;
        let logits = self.execute(&mut sequence.cache, position, &token)?;
        let required_logits = buffers.required_logits();
        let logits_written = self.copy_logits(&logits, buffers.logits_mut(), required_logits)?;
        sequence.position = sequence
            .position
            .checked_add(1)
            .ok_or_else(|| numeric_sequence_error(self.backend))?;

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
    expected_footprint: MemoryFootprint,
}

impl CandleLlamaSequence {
    /// Returns the exact planned maximum sequence-specific ownership.
    #[must_use]
    pub const fn expected_footprint(&self) -> MemoryFootprint {
        self.expected_footprint
    }

    /// Returns the maximum prompt tokens accepted by one prefill call.
    #[must_use]
    pub const fn maximum_prefill_batch(&self) -> usize {
        self.maximum_prefill
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

const fn dtype_bytes(dtype: DType) -> Option<u64> {
    match dtype {
        DType::F32 => Some(4),
        DType::F16 | DType::BF16 => Some(2),
        _ => None,
    }
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
