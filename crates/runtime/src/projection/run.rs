use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_capability::InvocationId;
use milkdrift_persistence::{
    AttemptId, AuthorityDecision, CurrencyCode, EvidenceReference, LeaseId, NodeExecutionId,
    Reason, ReconciliationDecisionId, ReconciliationPlanId, RunOutcome, RunSequence, SignalId,
    TimerId,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, BranchId, IterationId, RunId, ScopeReference,
    SubworkflowId, WorkspaceBudget, WorkspaceScope, WorkspaceValueReference,
};

use super::node::{
    LeaseProjection, NodeAttemptProjection, NodeExecutionProjection, RetryProjection,
    TimerProjection,
};
use super::reconciliation::{
    ReconciliationCancellationProjection, ReconciliationProjection,
    ReconciliationRemediationProjection, RecoveryProjection, RemediationProjection,
};
use super::structured::{
    BranchProjection, IterationProjection, JoinProjection, RepeatContinuationProjection,
    RepeatTermination, SignalProjection, SubworkflowProjection, SubworkflowUsageSummary,
    WaitProjection,
};

/// Current lifecycle derived exclusively from authoritative run facts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub enum RunLifecycle {
    /// No creation fact has been applied.
    #[default]
    Uncreated,
    /// The aggregate exists but has not started.
    Created,
    /// The run is admitting and executing work.
    Running,
    /// New admission and dispatch are paused.
    Paused,
    /// Durable cancellation intent has been recorded.
    Cancelling,
    /// The run reached a truthful terminal boundary.
    Terminal(RunOutcome),
}

impl RunLifecycle {
    /// Returns whether the run exists but has not started.
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::Created)
    }

    /// Returns whether the run is started and nonterminal.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Cancelling)
    }

    /// Returns whether the run reached a terminal outcome.
    #[must_use]
    pub const fn is_completed(self) -> bool {
        matches!(self, Self::Terminal(_))
    }

    /// Returns the terminal outcome, when present.
    #[must_use]
    pub const fn outcome(self) -> Option<RunOutcome> {
        match self {
            Self::Terminal(outcome) => Some(outcome),
            Self::Uncreated | Self::Created | Self::Running | Self::Paused | Self::Cancelling => {
                None
            }
        }
    }
}

/// One exact revision pin and the sequence at which it became effective.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RevisionPin {
    pub(super) revision: RevisionId,
    pub(super) digest: ContentDigest,
    pub(super) effective_sequence: RunSequence,
    pub(super) plan: Option<ReconciliationPlanId>,
}

impl RevisionPin {
    /// Exact immutable revision.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Semantic content digest of the revision.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    /// First event sequence governed by this pin.
    #[must_use]
    pub const fn effective_sequence(&self) -> RunSequence {
        self.effective_sequence
    }

    /// Reconciliation plan authorizing a prospective repin, if any.
    #[must_use]
    pub const fn plan(&self) -> Option<&ReconciliationPlanId> {
        self.plan.as_ref()
    }
}

/// Durable cancellation intent for the aggregate.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RunCancellation {
    pub(super) reason: Reason,
    pub(super) evidence: Vec<EvidenceReference>,
    pub(super) sequence: RunSequence,
}

/// Durable internal drain intent selected by an explicit non-cancellation terminal.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RunTerminationIntent {
    pub(super) outcome: RunOutcome,
    pub(super) reason: Reason,
    pub(super) sequence: RunSequence,
}

impl RunTerminationIntent {
    /// Outcome to record after all structured ownership becomes quiescent.
    #[must_use]
    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    /// Bounded rationale for draining already-owned work.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Sequence at which the terminal selection became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

impl RunCancellation {
    /// Recorded cancellation rationale.
    #[must_use]
    pub const fn reason(&self) -> &Reason {
        &self.reason
    }

    /// Supporting durable evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceReference] {
        &self.evidence
    }

    /// Sequence at which cancellation intent became durable.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Aggregate resource and durable workspace-budget usage visible from event facts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct ResourceUsage {
    pub(super) input_units: Option<u64>,
    pub(super) output_units: Option<u64>,
    pub(super) duration_ms: Option<u64>,
    #[serde(with = "super::serde_map")]
    pub(super) cost_micros: BTreeMap<CurrencyCode, u64>,
    pub(super) workspace_value_references: u64,
    pub(super) artifacts: u64,
    pub(super) artifact_bytes: u64,
}

impl ResourceUsage {
    /// Sum of observed provider-defined input units.
    #[must_use]
    pub const fn input_units(&self) -> Option<u64> {
        self.input_units
    }

