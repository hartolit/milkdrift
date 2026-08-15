pub(crate) use std::collections::{BTreeMap, BTreeSet};
pub(crate) use std::num::{NonZeroU32, NonZeroU64, NonZeroUsize};
pub(crate) use std::sync::{Arc, Mutex, mpsc};
pub(crate) use std::time::{Duration, Instant};

pub(crate) use domain_contracts::{
    BackendFailure, BackendFailureKind, BackendId, BackendLoadFailure, BackendSequence,
    CapabilitySet, DecodeBufferRequirements, DecodeInput, DecodeOutcome, DeviceId, DeviceKind,
    DrainTimeout, ExecutionDevice, FailedLoad, FailedLoadOwner, FinishReason, LoadConfiguration,
    LoadError, LoadPlan, LoadedModel, MemoryBudget, MemoryFootprint, ModelArchitecture,
    ModelCapabilities, ModelDescriptor, ModelError, ModelGeneration, ModelHandle, ModelId,
    ModelLoader, ModelMetadata, PrefillBufferRequirements, PrefillInput, PrefillOutcome,
    PreparedDecodeBuffers, PreparedLoad, PreparedPrefillBuffers, QuantizationFormat, RequestId,
    ScalarType, ScalarTypeSet, SequenceConfiguration, SequenceError, SequenceId, SequencePlan,
    SequenceReservation, SequenceState, SynchronizationError, TokenId, UnloadPolicy,
};
pub(crate) use host_runtime::TokenOutputRecordKind;
pub(crate) use inference_runtime::{
    CleanupPoll, CleanupResource, CommandTicket, FailureClass, GenerationOutcome,
    GenerationOutputCapacityPolicy, GenerationOutputState, GenerationRequest,
    GenerationStopSequence, HostedRuntime, HostedRuntimeConfiguration, InferenceRuntime,
    RetainedOwnership, RuntimeCommand, RuntimeError, RuntimeEvent, RuntimeLimits, RuntimeThread,
    start_hosted_runtime,
};
pub(crate) use sampling::SamplingConfig;

pub(crate) const BACKEND: BackendId = BackendId::new(93);
pub(crate) const MODEL: ModelId = ModelId::new(1);
pub(crate) const MODEL_HOST_BYTES: u64 = 100;
pub(crate) const SEQUENCE_HOST_BYTES: u64 = 32;

pub(crate) const fn cpu_device() -> ExecutionDevice {
    ExecutionDevice::new(DeviceId::new(0), DeviceKind::Cpu)
}

pub(crate) const GENERATION_OPERATIONS: CapabilitySet = CapabilitySet::PREFILL
    .union(CapabilitySet::INCREMENTAL_DECODE)
    .union(CapabilitySet::MULTIPLE_SEQUENCES)
    .union(CapabilitySet::EXPLICIT_SYNCHRONIZATION);

pub(crate) type TestResult<T = ()> = Result<T, String>;

mod backend;
mod host;

pub(crate) use backend::*;
pub(crate) use host::*;
