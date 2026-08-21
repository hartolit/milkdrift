use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CapabilityRequirement, ErrorClass,
    IdempotencyBehavior, IdempotencyKey, InvocationId, InvocationRequest, InvocationTerminal,
    ResolvedCapabilitySnapshot, SideEffectClass, TerminalStatus,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, BranchId, CausalReference,
    ContentDigest as ArtifactContentDigest, IterationId, MediaType, RunId, ScopeReference,
    SubworkflowId, WorkspaceBudget, WorkspaceScope, WorkspaceValueReference,
};
use serde::{Deserialize, Serialize};

use crate::{
    ActorRef, AttemptId, BoundedDetail, CommandId, CorrelationKey, CurrencyCode, EvidenceReference,
    LeaseId, NodeExecutionId, PersistenceError, Reason, ReconciliationDecisionId, ReconciliationId,
    ReconciliationPlanId, RepeatDecisionId, RunSequence, SignalId, SignalTypeId, TimerId,
    TimestampMillis, WorkerId, bounded::MAX_EVIDENCE_REFERENCES,
};

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

/// Closed schema-v1 run facts. Variants describe observations, never requested actions.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)] // Durable facts remain direct typed schema fields.
pub enum RunEventKind {
    /// A run aggregate was created and pinned to an exact revision.
    RunCreated {
        /// Workflow lineage.
        workflow: WorkflowId,
        /// Exact immutable revision.
        revision: RevisionId,
        /// Semantic content digest of the pinned revision.
        revision_digest: ContentDigest,
        /// Root durable workspace scope.
        root_scope: WorkspaceScope,
        /// Immutable workspace/artifact limits recoverable solely from history.
        workspace_budget: WorkspaceBudget,
        /// Exact bounded input references.
        inputs: Vec<WorkspaceValueReference>,
    },
    /// A prospective revision became the pin from a recorded sequence onward.
    RevisionPinned {
        /// Previous exact revision.
        previous: RevisionId,
        /// Newly pinned exact revision.
        revision: RevisionId,
        /// New revision semantic digest.
        revision_digest: ContentDigest,
        /// Plan authorizing this prospective boundary.
        plan: ReconciliationPlanId,
    },
    /// A created run entered execution.
    RunStarted,
    /// Admission and dispatch were paused.
    RunPaused {
        /// Why the transition occurred.
        reason: Reason,
        /// References supporting the transition.
        evidence: Vec<EvidenceReference>,
    },
    /// A paused run resumed.
    RunResumed {
        /// Why the transition occurred.
        reason: Reason,
        /// References supporting the transition.
        evidence: Vec<EvidenceReference>,
    },
    /// Durable cancellation intent was recorded.
    RunCancellationRequested {
        /// Why cancellation was requested.
        reason: Reason,
        /// Evidence/authority references.
        evidence: Vec<EvidenceReference>,
    },
    /// An explicit terminal selected a non-cancellation outcome and began a
    /// structured drain of already-owned work.
    RunTerminationRequested {
        /// Desired terminal outcome once every owned scope is quiescent.
        outcome: RunOutcome,
        /// Why structured draining began.
        reason: Reason,
    },
    /// A run reached a truthful terminal boundary.
    RunTerminal {
        /// Semantic outcome.
        outcome: RunOutcome,
        /// Exact terminal workspace values.
        outputs: Vec<WorkspaceValueReference>,
        /// Content-addressed terminal artifact references.
        artifacts: Vec<ArtifactReference>,
        /// Bounded terminal rationale when relevant.
        reason: Option<Reason>,
    },
    /// A node became durably eligible in an exact scope.
    NodeBecameEligible {
        /// Stable blueprint node.
        node: NodeId,
        /// Stable execution identity.
        execution: NodeExecutionId,
        /// Workspace scope used by the execution.
        scope: ScopeReference,
        /// Closed ownership deciding whether executor attempts may exist.
        mode: NodeExecutionMode,
    },
    /// A never-dispatched logical execution was cancelled without fabricating an attempt.
    NodeExecutionCancelledBeforeDispatch {
        /// Exact logical execution.
        execution: NodeExecutionId,
        /// Bounded causal cancellation rationale.
        reason: Reason,
    },
    /// Durable cancellation intent targeted one exact active executor attempt.
    NodeExecutionCancellationRequested {
        /// Exact logical execution.
        execution: NodeExecutionId,
        /// Latest scheduled, leased, or running attempt.
        attempt: AttemptId,
        /// Bounded causal cancellation rationale.
        reason: Reason,
    },
    /// A specific attempt/invocation was scheduled before dispatch.
    NodeScheduled {
        /// Stable blueprint node.
        node: NodeId,
        /// Logical execution identity.
        execution: NodeExecutionId,
        /// Immutable attempt identity.
        attempt: AttemptId,
        /// Executor-facing invocation identity.
        invocation: InvocationId,
        /// Stable external idempotency key, when supported.
        idempotency_key: Option<IdempotencyKey>,
        /// Exact bounded request delivered to the executor after durable scheduling.
        request: InvocationRequest,
    },
    /// Exact capability resolution facts were frozen before dispatch.
    CapabilityResolved {
        /// Owning execution.
        execution: NodeExecutionId,
        /// Owning immutable attempt.
        attempt: AttemptId,
        /// Blueprint-owned selection requirement.
        requirement: CapabilityRequirement,
        /// Exact canonical capability/operation snapshot supplied before dispatch.
        snapshot: ResolvedCapabilitySnapshot,
    },
    /// Side-effect and idempotency facts were frozen before dispatch.
    SideEffectClassified {
        /// Owning attempt.
        attempt: AttemptId,
        /// Declared potential side-effect class.
        side_effect: SideEffectClass,
        /// Advertised executor idempotency behavior.
        idempotency: IdempotencyBehavior,
        /// Stable propagated key, when one exists.
        idempotency_key: Option<IdempotencyKey>,
    },
    /// A worker lease was granted durably.
    LeaseGranted {
        /// Lease identity.
        lease: LeaseId,
        /// Owning execution.
        execution: NodeExecutionId,
        /// Owning attempt.
        attempt: AttemptId,
        /// Worker/controller identity.
        worker: WorkerId,
        /// Boundary-clock expiration fact.
        expires_at: TimestampMillis,
    },
    /// A valid lease was extended by an authenticated heartbeat.
    LeaseHeartbeatRecorded {
        /// Lease identity.
        lease: LeaseId,
        /// New expiration fact.
        expires_at: TimestampMillis,
    },
    /// A lease expired according to a recorded boundary-clock observation.
    LeaseExpired {
        /// Expired lease identity.
        lease: LeaseId,
        /// Recovery classification at expiry.
        classification: RecoveryClassification,
    },
    /// Work was assigned a new lease after recovery.
    NodeReLeased {
        /// Previous expired lease.
        previous_lease: LeaseId,
        /// New durable lease.
        lease: LeaseId,
        /// Owning attempt.
        attempt: AttemptId,
        /// New worker.
        worker: WorkerId,
        /// New expiration fact.
        expires_at: TimestampMillis,
    },
    /// Executor admission/start was observed.
    NodeStarted {
        /// Owning execution.
        execution: NodeExecutionId,
        /// Immutable attempt.
        attempt: AttemptId,
        /// Invocation correlation identity.
        invocation: InvocationId,
    },
    /// Bounded executor progress was observed.
    NodeProgressRecorded {
        /// Owning attempt.
        attempt: AttemptId,
        /// Monotonic executor-report sequence.
        report_sequence: u64,
        /// Redacted bounded detail.
        detail: BoundedDetail,
        /// Provider-defined completed units.
        completed_units: Option<u64>,
        /// Provider-defined total units.
        total_units: Option<u64>,
    },
    /// Provider-neutral resource/budget observations were recorded for an attempt.
    AttemptUsageRecorded {
        /// Owning immutable attempt.
        attempt: AttemptId,
        /// Bounded exact usage facts.
        usage: AttemptUsage,
    },
    /// Executor cancellation support/terminal-boundary acknowledgement was observed.
    InvocationCancellationAcknowledged {
        /// Owning immutable attempt.
        attempt: AttemptId,
        /// Validated executor acknowledgement.
        acknowledgement: CancellationAcknowledgement,
    },
    /// A node output became durably addressable.
    NodeOutputPublished {
        /// Owning execution.
        execution: NodeExecutionId,
        /// Owning attempt.
        attempt: AttemptId,
        /// Monotonic executor-report sequence.
        report_sequence: u64,
        /// Exact immutable workspace value.
        value: WorkspaceValueReference,
        /// Optional content-addressed large value.
        artifact: Option<ArtifactReference>,
    },
    /// A deterministic runtime-owned node output became durably addressable.
    DeterministicOutputPublished {
        /// Owning logical execution.
        execution: NodeExecutionId,
        /// Exact immutable workspace value.
        value: WorkspaceValueReference,
        /// Optional content-addressed large value.
        artifact: Option<ArtifactReference>,
    },
    /// A runtime-owned deterministic node reached a terminal outcome without an executor attempt.
    DeterministicNodeTerminal {
        /// Owning logical execution.
        execution: NodeExecutionId,
        /// Truthful deterministic outcome.
        outcome: NodeOutcome,
        /// Classified failure where relevant.
        error_class: Option<ErrorClass>,
        /// Bounded deterministic result/failure detail.
        detail: Option<BoundedDetail>,
    },
    /// An executor-owned node failed before any attempt or lease was created because
    /// its immutable invocation inputs could not be materialized within the durable
    /// request/event contract.
    NodePreDispatchFailed {
        /// Owning logical execution.
        execution: NodeExecutionId,
        /// Truthful immutable request/materialization failure classification.
        error_class: ErrorClass,
        /// Bounded deterministic failure detail.
        detail: Option<BoundedDetail>,
    },
    /// Runtime successor planning durably examined one successful execution.
    StructuredSuccessorScanCompleted {
        /// Successful execution whose current-revision outgoing routes were examined.
        execution: NodeExecutionId,
    },
    /// An immutable attempt reached a known terminal outcome.
    NodeTerminal {
        /// Owning execution.
        execution: NodeExecutionId,
        /// Immutable attempt.
        attempt: AttemptId,
        /// Monotonic executor-report sequence.
        report_sequence: u64,
        /// Truthful known outcome (uncertainty has a separate fact).
        outcome: NodeOutcome,
        /// Classified failure where relevant.
        error_class: Option<ErrorClass>,
        /// Bounded redacted result/failure detail.
        detail: Option<BoundedDetail>,
    },
    /// A bounded retry and its durable backoff timer were selected.
    NodeRetryScheduled {
        /// Logical execution.
        execution: NodeExecutionId,
        /// Completed prior attempt.
        previous_attempt: AttemptId,
        /// Next immutable attempt.
        next_attempt: AttemptId,
        /// One-based attempt number.
        attempt_number: u32,
        /// Durable timer controlling retry admission.
        timer: TimerId,
        /// Recorded delay including deterministic/recorded jitter.
        fire_at: TimestampMillis,
        /// Stable retry classification.
        error_class: ErrorClass,
        /// Policy rationale.
        reason: Reason,
    },
    /// An externally visible outcome cannot be established honestly.
    ExternalOutcomeUncertain {
        /// Owning attempt.
        attempt: AttemptId,
        /// Monotonic executor-report sequence at which uncertainty was reported.
        report_sequence: u64,
        /// Side-effect classification governing recovery.
        side_effect: SideEffectClass,
        /// Why certainty was lost.
        reason: Reason,
        /// Supporting references, never arbitrary evidence bytes.
        evidence: Vec<EvidenceReference>,
    },
    /// A terminal observation arrived after active lease ownership was lost.
    ///
    /// This is evidence only. It never rewrites an uncertainty decision or a later
    /// retry result; explicit recovery authority decides how the evidence affects
    /// logical workflow state.
    LateTerminalEvidenceRecorded {
        /// Owning immutable attempt.
        attempt: AttemptId,
        /// Worker that historically owned a lease for the attempt.
        worker: WorkerId,
        /// Original executor-local report sequence.
        report_sequence: u64,
        /// Provider-neutral terminal observation.
        terminal: InvocationTerminal,
    },
    /// External work was explicitly retained rather than silently retried.
    ExternalOutcomeRetained {
        /// Owning attempt.
        attempt: AttemptId,
        /// Recorded authority decision.
        decision: ReconciliationDecisionId,
        /// Why work remains retained.
        reason: Reason,
    },
    /// A published content-addressed artifact became available for later events.
    ArtifactPublished {
        /// Validated metadata, integrity, sensitivity, retention, and provenance.
        metadata: ArtifactMetadata,
    },
    /// A branch-local workspace scope was created.
    BranchScopeCreated {
        /// Owning fork execution.
        fork_execution: NodeExecutionId,
        /// Exact declared fork control port owning this branch.
        port: PortId,
        /// Stable semantic branch.
        branch: BranchId,
        /// Validated child scope.
        scope: WorkspaceScope,
    },
    /// A deterministic branch condition selected one exact outgoing control port.
    BranchRouteSelected {
        /// Owning branch-node execution.
        execution: NodeExecutionId,
        /// Exact selected route; replay never re-evaluates the condition.
        selected_port: PortId,
    },
    /// A child execution became owned by a structured branch.
    BranchChildAdded {
        /// Owning branch.
        branch: BranchId,
        /// Owned child execution.
        execution: NodeExecutionId,
    },
    /// Structured cancellation intent was propagated into a branch.
    BranchCancellationRequested {
        /// Branch identity.
        branch: BranchId,
        /// Why cancellation propagated.
        reason: Reason,
    },
    /// One structured branch reached an independent terminal boundary.
    BranchTerminal {
        /// Stable branch identity.
        branch: BranchId,
        /// Truthful branch-local outcome.
        outcome: RunOutcome,
        /// Exact immutable branch-local outputs.
        outputs: Vec<WorkspaceValueReference>,
    },
    /// A join rule was satisfied over explicit branch result references.
    JoinSatisfied {
        /// Join execution.
        execution: NodeExecutionId,
        /// Exact synchronization rule.
        rule: JoinRule,
        /// Immutable branch results supplied to downstream composition.
        branches: Vec<BranchResultReference>,
        /// Branches retained instead of being cancelled after early satisfaction.
        retained_branches: Vec<BranchId>,
    },
    /// One isolated repeat iteration and workspace scope was created.
    RepeatIterationCreated {
        /// Owning repeat execution.
        repeat_execution: NodeExecutionId,
        /// Stable iteration identity.
        iteration: IterationId,
        /// One-based iteration number.
        iteration_number: u32,
        /// Isolated child scope.
        scope: WorkspaceScope,
    },
    /// A repeat condition result was frozen so replay never re-evaluates it.
    RepeatConditionRecorded {
        /// Stable iteration.
        iteration: IterationId,
        /// Deterministic boolean result.
        result: bool,
    },
    /// A repeat reached an exact durable boundary requiring external authority.
    RepeatContinuationRequested {
        /// Owning repeat execution.
        repeat_execution: NodeExecutionId,
        /// Latest true-condition iteration at the boundary.
        frontier_iteration: IterationId,
        /// Original configured iteration limit.
        initial_iteration_limit: u32,
        /// Current iteration limit after all prior approvals.
        effective_iteration_limit: u32,
        /// Exact iteration or resource boundary that was reached.
        cause: RepeatContinuationCause,
    },
    /// Authority decided whether a repeat at an approval boundary may continue.
    RepeatContinuationDecided {
        /// Owning repeat execution awaiting approval.
        repeat_execution: NodeExecutionId,
        /// Stable idempotency identity of this authority decision.
        decision: RepeatDecisionId,
        /// Actor exercising authority.
        actor: ActorRef,
        /// Closed approval/rejection outcome.
        outcome: RepeatContinuationDecision,
        /// Additional iterations authorized only for an approved decision.
        approved_additional_iterations: Option<u32>,
        /// Bounded authority rationale.
        reason: Reason,
        /// Supporting durable evidence references.
        evidence: Vec<EvidenceReference>,
    },
    /// An explicit repeat reached a deterministic terminal reason.
    RepeatTerminated {
        /// Owning repeat execution.
        repeat_execution: NodeExecutionId,
        /// Termination classification.
        termination: RepeatTerminationReason,
        /// Last created iteration, if any.
        last_iteration: Option<IterationId>,
    },
    /// A durable wait/timer was registered.
    TimerRegistered {
        /// Timer identity.
        timer: TimerId,
        /// Waiting execution, if node-backed.
        execution: Option<NodeExecutionId>,
        /// Exact boundary-clock deadline.
        fire_at: TimestampMillis,
    },
    /// A boundary-clock observation fired a registered timer.
    TimerFired {
        /// Timer identity.
        timer: TimerId,
        /// Recorded observation timestamp.
        observed_at: TimestampMillis,
    },
    /// A pending durable timer was explicitly cancelled by its structured owner.
    TimerCancelled {
        /// Timer identity.
        timer: TimerId,
        /// Bounded causal cancellation rationale.
        reason: Reason,
    },
    /// A wait node registered an exact durable signal/timer condition.
    WaitRegistered {
        /// Waiting execution.
        execution: NodeExecutionId,
        /// Exact condition used for durable recovery.
        condition: WaitCondition,
    },
    /// A wait was satisfied by one recorded timer/signal fact.
    WaitSatisfied {
        /// Waiting execution.
        execution: NodeExecutionId,
        /// Exact satisfaction cause.
        cause: WaitSatisfaction,
    },
    /// An unsatisfied durable wait was explicitly cancelled by its structured owner.
    WaitCancelled {
        /// Waiting execution.
        execution: NodeExecutionId,
        /// Bounded causal cancellation rationale.
        reason: Reason,
    },
    /// A typed external signal was received durably.
    SignalReceived {
        /// Signal identity and delivery idempotency key.
        signal: SignalId,
        /// Semantic payload type.
        signal_type: SignalTypeId,
        /// Optional correlation identity.
        correlation: Option<CorrelationKey>,
        /// Explicit consumption shape.
        mode: SignalDeliveryMode,
        /// Bounded typed payload.
        payload: BoundedJson,
    },
    /// A bounded internal pass advanced through the ordered broadcast-wait catalog.
    ///
    /// The cursor is durable so a large or mostly incompatible wait catalog cannot
    /// force one command/tick to rescan or retain the whole fanout in memory.
    SignalBroadcastScanAdvanced {
        /// Broadcast signal being drained.
        signal: SignalId,
        /// Last ordered wait execution examined, absent only for an empty catalog.
        through_execution: Option<NodeExecutionId>,
        /// Whether the scan reached the end of the current eligible catalog.
        complete: bool,
    },
    /// A duplicate delivery was observed without consuming twice.
    SignalDeduplicated {
        /// Duplicate signal identity.
        signal: SignalId,
        /// Exact later command whose duplicate observation was recorded.
        duplicate_command: CommandId,
    },
    /// A waiter consumed one signal exactly once.
    SignalConsumed {
        /// Signal identity.
        signal: SignalId,
        /// Waiting execution.
        execution: NodeExecutionId,
    },
    /// A pinned child run and structured ownership link were created.
    SubworkflowCreated {
        /// Stable parent-local subworkflow identity.
        subworkflow: SubworkflowId,
        /// Parent node execution.
        parent_execution: NodeExecutionId,
        /// Child run aggregate.
        child_run: RunId,
        /// Exact pinned child revision.
        child_revision: RevisionId,
        /// Child workspace scope.
        scope: WorkspaceScope,
        /// Structured or detached ownership.
        ownership: SubworkflowOwnership,
        /// Exact child inputs.
        inputs: Vec<WorkspaceValueReference>,
    },
    /// Child termination and output binding were observed by its parent.
    SubworkflowTerminal {
        /// Parent-local child identity.
        subworkflow: SubworkflowId,
        /// Child aggregate.
        child_run: RunId,
        /// Child outcome.
        outcome: RunOutcome,
        /// Exact bound outputs.
        outputs: Vec<WorkspaceValueReference>,
    },
    /// One exact child-run output was imported into an immutable parent value.
    SubworkflowOutputImported {
        /// Parent-local child identity used by projection to prove ownership.
        subworkflow: SubworkflowId,
        /// Exact source value in the child run.
        child_value: WorkspaceValueReference,
        /// Exact imported value in the parent run.
        parent_value: WorkspaceValueReference,
    },
    /// Structured cancellation intent propagated from parent to attached child.
    SubworkflowCancellationRequested {
        /// Parent-local child identity.
        subworkflow: SubworkflowId,
        /// Child aggregate.
        child_run: RunId,
        /// Bounded cancellation rationale.
        reason: Reason,
    },
    /// A prospective adoption request was accepted against an exact pin.
    RevisionAdoptionRequested {
        /// Reconciliation request identity.
        reconciliation: ReconciliationId,
        /// Exact old revision.
        from_revision: RevisionId,
        /// Exact requested new revision.
        to_revision: RevisionId,
        /// Requested prospective policy.
        policy: ReconciliationPolicy,
    },
    /// A deterministic reconciliation plan was persisted before application.
    ReconciliationPlanRecorded {
        /// Owning request.
        reconciliation: ReconciliationId,
        /// Immutable plan identity.
        plan: ReconciliationPlanId,
        /// Exact old revision.
        from_revision: RevisionId,
        /// Exact new revision.
        to_revision: RevisionId,
        /// Run sequence whose projection was compared.
        based_on_sequence: RunSequence,
        /// Closed classifications and prospective actions.
        items: Vec<ReconciliationItem>,
    },
    /// An authority decision over a plan item was recorded.
    ReconciliationDecisionRecorded {
        /// Immutable plan.
        plan: ReconciliationPlanId,
        /// Stable decision identity.
        decision: ReconciliationDecisionId,
        /// Authorized actor reference.
        actor: ActorRef,
        /// Closed decision.
        outcome: AuthorityDecision,
        /// Bounded rationale.
        reason: Reason,
        /// Evidence references.
        evidence: Vec<EvidenceReference>,
    },
    /// An immutable plan was applied prospectively at an exact sequence boundary.
    ReconciliationApplied {
        /// Applied plan.
        plan: ReconciliationPlanId,
        /// Exact old pin.
        from_revision: RevisionId,
        /// Exact new pin.
        to_revision: RevisionId,
        /// Sequence to which the plan was bound before this fact was appended.
        based_on_sequence: RunSequence,
    },
    /// Never-started work was removed prospectively by an exact reconciliation plan.
    ReconciliationExecutionRemoved {
        /// Immutable authorizing plan.
        plan: ReconciliationPlanId,
        /// Exact never-started logical execution removed from future scheduling.
        execution: NodeExecutionId,
    },
    /// Safe active work received durable cancellation intent from a reconciliation plan.
    ReconciliationCancellationRequested {
        /// Immutable authorizing plan.
        plan: ReconciliationPlanId,
        /// Exact logical execution being cancelled prospectively.
        execution: NodeExecutionId,
        /// Exact active attempt receiving cancellation intent.
        attempt: AttemptId,
        /// Bounded deterministic rationale.
        reason: Reason,
    },
    /// A reconciliation plan created explicit prospective remediation work.
    ReconciliationRemediationCreated {
        /// Immutable authorizing plan.
        plan: ReconciliationPlanId,
        /// Existing execution whose truth requires remediation.
        source_execution: NodeExecutionId,
        /// Existing exact attempt when the source had one.
        source_attempt: Option<AttemptId>,
        /// New independently scheduled execution.
        execution: NodeExecutionId,
        /// Exact target node in the adopted revision.
        node: NodeId,
        /// Durable workspace scope owned by the new execution.
        scope: ScopeReference,
        /// Closed dispatch ownership derived from the target node kind.
        mode: NodeExecutionMode,
        /// Bounded deterministic rationale.
        reason: Reason,
    },
    /// A recovery controller began examining a run from durable history.
    RecoveryStarted {
        /// Stable controller identity.
        controller: WorkerId,
        /// Journal head examined.
        through_sequence: RunSequence,
    },
    /// Recovery classified one attempt/lease obligation.
    RecoveryClassified {
        /// Attempt being recovered.
        attempt: AttemptId,
        /// Lease when one existed.
        lease: Option<LeaseId>,
        /// Truthful recovery result.
        classification: RecoveryClassification,
        /// Bounded rationale.
        reason: Reason,
    },
    /// An operator/controller resolved retained or uncertain work.
    RecoveryDecisionRecorded {
        /// Attempt being resolved.
        attempt: AttemptId,
        /// Stable decision identity.
        decision: ReconciliationDecisionId,
        /// Actor reference.
        actor: ActorRef,
        /// Closed resolution.
        outcome: AuthorityDecision,
        /// Bounded rationale.
        reason: Reason,
        /// Supporting evidence references.
        evidence: Vec<EvidenceReference>,
    },
    /// Explicit compensation/remediation was created as new work, preserving history.
    RemediationWorkCreated {
        /// Prior attempt whose truth requires remediation.
        source_attempt: AttemptId,
        /// New logical execution with independent immutable history.
        execution: NodeExecutionId,
        /// Exact remediation target node in the current pinned revision.
        node: NodeId,
        /// Durable workspace scope inherited from the source execution.
        scope: ScopeReference,
        /// Closed dispatch ownership derived from the remediation target.
        mode: NodeExecutionMode,
        /// Authority decision allowing this new work.
        decision: ReconciliationDecisionId,
        /// Bounded rationale.
        reason: Reason,
    },
}

