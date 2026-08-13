//! Llama source inspection, exact load preparation, and transactional materialization.

mod cleanup;
mod config;
mod configuration_policy;
mod construction;
mod footprint;
mod identity;
mod manifest;
mod payload;
mod prepared;
mod safetensors;
mod scalar;
mod schema;
mod transfer_batch;
mod transfer_plan;

use std::collections::HashMap;

use candle_core::Device;
use candle_transformers::models::llama::Config;
use domain_contracts::{
    BackendFailureKind, BackendId, CapabilitySet, DeviceKind, ExecutionDevice, FailedLoad,
    LoadConfiguration, LoadError, LoadPlan, ModelArchitecture, ModelCapabilities, ModelDescriptor,
    ModelLoader, ModelMetadata, QuantizationFormat, ScalarType,
};

use crate::device::{CandleDeviceSummary, prepare_execution_device};
use crate::failure::{
    CODE_MODEL_LOAD, CODE_TENSOR_MAP_ALLOCATION, CODE_UNSUPPORTED_SCALAR, candle_cuda_failure_kind,
    failure,
};
use crate::model::{CandleLlamaModel, CandleLlamaModelParameters};
use crate::source::{CandleConfigurationSource, CandleLlamaSource};

pub use self::cleanup::CandleLlamaFailedPreparation;
use self::footprint::{calculate, sequence_cache_bytes_per_token, validate_memory_plan};
use self::manifest::{InspectedShard, InspectionLimits};
pub use self::prepared::CandleLlamaPreparedLoad;
use self::scalar::{execution_scalar_type, select_execution_dtype, select_required_primary};
use self::transfer_batch::TransferBatchOwner;
use self::transfer_plan::{MAXIMUM_BATCH_ENTRIES, TransferPlan};

pub(super) const VERIFICATION_BUFFER_BYTES: usize = 64 * 1024;
pub(super) const VERIFICATION_BUFFER_BYTES_U64: u64 = 64 * 1024;

/// Cold-path loader for unquantized Hugging Face Llama Safetensors.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")),
    derive(Copy, PartialEq, Eq)
)]
pub struct CandleLlamaLoader {
    backend: BackendId,
    #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
    load_observation: Option<crate::CandleLoadObservationRecorder>,
}

impl CandleLlamaLoader {
    /// Creates a loader with the stable backend identifier assigned by the app.
    #[must_use]
    pub const fn new(backend: BackendId) -> Self {
        Self {
            backend,
            #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
            load_observation: None,
        }
    }

    /// Creates a one-attempt loader carrying non-production observation state.
    #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
    #[doc(hidden)]
    #[must_use]
    pub fn with_load_observation(
        backend: BackendId,
        observation: crate::CandleLoadObservationRecorder,
    ) -> Self {
        Self {
            backend,
            load_observation: Some(observation),
        }
    }

    /// Returns this adapter's backend identifier.
    #[must_use]
    pub const fn backend_id(&self) -> BackendId {
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
        &self,
        execution_device: ExecutionDevice,
    ) -> Result<CandleDeviceSummary, LoadError> {
        prepare_execution_device(self.backend, execution_device).map(|prepared| prepared.summary)
    }

    fn inspect_source(
        backend: BackendId,
        source: &CandleLlamaSource,
    ) -> Result<InspectedSource, LoadError> {
        Self::inspect_source_with_limits(backend, source, &InspectionLimits::PRODUCTION)
    }