    /// Sum of observed provider-defined output units.
    #[must_use]
    pub const fn output_units(&self) -> Option<u64> {
        self.output_units
    }

    /// Sum of observed executor durations.
    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    /// Exact observed cost totals grouped by currency.
    #[must_use]
    pub const fn cost_micros(&self) -> &BTreeMap<CurrencyCode, u64> {
        &self.cost_micros
    }

    /// Number of distinct workspace value references carried by history.
    #[must_use]
    pub const fn workspace_value_references(&self) -> u64 {
        self.workspace_value_references
    }

    /// Number of uniquely published artifact metadata records.
    #[must_use]
    pub const fn artifacts(&self) -> u64 {
        self.artifacts
    }

    /// Sum of exact bytes across uniquely published artifacts.
    #[must_use]
    pub const fn artifact_bytes(&self) -> u64 {
        self.artifact_bytes
    }
}

/// Truthful terminal output summary with references into published provenance.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct RunTerminalProjection {
    pub(super) outcome: RunOutcome,
    pub(super) outputs: Vec<WorkspaceValueReference>,
    pub(super) artifacts: Vec<ArtifactReference>,
    pub(super) reason: Option<Reason>,
    pub(super) sequence: RunSequence,
}

impl RunTerminalProjection {
    /// Semantic terminal outcome.
    #[must_use]
    pub const fn outcome(&self) -> RunOutcome {
        self.outcome
    }

    /// Exact terminal workspace values.
    #[must_use]
    pub fn outputs(&self) -> &[WorkspaceValueReference] {
        &self.outputs
    }

    /// Exact terminal content-addressed artifacts.
    #[must_use]
    pub fn artifacts(&self) -> &[ArtifactReference] {
        &self.artifacts
    }

    /// Bounded terminal rationale, when relevant.
    #[must_use]
    pub const fn reason(&self) -> Option<&Reason> {
        self.reason.as_ref()
    }

    /// Terminal event sequence.
    #[must_use]
    pub const fn sequence(&self) -> RunSequence {
        self.sequence
    }
}

