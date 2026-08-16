//! Stable runtime, admission, and host-transport failures.

use domain_contracts::{
    CapacityExhausted, LifecycleError, LoadError, MemoryFootprint, MemoryKind, ModelError,
    ModelHandle, ModelId, RequestId, SequenceError, SynchronizationError,
};

use core::fmt::{self, Debug, Formatter};

use crate::RuntimeCommand;

/// Runtime operation that produced a primary or cleanup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeOperation {
    /// Materialization of one exact prepared model load.
    ModelLoad,
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
    /// Cleanup of resources retained by a failed prepared load.
    FailedLoadCleanup,
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
    /// Ownership exists without an established exact upper bound.
    UnverifiedOwnership,
    /// Runtime registry state was inconsistent.
    Invariant,
}

/// Bounded structured detail retained without recursive error chains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureDetail {
    /// Only the stable class is available or relevant.
    Class(FailureClass),
    /// Exact model-loading failure.
    Load(LoadError),
    /// Exact loaded-model failure.
    Model(ModelError),
    /// Exact sequence failure.
    Sequence(SequenceError),
    /// Exact synchronization or release failure.
    Synchronization(SynchronizationError),
    /// Exact lifecycle failure.
    Lifecycle(LifecycleError),
    /// Exact sampling failure.
    Sampling(SamplingFailure),
}

impl FailureDetail {
    /// Returns the stable class represented by this detail.
    #[must_use]
    pub const fn class(self) -> FailureClass {
        match self {
            Self::Class(class) => class,
            Self::Load(_) => FailureClass::Load,
            Self::Model(_) => FailureClass::Model,
            Self::Sequence(_) => FailureClass::Sequence,
            Self::Synchronization(_) => FailureClass::Synchronization,
            Self::Lifecycle(_) => FailureClass::Lifecycle,
            Self::Sampling(_) => FailureClass::Sampling,
        }
    }
}

/// Primary failure plus the independently important cleanup failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupFailureReport {
    /// Operation that produced the original outcome.
    pub primary_operation: RuntimeOperation,
    /// Stable classification of the original outcome.
    pub primary_failure: FailureClass,
    /// Structured bounded identity of the original outcome.
    pub primary_detail: FailureDetail,
    /// Explicit cleanup operation that subsequently failed.
    pub cleanup_operation: RuntimeOperation,
    /// Stable classification of the cleanup failure.
    pub cleanup_failure: FailureClass,
    /// Structured bounded identity of the cleanup failure.
    pub cleanup_detail: FailureDetail,
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
            primary_detail: FailureDetail::Class(primary_failure),
            cleanup_operation,
            cleanup_failure,
            cleanup_detail: FailureDetail::Class(cleanup_failure),
        }
    }

    /// Creates a report retaining exact bounded failure details where available.
    #[must_use]
    pub const fn with_details(
        primary_operation: RuntimeOperation,
        primary_detail: FailureDetail,
        cleanup_operation: RuntimeOperation,
        cleanup_detail: FailureDetail,
    ) -> Self {
        Self {
            primary_operation,
            primary_failure: primary_detail.class(),
            primary_detail,
            cleanup_operation,
            cleanup_failure: cleanup_detail.class(),
            cleanup_detail,
        }
    }
}

/// Checked conservative footprint evidence for unverified ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConservativeFootprint {
    /// All components and host/device totals were combined exactly.
    Known(MemoryFootprint),
    /// At least one checked component or domain total overflowed.
    ///
    /// Raw per-owner accepted and reported components remain available through
    /// [`RetainedOwnership::Unverified`]; no synthetic maximum is substituted.
    Overflow,
}

impl Default for ConservativeFootprint {
    fn default() -> Self {
        Self::Known(MemoryFootprint::default())
    }
}

/// Ownership disposition recorded beside one cleanup transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedOwnership {
    /// Explicit cleanup succeeded and no backend owner remains.
    Released,
    /// The owner remains covered by exact reservation arithmetic for its named phase.
    ///
    /// For a retained sequence this preserves the accepted logical-payload upper
    /// bound; it does not claim exact instantaneous physical allocation.
    Exact(MemoryFootprint),
    /// A retained backend owner contradicted its accepted contract.
    ///
    /// This can be a complete model, a failed-materialization owner whose
    /// lifetime-stable plan report changed, or a retained sequence whose report
    /// changed. Neither the accepted reservation nor the contradictory backend
    /// report is promoted to exact ownership.
    /// `conservative_footprint` preserves component-wise conservative evidence
    /// when checked arithmetic can represent it.
    Unverified {
        /// Reservation accepted before acquiring the backend owner.
        accepted_footprint: MemoryFootprint,
        /// Backend's contradictory retained-ownership footprint report.
        reported_footprint: MemoryFootprint,
        /// Checked component-wise conservative reservation evidence.
        conservative_footprint: ConservativeFootprint,
    },
}

impl RetainedOwnership {
    /// Returns the footprint only when exact reservation accounting remains established.
    #[must_use]
    pub const fn exact_footprint(self) -> Option<MemoryFootprint> {
        match self {
            Self::Exact(footprint) => Some(footprint),
            Self::Released | Self::Unverified { .. } => None,
        }
    }

    /// Returns whether this owner blocks all further resource admission.
    #[must_use]
    pub const fn blocks_admission(self) -> bool {
        matches!(self, Self::Unverified { .. })
    }