    fn inspect_source_with_limits(
        backend: BackendId,
        source: &CandleLlamaSource,
        limits: &InspectionLimits,
    ) -> Result<InspectedSource, LoadError> {
        let parsed_config = match source.configuration() {
            CandleConfigurationSource::Path(path) => {
                config::read_and_parse(backend, path.as_path())?
            }
            CandleConfigurationSource::Bytes(bytes) => {
                config::parse_bytes(backend, bytes.as_slice())?
            }
        };
        let mut shards = manifest::inspect_weight_shards(backend, source.weight_shards(), limits)?;
        let observed_tensor_scalar_types = manifest::observed_scalar_types(&shards);
        let required_schema =
            schema::validate_and_mark(backend, &parsed_config.config, &mut shards)?;
        let primary_scalar_type = select_required_primary(
            backend,
            required_schema.scalar_types,
            parsed_config.declaration,
        )?;
        let cpu_execution_dtype =
            select_execution_dtype(backend, primary_scalar_type, DeviceKind::Cpu, false)?;
        let cpu_footprints = calculate(
            backend,
            &shards,
            DeviceKind::Cpu,
            cpu_execution_dtype,
            None,
            0,
        )?;
        let sequence_cache_bytes_per_token =
            sequence_cache_bytes_per_token(backend, &parsed_config.config, cpu_execution_dtype)?;

        let context_length = u32::try_from(parsed_config.config.max_position_embeddings)
            .map_err(|_| numeric_error(backend))?;
        let vocabulary_size =
            u32::try_from(parsed_config.config.vocab_size).map_err(|_| numeric_error(backend))?;
        let operations = CapabilitySet::PREFILL
            .union(CapabilitySet::INCREMENTAL_DECODE)
            .union(CapabilitySet::MULTIPLE_SEQUENCES)
            .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);
        let descriptor = ModelDescriptor {
            backend,
            metadata: ModelMetadata {
                architecture: ModelArchitecture::Llama,
                configuration_declared_scalar_type: parsed_config.declaration,
                observed_tensor_scalar_types,
                quantization: QuantizationFormat::None,
                vocabulary_size,
                context_length,
            },
            capabilities: ModelCapabilities {
                operations,
                maximum_context_tokens: context_length,
                maximum_sequences: u32::MAX,
                maximum_prefill_batch: context_length,
            },
            estimated_footprint: cpu_footprints.final_footprint,
            sequence_cache_bytes_per_token,
        };

        Ok(InspectedSource {
            config: parsed_config.config,
            descriptor,
            primary_scalar_type,
            required_tensor_count: required_schema.tensor_count,
            shards,
        })
    }
}

