use std::collections::BTreeSet;

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, ContextBudget, ContextCategory, ContextOrdering,
    ContextSessionPolicy, ContextTruncation, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node,
    NodeId, NodeKind, PortId, TaskContextPolicy, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CapabilityRequirement, InvocationValueReference, OperationId,
};
use milkdrift_model::{AuthorityFact, ContextProducerFact, ContextSemanticKind, ContextSource};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CausalContextBuilder, ContextBuildIdentity, ContextBuildRequest, ContextCandidate,
    ContextCandidateAvailability, materialize_selected_context,
};
use milkdrift_workspace::{ArtifactSensitivity, ContentDigest, RunId};

use crate::{EvidenceResult, ScenarioMeasurement};

const DISCOVERY_CANDIDATES: u32 = 2_048;
const MATERIALIZED_CANDIDATES: u32 = 64;

/// Builds and deterministically selects a bounded manifest from synthetic candidate metadata.
pub fn context_discovery_and_selection() -> EvidenceResult<ScenarioMeasurement> {
    let policy = policy(128)?;
    let revision = revision(policy.clone())?;
    let mut candidates = Vec::with_capacity(usize::try_from(DISCOVERY_CANDIDATES)?);
    for index in 0..DISCOVERY_CANDIDATES {
        let name = format!("prompt-{index:05}");
        let value = BoundedJson::new(serde_json::json!({"name": name, "index": index}))?;
        let bytes = serde_json::to_vec(value.value())?;
        candidates.push(candidate(
            &revision,
            ContextSemanticKind::DirectInput,
            ContextSource::DirectInput {
                name,
                reference: InvocationValueReference::Inline { value },
            },
            None,
            &bytes,
            index,
        )?);
    }
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::new(),
        candidates,
    })?;
    if manifest.entries().len() != 128
        || manifest
            .entries()
            .len()
            .saturating_add(manifest.omissions().len())
            != usize::try_from(DISCOVERY_CANDIDATES)?
    {
        return Err(std::io::Error::other("context selection accounting changed").into());
    }
    let encoded = serde_json::to_vec(&manifest)?;
    Ok(ScenarioMeasurement::new(
        "context/synthetic_candidate_selection_2048_to_128",
        u64::from(DISCOVERY_CANDIDATES),
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

/// Materializes only the exact selected node-execution sources through the runtime boundary.
pub fn context_materialization() -> EvidenceResult<ScenarioMeasurement> {
    let policy = policy(MATERIALIZED_CANDIDATES)?;
    let revision = revision(policy.clone())?;
    let source_node = NodeId::new("source")?;
    let mut candidates = Vec::with_capacity(usize::try_from(MATERIALIZED_CANDIDATES)?);
    for index in 0..MATERIALIZED_CANDIDATES {
        let execution = NodeExecutionId::new(format!("execution-{index:04}"))?;
        let attempt = AttemptId::new(format!("attempt-{index:04}"))?;
        let source = ContextSource::NodeExecution {
            node: source_node.clone(),
            execution: execution.clone(),
            attempt: Some(attempt.clone()),
            event_sequence: None,
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "node": source_node,
            "execution": execution,
            "attempt": attempt,
            "event_sequence": null,
        }))?;
        candidates.push(candidate(
            &revision,
            ContextSemanticKind::SuccessfulOutput,
            source,
            Some(NodeId::new("source")?),
            &bytes,
            index,
        )?);
    }
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::new(),
        candidates,
    })?;
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let materialized = materialize_selected_context(&store, &manifest)?;
    if materialized.len() != usize::try_from(MATERIALIZED_CANDIDATES)? {
        return Err(std::io::Error::other("selected-only materialization count changed").into());
    }
    let encoded = serde_json::to_vec(&materialized)?;
    Ok(ScenarioMeasurement::new(
        "context/materialize_selected_64",
        u64::from(MATERIALIZED_CANDIDATES),
        u64::try_from(encoded.len())?,
        &encoded,
    ))
}

