//! GGUF inspection, admission planning, and llama.cpp model loading.

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::Arc;

use domain_contracts::{
    BackendFailureKind, BackendId, CapabilitySet, DeviceKind, LoadConfiguration, LoadError,
    LoadPlan, MemoryFootprint, ModelCapabilities, ModelDescriptor, ModelLoader, ModelMetadata,
};
use llama_cpp_2::context::params::{KvCacheType, LlamaContextParams};
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::model::params::LlamaModelParams;

use crate::digest::{Sha256Digest, sha256_file};
use crate::failure::{
    CODE_CONTENT_DIGEST_MISMATCH, CODE_CONTENT_DIGEST_READ, CODE_CONTEXT_CREATE,
    CODE_METADATA_FORMAT, CODE_METADATA_OPEN, CODE_METADATA_READ, CODE_MODEL_LOAD,
    CODE_MODEL_MISMATCH, CODE_NUMERIC_OVERFLOW, failure,
};
use crate::metadata::{GgufMetadata, MetadataError, inspect_metadata};
use crate::model::GgufModel;
use crate::source::GgufSource;

/// Process-level llama.cpp initialization token shared by loaders and models.
#[derive(Clone)]
pub struct GgufBackendRuntime {
    pub(crate) native: Arc<LlamaBackend>,
}

impl GgufBackendRuntime {
    /// Initializes llama.cpp for one explicitly injected runtime lifetime.
    ///
    /// llama.cpp permits one initialized backend per process. Clone this value
    /// when multiple loaders need to share that initialized backend.
    ///
    /// # Errors
    ///
    /// Returns [`BackendInitializationError`] when llama.cpp rejects process-level
    /// backend initialization.
    pub fn initialize() -> Result<Self, BackendInitializationError> {
        let backend = LlamaBackend::init().map_err(|_| BackendInitializationError)?;
        Ok(Self {
            native: Arc::new(backend),
        })
    }

    /// Returns whether this build and platform support memory-mapped weights.
    #[must_use]
    pub fn supports_mmap(&self) -> bool {
        self.native.supports_mmap()
    }

    /// Returns whether this build and platform support locking model pages.
    #[must_use]
    pub fn supports_mlock(&self) -> bool {
        self.native.supports_mlock()
    }
}

/// Failure to initialize the process-level llama.cpp runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendInitializationError;

impl Display for BackendInitializationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("llama.cpp backend initialization failed")
    }
}

impl Error for BackendInitializationError {}

/// CPU GGUF loader backed by one explicitly injected llama.cpp runtime.
#[derive(Clone)]
pub struct GgufLoader {
    backend: BackendId,
    runtime: GgufBackendRuntime,
}

impl GgufLoader {
    /// Creates a GGUF loader with a stable backend identity.
    #[must_use]
    pub const fn new(backend: BackendId, runtime: GgufBackendRuntime) -> Self {
        Self { backend, runtime }
    }

    /// Returns this adapter's stable backend identifier.
    #[must_use]
    pub const fn backend_id(&self) -> BackendId {
        self.backend
    }

    fn inspect_source(&self, source: &GgufSource) -> Result<InspectedSource, LoadError> {
        let digest_before = verify_source_digest_before(self.backend, source)?;
        let metadata = inspect_metadata(source.path(), source.inspection_limits())
            .map_err(|error| metadata_load_error(self.backend, &error))?;
        let execution = source.execution();
        if execution.context_tokens_per_sequence().get() > metadata.context_length() {
            return Err(LoadError::InvalidConfiguration);
        }

        let total_context = execution
            .total_context_tokens()
            .map_err(|_| LoadError::InvalidConfiguration)?;
        let file_bytes = std::fs::metadata(source.path())
            .map_err(|_| {
                LoadError::Backend(failure(
                    self.backend,
                    BackendFailureKind::InvalidModel,
                    CODE_METADATA_OPEN,
                ))
            })?
            .len();
        let cache_bytes_per_token = metadata
            .cache_bytes_per_token()
            .map_err(|error| metadata_load_error(self.backend, &error))?;
        let cache_bytes = cache_bytes_per_token
            .checked_mul(u64::from(total_context.get()))
            .ok_or_else(|| numeric_load_error(self.backend))?;
        let logits_bytes = u64::from(metadata.vocabulary_size())
            .checked_mul(4)
            .ok_or_else(|| numeric_load_error(self.backend))?;
        let host_working_bytes = cache_bytes
            .checked_add(logits_bytes)
            .ok_or_else(|| numeric_load_error(self.backend))?;

        let operations = CapabilitySet::PREFILL
            .union(CapabilitySet::INCREMENTAL_DECODE)
            .union(CapabilitySet::MULTIPLE_SEQUENCES)
            .union(CapabilitySet::SEQUENCE_RESET);
        let footprint = MemoryFootprint {
            host_weight_bytes: file_bytes,
            device_weight_bytes: 0,
            host_working_bytes,
            device_working_bytes: 0,
            cache_bytes_per_token,
        };
        let descriptor = ModelDescriptor {
            backend: self.backend,
            metadata: ModelMetadata {
                architecture: metadata.architecture(),
                scalar_type: metadata.scalar_type(),
                quantization: metadata.quantization(),
                vocabulary_size: metadata.vocabulary_size(),
                context_length: metadata.context_length(),
            },
            capabilities: ModelCapabilities {
                operations,
                maximum_context_tokens: execution.context_tokens_per_sequence().get(),
                maximum_sequences: execution.maximum_sequences().get(),
                maximum_prefill_batch: execution.maximum_prefill_batch().get(),
            },
            estimated_footprint: footprint,
        };
        verify_source_digest_after(self.backend, source, digest_before)?;
        Ok(InspectedSource {
            metadata,
            descriptor,
        })
    }