/// Pure read model obtained by replaying one run's ordered authoritative facts.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct RunProjection {
    pub(super) sequence: RunSequence,
    /// Last authoritative sequence whose high-volume historical detail was compacted.
    ///
    /// The journal remains authoritative for detail at or before this sequence while the
    /// projection retains only state required for future transitions. A non-empty live
    /// projection is compacted through its current sequence after every durable transition.
    pub(super) history_compacted_through: RunSequence,
    pub(super) run_id: Option<RunId>,
    pub(super) lifecycle: RunLifecycle,
    pub(super) workflow: Option<WorkflowId>,
    pub(super) revision: Option<RevisionId>,
    pub(super) revision_digest: Option<ContentDigest>,
    pub(super) pins: Vec<RevisionPin>,
    pub(super) root_scope: Option<WorkspaceScope>,
    pub(super) workspace_budget: Option<WorkspaceBudget>,
    pub(super) inputs: Vec<WorkspaceValueReference>,
    #[serde(with = "super::serde_map")]
    pub(super) scopes: BTreeMap<ScopeReference, WorkspaceScope>,
    pub(super) workspace_values: BTreeSet<WorkspaceValueReference>,
    pub(super) cancellation: Option<RunCancellation>,
    pub(super) termination: Option<RunTerminationIntent>,
    #[serde(with = "super::serde_map")]
    pub(super) node_executions: BTreeMap<NodeExecutionId, NodeExecutionProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) execution_ids_by_node: BTreeMap<NodeId, BTreeSet<NodeExecutionId>>,
    #[serde(with = "super::serde_map")]
    pub(super) latest_descendant_execution_by_scope_node:
        BTreeMap<(ScopeReference, NodeId), NodeExecutionId>,
    pub(super) active_execution_ids: BTreeSet<NodeExecutionId>,
    pub(super) eligible_executions: BTreeSet<NodeExecutionId>,
    pub(super) pending_successor_executions: BTreeSet<NodeExecutionId>,
    pub(super) reserved_executions: BTreeSet<NodeExecutionId>,
    #[serde(with = "super::serde_map")]
    pub(super) attempts: BTreeMap<AttemptId, NodeAttemptProjection>,
    pub(super) active_attempt_ids: BTreeSet<AttemptId>,
    pub(super) invocations: BTreeSet<InvocationId>,
    #[serde(with = "super::serde_map")]
    pub(super) leases: BTreeMap<LeaseId, LeaseProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) active_lease_by_attempt: BTreeMap<AttemptId, LeaseId>,
    #[serde(with = "super::serde_map")]
    pub(super) timers: BTreeMap<TimerId, TimerProjection>,
    pub(super) pending_timer_ids: BTreeSet<TimerId>,
    #[serde(with = "super::serde_map")]
    pub(super) pending_timers_by_execution: BTreeMap<NodeExecutionId, BTreeSet<TimerId>>,
    #[serde(with = "super::serde_map")]
    pub(super) retries: BTreeMap<TimerId, RetryProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) retry_by_attempt: BTreeMap<AttemptId, TimerId>,
    #[serde(with = "super::serde_map")]
    pub(super) branches: BTreeMap<BranchId, BranchProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) branch_by_fork_port: BTreeMap<(NodeExecutionId, PortId), BranchId>,
    #[serde(with = "super::serde_map")]
    pub(super) branch_ids_by_fork_execution: BTreeMap<NodeExecutionId, BTreeSet<BranchId>>,
    pub(super) active_branch_ids: BTreeSet<BranchId>,
    pub(super) cancelling_branch_ids: BTreeSet<BranchId>,
    #[serde(with = "super::serde_map")]
    pub(super) active_scope_ownership: BTreeMap<ScopeReference, u64>,
    #[serde(with = "super::serde_map")]
    pub(super) active_structured_children_by_execution: BTreeMap<NodeExecutionId, u32>,
    #[serde(with = "super::serde_map")]
    pub(super) branch_owner: BTreeMap<NodeExecutionId, BranchId>,
    #[serde(with = "super::serde_map")]
    pub(super) branch_routes: BTreeMap<NodeExecutionId, PortId>,
    #[serde(with = "super::serde_map")]
    pub(super) joins: BTreeMap<NodeExecutionId, JoinProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) iterations: BTreeMap<IterationId, IterationProjection>,
    pub(super) active_iteration_ids: BTreeSet<IterationId>,
    #[serde(with = "super::serde_map")]
    pub(super) latest_iteration: BTreeMap<NodeExecutionId, IterationId>,
    #[serde(with = "super::serde_map")]
    pub(super) repeat_continuations: BTreeMap<NodeExecutionId, RepeatContinuationProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) repeat_terminations: BTreeMap<NodeExecutionId, RepeatTermination>,
    #[serde(with = "super::serde_map")]
    pub(super) signals: BTreeMap<SignalId, SignalProjection>,
    pub(super) pending_broadcast_signals: BTreeSet<(RunSequence, SignalId)>,
    #[serde(with = "super::serde_map")]
    pub(super) waits: BTreeMap<NodeExecutionId, WaitProjection>,
    pub(super) pending_wait_execution_ids: BTreeSet<NodeExecutionId>,
    #[serde(with = "super::serde_map")]
    pub(super) subworkflows: BTreeMap<SubworkflowId, SubworkflowProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) subworkflow_usage_by_execution: BTreeMap<NodeExecutionId, SubworkflowUsageSummary>,
    pub(super) active_subworkflow_ids: BTreeSet<SubworkflowId>,
    pub(super) active_attached_subworkflow_ids: BTreeSet<SubworkflowId>,
    pub(super) child_runs: BTreeSet<RunId>,
    #[serde(with = "super::serde_map")]
    pub(super) artifacts: BTreeMap<ArtifactId, ArtifactMetadata>,
    pub(super) reconciliation: ReconciliationProjection,
    pub(super) pending_pin: Option<ReconciliationPlanId>,
    #[serde(with = "super::serde_map")]
    pub(super) reconciliation_cancellations:
        BTreeMap<NodeExecutionId, ReconciliationCancellationProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) pending_reconciliation_restarts: BTreeMap<(NodeId, ScopeReference), NodeExecutionId>,
    #[serde(with = "super::serde_map")]
    pub(super) reconciliation_remediations:
        BTreeMap<NodeExecutionId, ReconciliationRemediationProjection>,
    #[serde(with = "super::serde_map")]
    pub(super) recovery_decisions:
        BTreeMap<ReconciliationDecisionId, (AttemptId, AuthorityDecision)>,
    pub(super) recovery: Vec<RecoveryProjection>,
    pub(super) current_recovery: Option<usize>,
    #[serde(with = "super::serde_map")]
    pub(super) remediations: BTreeMap<NodeExecutionId, RemediationProjection>,
    pub(super) resource_usage: ResourceUsage,
    pub(super) terminal: Option<RunTerminalProjection>,
}
