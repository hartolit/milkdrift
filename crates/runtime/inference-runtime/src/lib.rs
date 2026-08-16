//! Exclusive model ownership, admission control, cancellation, and bounded hosting.

#![forbid(unsafe_code)]
#![expect(
    clippy::result_large_err,
    clippy::large_types_passed_by_value,
    reason = "runtime errors retain bounded allocation-free primary, cleanup, and ownership evidence; by-value Copy transitions avoid boxing and cold-path allocation"
)]

mod command;
mod configuration;
mod error;
mod generation;
mod runtime;
mod worker;

pub use command::{
    CommandTicket, DecodeReceipt, LoadReceipt, ModelSnapshot, PrefillReceipt, RequestStartReceipt,
    RetainedModelSnapshot, RuntimeCommand, RuntimeEvent, RuntimeSnapshot, ShutdownReceipt,
    UnloadReceipt, UnloadStatus, UnverifiedOwnershipSummary,
};
pub use configuration::{CleanupRetryPolicy, HostedRuntimeConfiguration, RuntimeLimits};
pub use domain_contracts::MemoryKind;
pub use error::{
    CleanupFailureReport, CleanupPoll, CleanupResource, CleanupRetryState, CleanupRetryStateError,
    ConservativeFootprint, FailureClass, FailureDetail, RetainedOwnership, RuntimeError,
    RuntimeOperation, RuntimeReceiveError, RuntimeSubmitError, SamplingFailure,
    TerminalRetentionSummary,
};
pub use generation::{
    GenerationAdmission, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, GenerationStopSequence,
};
pub use runtime::InferenceRuntime;
pub use sampling::SamplingConfig;
pub use worker::{HostedRuntime, HostedRuntimeStartError, RuntimeThread, start_hosted_runtime};
