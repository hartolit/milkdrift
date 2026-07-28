//! Allocation-free error taxonomy shared by engines and backend adapters.

use crate::{BackendId, CancellationReason, CapacityExhausted};

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
        /// Required bytes.
        required_bytes: u64,
        /// Available bytes.
        available_bytes: u64,
    },
    /// Loading was cancelled.
    Cancelled(CancellationReason),
    /// Backend-native loading failure.
    Backend(BackendFailure),
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
