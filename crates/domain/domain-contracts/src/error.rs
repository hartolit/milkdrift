//! Allocation-free error taxonomy shared by engines and backend adapters.

use crate::{BackendId, CancellationReason, CapacityExhausted, MemoryKind, ScalarType};

/// Stable classification of a backend-native failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackendFailureKind {
    /// Model source or metadata could not be decoded.
    InvalidModel,
    /// The backend does not implement the requested model or operation.
    Unsupported,
    /// A host-memory allocation or reservation failed.
    HostMemory,
    /// A device-memory allocation or reservation failed.
    DeviceMemory,
    /// Device initialization or driver selection failed.
    DeviceInitialization,
    /// A device command failed.
    DeviceExecution,
    /// Backend synchronization failed.
    Synchronization,
    /// Native foreign-function interface reported an error.
    ForeignFunction,
    /// Backend state violated its documented lifecycle.
    InvalidState,
    /// Failure not covered by a stable category.
    Internal,
}

/// Stable, allocation-free representation of a backend-native error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendFailure {
    /// Backend that produced the failure.
    pub backend: BackendId,
    /// Stable failure category.
    pub kind: BackendFailureKind,
    /// Backend-defined numeric detail code, or zero when unavailable.
    pub code: u32,
}

impl BackendFailure {
    /// Creates a backend failure.
    #[must_use]
    pub const fn new(backend: BackendId, kind: BackendFailureKind, code: u32) -> Self {
        Self {
            backend,
            kind,
            code,
        }
    }
}

/// Stable lifecycle stage at which a model load failed.
///
/// Stages describe portable backend work rather than an adapter's functions,
/// libraries, or serialization types. A stage proves only where the failure
/// was observed; it does not identify a source or establish which earlier
/// stages completed unless the carrying API documents that ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadFailureStage {
    /// Configuration or source authority was inspected.
    ConfigurationInspection,
    /// An artifact manifest, header, or metadata representation was decoded.
    ArtifactDecoding,
    /// Model schema, shape, scalar policy, or compatibility was validated.
    CompatibilityValidation,
    /// Source payload bytes were read or verified against accepted identity.
    PayloadRead,
    /// Source bytes were materialized as a host tensor.
    HostMaterialization,
    /// A source tensor was converted to the selected execution scalar.
    ScalarConversion,
    /// A host tensor was transferred to the selected execution device.
    DeviceTransfer,
    /// A tensor was inserted into retained model-loading storage.
    RetainedPlacement,
    /// The backend model was constructed from accepted tensors.
    ModelConstruction,
    /// In-flight load work was synchronized before publication or commit.
    LoadSynchronization,
    /// A partial-load owner was synchronized for explicit cleanup.
    PartialLoadCleanupSynchronization,
}

/// Deterministic location of one tensor involved in a load failure.
///
/// `shard_ordinal` is zero-based in the backend's accepted deterministic
/// selected-shard order. `tensor_ordinal` is zero-based in that shard's
/// accepted deterministic tensor-manifest order. The name fingerprint is a
/// stable diagnostic correlation value chosen and documented by the adapter;
/// it is not authentication, source identity, or a reversible tensor name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TensorFailureLocation {
    /// Zero-based accepted selected-shard ordinal.
    pub shard_ordinal: u16,
    /// Zero-based accepted tensor-manifest ordinal within the shard.
    pub tensor_ordinal: u32,
    /// Stable adapter-defined fingerprint of the tensor name bytes.
    pub tensor_name_hash: u64,
    /// Project-owned classification of the observed source scalar, when known.
    pub observed_scalar: Option<ScalarType>,
}

impl TensorFailureLocation {
    /// Creates one deterministic tensor failure location.
    #[must_use]
    pub const fn new(
        shard_ordinal: u16,
        tensor_ordinal: u32,
        tensor_name_hash: u64,
        observed_scalar: Option<ScalarType>,
    ) -> Self {
        Self {
            shard_ordinal,
            tensor_ordinal,
            tensor_name_hash,
            observed_scalar,
        }
    }
}

/// Bounded provenance for a backend model-load failure.
///
/// Context proves the stage at which the backend classified the failure and,
/// when present, the accepted manifest coordinate of the responsible tensor.
/// A stage without a tensor is intentional for configuration-, shard-, batch-,
/// capacity-, construction-, synchronization-, or cleanup-wide failures. It
/// carries no paths, names, vendor diagnostics, or source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadFailureContext {
    /// Portable stage at which the failure was observed.
    pub stage: LoadFailureStage,
    /// Responsible tensor when one accepted coordinate is truthful.
    pub tensor: Option<TensorFailureLocation>,
}

