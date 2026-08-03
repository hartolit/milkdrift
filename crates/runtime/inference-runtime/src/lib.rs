//! Exclusive model ownership, admission control, cancellation, and bounded hosting.

#![forbid(unsafe_code)]

mod command;
mod configuration;
mod error;
mod generation;
mod runtime;
mod worker;

pub use command::{
    CommandTicket, DecodeReceipt, LoadReceipt, ModelSnapshot, PrefillReceipt, RequestStartReceipt,
    RuntimeCommand, RuntimeEvent, RuntimeSnapshot, ShutdownReceipt, UnloadReceipt, UnloadStatus,
};
pub use configuration::{CleanupRetryPolicy, HostedRuntimeConfiguration, RuntimeLimits};
pub use domain_contracts::MemoryKind;
pub use error::{
    CleanupFailureReport, CleanupPoll, CleanupResource, CleanupRetryState, FailureClass,
    RuntimeError, RuntimeOperation, RuntimeReceiveError, RuntimeSubmitError, SamplingFailure,
};
pub use generation::{
    GenerationAdmission, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, GenerationStopSequence,
};
pub use runtime::InferenceRuntime;
pub use sampling::SamplingConfig;
pub use worker::{HostedRuntime, HostedRuntimeStartError, RuntimeThread, start_hosted_runtime};
