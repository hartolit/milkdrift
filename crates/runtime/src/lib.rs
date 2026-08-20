//! Durable run command handling, deterministic projection, scheduling, execution,
//! recovery, and prospective revision reconciliation for Milkdrift.
//!
//! The crate owns workflow state transitions but no database, provider, operating
//! system process, network, asynchronous runtime, HTTP API, or UI. Every accepted
//! transition is expressed as a persistence-owned append-only event. Executor
//! adapters report bounded observations through [`TaskExecutor`]; they cannot mutate
//! projections or append history.

mod boundary;
mod command;
mod engine;
mod error;
mod executor;
mod projection;
mod query;
mod reconciliation;
mod scheduler;

pub use boundary::{
    BoundaryClock, IdGenerator, ManualClock, SequentialIdGenerator, SystemBoundaryClock,
};
pub use command::{
    ExternalWorkAction, MAX_COMMAND_ITEMS, RUN_COMMAND_SCHEMA_VERSION_V1, RunCommand,
    RunCommandDocument, WorkerReport,
};
pub use engine::{
    CommandExecution, RecoveryResult, RuntimeConfig, RuntimeHealth, RuntimeService, RuntimeStore,
    SchedulerTickResult,
};
pub use error::RuntimeError;
pub use executor::{
    DeterministicExecutor, ExecutionDispatch, ExecutionReportBatch, ExecutorError,
    MAX_REPORTS_PER_DISPATCH, ResolvedCapability, TaskExecutor,
};
pub use projection::*;
pub use reconciliation::{
    HistoricalExecutionState, NodeHistory, ReconciliationPlan, plan_reconciliation,
    validate_plan_is_fresh,
};
pub use scheduler::{
    AdmissionRequest, AdmissionUsage, EvaluationContext, RetryPolicy, SchedulerLimits,
    evaluate_condition, select_fair_runnable,
};
