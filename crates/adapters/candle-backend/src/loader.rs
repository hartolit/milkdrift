//! Llama source inspection, memory planning, and model loading.

use std::collections::HashMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{Config, Llama, LlamaConfig};
use domain_contracts::{
    BackendFailureKind, BackendId, CapabilitySet, DeviceKind, LoadConfiguration, LoadError,
    LoadPlan, MemoryFootprint, ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelLoader,
    ModelMetadata, QuantizationFormat,
};

use crate::failure::{
    CODE_CONFIG_DECODE, CODE_CONFIG_READ, CODE_DUPLICATE_TENSOR, CODE_MODEL_LOAD,
    CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW, CODE_WEIGHT_LOAD, CODE_WEIGHT_METADATA, failure,
};
use crate::model::CandleLlamaModel;
use crate::source::CandleLlamaSource;

/// Cold-path CPU loader for unquantized Hugging Face Llama Safetensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandleLlamaLoader {
    backend: BackendId,
}

impl CandleLlamaLoader {
    /// Creates a loader with the stable backend identifier assigned by the app.
    #[must_use]
    pub const fn new(backend: BackendId) -> Self {
        Self { backend }
    }

    /// Returns this adapter's backend identifier.
    #[must_use]
    pub const fn backend_id(self) -> BackendId {
        self.backend
    }

    fn inspect_source(self, source: &CandleLlamaSource) -> Result<InspectedSource, LoadError> {
        let bytes = fs::read(source.config_path()).map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::InvalidModel,
                CODE_CONFIG_READ,
            ))
        })?;
        let hugging_face: LlamaConfig = serde_json::from_slice(&bytes).map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::InvalidModel,
                CODE_CONFIG_DECODE,
            ))
        })?;
        let config = hugging_face.into_config(false);
        validate_config(&config)?;

        let (source_weight_bytes, largest_shard_bytes) =
            source
                .weight_paths()
                .iter()
                .try_fold((0_u64, 0_u64), |(total, largest), path| {
                    let length = fs::metadata(path)
                        .map_err(|_| {
                            LoadError::Backend(failure(
                                self.backend,
                                BackendFailureKind::InvalidModel,
                                CODE_WEIGHT_METADATA,
                            ))
                        })?
                        .len();
                    let total = total.checked_add(length).ok_or_else(|| {
                        LoadError::Backend(failure(
                            self.backend,
                            BackendFailureKind::InvalidModel,
                            CODE_NUMERIC_OVERFLOW,
                        ))
                    })?;
                    Ok::<_, LoadError>((total, largest.max(length)))
                })?;

        let context_length =
            u32::try_from(config.max_position_embeddings).map_err(|_| LoadError::InvalidSource)?;
        let vocabulary_size =
            u32::try_from(config.vocab_size).map_err(|_| LoadError::InvalidSource)?;
        let scalar_type = source.scalar_type();
        let host_weight_bytes = scaled_weight_bytes(
            source_weight_bytes,
            scalar_type.weight_bytes_per_element(),
            scalar_type.execution_bytes_per_element(),
        )?;
        let cache_bytes_per_token =
            cache_bytes_per_token(&config, scalar_type.execution_bytes_per_element())?;
        let metadata = ModelMetadata {
            architecture: ModelArchitecture::Llama,
            scalar_type: source.scalar_type().domain_type(),
            quantization: QuantizationFormat::None,
            vocabulary_size,
            context_length,
        };
        let operations = CapabilitySet::PREFILL
            .union(CapabilitySet::INCREMENTAL_DECODE)
            .union(CapabilitySet::MULTIPLE_SEQUENCES)
            .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        let footprint = MemoryFootprint {
            host_weight_bytes,
            device_weight_bytes: 0,
            // Candle's safe loader reads a complete shard before materializing its
            // tensors. Reserving the largest shard models that transient peak.
            host_working_bytes: largest_shard_bytes,
            device_working_bytes: 0,
            cache_bytes_per_token,
        };
        let descriptor = ModelDescriptor {
            backend: self.backend,
            metadata,
            capabilities: ModelCapabilities {
                operations,
                maximum_context_tokens: context_length,
                maximum_sequences: u32::MAX,
                maximum_prefill_batch: context_length,
            },
            estimated_footprint: footprint,
        };

        Ok(InspectedSource { config, descriptor })
    }
}