fn policy(max_items: u32) -> EvidenceResult<TaskContextPolicy> {
    Ok(TaskContextPolicy::new(
        true,
        Some(2),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([
            ContextCategory::DirectInput,
            ContextCategory::SuccessfulOutput,
        ]),
        BTreeSet::new(),
        None,
        ContextBudget::new(max_items, 8_388_608, 8_388_608, Some(8_388_608))?
            .with_discovery_limits(4_096, 32, 1_048_576, 128, 2_097_152)?,
        ContextOrdering::CausalKindSource,
        ContextTruncation::OmitOversized,
        ContextSessionPolicy::Fresh,
        true,
    )?)
}

fn revision(policy: TaskContextPolicy) -> EvidenceResult<BlueprintRevision> {
    let source = NodeId::new("source")?;
    let current = NodeId::new("current")?;
    let done = NodeId::new("done")?;
    let output = PortId::new("out")?;
    let input = PortId::new("in")?;
    let source_node = Node::new(
        source.clone(),
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new("tool.source")?))?,
    )?
    .with_control_output(output.clone())?;
    let current_node = Node::new(
        current.clone(),
        NodeKind::task(
            CapabilityRequirement::new(OperationId::new("model.generate")?),
            policy,
        )?,
    )?
    .with_control_input(input.clone())?
    .with_control_output(output.clone())?;
    let terminal = Node::new(
        done.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(input.clone())?;
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("evidence-context")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: source_node },
            Mutation::AddNode { node: current_node },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("source-current")?,
                    EdgeKind::Control,
                    source,
                    output.clone(),
                    current.clone(),
                    input.clone(),
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("current-done")?,
                    EdgeKind::Control,
                    current,
                    output,
                    done,
                    input,
                ),
            },
        ])?,
        AuthorRef::new("system:evidence")?,
        "operational evidence context graph",
    )?)
}

fn identity(revision: &BlueprintRevision) -> EvidenceResult<ContextBuildIdentity> {
    Ok(ContextBuildIdentity {
        run: RunId::new("run-evidence-context")?,
        revision: revision.id().clone(),
        node: NodeId::new("current")?,
        execution: NodeExecutionId::new("execution-current")?,
        attempt: AttemptId::new("attempt-current")?,
    })
}

fn candidate(
    revision: &BlueprintRevision,
    kind: ContextSemanticKind,
    source: ContextSource,
    node: Option<NodeId>,
    bytes: &[u8],
    index: u32,
) -> EvidenceResult<ContextCandidate> {
    let (execution, attempt) = match &source {
        ContextSource::NodeExecution {
            execution, attempt, ..
        } => (Some(execution.clone()), attempt.clone()),
        _ => (
            Some(NodeExecutionId::new(format!(
                "candidate-execution-{index:05}"
            ))?),
            Some(AttemptId::new(format!("candidate-attempt-{index:05}"))?),
        ),
    };
    Ok(ContextCandidate {
        kind,
        source: Some(source),
        content_digest: ContentDigest::for_bytes(bytes),
        source_revision: revision.id().clone(),
        execution,
        attempt,
        source_sequence: None,
        occurred_at_ms: Some(u64::from(index)),
        causal_distance: None,
        producer: ContextProducerFact::default(),
        node,
        roles: BTreeSet::new(),
        scope: None,
        exposed_across_scope: false,
        required: false,
        availability: ContextCandidateAvailability::Available,
        selected_bytes: u64::try_from(bytes.len())?,
        selected_artifact_bytes: 0,
        estimated_model_input_units: Some(u64::try_from(bytes.len())?),
        sensitivity: ArtifactSensitivity::Public,
        authority: AuthorityFact {
            required: false,
            authorized: true,
            authority_reference: None,
        },
        artifact: None,
        causal_parents: Vec::new(),
    })
}
