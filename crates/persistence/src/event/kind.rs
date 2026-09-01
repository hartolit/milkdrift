use std::collections::BTreeMap;

use milkdrift_authority::{ActorRef, AuthorityDecisionSnapshot, ExecutionAuthorityBasis};
use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CapabilityRequirement, ErrorClass,
    IdempotencyBehavior, IdempotencyKey, InvocationId, InvocationRequest, InvocationTerminal,
    ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_workspace::{
    ArtifactMetadata, ArtifactReference, BranchId, IterationId, RunId, ScopeReference,
    SubworkflowId, WorkspaceBudget, WorkspaceScope, WorkspaceValueReference,
};
use serde::{Deserialize, Serialize};

use crate::{
    AttemptId, BoundedDetail, CommandId, CorrelationKey, CurrencyCode, EvidenceReference, LeaseId,
    NodeExecutionId, Reason, ReconciliationDecisionId, ReconciliationId, ReconciliationPlanId,
    RepeatDecisionId, RunSequence, SignalId, SignalTypeId, TimerId, TimestampMillis, WorkerId,
};

use super::model::{
    AttemptUsage, AuthorityDecision, BranchResultReference, ControllerAssessmentBoundary,
    ControllerAssessmentOutcome, JoinRule, NodeExecutionMode, NodeOutcome, ReconciliationItem,
    ReconciliationPolicy, RecoveryClassification, RepeatContinuationCause,
    RepeatContinuationDecision, RepeatTerminationReason, RunOutcome, SignalDeliveryMode,
    SubworkflowOwnership, SubworkflowResourceUsage, WaitCondition, WaitSatisfaction,
};

/// Closed versioned run facts. Variants describe observations, never requested actions.
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
    /// Exact immutable authority inherited by every capability-backed child of this run.
    ExecutionAuthorityEstablished {
        /// Minimal frozen actor/grant/policy/run basis.
        basis: ExecutionAuthorityBasis,
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
    /// Exact canonical decision allowing one generation to be selected.
    CapabilityResolutionDecisionRecorded {
        /// Owning execution.
        execution: NodeExecutionId,
        /// Owning immutable attempt.
        attempt: AttemptId,
        /// Exact generation bound by the decision.
        snapshot: ResolvedCapabilitySnapshot,
        /// Fresh canonical authorization decision.
        authorization: AuthorityDecisionSnapshot,
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
    /// Canonical exact-candidate decision made when a leased effect is claimed.
    CapabilityEntryDecisionRecorded {
        /// Immutable attempt at the entry boundary.
        attempt: AttemptId,
        /// Fresh allow or deny decision under claim-time revocation/validity state.
        authorization: AuthorityDecisionSnapshot,
    },
    /// Final canonical decision committed immediately before adapter code is called.
    CapabilityAdapterEntryDecisionRecorded {
        /// Immutable running attempt at the adapter boundary.
        attempt: AttemptId,
        /// Fresh allow or deny decision under current revocation/validity state.
        authorization: AuthorityDecisionSnapshot,
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
    /// Exact candidate authority denied before an attempt could be scheduled.
    CapabilityResolutionDenied {
        /// Logical execution that remains attempt-free.
        execution: NodeExecutionId,
        /// Exact canonical denied candidate decision.
        authorization: AuthorityDecisionSnapshot,
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
    /// The canonical controller lifecycle assessed one autonomous boundary.
    ControllerAssessmentRecorded {
        /// Stable controller identity from the immutable policy document.
        controller_id: String,
        /// Digest of every executable controller-policy field.
        policy_digest: String,
        /// Immutable revision governing this controller execution.
        governing_revision: RevisionId,
        /// Exact repeat node owned by the policy.
        controller_node: NodeId,
        /// Exact logical repeat occurrence.
        controller_execution: NodeExecutionId,
        /// Stable identity derived from controller, execution, boundary, and cycle facts.
        assessment_id: String,
        /// Stable cycle identity when a cycle is being considered.
        cycle_id: Option<String>,
        /// Boundary assessed by production code.
        boundary: ControllerAssessmentBoundary,
        /// Last authoritative parent sequence included in the assessment input.
        through_sequence: RunSequence,
        /// Exact typed progress snapshot encoded by the controller-policy owner.
        progress: BoundedJson,
        /// Closed decision consumed by deterministic runtime control flow.
        outcome: ControllerAssessmentOutcome,
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
        /// Child cost totals folded into the parent's bounded repeat budget summary.
        #[serde(default)]
        cost_micros: BTreeMap<CurrencyCode, u64>,
        /// Complete controller-relevant child usage; legacy cost remains above for v1 readers.
        #[serde(default, skip_serializing_if = "SubworkflowResourceUsage::is_empty")]
        usage: SubworkflowResourceUsage,
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
        /// Actor that produced the immutable prospective revision request.
        ///
        /// Absent only in histories written before controller attribution existed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        requested_by: Option<ActorRef>,
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
        /// Decision idempotency identity within the immutable plan.
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
        /// Decision idempotency identity within the retained attempt.
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