impl RunEventKind {
    /// Derives every complete content-addressed artifact reference retained by this fact.
    ///
    /// The atomic journal uses this as the sole event-side ownership source. Executor
    /// requests may carry provider-neutral artifact references, but durable history
    /// requires their media type and exact size so verification and workspace accounting
    /// cannot be bypassed by a direct blueprint artifact binding.
    pub fn required_artifacts(&self) -> Result<Vec<ArtifactReference>, PersistenceError> {
        match self {
            Self::RunTerminal { artifacts, .. } => Ok(artifacts.clone()),
            Self::NodeScheduled { request, .. } => request
                .inputs()
                .iter()
                .filter_map(|input| input.value().artifact())
                .map(workspace_artifact_reference)
                .collect(),
            Self::NodeOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::DeterministicOutputPublished {
                artifact: Some(reference),
                ..
            } => Ok(vec![reference.clone()]),
            Self::ArtifactPublished { metadata } => {
                let mut references = vec![metadata.reference().clone()];
                for causal in std::iter::once(metadata.provenance().producer())
                    .chain(metadata.provenance().causes())
                {
                    if let CausalReference::Artifact { reference } = causal {
                        references.push(reference.clone());
                    }
                }
                Ok(references)
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) fn validate_for_run(&self, run: &RunId) -> Result<(), PersistenceError> {
        if matches!(
            self,
            Self::NodeProgressRecorded {
                report_sequence: 0,
                ..
            } | Self::NodeOutputPublished {
                report_sequence: 0,
                ..
            } | Self::NodeTerminal {
                report_sequence: 0,
                ..
            } | Self::ExternalOutcomeUncertain {
                report_sequence: 0,
                ..
            } | Self::LateTerminalEvidenceRecorded {
                report_sequence: 0,
                ..
            }
        ) {
            return Err(PersistenceError::InvalidDocument(
                "executor report sequences are one-based".to_owned(),
            ));
        }
        let check_references = |location: &'static str, count: usize| {
            if count > MAX_REFERENCES_PER_EVENT {
                Err(PersistenceError::Bounds {
                    location,
                    reason: format!("at most {MAX_REFERENCES_PER_EVENT} references are allowed"),
                })
            } else {
                Ok(())
            }
        };
        let check_evidence = |evidence: &[EvidenceReference]| {
            if evidence.len() > MAX_EVIDENCE_REFERENCES {
                Err(PersistenceError::Bounds {
                    location: "event.evidence",
                    reason: format!("at most {MAX_EVIDENCE_REFERENCES} references are allowed"),
                })
            } else if evidence
                .iter()
                .map(|item| &item.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != evidence.len()
            {
                Err(PersistenceError::InvalidDocument(
                    "event evidence identities must be distinct".to_owned(),
                ))
            } else {
                Ok(())
            }
        };

        match self {
            Self::RunCreated { inputs, .. } | Self::SubworkflowCreated { inputs, .. } => {
                check_references("event.inputs", inputs.len())?;
                if inputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != inputs.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "event input references must be distinct".to_owned(),
                    ));
                }
            }
            Self::RunTerminal {
                outputs, artifacts, ..
            } => {
                check_references("event.outputs", outputs.len())?;
                check_references("event.artifacts", artifacts.len())?;
                if outputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != outputs.len()
                    || artifacts
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != artifacts.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "terminal output and artifact references must be distinct".to_owned(),
                    ));
                }
            }
            Self::RunTerminationRequested { outcome, .. } if *outcome != RunOutcome::Failed => {
                return Err(PersistenceError::InvalidDocument(
                    "internal run termination currently supports only an explicit failed outcome"
                        .to_owned(),
                ));
            }
            Self::SignalBroadcastScanAdvanced {
                through_execution: None,
                complete: false,
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "an incomplete broadcast scan must advance through one wait execution"
                        .to_owned(),
                ));
            }
            Self::RecoveryDecisionRecorded {
                outcome: AuthorityDecision::ResolveSucceeded | AuthorityDecision::ResolveFailed,
                evidence,
                ..
            } if evidence.is_empty() => {
                return Err(PersistenceError::InvalidDocument(
                    "terminal external-work resolution requires at least one evidence reference"
                        .to_owned(),
                ));
            }
            Self::RunPaused { evidence, .. }
            | Self::RunResumed { evidence, .. }
            | Self::RunCancellationRequested { evidence, .. }
            | Self::ExternalOutcomeUncertain { evidence, .. }
            | Self::ReconciliationDecisionRecorded { evidence, .. }
            | Self::RecoveryDecisionRecorded { evidence, .. } => check_evidence(evidence)?,
            Self::RepeatContinuationRequested {
                initial_iteration_limit,
                effective_iteration_limit,
                cause,
                ..
            } => {
                let limits_valid = *initial_iteration_limit > 0
                    && *initial_iteration_limit <= *effective_iteration_limit
                    && *effective_iteration_limit <= MAX_REPEAT_EFFECTIVE_ITERATIONS;
                let cause_valid = match cause {
                    RepeatContinuationCause::IterationLimit => true,
                    RepeatContinuationCause::DurationBudget {
                        maximum_ms,
                        observed_ms,
                    } => *maximum_ms > 0 && observed_ms >= maximum_ms,
                    RepeatContinuationCause::CostBudget {
                        maximum_micros,
                        observed_micros,
                        ..
                    } => *maximum_micros > 0 && observed_micros >= maximum_micros,
                };
                if !limits_valid || !cause_valid {
                    return Err(PersistenceError::InvalidDocument(format!(
                        "repeat continuation request requires limits within 1..={MAX_REPEAT_EFFECTIVE_ITERATIONS} and a truthfully exhausted typed cause"
                    )));
                }
            }
            Self::RepeatContinuationDecided {
                outcome,
                approved_additional_iterations,
                evidence,
                ..
            } => {
                check_evidence(evidence)?;
                let valid = match (outcome, approved_additional_iterations) {
                    (RepeatContinuationDecision::Approved, Some(additional)) => {
                        (1..=MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS).contains(additional)
                    }
                    (RepeatContinuationDecision::Rejected, None) => true,
                    (RepeatContinuationDecision::Approved, None)
                    | (RepeatContinuationDecision::Rejected, Some(_)) => false,
                };
                if !valid {
                    return Err(PersistenceError::InvalidDocument(format!(
                        "repeat approval requires 1..={MAX_REPEAT_CONTINUATION_ADDITIONAL_ITERATIONS} additional iterations and rejection forbids them"
                    )));
                }
            }
            Self::NodeScheduled {
                invocation,
                idempotency_key,
                request,
                ..
            } if request.invocation() != invocation
                || request.idempotency_key() != idempotency_key.as_ref() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "scheduled invocation/idempotency facts contradict the persisted request"
                        .to_owned(),
                ));
            }
            Self::CapabilityResolved {
                requirement,
                snapshot,
                ..
            } => {
                requirement
                    .validate()
                    .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
                if requirement.operation() != snapshot.operation()
                    || requirement
                        .exact_capability()
                        .is_some_and(|identity| identity != snapshot.capability())
                    || requirement
                        .provider_profile_ref()
                        .is_some_and(|profile| Some(profile) != snapshot.provider_profile())
                {
                    return Err(PersistenceError::InvalidDocument(
                        "resolved capability snapshot contradicts the recorded requirement"
                            .to_owned(),
                    ));
                }
            }
            Self::NodeProgressRecorded {
                completed_units,
                total_units: Some(total),
                ..
            } if completed_units.is_some_and(|completed| completed > *total) => {
                return Err(PersistenceError::InvalidDocument(
                    "completed progress units exceed total units".to_owned(),
                ));
            }
            Self::LateTerminalEvidenceRecorded { terminal, .. }
                if terminal.status() == TerminalStatus::Uncertain =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "late terminal evidence must add a known terminal observation".to_owned(),
                ));
            }
            Self::NodeRetryScheduled {
                attempt_number: 0, ..
            }
            | Self::RepeatIterationCreated {
                iteration_number: 0,
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "attempt and iteration numbers are one-based".to_owned(),
                ));
            }
            Self::NodeTerminal {
                outcome,
                error_class,
                ..
            }
            | Self::DeterministicNodeTerminal {
                outcome,
                error_class,
                ..
            } if matches!(outcome, NodeOutcome::Failed | NodeOutcome::Rejected)
                != error_class.is_some() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "node failure/rejection requires an error class and success/cancellation forbids one"
                        .to_owned(),
                ));
            }
            Self::AttemptUsageRecorded { usage, .. }
                if usage.input_units.is_none()
                    && usage.output_units.is_none()
                    && usage.duration_ms.is_none()
                    && usage.cost.is_none() =>
            {
                return Err(PersistenceError::InvalidDocument(
                    "an attempt usage fact must contain at least one observation".to_owned(),
                ));
            }
            Self::JoinSatisfied {
                rule: JoinRule::Quorum { required: 0 },
                ..
            } => {
                return Err(PersistenceError::InvalidDocument(
                    "join quorum must be greater than zero".to_owned(),
                ));
            }
            Self::JoinSatisfied {
                branches,
                retained_branches,
                rule,
                ..
            } => {
                check_references("event.branches", branches.len())?;
                check_references("event.retained_branches", retained_branches.len())?;
                for branch in branches {
                    check_references("event.branch.outputs", branch.outputs.len())?;
                    if branch
                        .outputs
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        != branch.outputs.len()
                    {
                        return Err(PersistenceError::InvalidDocument(
                            "branch output references must be distinct".to_owned(),
                        ));
                    }
                }
                let branch_ids: std::collections::BTreeSet<_> =
                    branches.iter().map(|branch| &branch.branch).collect();
                let retained_ids: std::collections::BTreeSet<_> =
                    retained_branches.iter().collect();
                let successful = branches
                    .iter()
                    .filter(|branch| branch.outcome == RunOutcome::Succeeded)
                    .count();
                let rule_satisfied = match rule {
                    JoinRule::All | JoinRule::AnyCompletion => !branches.is_empty(),
                    JoinRule::FirstSuccess => successful > 0,
                    JoinRule::Quorum { required } => {
                        usize::try_from(*required).is_ok_and(|required| successful >= required)
                    }
                };
                if branch_ids.len() != branches.len()
                    || retained_ids.len() != retained_branches.len()
                    || !branch_ids.is_disjoint(&retained_ids)
                    || !rule_satisfied
                {
                    return Err(PersistenceError::InvalidDocument(
                        "join results must use distinct branches and truthfully satisfy the recorded rule"
                            .to_owned(),
                    ));
                }
            }
            Self::SubworkflowTerminal { outputs, .. } | Self::BranchTerminal { outputs, .. } => {
                check_references("event.outputs", outputs.len())?;
                if outputs
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != outputs.len()
                {
                    return Err(PersistenceError::InvalidDocument(
                        "branch/subworkflow output references must be distinct".to_owned(),
                    ));
                }
            }
            Self::ReconciliationPlanRecorded { items, .. } => {
                if items.len() > MAX_RECONCILIATION_PLAN_ITEMS {
                    return Err(PersistenceError::Bounds {
                        location: "event.reconciliation.items",
                        reason: format!(
                            "at most {MAX_RECONCILIATION_PLAN_ITEMS} items are allowed so actions, application, and revision pin fit one atomic commit"
                        ),
                    });
                }
                let unique: std::collections::BTreeSet<_> = items
                    .iter()
                    .map(|item| (item.node.as_ref(), item.execution.as_ref()))
                    .collect();
                let invalid_identity = items.iter().any(|item| {
                    item.node.is_none()
                        && item.execution.is_none()
                        && item.classification
                            != ReconciliationClassification::IncompatibleInterfaceOrSubworkflow
                });
                if unique.len() != items.len() || invalid_identity {
                    return Err(PersistenceError::InvalidDocument(
                        "reconciliation items must be distinct; only workflow-interface incompatibilities may omit both node and execution"
                            .to_owned(),
                    ));
                }
            }
            _ => {}
        }
        // Validate provider-neutral artifact references even before a commit request
        // derives its exact ownership/accounting set.
        let _ = self.required_artifacts()?;
        self.validate_workspace_run(run)?;
        Ok(())
    }

    fn validate_workspace_run(&self, run: &RunId) -> Result<(), PersistenceError> {
        let value_in_run = |value: &WorkspaceValueReference| value.scope().run() == run;
        let scope_in_run = |scope: &WorkspaceScope| scope.reference().run() == run;
        let valid = match self {
            Self::RunCreated {
                root_scope, inputs, ..
            } => {
                scope_in_run(root_scope)
                    && root_scope.kind().is_run_root()
                    && inputs.iter().all(value_in_run)
            }
            Self::RunTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            Self::NodeBecameEligible { scope, .. } => scope.run() == run,
            Self::NodeOutputPublished { value, .. } => value_in_run(value),
            Self::DeterministicOutputPublished { value, .. } => value_in_run(value),
            Self::BranchScopeCreated { scope, .. } | Self::RepeatIterationCreated { scope, .. } => {
                scope_in_run(scope)
            }
            Self::BranchTerminal { outputs, .. } => outputs.iter().all(value_in_run),
            Self::JoinSatisfied { branches, .. } => branches
                .iter()
                .all(|branch| branch.scope.run() == run && branch.outputs.iter().all(value_in_run)),
            Self::SubworkflowCreated {
                child_run,
                scope,
                inputs,
                ..
            } => child_run != run && scope_in_run(scope) && inputs.iter().all(value_in_run),
            Self::SubworkflowTerminal {
                child_run, outputs, ..
            } => child_run != run && outputs.iter().all(|value| value.scope().run() == child_run),
            Self::SubworkflowOutputImported {
                child_value,
                parent_value,
                ..
            } => {
                child_value.scope().run() != run
                    && parent_value.scope().run() == run
                    && child_value.scope().run() != parent_value.scope().run()
            }
            Self::SubworkflowCancellationRequested { child_run, .. } => child_run != run,
            Self::ReconciliationRemediationCreated { scope, .. }
            | Self::RemediationWorkCreated { scope, .. } => scope.run() == run,
            _ => true,
        };
        if valid {
            Ok(())
        } else {
            Err(PersistenceError::InvalidDocument(
                "workspace scopes/value references in an event must belong to its run aggregate"
                    .to_owned(),
            ))
        }
    }
}

