use std::collections::BTreeMap;

use milkdrift_blueprint::NodeId;
use milkdrift_workspace::{BranchId, ScopeReference, WorkspaceValueReference};
use serde::{Deserialize, Serialize};

use crate::{
    CorrelationKey, CurrencyCode, NodeExecutionId, Reason, SignalId, SignalTypeId, TimerId,
};

/// Terminal outcome of a run or structured child scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    /// Semantic success.
    Succeeded,
    /// Semantic failure.
    Failed,
    /// Cancellation reached a truthful terminal boundary.
    Cancelled,
}

/// Terminal outcome of an individual node attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeOutcome {
    /// Outputs were durably published.
    Succeeded,
    /// The executor or deterministic node semantics failed.
    Failed,
    /// Cancellation was confirmed.
    Cancelled,
    /// Work was rejected before it could begin.
    Rejected,
}

/// Closed dispatch ownership for one logical node execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionMode {
    /// Work may create executor-facing attempts and invocations.
    Executor,
    /// Work is interpreted directly by the runtime without an executor attempt.
    Runtime,
}

/// Stable branch-join synchronization rule recorded in history.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum JoinRule {
    /// Every owned branch reaches a terminal result.
    All,
    /// The first branch completion of any outcome satisfies the join.
    AnyCompletion,
    /// The first successful branch satisfies the join.
    FirstSuccess,
    /// At least `required` successful branches satisfy the join.
    Quorum {
        /// Required successful branch count.
        required: u32,
    },
}

/// Reason an explicit repeat terminated.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatTerminationReason {
    /// The recorded condition evaluated to false.
    ConditionFalse,
    /// Immutable inputs could not be evaluated under the declared condition.
    ConditionEvaluationFailed,
    /// Maximum iteration count was reached.
    MaximumIterations,
    /// A time, cost, resource, or revision budget was exhausted.
    BudgetExhausted,
    /// The body failed under its configured policy.
    BodyFailure,
    /// Cancellation propagated into the repeat.
    Cancelled,
}

/// Closed authority outcome for a repeat awaiting continuation approval.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepeatContinuationDecision {
    /// Authorize a bounded number of additional iterations.
    Approved,
    /// Refuse continuation and require deterministic repeat termination.
    Rejected,
}

/// Exact hard boundary that caused a repeat to request continuation authority.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum RepeatContinuationCause {
    /// The current iteration reached the exact effective iteration limit.
    IterationLimit,
    /// The configured elapsed-duration budget was observed exhausted.
    DurationBudget {
        /// Configured nonzero duration ceiling.
        maximum_ms: u64,
        /// Recorded elapsed duration at the authority boundary.
        observed_ms: u64,
    },
    /// The configured aggregate cost budget was observed exhausted.
    CostBudget {
        /// Configured nonzero cost ceiling in millionths.
        maximum_micros: u64,
        /// Recorded aggregate cost at the authority boundary.
        observed_micros: u64,
        /// Exact currency ledger whose configured ceiling was exhausted.
        currency: CurrencyCode,
    },
    /// A typed controller policy reached its exact human-checkpoint interval.
    ControllerCheckpoint {
        /// Stable digest-derived checkpoint identity.
        checkpoint_id: String,
        /// Number of controller cycles accepted before this checkpoint.
        completed_cycles: u32,
    },
}

/// Runtime boundary at which the ordinary controller lifecycle was assessed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerAssessmentBoundary {
    /// Controller activation before its first cycle is admitted.
    Activation,
    /// Entry to a later explicit repeat cycle.
    CycleEntry,
    /// Re-evaluation immediately before an authorized checkpoint continuation.
    CheckpointContinuation,
    /// Validation of a controller-produced proposal before persistence.
    ProposalAcceptance,
    /// Re-evaluation before proposal approval.
    ProposalApproval,
    /// Re-evaluation before proposal application.
    ProposalApplication,
}

