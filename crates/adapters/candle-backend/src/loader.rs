//! Llama source inspection, device-aware memory planning, and model loading.

use std::collections::HashMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{Config, Llama, LlamaConfig};
use domain_contracts::{
    BackendFailureKind, BackendId, CapabilitySet, DeviceKind, ExecutionDevice, LoadConfiguration,
    LoadError, LoadPlan, MemoryBudget, MemoryFootprint, MemoryKind, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelLoader, ModelMetadata, QuantizationFormat,
};

use crate::device::{CandleDeviceSummary, PreparedExecutionDevice, prepare_execution_device};
use crate::failure::{
    CODE_CONFIG_DECODE, CODE_CONFIG_READ, CODE_DUPLICATE_TENSOR, CODE_MODEL_LOAD,
    CODE_MODEL_LOAD_PANIC, CODE_NUMERIC_OVERFLOW, CODE_UNSUPPORTED_SCALAR, CODE_WEIGHT_LOAD,
    CODE_WEIGHT_METADATA, candle_cuda_failure_kind, failure,
};
use crate::model::{CandleLlamaModel, CandleLlamaModelParameters};
use crate::source::{CandleLlamaSource, CandleScalarType};

/// Cold-path loader for unquantized Hugging Face Llama Safetensors.
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

    /// Initializes an explicitly selected device and returns stable observed facts.
    ///
    /// CPU discovery does not load a CUDA library in a default build. A CUDA
    /// request in a build without the `cuda` feature fails explicitly.
    ///
    /// # Errors
    ///
    /// Returns [`LoadError`] when the identity is invalid, support was not
    /// compiled, or native device initialization or discovery fails.
    pub fn discover_device(
        self,
        execution_device: ExecutionDevice,
    ) -> Result<CandleDeviceSummary, LoadError> {
        prepare_execution_device(self.backend, execution_device).map(|prepared| prepared.summary)
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
        let cpu_execution_dtype = source
            .scalar_type()
            .execution_dtype(DeviceKind::Cpu, false)
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let cpu_footprint = planned_model_footprint(
            DeviceKind::Cpu,
            scaled_weight_bytes(
                source_weight_bytes,
                source.scalar_type().weight_bytes_per_element(),
                dtype_bytes(cpu_execution_dtype),
            )?,
            largest_shard_bytes,
            cache_bytes_per_token(&config, dtype_bytes(cpu_execution_dtype))?,
        )?;
        let metadata = ModelMetadata {
            architecture: ModelArchitecture::Llama,
            scalar_type: source.scalar_type().source_scalar_type(),
            quantization: QuantizationFormat::None,
            vocabulary_size,
            context_length,
        };
        let operations = CapabilitySet::PREFILL
            .union(CapabilitySet::INCREMENTAL_DECODE)
            .union(CapabilitySet::MULTIPLE_SEQUENCES)
            .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        let descriptor = ModelDescriptor {
            backend: self.backend,
            metadata,
            capabilities: ModelCapabilities {
                operations,
                maximum_context_tokens: context_length,
                maximum_sequences: u32::MAX,
                maximum_prefill_batch: context_length,
            },
            // Inspection is artifact-only and therefore retains the mandatory
            // CPU estimate. Device-specific charging belongs to LoadPlan.
            estimated_footprint: cpu_footprint,
        };

        Ok(InspectedSource {
            config,
            descriptor,
            source_weight_bytes,
            largest_shard_bytes,
        })
    }

    fn prepare_load(
        self,
        source: &CandleLlamaSource,
        configuration: &LoadConfiguration,
    ) -> Result<PreparedLoad, LoadError> {
        let inspected = self.inspect_source(source)?;
        let prepared_device =
            prepare_execution_device(self.backend, configuration.execution_device)?;
        let execution_dtype = source
            .scalar_type()
            .execution_dtype(
                configuration.execution_device.kind,
                prepared_device.summary.supports_bf16,
            )
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let execution_scalar_type = CandleScalarType::execution_scalar_type(execution_dtype)
            .ok_or_else(|| unsupported_scalar(self.backend))?;
        let execution_bytes = dtype_bytes(execution_dtype);
        let execution_weight_bytes = scaled_weight_bytes(
            inspected.source_weight_bytes,
            source.scalar_type().weight_bytes_per_element(),
            execution_bytes,
        )?;
        let footprint = planned_model_footprint(
            configuration.execution_device.kind,
            execution_weight_bytes,
            inspected.largest_shard_bytes,
            cache_bytes_per_token(&inspected.config, execution_bytes)?,
        )?;
        validate_memory_plan(
            footprint,
            configuration.memory_budget,
            prepared_device.summary.available_memory_bytes,
        )?;

        Ok(PreparedLoad {
            plan: LoadPlan {
                descriptor: inspected.descriptor,
                execution_scalar_type,
                expected_footprint: footprint,
            },
            inspected,
            prepared_device,
            execution_dtype,
        })
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
        self.prepare_load(source, configuration)
            .map(|prepared| prepared.plan)
    }

    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError> {
        let prepared = self.prepare_load(source, configuration)?;
        let PreparedLoad {
            plan,
            inspected,
            prepared_device,
            execution_dtype,
        } = prepared;
        let device = prepared_device.device;
        let weight_dtype = source.scalar_type().weight_dtype();
        let mut tensors = HashMap::<String, Tensor>::new();

        for path in source.weight_paths() {
            let shard = candle_core::safetensors::load(path, &device).map_err(|error| {
                map_candle_load_error(self.backend, &device, &error, CODE_WEIGHT_LOAD)
            })?;
            for (name, tensor) in shard {
                if tensor.dtype() != weight_dtype {
                    return Err(LoadError::UnsupportedFormat);
                }
                let tensor = if weight_dtype == execution_dtype {
                    tensor
                } else {
                    tensor.to_dtype(execution_dtype).map_err(|error| {
                        map_candle_load_error(self.backend, &device, &error, CODE_WEIGHT_LOAD)
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
        .map_err(|error| map_candle_load_error(self.backend, &device, &error, CODE_MODEL_LOAD))?;

        Ok(CandleLlamaModel::new(
            CandleLlamaModelParameters {
                backend: self.backend,
                handle: configuration.handle,
                execution_device: configuration.execution_device,
                descriptor: plan.descriptor,
                accounted_footprint: plan.expected_footprint,
                config: inspected.config,
                dtype: execution_dtype,
                execution_scalar_type: plan.execution_scalar_type,
                device,
            },
            loaded,
        ))
    }
}

struct PreparedLoad {
    plan: LoadPlan,
    inspected: InspectedSource,
    prepared_device: PreparedExecutionDevice,
    execution_dtype: DType,
}

#[derive(Clone, Debug)]
struct InspectedSource {
    config: Config,
    descriptor: ModelDescriptor,
    source_weight_bytes: u64,
    largest_shard_bytes: u64,
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

fn planned_model_footprint(
    device_kind: DeviceKind,
    execution_weight_bytes: u64,
    largest_shard_bytes: u64,
    cache_bytes_per_token: u64,
) -> Result<MemoryFootprint, LoadError> {
    match device_kind {
        DeviceKind::Cpu => Ok(MemoryFootprint {
            host_weight_bytes: execution_weight_bytes,
            device_weight_bytes: 0,
            // Candle's safe loader reads a complete shard before materializing
            // its tensors. This conservatively reserves that transient peak.
            host_working_bytes: largest_shard_bytes,
            device_working_bytes: 0,
            cache_bytes_per_token,
        }),
        DeviceKind::Cuda => Ok(MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: execution_weight_bytes,
            host_working_bytes: largest_shard_bytes,
            device_working_bytes: 0,
            cache_bytes_per_token,
        }),
        _ => Err(LoadError::InvalidConfiguration),
    }
}

fn validate_memory_plan(
    footprint: MemoryFootprint,
    budget: MemoryBudget,
    currently_available_device_bytes: Option<u64>,
) -> Result<(), LoadError> {
    let required_host = footprint.host_bytes();
    if required_host > budget.host_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Host,
            required_bytes: required_host,
            available_bytes: budget.host_bytes,
        });
    }
    let required_device = footprint.device_bytes();
    if required_device > budget.device_bytes {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: budget.device_bytes,
        });
    }
    if let Some(available) = currently_available_device_bytes
        && required_device > available
    {
        return Err(LoadError::InsufficientMemory {
            kind: MemoryKind::Device,
            required_bytes: required_device,
            available_bytes: available,
        });
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

const fn dtype_bytes(dtype: DType) -> u64 {
    match dtype {
        DType::F32 => 4,
        DType::F16 | DType::BF16 => 2,
        _ => 0,
    }
}

const fn unsupported_scalar(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::Unsupported,
        CODE_UNSUPPORTED_SCALAR,
    ))
}