fn workspace_artifact_reference(
    reference: &milkdrift_capability::ArtifactReference,
) -> Result<ArtifactReference, PersistenceError> {
    let media_type = reference.media_type().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact media type".to_owned(),
        )
    })?;
    let size_bytes = reference.size_bytes().ok_or_else(|| {
        PersistenceError::InvalidDocument(
            "scheduled artifact input requires an exact byte size".to_owned(),
        )
    })?;
    let artifact = ArtifactId::new(reference.identity()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid identity: {error}"
        ))
    })?;
    let digest = ArtifactContentDigest::from_hex(reference.digest()).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid digest: {error}"
        ))
    })?;
    let media_type = MediaType::new(media_type).map_err(|error| {
        PersistenceError::InvalidDocument(format!(
            "scheduled artifact input has an invalid media type: {error}"
        ))
    })?;
    Ok(ArtifactReference::new(
        artifact, digest, media_type, size_bytes,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use milkdrift_capability::{
        ArtifactReference as CapabilityArtifactReference, CapabilityId, InputReference,
        InvocationValueReference, OperationId,
    };

    use super::*;

    fn scheduled_with_artifact(
        media_type: Option<String>,
        size_bytes: Option<u64>,
    ) -> Result<RunEventKind, Box<dyn std::error::Error>> {
        let invocation = InvocationId::new("invocation-artifact")?;
        let input = InputReference::new(
            "source",
            InvocationValueReference::Artifact {
                reference: CapabilityArtifactReference::new(
                    "artifact-source",
                    "a".repeat(64),
                    media_type,
                    size_bytes,
                )?,
            },
        )?;
        let request = InvocationRequest::new(
            invocation.clone(),
            CapabilityId::new("artifact-consumer")?,
            OperationId::new("artifact.consume")?,
            None,
            None,
            vec![input],
            BTreeMap::new(),
        )?;
        Ok(RunEventKind::NodeScheduled {
            node: NodeId::new("consume")?,
            execution: NodeExecutionId::new("execution-consume")?,
            attempt: AttemptId::new("attempt-consume")?,
            invocation,
            idempotency_key: None,
            request,
        })
    }

    #[test]
    fn scheduled_artifact_inputs_are_exact_atomic_ownership_requirements()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts =
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), Some(7))?
                .required_artifacts()?;
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact().as_str(), "artifact-source");
        assert_eq!(artifacts[0].digest().to_hex(), "a".repeat(64));
        assert_eq!(
            artifacts[0].media_type().as_str(),
            "application/octet-stream"
        );
        assert_eq!(artifacts[0].size_bytes(), 7);

        assert!(
            scheduled_with_artifact(None, Some(7))?
                .required_artifacts()
                .is_err()
        );
        assert!(
            scheduled_with_artifact(Some("application/octet-stream".to_owned()), None)?
                .required_artifacts()
                .is_err()
        );
        Ok(())
    }
}