impl ModelLoader for CandleLlamaLoader {
    type Source = CandleLlamaSource;
    type Prepared = CandleLlamaPreparedLoad;
    type FailedPreparation = CandleLlamaFailedPreparation;
    type Model = CandleLlamaModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        Self::inspect_source(self.backend, source).map(|inspected| inspected.descriptor)
    }

    fn prepare_load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Prepared, LoadError> {
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        let load_observation = self.load_observation.take();
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &load_observation {
            observation.preparation_started();
        }
        let result: Result<Self::Prepared, LoadError> = (|| {
            // Config, every header/tensor, duplicate detection, exact offsets, and
            // required schema validation complete before device initialization.
            let mut inspected = Self::inspect_source(self.backend, source)?;
            #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
            if let Some(observation) = &load_observation {
                identity::establish_all_observed(self.backend, &mut inspected.shards, observation)?;
            } else {
                identity::establish_all(self.backend, &mut inspected.shards)?;
            }
            #[cfg(not(any(feature = "benchmark-observation", feature = "cuda-hardware-tests")))]
            identity::establish_all(self.backend, &mut inspected.shards)?;

            let mut final_tensors = HashMap::new();
            final_tensors
                .try_reserve(inspected.required_tensor_count)
                .map_err(|_| host_memory_failure(self.backend, CODE_TENSOR_MAP_ALLOCATION))?;

            let prepared_device =
                prepare_execution_device(self.backend, configuration.execution_device)?;
            let execution_dtype = select_execution_dtype(
                self.backend,
                inspected.primary_scalar_type,
                configuration.execution_device.kind,
                prepared_device.summary.supports_bf16,
            )?;
            let execution_scalar_type = execution_scalar_type(self.backend, execution_dtype)?;
            let (transfer_plan, transfer_batch, transfer_owner_metadata_bytes) =
                match configuration.execution_device.kind {
                    DeviceKind::Cpu => (None, None, 0),
                    DeviceKind::Cuda => {
                        let transfer_plan =
                            TransferPlan::build(self.backend, &inspected.shards, execution_dtype)?;
                        let transfer_batch =
                            TransferBatchOwner::allocate(self.backend, MAXIMUM_BATCH_ENTRIES)?;
                        let metadata_bytes = transfer_batch.metadata_bytes(self.backend)?;
                        (Some(transfer_plan), Some(transfer_batch), metadata_bytes)
                    }
                    _ => return Err(LoadError::InvalidConfiguration),
                };
            let footprints = calculate(
                self.backend,
                &inspected.shards,
                configuration.execution_device.kind,
                execution_dtype,
                transfer_plan.as_ref(),
                transfer_owner_metadata_bytes,
            )?;
            inspected.descriptor.sequence_cache_bytes_per_token =
                sequence_cache_bytes_per_token(self.backend, &inspected.config, execution_dtype)?;
            validate_memory_plan(
                self.backend,
                footprints.loading_peak_footprint,
                configuration.memory_budget,
                prepared_device.summary.available_memory_bytes,
            )?;

            let plan = LoadPlan {
                accepted_configuration: *configuration,
                descriptor: inspected.descriptor,
                execution_scalar_type,
                final_footprint: footprints.final_footprint,
                loading_peak_footprint: footprints.loading_peak_footprint,
            };
            Ok(CandleLlamaPreparedLoad {
                backend: self.backend,
                plan,
                config: Some(inspected.config),
                execution_dtype,
                device: Some(prepared_device.device),
                shards: inspected.shards,
                final_tensors,
                pending_source_tensor: None,
                pending_host_tensor: None,
                pending_device_tensor: None,
                transfer_plan,
                transfer_batch,
                next_transfer_batch_index: 0,
                next_transfer_entry_index: 0,
                constructed_model: None,
                #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
                load_observation: load_observation.clone(),
                cleanup_complete: false,
            })
        })();
        #[cfg(any(feature = "benchmark-observation", feature = "cuda-hardware-tests"))]
        if let Some(observation) = &load_observation {
            match &result {
                Ok(prepared) => observation.preparation_succeeded(&prepared.plan),
                Err(_) => observation.preparation_failed(),
            }
        }
        result
    }

    fn load_prepared(
        &mut self,
        mut prepared: Self::Prepared,
    ) -> Result<Self::Model, FailedLoad<Self::FailedPreparation>> {
        prepared.record_materialization_started();
        if let Err(error) = prepared.materialize() {
            return Err(failed_materialization(error, prepared));
        }
        if prepared.constructed_model.is_none()
            || prepared.config.is_none()
            || prepared.device.is_none()
            || prepared.pending_source_tensor.is_some()
            || prepared.pending_host_tensor.is_some()
            || prepared.pending_device_tensor.is_some()
            || prepared
                .transfer_batch
                .as_ref()
                .is_some_and(|batch| !batch.is_empty())
            || prepared.next_transfer_entry_index != 0
            || prepared
                .transfer_plan
                .as_ref()
                .is_some_and(|plan| prepared.next_transfer_batch_index != plan.len())
        {
            return Err(failed_materialization(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        }

        let Some(loaded) = prepared.constructed_model.take() else {
            return Err(failed_materialization(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };
        let Some(config) = prepared.config.take() else {
            prepared.constructed_model = Some(loaded);
            return Err(failed_materialization(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };
        let Some(device) = prepared.device.take() else {
            prepared.constructed_model = Some(loaded);
            prepared.config = Some(config);
            return Err(failed_materialization(
                invalid_model_failure(self.backend, CODE_MODEL_LOAD),
                prepared,
            ));
        };

        // The constructed Llama owns shallow handles for every required tensor.
        // Synchronization already completed, so the load map and retained source
        // inventory can now be explicitly released before publication.
        prepared.final_tensors.clear();
        prepared.shards.clear();
        if let Some(batch) = prepared.transfer_batch.as_mut() {
            batch.clear();
        }
        prepared.transfer_batch = None;
        prepared.transfer_plan = None;
        prepared.cleanup_complete = true;
        prepared.record_materialization_succeeded();
        let configuration = prepared.plan.accepted_configuration;
        Ok(CandleLlamaModel::new(
            CandleLlamaModelParameters {
                backend: prepared.backend,
                handle: configuration.handle,
                execution_device: configuration.execution_device,
                descriptor: prepared.plan.descriptor,
                reported_footprint: prepared.plan.final_footprint,
                config,
                dtype: prepared.execution_dtype,
                execution_scalar_type: prepared.plan.execution_scalar_type,
                device,
            },
            loaded,
        ))
    }
}

fn failed_materialization(
    error: LoadError,
    prepared: CandleLlamaPreparedLoad,
) -> FailedLoad<CandleLlamaFailedPreparation> {
    prepared.record_materialization_failed();
    FailedLoad::new(error, prepared.into_failed())
}

#[derive(Debug)]
struct InspectedSource {
    config: Config,
    descriptor: ModelDescriptor,
    primary_scalar_type: ScalarType,
    required_tensor_count: usize,
    shards: Vec<InspectedShard>,
}

pub(super) const fn unsupported_scalar(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::Unsupported,
        CODE_UNSUPPORTED_SCALAR,
    ))
}

pub(super) const fn invalid_model_failure(backend: BackendId, code: u32) -> LoadError {
    LoadError::Backend(failure(backend, BackendFailureKind::InvalidModel, code))
}

pub(super) const fn host_memory_failure(backend: BackendId, code: u32) -> LoadError {
    LoadError::Backend(failure(backend, BackendFailureKind::HostMemory, code))
}

fn numeric_error(backend: BackendId) -> LoadError {
    invalid_model_failure(backend, crate::failure::CODE_NUMERIC_OVERFLOW)
}

pub(super) fn map_candle_load_error(
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