/// Durable closed result of one controller-policy assessment.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ControllerAssessmentOutcome {
    /// The assessed boundary may proceed.
    Continue,
    /// An exact human checkpoint must be decided through ordinary authority.
    HumanCheckpoint {
        /// Stable digest-derived checkpoint identity.
        checkpoint_id: String,
    },
    /// A hard immutable controller ceiling prevents the boundary.
    BoundReached {
        /// Stable snake-case controller dimension.
        bound: String,
        /// Exact accounted value, absent for fail-closed unknown usage or account integrity.
        current: Option<u64>,
        /// Exact enforced limit: the configured ceiling or the admitted reservation envelope.
        limit: u64,
        /// Whether missing authoritative usage caused the fail-closed result.
        unknown_usage: bool,
    },
}

/// Durable signal consumption shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalDeliveryMode {
    /// At most one compatible waiter may consume the signal.
    OneShot,
    /// Every compatible waiter existing at receipt may consume it once.
    Broadcast,
}

/// Exact durable wait condition registered by a wait node.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WaitCondition {
    /// Satisfaction requires one durable timer firing.
    Timer {
        /// Previously registered durable timer.
        timer: TimerId,
    },
    /// Satisfaction requires one compatible typed signal.
    Signal {
        /// Semantic signal type.
        signal_type: SignalTypeId,
        /// Optional correlation identity.
        correlation: Option<CorrelationKey>,
    },
    /// Either a signal or timeout may satisfy the wait exactly once.
    SignalOrTimer {
        /// Previously registered timeout timer.
        timer: TimerId,
        /// Semantic signal type.
        signal_type: SignalTypeId,
        /// Optional correlation identity.
        correlation: Option<CorrelationKey>,
    },
}

/// Recorded cause that satisfied a durable wait.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum WaitSatisfaction {
    /// A registered timer fired.
    Timer {
        /// Fired timer identity.
        timer: TimerId,
    },
    /// A compatible signal was consumed.
    Signal {
        /// Consumed signal identity.
        signal: SignalId,
    },
}

/// Ownership of a child-subworkflow execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubworkflowOwnership {
    /// Parent owns the child and cancellation/liveness is structured.
    Attached,
    /// Parent may terminate while the explicitly detached child continues.
    Detached,
}

/// Truthful classification after a lease/recovery boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClassification {
    /// Work is known not to have started and may be safely dispatched.
    NotStarted,
    /// Pure/read-only/idempotent work may be retried under policy.
    Retryable,
    /// A currently valid lease prevents duplicate dispatch.
    LeaseStillValid,
    /// The externally visible outcome cannot be established.
    Uncertain,
    /// An observed terminal outcome needs no redispatch.
    TerminalObserved,
}

/// Prospective policy requested for live revision adoption.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationPolicy {
    /// Active invocations finish; later invocations use the new definition.
    FinishCurrentThenAdopt,
    /// Cancel and restart only where cancellation and side effects permit.
    CancelAndRestartSafeWork,
    /// Preserve truth and insert explicit compensation/remediation work.
    CompensateOrRemediate,
    /// Remove only never-started work with no completed dependent truth.
    RemoveUnstartedOnly,
    /// No application without an authorized recorded decision.
    RequireAuthority,
}

/// Classification of one item in an immutable reconciliation plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationClassification {
    /// Completed work has identical semantics and dependencies.
    UnchangedCompleted,
    /// Completed pure/read-only work changed; history remains bound to the old revision.
    ChangedCompleted,
    /// Active work has identical semantics and dependencies.
    UnchangedActive,
    /// Pending/not-yet-created work has identical semantics and dependencies.
    UnchangedPending,
    /// An active invocation's configuration or dependencies changed.
    ChangedActive,
    /// Pending work changed before execution.
    ChangedPending,
    /// Work exists only in the new revision.
    Added,
    /// Never-started work was removed.
    RemovedPending,
    /// Changed/removed work already produced completed or uncertain effects.
    CompletedOrUncertainSideEffects,
    /// Dependency changes affect an already-started descendant.
    StartedDescendantDependencyChanged,
    /// Workflow interface or pinned child contract is incompatible.
    IncompatibleInterfaceOrSubworkflow,
    /// A controller or operator must decide.
    RequiresAuthority,
}

