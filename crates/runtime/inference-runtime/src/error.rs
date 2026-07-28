//! Stable runtime, admission, and host-transport failures.

use domain_contracts::{
    CapacityExhausted, LifecycleError, LoadError, ModelError, ModelHandle, ModelId, RequestId,
    SequenceError, SynchronizationError,
};

use core::fmt::{self, Debug, Formatter};

use crate::RuntimeCommand;

/// Runtime operation that produced a primary or cleanup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeOperation {
    /// Validation after loading a native model.
    ModelAdmission,
    /// Validation after creating a native sequence.
    SequenceAdmission,
    /// Prompt prefill.
    Prefill,
    /// Incremental decode.
    Decode,
    /// Token sampling.
    Sampling,
    /// Explicit request completion.
    Completion,
    /// Request cancellation.
    Cancellation,
    /// Sequence destruction.
    SequenceDestruction,
    /// Model unload preparation.
    ModelUnload,
    /// Runtime shutdown.
    Shutdown,
}

/// Allocation-free stable classification retained across cleanup boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureClass {
    /// A backend contradicted an accepted plan or identity.
    BackendContract,
    /// Model loading failed.
    Load,
    /// A loaded-model operation failed.
    Model,
    /// A sequence operation failed.
    Sequence,
    /// Synchronization or unload preparation failed.
    Synchronization,
    /// A lifecycle transition failed.
    Lifecycle,
    /// A fixed capacity or aggregate memory bound was exceeded.
    Capacity,
    /// Sampling configuration or execution failed.
    Sampling,
    /// Generation reached an expected terminal condition before cleanup failed.
    Completion,
    /// A request was cancelled.
    Cancellation,
    /// Runtime shutdown terminated generation.
    Shutdown,
    /// Runtime registry state was inconsistent.
    Invariant,
}

/// Primary failure plus the independently important cleanup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupFailureReport {
    /// Operation that produced the original outcome.
    pub primary_operation: RuntimeOperation,
    /// Stable classification of the original outcome.
    pub primary_failure: FailureClass,
    /// Explicit cleanup operation that subsequently failed.
    pub cleanup_operation: RuntimeOperation,
    /// Stable classification of the cleanup failure.
    pub cleanup_failure: FailureClass,
}

impl CleanupFailureReport {
    /// Creates a structured two-failure report without allocation.
    #[must_use]
    pub const fn new(
        primary_operation: RuntimeOperation,
        primary_failure: FailureClass,
        cleanup_operation: RuntimeOperation,
        cleanup_failure: FailureClass,
    ) -> Self {
        Self {
            primary_operation,
            primary_failure,
            cleanup_operation,
            cleanup_failure,
        }
    }
}

/// Runtime-owned resource retained after explicit cleanup failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupResource {
    /// One loaded model retained only for unload retry.
    Model {
        /// Logical model identity.
        model_id: ModelId,
    },
    /// One backend sequence retained only for destruction retry.
    Sequence {
        /// Logical model identity that owns the sequence.
        model_id: ModelId,
        /// Request identity formerly associated with the sequence.
        request_id: RequestId,
        /// Backend sequence identity.
        sequence_id: domain_contracts::SequenceId,
    },
}

/// Observable retry state for one retained cleanup resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupRetryState {
    /// Retained resource identity.
    pub resource: CleanupResource,
    /// Primary and cleanup failure classifications.
    pub failure: CleanupFailureReport,
    /// Total cleanup attempts already performed, including the initial failure.
    pub attempts: u32,
    /// Maximum total attempts permitted by policy.
    pub maximum_attempts: u32,
}

impl CleanupRetryState {
    /// Returns whether policy forbids another automatic retry.
    #[must_use]
    pub const fn exhausted(self) -> bool {
        self.attempts >= self.maximum_attempts
    }
}

/// Result of one bounded cleanup-maintenance opportunity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupPoll {
    /// No retryable retained resource was available.
    Idle,
    /// One retained resource was released successfully.
    Released(CleanupRetryState),
    /// One retry failed but at least one policy attempt remains.
    RetryFailed(CleanupRetryState),
    /// One retry failed and exhausted the total-attempt policy.
    Exhausted(CleanupRetryState),
}

/// Stable sampling failure category exposed by the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SamplingFailure {
    /// Immutable sampling settings were invalid.
    InvalidConfiguration,
    /// Logits or candidate weights could not produce a token.
    NoCandidate,
    /// Sampling input referred to an invalid token or vocabulary.
    InvalidInput,
    /// Caller-owned sampling workspace was too small.
    CapacityExhausted(CapacityExhausted),
}

impl From<sampling::SamplingError> for SamplingFailure {
    fn from(value: sampling::SamplingError) -> Self {
        match value {
            sampling::SamplingError::InvalidConfiguration(_) => Self::InvalidConfiguration,
            sampling::SamplingError::EmptyLogits | sampling::SamplingError::NoCandidate => {
                Self::NoCandidate
            }
            sampling::SamplingError::CapacityExhausted(capacity) => {
                Self::CapacityExhausted(capacity)
            }
            _ => Self::InvalidInput,
        }
    }
}

/// Memory domain whose aggregate budget was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// Host-addressable memory.
    Host,
    /// Device-local memory.
    Device,
}

