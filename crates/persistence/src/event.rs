mod kind;
mod model;
mod references;

const MAX_REFERENCES_PER_EVENT: usize = 512;
/// Maximum plan items whose per-item actions plus apply/pin fit one atomic commit.
pub const MAX_RECONCILIATION_PLAN_ITEMS: usize = 510;
/// Maximum iterations one repeat-continuation decision may authorize.
pub const MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS: u32 = 1_000;
/// Maximum request/decision cycles retained by one repeat execution.
pub const MAX_REPEAT_CONTINUATION_CYCLES: usize = 64;
/// Maximum repeat-continuation authority decisions retained by one repeat execution.
pub const MAX_REPEAT_CONTINUATION_DECISIONS: usize = MAX_REPEAT_CONTINUATION_CYCLES;
/// Absolute effective iteration limit after all repeat-continuation approvals.
pub const MAX_REPEAT_EFFECTIVE_ITERATIONS: u32 = 100_000;

pub use kind::RunEventKind;
pub use model::{
    AttemptUsage, AuthorityDecision, BranchResultReference, ControllerAssessmentBoundary,
    ControllerAssessmentOutcome, JoinRule, MonetaryUsage, NodeExecutionMode, NodeOutcome,
    ReconciliationAction, ReconciliationClassification, ReconciliationItem, ReconciliationPolicy,
    RecoveryClassification, RepeatContinuationCause, RepeatContinuationDecision,
    RepeatTerminationReason, RunOutcome, SignalDeliveryMode, SubworkflowOwnership,
    SubworkflowResourceUsage, WaitCondition, WaitSatisfaction,
};
