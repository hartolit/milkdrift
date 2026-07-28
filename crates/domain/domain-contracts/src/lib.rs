#![no_std]
#![forbid(unsafe_code)]
#![doc = "Portable, allocation-neutral contracts shared by inference engines and backends."]

pub mod backend;
pub mod capacity;
pub mod error;
pub mod generation;
pub mod identifiers;
pub mod lifecycle;
pub mod model;
pub mod output;
pub mod sequence;
pub mod time;

pub use backend::{BackendSequence, LoadedModel, ModelLoader, decode_checked, prefill_checked};
pub use capacity::{CapacityExhausted, CapacityResource};
pub use error::{
    BackendFailure, BackendFailureKind, LoadError, ModelError, SequenceError, SynchronizationError,
};
pub use generation::{
    CancellationReason, CancellationStatus, DecodeOutcome, FinishReason, GenerationControl,
    GenerationUsage, PrefillOutcome, YieldReason,
};
pub use identifiers::{
    ArtifactId, BackendId, DeviceId, ModelGeneration, ModelHandle, ModelId, RequestId, SequenceId,
    TaskId, TokenId,
};
pub use lifecycle::{
    DrainTimeout, DrainWindow, LifecycleAction, LifecycleError, LifecycleFailurePhase,
    ModelLifecycle, ModelLifecycleState, UnloadPolicy,
};
pub use model::{
    CapabilitySet, DeviceKind, LoadConfiguration, LoadPlan, MemoryBudget, MemoryFootprint,
    ModelArchitecture, ModelCapabilities, ModelDescriptor, ModelMetadata, QuantizationFormat,
    ScalarType, SequenceConfiguration, SequencePlan,
};
pub use output::{ByteRange, OutputBatch, OutputCursor, OutputRecord, OutputRecordKind};
pub use sequence::{
    DecodeBufferRequirements, DecodeBuffers, DecodeInput, PrefillBufferRequirements,
    PrefillBuffers, PrefillInput, PreparedDecodeBuffers, PreparedPrefillBuffers, SequenceState,
};
pub use time::MonotonicMillis;
