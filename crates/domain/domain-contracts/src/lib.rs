#![no_std]
#![forbid(unsafe_code)]
#![doc = "Portable, allocation-neutral contracts shared by inference engines and backends."]

mod backend;
mod capacity;
mod error;
mod generation;
mod identifiers;
mod lifecycle;
mod model;
mod sequence;
mod time;

pub use backend::{
    BackendSequence, FailedLoad, FailedLoadOwner, LoadedModel, ModelLoader, PreparedLoad,
    decode_checked, prefill_checked,
};
pub use capacity::{CapacityExhausted, CapacityResource};
pub use error::{
    BackendFailure, BackendFailureKind, BackendLoadFailure, LoadError, LoadFailureContext,
    LoadFailureStage, ModelError, SequenceError, SynchronizationError, TensorFailureLocation,
};
pub use generation::{
    CancellationReason, CancellationStatus, DecodeOutcome, FinishReason, GenerationControl,
    GenerationUsage, PrefillOutcome, YieldReason,
};
pub use identifiers::{
    ArtifactId, BackendId, DeviceId, ModelGeneration, ModelHandle, ModelId, RequestId, SequenceId,
    TokenId,
};
pub use lifecycle::{
    DrainTimeout, DrainWindow, LifecycleAction, LifecycleError, LifecycleFailurePhase,
    ModelLifecycle, ModelLifecycleState, UnloadPolicy,
};
pub use model::{
    CapabilitySet, DeviceKind, ExecutionDevice, LoadConfiguration, LoadPlan, MemoryBudget,
    MemoryFootprint, MemoryKind, ModelArchitecture, ModelCapabilities, ModelDescriptor,
    ModelMetadata, QuantizationFormat, ScalarType, ScalarTypeSet, SequenceConfiguration,
    SequencePlan, SequenceReservation,
};
pub use sequence::{
    DecodeBufferRequirements, DecodeBuffers, DecodeInput, PrefillBufferRequirements,
    PrefillBuffers, PrefillInput, PreparedDecodeBuffers, PreparedPrefillBuffers, SequenceState,
};
pub use time::MonotonicMillis;