/// Inference registry or backend operation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeError {
    /// The logical model identity is already resident.
    ModelAlreadyLoaded(ModelId),
    /// No resident model has the requested logical identity.
    ModelNotLoaded(ModelId),
    /// A retained handle addresses an older or otherwise different generation.
    StaleModelHandle {
        /// Handle supplied by the caller.
        provided: ModelHandle,
        /// Current resident or most recently completed generation.
        current: ModelHandle,
    },
    /// The request identity is already active.
    RequestAlreadyActive(RequestId),
    /// No active request has the supplied identity.
    RequestNotActive(RequestId),
    /// The sequence identity is already owned by another active request.
    SequenceAlreadyActive(domain_contracts::SequenceId),
    /// A model-generation counter could not be incremented.
    ModelGenerationExhausted(ModelId),
    /// Runtime shutdown has begun and new work is rejected.
    ShuttingDown,
    /// The configured resident-model count was exceeded.
    LoadedModelLimit {
        /// Model count required after the attempted admission.
        required: u32,
        /// Configured resident-model limit.
        available: u32,
    },
    /// Aggregate memory accounting overflowed its integer representation.
    MemoryArithmeticOverflow,
    /// Aggregate memory accounting underflowed, indicating an internal invariant failure.
    MemoryArithmeticUnderflow,
    /// A fixed registry or backend capacity was exhausted.
    CapacityExhausted(CapacityExhausted),
    /// Aggregate model or sequence memory admission failed.
    InsufficientMemory {
        /// Memory domain that exceeded its hard limit.
        kind: MemoryKind,
        /// Total resident bytes required after the attempted admission.
        required_bytes: u64,
        /// Configured aggregate byte limit.
        available_bytes: u64,
    },
    /// Model loading failed.
    Load(LoadError),
    /// Loaded-model operation failed.
    Model(ModelError),
    /// Sequence operation failed.
    Sequence(SequenceError),
    /// Synchronization or unload preparation failed.
    Synchronization(SynchronizationError),
    /// Lifecycle transition failed.
    Lifecycle(LifecycleError),
    /// Backend returned a handle or metadata inconsistent with its accepted plan.
    BackendContractViolation,
    /// The addressed model owns a resource awaiting explicit cleanup.
    ModelDegraded(ModelId),
    /// Sampling failed inside the generation kernel.
    Sampling(SamplingFailure),
    /// Cleanup failed after an independently important primary outcome.
    CleanupFailed(CleanupFailureReport),
    /// Automatic cleanup retries are exhausted while ownership remains retained.
    CleanupRetryExhausted(CleanupRetryState),
}

impl RuntimeError {
    /// Returns the allocation-free stable class used in cleanup reports.
    #[must_use]
    pub const fn failure_class(self) -> FailureClass {
        match self {
            Self::Load(_) => FailureClass::Load,
            Self::Model(_) => FailureClass::Model,
            Self::Sequence(_) => FailureClass::Sequence,
            Self::Synchronization(_) => FailureClass::Synchronization,
            Self::Lifecycle(_) => FailureClass::Lifecycle,
            Self::CapacityExhausted(_)
            | Self::InsufficientMemory { .. }
            | Self::LoadedModelLimit { .. } => FailureClass::Capacity,
            Self::Sampling(_) => FailureClass::Sampling,
            Self::ShuttingDown => FailureClass::Shutdown,
            Self::BackendContractViolation => FailureClass::BackendContract,
            Self::CleanupFailed(report) => report.primary_failure,
            Self::CleanupRetryExhausted(state) => state.failure.primary_failure,
            Self::ModelAlreadyLoaded(_)
            | Self::ModelNotLoaded(_)
            | Self::StaleModelHandle { .. }
            | Self::RequestAlreadyActive(_)
            | Self::RequestNotActive(_)
            | Self::SequenceAlreadyActive(_)
            | Self::ModelGenerationExhausted(_)
            | Self::MemoryArithmeticOverflow
            | Self::MemoryArithmeticUnderflow
            | Self::ModelDegraded(_) => FailureClass::Invariant,
        }
    }
}

impl From<CapacityExhausted> for RuntimeError {
    fn from(value: CapacityExhausted) -> Self {
        Self::CapacityExhausted(value)
    }
}

impl From<LoadError> for RuntimeError {
    fn from(value: LoadError) -> Self {
        Self::Load(value)
    }
}

impl From<ModelError> for RuntimeError {
    fn from(value: ModelError) -> Self {
        Self::Model(value)
    }
}

impl From<SequenceError> for RuntimeError {
    fn from(value: SequenceError) -> Self {
        Self::Sequence(value)
    }
}

impl From<SynchronizationError> for RuntimeError {
    fn from(value: SynchronizationError) -> Self {
        Self::Synchronization(value)
    }
}

impl From<LifecycleError> for RuntimeError {
    fn from(value: LifecycleError) -> Self {
        Self::Lifecycle(value)
    }
}

/// Non-blocking submission failure retaining ownership of the command.
pub enum RuntimeSubmitError<S> {
    /// The bounded command queue is full.
    Full(RuntimeCommand<S>),
    /// The runtime worker has stopped.
    Disconnected(RuntimeCommand<S>),
}

/// Event receive failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeReceiveError {
    /// No event arrived before the requested timeout.
    Timeout,
    /// The runtime worker has stopped and no events remain.
    Disconnected,
}

impl<S> Debug for RuntimeSubmitError<S> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("RuntimeSubmitError::Full(..)"),
            Self::Disconnected(_) => formatter.write_str("RuntimeSubmitError::Disconnected(..)"),
        }
    }
}