    /// Returns whether explicit cleanup proved that no owner remains.
    #[must_use]
    pub const fn is_released(self) -> bool {
        matches!(self, Self::Released)
    }
}

/// Resource identity addressed by a retained or completed cleanup transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupResource {
    /// One loaded model retained only for unload retry.
    Model {
        /// Exact verified model generation retained after ordinary unload failure.
        handle: ModelHandle,
    },
    /// One complete model that contradicted its accepted prepared-load contract.
    IncompatibleModel {
        /// Accepted model generation reserved by the failed admission transaction.
        handle: ModelHandle,
    },
    /// One failed prepared load retained only for partial-load cleanup retry.
    FailedLoad {
        /// Accepted model generation assigned to the failed transaction.
        handle: ModelHandle,
    },
    /// One backend sequence retained only for destruction retry.
    Sequence {
        /// Exact model generation that owns the sequence.
        handle: ModelHandle,
        /// Request identity formerly associated with the sequence.
        request_id: RequestId,
        /// Backend sequence identity.
        sequence_id: domain_contracts::SequenceId,
    },
}

/// Observable state for one retained or successfully released cleanup resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CleanupRetryState {
    /// Stable resource identity for the cleanup transaction.
    pub resource: CleanupResource,
    /// Primary and cleanup failure classifications and bounded details.
    pub failure: CleanupFailureReport,
    /// Ownership certainty before retry, or [`RetainedOwnership::Released`] after success.
    pub ownership: RetainedOwnership,
    /// Total cleanup attempts already performed, including the initial failure.
    pub attempts: u32,
    /// Maximum total attempts permitted by policy.
    pub maximum_attempts: u32,
}

impl CleanupRetryState {
    /// Returns whether unreleased ownership has exhausted its automatic retry budget.
    #[must_use]
    pub const fn exhausted(self) -> bool {
        !self.ownership.is_released() && self.attempts >= self.maximum_attempts
    }

    pub(crate) const fn released(mut self) -> Self {
        self.ownership = RetainedOwnership::Released;
        self
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

/// Bounded summary of ownership deliberately retained until process exit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalRetentionSummary {
    /// Failed materialization preparations retained for cleanup.
    pub failed_preparations: u32,
    /// Previously verified models retained after ordinary unload failure.
    pub verified_models: u32,
    /// Contract-violating complete models with unverified ownership.
    pub incompatible_models: u32,
    /// Backend sequences retained after destruction failure.
    pub sequences: u32,
    /// Checked aggregate conservative evidence for every unverified retained owner.
    pub unverified_conservative_footprint: ConservativeFootprint,
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
    /// A retained backend owner has no established exact upper bound.
    ///
    /// Cleanup, inspection, shutdown, and already admitted healthy execution remain
    /// available, but every new model, sequence, cache, or workspace admission fails closed.
    AdmissionBlockedByUnverifiedOwnership {
        /// Number of retained owners carrying unverified ownership.
        owners: u32,
    },
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
        required_bytes: domain_contracts::ByteCount,
        /// Configured aggregate byte limit.
        available_bytes: domain_contracts::ByteCount,
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
    /// Backend contradicted an admitted descriptor, identity, or operation result.
    BackendContractViolation,
    /// The addressed model owns a resource awaiting explicit cleanup.
    ModelDegraded(ModelId),
    /// Sampling failed inside the generation kernel.
    Sampling(SamplingFailure),
    /// Cleanup failed after an independently important primary outcome.
    ///
    /// The complete retry state preserves resource identity, ownership certainty,
    /// attempt accounting, and the independent primary and cleanup failures.
    CleanupFailed(CleanupRetryState),
    /// Automatic cleanup retries are exhausted while ownership remains retained.
    CleanupRetryExhausted(CleanupRetryState),
    /// Terminal shutdown retained all unresolved owners until process exit.
    TerminalCleanupRetention {
        /// First deterministic exhausted owner for focused diagnosis.
        first: CleanupRetryState,
        /// Bounded summary of every owner crossing the process reclamation boundary.
        summary: TerminalRetentionSummary,
    },
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
            Self::AdmissionBlockedByUnverifiedOwnership { .. } => FailureClass::UnverifiedOwnership,
            Self::BackendContractViolation => FailureClass::BackendContract,
            Self::CleanupFailed(state) | Self::CleanupRetryExhausted(state) => {
                state.failure.primary_failure
            }
            Self::TerminalCleanupRetention { first, .. } => first.failure.primary_failure,
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

    /// Returns bounded structured detail for retention beside a cleanup owner.
    #[must_use]
    pub const fn failure_detail(self) -> FailureDetail {
        match self {
            Self::Load(error) => FailureDetail::Load(error),
            Self::Model(error) => FailureDetail::Model(error),
            Self::Sequence(error) => FailureDetail::Sequence(error),
            Self::Synchronization(error) => FailureDetail::Synchronization(error),
            Self::Lifecycle(error) => FailureDetail::Lifecycle(error),
            Self::Sampling(error) => FailureDetail::Sampling(error),
            Self::CleanupFailed(state) | Self::CleanupRetryExhausted(state) => {
                state.failure.primary_detail
            }
            Self::TerminalCleanupRetention { first, .. } => first.failure.primary_detail,
            other => FailureDetail::Class(other.failure_class()),
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