fn map_candle_load_error(
    backend: BackendId,
    device: &Device,
    error: &candle_core::Error,
    code: u32,
) -> LoadError {
    let kind = if device.is_cuda() {
        candle_cuda_failure_kind(error).unwrap_or(BackendFailureKind::InvalidModel)
    } else {
        BackendFailureKind::InvalidModel
    };
    LoadError::Backend(failure(backend, kind, code))
}

#[cfg(test)]
mod tests {
    use super::{planned_model_footprint, scaled_weight_bytes, validate_memory_plan};
    use domain_contracts::{DeviceKind, LoadError, MemoryBudget, MemoryFootprint, MemoryKind};

    #[test]
    fn cpu_and_cuda_plans_charge_distinct_memory_domains() {
        assert_eq!(
            planned_model_footprint(DeviceKind::Cpu, 400, 80, 32),
            Ok(MemoryFootprint {
                host_weight_bytes: 400,
                device_weight_bytes: 0,
                host_working_bytes: 80,
                device_working_bytes: 0,
                cache_bytes_per_token: 32,
            })
        );

        assert_eq!(
            planned_model_footprint(DeviceKind::Cuda, 200, 80, 16),
            Ok(MemoryFootprint {
                host_weight_bytes: 0,
                device_weight_bytes: 200,
                host_working_bytes: 80,
                device_working_bytes: 0,
                cache_bytes_per_token: 16,
            })
        );
    }

