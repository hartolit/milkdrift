use std::error::Error;

use milkdrift_blueprint::{ContentDigest, NodeId, PortId, RevisionId, WorkflowId};
use milkdrift_capability::{
    AdmissionConstraints, BoundedJson, CapabilityCategory, CapabilityId, CapabilityRequirement,
    DescriptorBuilder, ErrorClass, IdempotencyBehavior, IdempotencyKey, InvocationId,
    InvocationRequest, Locality, OperationContract, OperationId, ProviderProfileRef,
    ResolvedCapabilitySnapshot, ResolvedCapabilitySnapshotDocument, SideEffectClass,
};
use milkdrift_persistence::{
    ActorRef, AttemptId, AuthorityDecision, BoundedDetail, BranchResultReference, CommandId,
    CurrencyCode, EventId, JoinRule, LeaseId, MAX_REPEAT_CONTINUATION_CYCLES, NodeExecutionId,
    NodeExecutionMode, NodeOutcome, Reason, ReconciliationAction, ReconciliationClassification,
    ReconciliationDecisionId, ReconciliationId, ReconciliationItem, ReconciliationPlanId,
    ReconciliationPolicy, RecoveryClassification, RepeatContinuationCause,
    RepeatContinuationDecision, RepeatDecisionId, RepeatTerminationReason, RunEventEnvelope,
    RunEventKind, RunOutcome, RunSequence, SignalDeliveryMode, SignalId, SignalTypeId,
    SubworkflowOwnership, TimerId, TimestampMillis, WaitCondition, WaitSatisfaction, WorkerId,
};
use milkdrift_workspace::{
    BranchId, IterationId, RunId, ScopeId, ScopeReference, SubworkflowId, WorkspaceBudget,
    WorkspaceScope, WorkspaceValueReference,
};

use crate::RuntimeError;

use super::{
    AttemptState, NodeAttemptProjection, NodeExecutionCancellationProjection,
    NodeExecutionProjection, NodeExecutionState, RepeatContinuationRequestProjection, RunLifecycle,
    RunProjection, SubworkflowProjection, WaitProjection,
};

type TestResult = Result<(), Box<dyn Error>>;

struct Fixture {
    run: RunId,
    root: WorkspaceScope,
    workflow: WorkflowId,
    revision: RevisionId,
    digest: ContentDigest,
    budget: WorkspaceBudget,
}

fn fixture(name: &str) -> Result<Fixture, Box<dyn Error>> {
    let run = RunId::new(format!("run-{name}"))?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    Ok(Fixture {
        run,
        root,
        workflow: WorkflowId::new(format!("workflow-{name}"))?,
        revision: revision('a')?,
        digest: digest('1')?,
        budget: WorkspaceBudget::new(100, 10_000, 100_000, 100, 100_000, 1_000_000)?,
    })
}

fn revision(character: char) -> Result<RevisionId, Box<dyn Error>> {
    Ok(serde_json::from_str(&format!(
        "\"rev_{}\"",
        character.to_string().repeat(64)
    ))?)
}

fn digest(character: char) -> Result<ContentDigest, Box<dyn Error>> {
    Ok(serde_json::from_str(&format!(
        "\"b3_{}\"",
        character.to_string().repeat(64)
    ))?)
}

fn envelope(
    sequence: u64,
    run: &RunId,
    kind: RunEventKind,
) -> Result<RunEventEnvelope, milkdrift_persistence::PersistenceError> {
    RunEventEnvelope::new(
        EventId::new(format!("event-{sequence}"))?,
        run.clone(),
        RunSequence::new(sequence),
        TimestampMillis::new(sequence.saturating_mul(100)),
        kind,
    )
}

fn created(
    fixture: &Fixture,
    sequence: u64,
) -> Result<RunEventEnvelope, milkdrift_persistence::PersistenceError> {
    envelope(
        sequence,
        &fixture.run,
        RunEventKind::RunCreated {
            workflow: fixture.workflow.clone(),
            revision: fixture.revision.clone(),
            revision_digest: fixture.digest.clone(),
            root_scope: fixture.root.clone(),
            workspace_budget: fixture.budget.clone(),
            inputs: Vec::new(),
        },
    )
}

fn eligible(
    sequence: u64,
    fixture: &Fixture,
    node: &str,
    execution: &NodeExecutionId,
    scope: &ScopeReference,
) -> Result<RunEventEnvelope, Box<dyn Error>> {
    Ok(envelope(
        sequence,
        &fixture.run,
        RunEventKind::NodeBecameEligible {
            node: NodeId::new(node)?,
            execution: execution.clone(),
            scope: scope.clone(),
            mode: NodeExecutionMode::Executor,
        },
    )?)
}

fn runtime_eligible(
    sequence: u64,
    fixture: &Fixture,
    node: &str,
    execution: &NodeExecutionId,
    scope: &ScopeReference,
) -> Result<RunEventEnvelope, Box<dyn Error>> {
    Ok(envelope(
        sequence,
        &fixture.run,
        RunEventKind::NodeBecameEligible {
            node: NodeId::new(node)?,
            execution: execution.clone(),
            scope: scope.clone(),
            mode: NodeExecutionMode::Runtime,
        },
    )?)
}

fn invocation_request(
    invocation: &InvocationId,
    idempotency_key: Option<IdempotencyKey>,
) -> Result<InvocationRequest, Box<dyn Error>> {
    Ok(InvocationRequest::new(
        invocation.clone(),
        CapabilityId::new("publisher-primary")?,
        OperationId::new("tool.publish")?,
        Some(ProviderProfileRef::new("publisher-prod")?),
        idempotency_key,
        Vec::new(),
        std::collections::BTreeMap::new(),
    )?)
}

fn resolved_snapshot_at(
    descriptor_revision: u64,
) -> Result<ResolvedCapabilitySnapshot, Box<dyn Error>> {
    let base = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
    ))?;
    let operation = base.body().operation().clone();
    let descriptor = DescriptorBuilder::new(
        base.body().capability().clone(),
        descriptor_revision,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(1, 1)?,
        Locality::Remote,
    )
    .provider_profile(base.body().provider_profile().cloned())
    .operations(std::collections::BTreeMap::from([(
        operation.clone(),
        base.body().operation_contract().clone(),
    )]))
    .build()?;
    Ok(ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &operation,
    )?)
}

fn resolved_snapshot_with_side_effect(
    descriptor_revision: u64,
    side_effect: SideEffectClass,
    idempotency: IdempotencyBehavior,
) -> Result<ResolvedCapabilitySnapshot, Box<dyn Error>> {
    let base = ResolvedCapabilitySnapshotDocument::from_json(include_bytes!(
        "../../../capability/tests/fixtures/resolved-capability-snapshot-v1.json"
    ))?;
    let operation = base.body().operation().clone();
    let contract = base.body().operation_contract();
    let operation_contract = OperationContract::new(
        contract.input().clone(),
        contract.output().clone(),
        contract.streaming().clone(),
        contract.cancellation(),
        idempotency,
        side_effect,
        contract.features().clone(),
    )?;
    let descriptor = DescriptorBuilder::new(
        base.body().capability().clone(),
        descriptor_revision,
        CapabilityCategory::Tool,
        AdmissionConstraints::new(1, 1)?,
        Locality::Remote,
    )
    .provider_profile(base.body().provider_profile().cloned())
    .operations(std::collections::BTreeMap::from([(
        operation.clone(),
        operation_contract,
    )]))
    .build()?;
    Ok(ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &operation,
    )?)
}

mod core;
mod recovery;
mod structured;