impl ModelLoader for CandleLlamaLoader {
    type Source = CandleLlamaSource;
    type Model = CandleLlamaModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        self.inspect_source(source)
            .map(|inspected| inspected.descriptor)
    }

    fn plan_load(
        &self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<LoadPlan, LoadError> {
        if configuration.device_kind != DeviceKind::Cpu || configuration.device.get() != 0 {
            return Err(LoadError::InvalidConfiguration);
        }

        let inspected = self.inspect_source(source)?;
        let required = inspected.descriptor.estimated_footprint.host_bytes();
        if required > configuration.memory_budget.host_bytes {
            return Err(LoadError::InsufficientMemory {
                required_bytes: required,
                available_bytes: configuration.memory_budget.host_bytes,
            });
        }

        Ok(LoadPlan {
            descriptor: inspected.descriptor,
            expected_footprint: inspected.descriptor.estimated_footprint,
        })
    }

    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError> {
        let plan = self.plan_load(source, configuration)?;
        let inspected = self.inspect_source(source)?;
        let device = Device::Cpu;
        let scalar_type = source.scalar_type();
        let weight_dtype = scalar_type.weight_dtype();
        let execution_dtype = scalar_type.execution_dtype();
        let mut tensors = HashMap::<String, Tensor>::new();

        for path in source.weight_paths() {
            let shard = candle_core::safetensors::load(path, &device).map_err(|_| {
                LoadError::Backend(failure(
                    self.backend,
                    BackendFailureKind::InvalidModel,
                    CODE_WEIGHT_LOAD,
                ))
            })?;
            for (name, tensor) in shard {
                if tensor.dtype() != weight_dtype {
                    return Err(LoadError::UnsupportedFormat);
                }
                let tensor = if weight_dtype == execution_dtype {
                    tensor
                } else {
                    tensor.to_dtype(execution_dtype).map_err(|_| {
                        LoadError::Backend(failure(
                            self.backend,
                            BackendFailureKind::InvalidModel,
                            CODE_WEIGHT_LOAD,
                        ))
                    })?
                };
                if tensors.insert(name, tensor).is_some() {
                    return Err(LoadError::Backend(failure(
                        self.backend,
                        BackendFailureKind::InvalidModel,
                        CODE_DUPLICATE_TENSOR,
                    )));
                }
            }
        }

        let variable_builder = VarBuilder::from_tensors(tensors, execution_dtype, &device);
        let loaded = catch_unwind(AssertUnwindSafe(|| {
            Llama::load(variable_builder, &inspected.config)
        }))
        .map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::InvalidModel,
                CODE_MODEL_LOAD_PANIC,
            ))
        })?
        .map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::InvalidModel,
                CODE_MODEL_LOAD,
            ))
        })?;

        Ok(CandleLlamaModel::new(
            self.backend,
            configuration.handle,
            plan.descriptor,
            inspected.config,
            execution_dtype,
            device,
            loaded,
        ))
    }
}

#[derive(Clone, Debug)]
struct InspectedSource {
    config: Config,
    descriptor: ModelDescriptor,
}

fn validate_config(config: &Config) -> Result<(), LoadError> {
    let required_non_zero = [
        config.hidden_size,
        config.intermediate_size,
        config.vocab_size,
        config.num_hidden_layers,
        config.num_attention_heads,
        config.num_key_value_heads,
        config.max_position_embeddings,
    ];
    let head_dimension = if required_non_zero.contains(&0) {
        return Err(LoadError::InvalidSource);
    } else {
        config.hidden_size / config.num_attention_heads
    };
    if !config
        .hidden_size
        .is_multiple_of(config.num_attention_heads)
        || !config
            .num_attention_heads
            .is_multiple_of(config.num_key_value_heads)
        || !head_dimension.is_multiple_of(2)
    {
        return Err(LoadError::InvalidSource);
    }
    Ok(())
}

fn scaled_weight_bytes(
    source_bytes: u64,
    source_bytes_per_element: u64,
    execution_bytes_per_element: u64,
) -> Result<u64, LoadError> {
    source_bytes
        .checked_mul(execution_bytes_per_element)
        .map(|bytes| bytes / source_bytes_per_element)
        .ok_or(LoadError::InvalidSource)
}

fn cache_bytes_per_token(config: &Config, scalar_bytes: u64) -> Result<u64, LoadError> {
    let head_dimension = config.hidden_size / config.num_attention_heads;
    let factors = [
        u64::try_from(config.num_hidden_layers),
        Ok(2_u64),
        u64::try_from(config.num_key_value_heads),
        u64::try_from(head_dimension),
        Ok(scalar_bytes),
    ];
    factors.into_iter().try_fold(1_u64, |total, factor| {
        let factor = factor.map_err(|_| LoadError::InvalidSource)?;
        total.checked_mul(factor).ok_or(LoadError::InvalidSource)
    })
}