impl LoadFailureContext {
    /// Creates stage-only provenance for a non-tensor-specific failure.
    #[must_use]
    pub const fn stage(stage: LoadFailureStage) -> Self {
        Self {
            stage,
            tensor: None,
        }
    }

    /// Creates provenance for one accepted tensor coordinate.
    #[must_use]
    pub const fn tensor(stage: LoadFailureStage, tensor: TensorFailureLocation) -> Self {
        Self {
            stage,
            tensor: Some(tensor),
        }
    }
}

/// Backend loading failure with optional bounded portable provenance.
///
/// The stable backend/kind/code identity remains authoritative. Context adds
/// correlation evidence without replacing that identity or exposing adapter
/// error types. The representation is fixed-size, allocation-free, and
/// suitable for portable `no_std` callers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackendLoadFailure {
    /// Stable backend-native failure identity.
    pub failure: BackendFailure,
    /// Bounded load-specific provenance, when the backend can supply it.
    pub context: Option<LoadFailureContext>,
}

impl BackendLoadFailure {
    /// Creates a backend load failure without provenance.
    #[must_use]
    pub const fn new(failure: BackendFailure) -> Self {
        Self {
            failure,
            context: None,
        }
    }

    /// Creates a backend load failure with stage-only provenance.
    #[must_use]
    pub const fn at_stage(failure: BackendFailure, stage: LoadFailureStage) -> Self {
        Self {
            failure,
            context: Some(LoadFailureContext::stage(stage)),
        }
    }

    /// Creates a backend load failure with tensor provenance.
    #[must_use]
    pub const fn at_tensor(
        failure: BackendFailure,
        stage: LoadFailureStage,
        tensor: TensorFailureLocation,
    ) -> Self {
        Self {
            failure,
            context: Some(LoadFailureContext::tensor(stage, tensor)),
        }
    }

    /// Returns the stable generic backend failure identity.
    #[must_use]
    pub const fn failure(self) -> BackendFailure {
        self.failure
    }

    /// Returns the bounded load context, when available.
    #[must_use]
    pub const fn context(self) -> Option<LoadFailureContext> {
        self.context
    }

    /// Returns the backend that produced the failure.
    #[must_use]
    pub const fn backend(self) -> BackendId {
        self.failure.backend
    }

    /// Returns the stable backend failure category.
    #[must_use]
    pub const fn kind(self) -> BackendFailureKind {
        self.failure.kind
    }

    /// Returns the stable backend-defined numeric detail code.
    #[must_use]
    pub const fn code(self) -> u32 {
        self.failure.code
    }

    /// Replaces any prior context with the supplied bounded context.
    #[must_use]
    pub const fn with_context(self, context: LoadFailureContext) -> Self {
        Self {
            failure: self.failure,
            context: Some(context),
        }
    }
}

/// Failure while inspecting, planning, or loading a model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    /// Source metadata or artifact structure is invalid.
    InvalidSource,
    /// Requested model architecture is unsupported.
    UnsupportedArchitecture,
    /// Requested weight or quantization format is unsupported.
    UnsupportedFormat,
    /// The supplied configuration is invalid.
    InvalidConfiguration,
    /// A fixed-capacity load-time structure was insufficient.
    CapacityExhausted(CapacityExhausted),
    /// The available resource budget is insufficient.
    InsufficientMemory {
        /// Memory domain whose bound was exceeded.
        kind: MemoryKind,
        /// Required bytes.
        required_bytes: u64,
        /// Available bytes.
        available_bytes: u64,
    },
    /// Loading was cancelled.
    Cancelled(CancellationReason),
    /// Backend-native loading failure.
    Backend(BackendLoadFailure),
}

/// Failure while operating on a loaded model outside an individual sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelError {
    /// Operation is not valid in the model's current lifecycle state.
    InvalidState,
    /// Requested operation or configuration is unsupported.
    Unsupported,
    /// A fixed-capacity structure was insufficient.
    CapacityExhausted(CapacityExhausted),
    /// Backend-native model failure.
    Backend(BackendFailure),
}

/// Failure while operating on one inference sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SequenceError {
    /// Operation is not valid in the sequence's current state.
    InvalidState,
    /// Token, logit, or scratch capacity was insufficient.
    CapacityExhausted(CapacityExhausted),
    /// Operation is not supported by this sequence implementation.
    Unsupported,
    /// Operation was cancelled.
    Cancelled(CancellationReason),
    /// Backend-native sequence failure.
    Backend(BackendFailure),
}

/// Failure while synchronizing or preparing backend resource destruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SynchronizationError {
    /// Synchronization is not valid in the current model state.
    InvalidState,
    /// Synchronization was cancelled.
    Cancelled(CancellationReason),
    /// Backend-native synchronization failure.
    Backend(BackendFailure),
}
