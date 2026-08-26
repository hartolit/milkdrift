use milkdrift_blueprint::{AuthorRef, ContentDigest, NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    ArtifactReference, InvocationId, ResolvedCapabilitySnapshot, SideEffectClass,
};
use milkdrift_persistence::{
    AttemptId, NodeExecutionId, ReconciliationPlanId, RunEventEnvelope, RunSequence,
};
use milkdrift_runtime::{
    AttemptState, AttemptTerminal, ExternalOutcomeObligation, LateTerminalEvidence,
    NodeExecutionState, PublishedNodeOutput, ReconciliationRequestState, RunLifecycle,
    SideEffectClassification,
};
use milkdrift_workspace::{RunId, WorkspaceBudget};
use serde::Serialize;

use crate::{PolicyClassification, ProposalDigest, ProposalId};

/// Bounded operational detail for one current node occurrence.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeExecutionRead {
    /// Exact logical occurrence.
    pub execution: NodeExecutionId,
    /// Stable semantic node.
    pub node: NodeId,
    /// Immutable revision governing the occurrence.
    pub revision: RevisionId,
    /// Current runtime-derived state.
    pub state: NodeExecutionState,
    /// Total admitted attempts, including compacted history.
    pub attempt_count: u32,
    /// Latest attempt identity retained as a stable journal-provenance anchor.
    pub latest_attempt_id: Option<AttemptId>,
    /// Current detailed attempt facts when they remain operationally live.
    pub latest_attempt: Option<AttemptInspection>,
    /// Most conservative current side-effect classification.
    pub side_effect: Option<SideEffectClass>,
    /// Explicit current output and artifact references.
    pub outputs: Vec<PublishedNodeOutput>,
}

/// Bounded current detail for the latest operational attempt.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptInspection {
    /// Exact immutable attempt identity.
    pub attempt: AttemptId,
    /// Executor-facing invocation identity after scheduling.
    pub invocation: Option<InvocationId>,
    /// Current attempt state, including terminal or uncertain truth.
    pub state: AttemptState,
    /// Exact capability, descriptor revision, provider profile, operation, and contract.
    pub capability: Option<ResolvedCapabilitySnapshot>,
    /// Exact immutable context-manifest artifact bound before dispatch.
    pub context_manifest: Option<ArtifactReference>,
    /// Frozen side-effect and external-idempotency facts.
    pub side_effect: Option<SideEffectClassification>,
    /// Attempt-owned output and artifact publications.
    pub outputs: Vec<PublishedNodeOutput>,
    /// Known terminal outcome and classified failure, when observed.
    pub terminal: Option<AttemptTerminal>,
    /// Truthful terminal evidence received after active worker ownership ended.
    pub late_terminal_evidence: Option<LateTerminalEvidence>,
    /// Durable reason and evidence when external truth remains unresolved or retained.
    pub external_outcome: Option<ExternalOutcomeObligation>,
}

/// Authorization-filtered current run inspection model.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunInspection {
    /// Exact aggregate.
    pub run: RunId,
    /// Current authoritative sequence.
    pub sequence: RunSequence,
    /// Current lifecycle.
    pub lifecycle: RunLifecycle,
    /// Workflow lineage when the run exists.
    pub workflow: Option<WorkflowId>,
    /// Current exact revision pin.
    pub revision: Option<RevisionId>,
    /// Current semantic revision digest.
    pub revision_digest: Option<ContentDigest>,
    /// Immutable run workspace and artifact ceilings.
    pub workspace_budget: Option<WorkspaceBudget>,
    /// Current bounded execution frontier.
    pub executions: Vec<NodeExecutionRead>,
    /// Current/latest reconciliation state.
    pub reconciliation: Option<ReconciliationStatusRead>,
    /// Observed input usage.
    pub input_units: Option<u64>,
    /// Observed output usage.
    pub output_units: Option<u64>,
    /// Observed duration.
    pub duration_ms: Option<u64>,
    /// Observed artifact bytes.
    pub artifact_bytes: u64,
}

/// Bounded immutable revision lineage and semantic-size inspection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionInspection {
    /// Exact revision.
    pub revision: RevisionId,
    /// Workflow lineage.
    pub workflow: WorkflowId,
    /// User-facing lineage sequence.
    pub lineage_sequence: u64,
    /// Semantic digest.
    pub content_digest: ContentDigest,
    /// Exact immutable parents.
    pub parents: Vec<RevisionId>,
    /// Bounded provenance author.
    pub author: AuthorRef,
    /// Bounded revision rationale/proposal link.
    pub reason: String,
    /// Semantic node count.
    pub node_count: usize,
    /// Semantic edge count.
    pub edge_count: usize,
}

/// Current state of one proposal-backed revision adoption.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciliationStatusRead {
    /// Proposed target revision.
    pub revision: RevisionId,
    /// Immutable plan when planning completed.
    pub plan: Option<ReconciliationPlanId>,
    /// Current request state.
    pub state: ReconciliationRequestState,
    /// Whether a recorded approval exists.
    pub approved: bool,
    /// Sequence that applied the plan, when any.
    pub applied_sequence: Option<RunSequence>,
    /// Sequence that made the plan stale, when any.
    pub stale_sequence: Option<RunSequence>,
}

/// Result of creating one exact immutable proposed revision.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalSubmission {
    /// Stable proposal identity.
    pub proposal: ProposalId,
    /// Verified canonical proposal digest.
    pub proposal_digest: ProposalDigest,
    /// Exact immutable prospective revision.
    pub proposed_revision: RevisionId,
    /// Deterministic policy evidence.
    pub classification: PolicyClassification,
    /// Live reconciliation state when a run was targeted.
    pub reconciliation: Option<ReconciliationStatusRead>,
    /// Whether this call completed a prospective pin.
    pub applied: bool,
}

/// Query result for a proposal and its exact prospective revision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalStatusRead {
    /// Stable proposal identity supplied by the caller.
    pub proposal: ProposalId,
    /// Exact proposed revision.
    pub proposed_revision: RevisionId,
    /// Live reconciliation state.
    pub reconciliation: ReconciliationStatusRead,
}

/// One bounded stable-cursor page of complete append-only run history.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimelinePage {
    /// Strictly contiguous event envelopes.
    pub events: Vec<RunEventEnvelope>,
    /// Inclusive sequence for the next page.
    pub next_sequence: Option<RunSequence>,
    /// Journal head observed by this page.
    pub observed_head: RunSequence,
}
