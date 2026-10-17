//! Own ordinary workflow admission, immutable task starts and durable execution evidence.
//!
//! [`TaskRuntime`] admits generation-guarded changes through one storage owner and freezes
//! exact definitions, inputs, capability selections and authority before execution. External
//! effects enter through [`TaskExecutor`]; observations become durable before they are acknowledged.
//! Missing terminal evidence remains uncertain across restart and never authorizes automatic retry.

mod boundary;
mod command;
mod error;
mod executor;
mod tasks;

use executor::MAX_REPORTS_PER_DISPATCH;

#[cfg(any(test, feature = "test-support"))]
pub use boundary::ManualClock;
pub use boundary::{BoundaryClock, IdGenerator, SequentialIdGenerator, SystemBoundaryClock};
pub use command::CommandAuthorityClaim;
pub use error::RuntimeError;
#[cfg(any(test, feature = "test-support"))]
pub use executor::DeterministicExecutor;
pub use executor::{
    CancellationDispatch, CapabilityResolutionContext, EffectAction, ExecutionDispatch,
    ExecutionReporter, ExecutorError, ObservationDisposition, PreparedExecution,
    ResolvedCapability, TaskExecutor,
};
pub use tasks::{
    FrozenTaskStart, ModelAdmission, ModelAdmissionStatus, TaskExecutionDuration, TaskRecord,
    TaskResult, TaskRuntime, TaskState, WorkflowRecord, WorkflowState,
};