/// Prospective action assigned to one reconciliation item.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationAction {
    /// Preserve the existing completed/active execution.
    Preserve,
    /// Use new semantics only for a later invocation.
    UseNewOnNextInvocation,
    /// Cancel and restart after a confirmed safe boundary.
    CancelAndRestart,
    /// Remove work that has never started.
    RemoveUnstarted,
    /// Create explicit compensating/remediation work.
    CompensateOrRemediate,
    /// Await a recorded authority decision.
    RequireAuthority,
    /// Reject because it would reinterpret or rewrite completed truth.
    RejectRetrospectiveRewrite,
}

/// Result of an authority decision over reconciliation or uncertain work.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityDecision {
    /// Approve the proposed prospective action.
    Approve,
    /// Reject the proposed action without changing past facts.
    Reject,
    /// Retain external work as an unresolved obligation.
    Retain,
    /// Authorize an external status query while leaving the obligation unresolved.
    Query,
    /// Permit a new retry attempt under explicit authority.
    Retry,
    /// Permit explicit compensation/remediation work.
    Compensate,
    /// Resolve the outcome as succeeded based on supplied evidence.
    ResolveSucceeded,
    /// Resolve the outcome as failed based on supplied evidence.
    ResolveFailed,
}

/// One branch result referenced by a satisfied join.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BranchResultReference {
    /// Structured branch identity.
    pub branch: BranchId,
    /// Branch-local scope.
    pub scope: ScopeReference,
    /// Truthful terminal outcome.
    pub outcome: RunOutcome,
    /// Small exact value references; bytes remain in workspace/artifact storage.
    pub outputs: Vec<WorkspaceValueReference>,
}

/// One immutable reconciliation classification and selected prospective action.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationItem {
    /// Stable semantic node when the item is node-backed.
    pub node: Option<NodeId>,
    /// Existing execution when one has already been created.
    pub execution: Option<NodeExecutionId>,
    /// Planner classification.
    pub classification: ReconciliationClassification,
    /// Prospective action; never a mutation of prior facts.
    pub action: ReconciliationAction,
    /// Bounded deterministic rationale.
    pub reason: Reason,
}

/// Exact observed monetary cost without floating-point ambiguity.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MonetaryUsage {
    /// Cost in one-millionth of the named currency unit.
    pub micros: u64,
    /// Validated three-letter uppercase currency code.
    pub currency: CurrencyCode,
}

/// Bounded provider-neutral resource observations for one attempt.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptUsage {
    /// Provider-defined input units when observed.
    pub input_units: Option<u64>,
    /// Provider-defined output units when observed.
    pub output_units: Option<u64>,
    /// Measured executor duration when observed.
    pub duration_ms: Option<u64>,
    /// Exact monetary observation when supplied.
    pub cost: Option<MonetaryUsage>,
}

/// Compact exact child-run accounting folded into its structured parent.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubworkflowResourceUsage {
    /// Sum of observed input units, absent when no child observation exists.
    pub input_units: Option<u64>,
    /// Sum of observed output units, absent when no child observation exists.
    pub output_units: Option<u64>,
    /// Logical artifact bytes published by the child run.
    pub artifact_bytes: u64,
    /// Exact cost totals grouped by currency.
    pub cost_micros: BTreeMap<CurrencyCode, u64>,
    /// Exact process-category attempts admitted by the child.
    pub process_invocations: u64,
    /// Exact model-category attempts admitted by the child.
    pub model_invocations: u64,
    /// Metered attempts missing input-unit observations.
    pub unknown_input_usage: u64,
    /// Metered attempts missing output-unit observations.
    pub unknown_output_usage: u64,
    /// Metered attempts missing cost observations.
    pub unknown_cost_usage: u64,
}

impl SubworkflowResourceUsage {
    pub(crate) fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}