    #[test]
    fn memory_plan_rejections_identify_host_and_device_domains() {
        let footprint = MemoryFootprint {
            host_weight_bytes: 0,
            device_weight_bytes: 200,
            host_working_bytes: 80,
            device_working_bytes: 20,
            cache_bytes_per_token: 0,
        };
        assert!(matches!(
            validate_memory_plan(
                footprint,
                MemoryBudget {
                    host_bytes: 79,
                    device_bytes: 220,
                },
                Some(220),
            ),
            Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Host,
                required_bytes: 80,
                available_bytes: 79,
            })
        ));
        assert!(matches!(
            validate_memory_plan(
                footprint,
                MemoryBudget {
                    host_bytes: 80,
                    device_bytes: 219,
                },
                Some(220),
            ),
            Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Device,
                required_bytes: 220,
                available_bytes: 219,
            })
        ));
        assert!(matches!(
            validate_memory_plan(
                footprint,
                MemoryBudget {
                    host_bytes: 80,
                    device_bytes: 220,
                },
                Some(219),
            ),
            Err(LoadError::InsufficientMemory {
                kind: MemoryKind::Device,
                required_bytes: 220,
                available_bytes: 219,
            })
        ));
    }

    #[test]
    fn weight_scaling_rejects_arithmetic_overflow() {
        assert_eq!(
            scaled_weight_bytes(u64::MAX, 2, 4),
            Err(LoadError::InvalidSource)
        );
    }
}