    fn plan_inspected(
        &self,
        source: &GgufSource,
        configuration: &LoadConfiguration,
        inspected: &InspectedSource,
    ) -> Result<LoadPlan, LoadError> {
        if configuration.device_kind != DeviceKind::Cpu || configuration.device.get() != 0 {
            return Err(LoadError::InvalidConfiguration);
        }
        if source.execution().use_mmap() && !self.runtime.supports_mmap() {
            return Err(LoadError::InvalidConfiguration);
        }
        if source.execution().use_mlock() && !self.runtime.supports_mlock() {
            return Err(LoadError::InvalidConfiguration);
        }

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
}

impl ModelLoader for GgufLoader {
    type Source = GgufSource;
    type Model = GgufModel;

    fn inspect(&self, source: &Self::Source) -> Result<ModelDescriptor, LoadError> {
        self.inspect_source(source)
            .map(|inspected| inspected.descriptor)
    }

    fn plan_load(
        &self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<LoadPlan, LoadError> {
        let inspected = self.inspect_source(source)?;
        self.plan_inspected(source, configuration, &inspected)
    }

    fn load(
        &mut self,
        source: &Self::Source,
        configuration: &LoadConfiguration,
    ) -> Result<Self::Model, LoadError> {
        let inspected = self.inspect_source(source)?;
        let plan = self.plan_inspected(source, configuration, &inspected)?;

        let execution = source.execution();
        let digest_before = verify_source_digest_before(self.backend, source)?;
        let model_params = LlamaModelParams::default()
            .with_use_mmap(execution.use_mmap())
            .with_use_mlock(execution.use_mlock());
        let model =
            LlamaModel::load_from_file(self.runtime.native.as_ref(), source.path(), &model_params)
                .map_err(|_| {
                    LoadError::Backend(failure(
                        self.backend,
                        BackendFailureKind::InvalidModel,
                        CODE_MODEL_LOAD,
                    ))
                })?;
        verify_source_digest_after(self.backend, source, digest_before)?;

        let loaded_vocabulary =
            u32::try_from(model.n_vocab()).map_err(|_| model_mismatch(self.backend))?;
        if loaded_vocabulary != inspected.metadata.vocabulary_size()
            || model.n_ctx_train() != inspected.metadata.context_length()
        {
            return Err(model_mismatch(self.backend));
        }

        let context_params = LlamaContextParams::default()
            .with_n_ctx(Some(
                execution
                    .total_context_tokens()
                    .map_err(|_| LoadError::InvalidConfiguration)?,
            ))
            .with_n_batch(execution.maximum_prefill_batch().get())
            .with_n_ubatch(execution.micro_batch_tokens().get())
            .with_n_seq_max(execution.maximum_sequences().get())
            .with_n_threads(execution.threads().get())
            .with_n_threads_batch(execution.batch_threads().get())
            .with_type_k(KvCacheType::F16)
            .with_type_v(KvCacheType::F16);

        GgufModel::new(
            self.backend,
            configuration.handle,
            plan.descriptor,
            self.runtime.clone(),
            model,
            context_params,
            execution,
        )
        .map(|model| model.with_content_digest(source.expected_digest()))
        .map_err(|_| {
            LoadError::Backend(failure(
                self.backend,
                BackendFailureKind::ForeignFunction,
                CODE_CONTEXT_CREATE,
            ))
        })
    }
}

struct InspectedSource {
    metadata: GgufMetadata,
    descriptor: ModelDescriptor,
}

const fn metadata_load_error(backend: BackendId, error: &MetadataError) -> LoadError {
    let (kind, code) = match error {
        MetadataError::Open(_) => (BackendFailureKind::InvalidModel, CODE_METADATA_OPEN),
        MetadataError::Read(_) => (BackendFailureKind::InvalidModel, CODE_METADATA_READ),
        MetadataError::NumericOverflow => (BackendFailureKind::InvalidModel, CODE_NUMERIC_OVERFLOW),
        MetadataError::InvalidFormat
        | MetadataError::LimitExceeded
        | MetadataError::InvalidValue
        | MetadataError::InvalidUtf8 => (BackendFailureKind::InvalidModel, CODE_METADATA_FORMAT),
    };
    LoadError::Backend(failure(backend, kind, code))
}

const fn numeric_load_error(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_NUMERIC_OVERFLOW,
    ))
}

const fn model_mismatch(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_MODEL_MISMATCH,
    ))
}

fn verify_source_digest_before(
    backend: BackendId,
    source: &GgufSource,
) -> Result<Option<Sha256Digest>, LoadError> {
    let Some(expected) = source.expected_digest() else {
        return Ok(None);
    };
    let actual = sha256_file(source.path()).map_err(|_| digest_read_error(backend))?;
    if actual != expected {
        return Err(digest_mismatch_error(backend));
    }
    Ok(Some(actual))
}

fn verify_source_digest_after(
    backend: BackendId,
    source: &GgufSource,
    before: Option<Sha256Digest>,
) -> Result<(), LoadError> {
    let Some(before) = before else {
        return Ok(());
    };
    let Some(expected) = source.expected_digest() else {
        return Err(digest_mismatch_error(backend));
    };
    let after = sha256_file(source.path()).map_err(|_| digest_read_error(backend))?;
    if after != before || after != expected {
        return Err(digest_mismatch_error(backend));
    }
    Ok(())
}

const fn digest_read_error(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_CONTENT_DIGEST_READ,
    ))
}

const fn digest_mismatch_error(backend: BackendId) -> LoadError {
    LoadError::Backend(failure(
        backend,
        BackendFailureKind::InvalidModel,
        CODE_CONTENT_DIGEST_MISMATCH,
    ))
}
