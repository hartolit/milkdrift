//! Deterministic causal context selection tests.

use std::collections::BTreeSet;

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, ContextArtifactRetention, ContextArtifactSelector,
    ContextArtifactSensitivity, ContextBudget, ContextCategory, ContextOrdering,
    ContextProvenanceClass, ContextSemanticRole, ContextSessionPolicy, ContextTruncation, Edge,
    EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind, PortId, TaskContextPolicy,
    TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    BoundedJson, CapabilityRequirement, InvocationValueReference, OperationId,
};
use milkdrift_model::{
    AuthorityFact, ContextInclusionReason, ContextOmissionReason, ContextProducerFact,
    ContextSemanticKind, ContextSource,
};
use milkdrift_persistence::{ArtifactStore, AttemptId, NodeExecutionId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CausalContextBuilder, ContextBuildIdentity, ContextBuildRequest, ContextCandidate,
    ContextCandidateAvailability, persist_context_manifest,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactSensitivity, ContentDigest, RunId, ScopeId, ScopeReference,
    WorkspaceBudget, WorkspaceScope, WorkspaceUsage,
};
use serde_json::json;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn policy(budget: ContextBudget) -> TestResult<TaskContextPolicy> {
    Ok(TaskContextPolicy::new(
        true,
        Some(2),
        BTreeSet::new(),
        BTreeSet::new(),
        BTreeSet::from([
            ContextCategory::DirectInput,
            ContextCategory::SuccessfulOutput,
        ]),
        BTreeSet::from([ContextCategory::RawProgress]),
        None,
        budget,
        ContextOrdering::CausalKindSource,
        ContextTruncation::OmitOversized,
        ContextSessionPolicy::Fresh,
        true,
    )?)
}

fn revision(policy: TaskContextPolicy) -> TestResult<BlueprintRevision> {
    let source = NodeId::new("source")?;
    let current = NodeId::new("current")?;
    let done = NodeId::new("done")?;
    let out = PortId::new("out")?;
    let input = PortId::new("in")?;
    let source_node = Node::new(
        source.clone(),
        NodeKind::task_direct_inputs(CapabilityRequirement::new(OperationId::new("tool.source")?))?,
    )?
    .with_control_output(out.clone())?;
    let current_node = Node::new(
        current.clone(),
        NodeKind::task(
            CapabilityRequirement::new(OperationId::new("model.generate")?),
            policy,
        )?,
    )?
    .with_control_input(input.clone())?
    .with_control_output(out.clone())?;
    let terminal = Node::new(
        done.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(input.clone())?;
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("context-test")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: source_node },
            Mutation::AddNode { node: current_node },
            Mutation::AddNode { node: terminal },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("source-current")?,
                    EdgeKind::Control,
                    source,
                    out.clone(),
                    current.clone(),
                    input.clone(),
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("current-done")?,
                    EdgeKind::Control,
                    current,
                    out,
                    done,
                    input,
                ),
            },
        ])?,
        AuthorRef::new("human:test")?,
        "context test graph",
    )?)
}

fn identity(revision: &BlueprintRevision) -> TestResult<ContextBuildIdentity> {
    Ok(ContextBuildIdentity {
        run: RunId::new("run-context")?,
        revision: revision.id().clone(),
        node: NodeId::new("current")?,
        execution: NodeExecutionId::new("execution-current")?,
        attempt: AttemptId::new("attempt-current")?,
    })
}

fn candidate(
    name: &str,
    kind: ContextSemanticKind,
    node: Option<NodeId>,
    bytes: u64,
) -> TestResult<ContextCandidate> {
    Ok(ContextCandidate {
        kind,
        source: Some(if kind == ContextSemanticKind::DirectInput {
            ContextSource::DirectInput {
                name: name.to_owned(),
                reference: InvocationValueReference::Inline {
                    value: BoundedJson::new(json!({"name":name}))?,
                },
            }
        } else {
            ContextSource::NodeExecution {
                node: node.clone().ok_or("node source missing")?,
                execution: NodeExecutionId::new(format!("execution-{name}"))?,
                attempt: Some(AttemptId::new(format!("attempt-{name}"))?),
                event_sequence: None,
            }
        }),
        content_digest: ContentDigest::for_bytes(name.as_bytes()),
        source_revision: serde_json::from_str(
            "\"rev_0000000000000000000000000000000000000000000000000000000000000000\"",
        )?,
        execution: Some(NodeExecutionId::new(format!("execution-{name}"))?),
        attempt: Some(AttemptId::new(format!("attempt-{name}"))?),
        source_sequence: None,
        occurred_at_ms: None,
        causal_distance: None,
        producer: ContextProducerFact::default(),
        node,
        roles: BTreeSet::new(),
        scope: None,
        exposed_across_scope: false,
        required: false,
        availability: ContextCandidateAvailability::Available,
        selected_bytes: bytes,
        selected_artifact_bytes: 0,
        estimated_model_input_units: Some(bytes),
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

#[test]
fn selection_is_deterministic_across_candidate_page_order() -> TestResult {
    let policy = policy(ContextBudget::new(8, 1_024, 1_024, Some(1_024))?)?;
    let revision = revision(policy.clone())?;
    let direct = candidate("prompt", ContextSemanticKind::DirectInput, None, 6)?;
    let ancestor = candidate(
        "source",
        ContextSemanticKind::SuccessfulOutput,
        Some(NodeId::new("source")?),
        7,
    )?;
    let build = |candidates| {
        CausalContextBuilder::build(ContextBuildRequest {
            identity: identity(&revision).map_err(|error| error.to_string())?,
            semantic: revision.semantic(),
            policy: &policy,
            visible_scopes: BTreeSet::new(),
            candidates,
        })
        .map_err(|error| error.to_string())
    };
    let first = build(vec![direct.clone(), ancestor.clone()])?;
    let second = build(vec![ancestor, direct])?;
    assert_eq!(first, second);
    assert_eq!(first.entries().len(), 2);
    assert_eq!(
        first.entries()[0].reason(),
        ContextInclusionReason::CausalAncestor
    );
    Ok(())
}

#[test]
fn branch_siblings_are_omitted_until_explicitly_exposed() -> TestResult {
    let policy = policy(ContextBudget::new(8, 1_024, 1_024, None)?)?;
    let revision = revision(policy.clone())?;
    let root = WorkspaceScope::run_root(RunId::new("run-context")?, ScopeId::new("root")?);
    let sibling = ScopeReference::new(RunId::new("run-context")?, ScopeId::new("sibling")?);
    let mut value = candidate(
        "source",
        ContextSemanticKind::SuccessfulOutput,
        Some(NodeId::new("source")?),
        7,
    )?;
    value.scope = Some(sibling);
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::from([root.reference().clone()]),
        candidates: vec![value.clone()],
    })?;
    assert!(manifest.entries().is_empty());
    assert_eq!(
        manifest.omissions()[0].reason,
        ContextOmissionReason::BranchIsolated
    );

    value.exposed_across_scope = true;
    let exposed = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::from([root.reference().clone()]),
        candidates: vec![value],
    })?;
    assert_eq!(exposed.entries().len(), 1);
    Ok(())
}

#[test]
fn exact_budget_boundary_and_required_fail_closed_are_stable() -> TestResult {
    let base_policy = policy(ContextBudget::new(1, 10, 1, Some(10))?)?;
    let base_revision = revision(base_policy.clone())?;
    let exact = candidate("prompt", ContextSemanticKind::DirectInput, None, 10)?;
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&base_revision)?,
        semantic: base_revision.semantic(),
        policy: &base_policy,
        visible_scopes: BTreeSet::new(),
        candidates: vec![exact],
    })?;
    assert_eq!(manifest.totals().bytes, 10);

    let mut missing = candidate("required", ContextSemanticKind::DirectInput, None, 1)?;
    missing.required = true;
    missing.availability = ContextCandidateAvailability::MissingOrCorrupt;
    assert!(
        CausalContextBuilder::build(ContextBuildRequest {
            identity: identity(&base_revision)?,
            semantic: base_revision.semantic(),
            policy: &base_policy,
            visible_scopes: BTreeSet::new(),
            candidates: vec![missing],
        })
        .is_err()
    );

    let mut denied = candidate("denied", ContextSemanticKind::DirectInput, None, 1)?;
    denied.required = true;
    denied.sensitivity = ArtifactSensitivity::Restricted;
    denied.authority = AuthorityFact {
        required: true,
        authorized: false,
        authority_reference: None,
    };
    assert!(
        CausalContextBuilder::build(ContextBuildRequest {
            identity: identity(&base_revision)?,
            semantic: base_revision.semantic(),
            policy: &base_policy,
            visible_scopes: BTreeSet::new(),
            candidates: vec![denied],
        })
        .is_err()
    );

    let unit_policy = policy(ContextBudget::new(1, 10, 1, Some(9))?)?;
    let unit_revision = revision(unit_policy.clone())?;
    let mut over_units = candidate(
        "provider-observation",
        ContextSemanticKind::DirectInput,
        None,
        1,
    )?;
    over_units.required = true;
    over_units.estimated_model_input_units = Some(10);
    assert!(matches!(
        CausalContextBuilder::build(ContextBuildRequest {
            identity: identity(&unit_revision)?,
            semantic: unit_revision.semantic(),
            policy: &unit_policy,
            visible_scopes: BTreeSet::new(),
            candidates: vec![over_units],
        }),
        Err(milkdrift_runtime::ContextBuildError::RequiredBudget(
            "model-input-unit"
        ))
    ));

    let zero_budget =
        ContextBudget::new(2, 10, 1, None)?.with_discovery_limits(16, 1, 10, 8, 32_768)?;
    let zero_policy = policy(zero_budget)?;
    let zero_revision = revision(zero_policy.clone())?;
    let mut first = candidate("zero-a", ContextSemanticKind::DirectInput, None, 0)?;
    let mut second = candidate("zero-b", ContextSemanticKind::DirectInput, None, 0)?;
    for (candidate, identity) in [(&mut first, "zero-a"), (&mut second, "zero-b")] {
        candidate.source = Some(ContextSource::DirectInput {
            name: identity.to_owned(),
            reference: InvocationValueReference::Artifact {
                reference: milkdrift_capability::ArtifactReference::new(
                    identity,
                    ContentDigest::for_bytes(&[]).to_hex(),
                    Some("application/octet-stream".to_owned()),
                    Some(0),
                )?,
            },
        });
        candidate.content_digest = ContentDigest::for_bytes(&[]);
        candidate.estimated_model_input_units = Some(0);
    }
    let zero_manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&zero_revision)?,
        semantic: zero_revision.semantic(),
        policy: &zero_policy,
        visible_scopes: BTreeSet::new(),
        candidates: vec![second, first],
    })?;
    assert_eq!(zero_manifest.totals().artifacts, 1);
    assert!(zero_manifest.entries()[0].selected_artifact());
    assert_eq!(
        zero_manifest.omissions()[0].reason,
        ContextOmissionReason::ArtifactItemBudget
    );
    Ok(())
}

#[test]
fn exact_nodes_roles_decisions_artifacts_and_exclusions_are_explicit() -> TestResult {
    let artifact_selector = ContextArtifactSelector::new(
        BTreeSet::from(["evidence".to_owned()]),
        BTreeSet::from(["application/json".to_owned()]),
        BTreeSet::from([ContextArtifactSensitivity::Public]),
        BTreeSet::from([ContextArtifactRetention::WhileReferenced]),
        BTreeSet::from([ContextProvenanceClass::Artifact]),
    )?;
    let policy = TaskContextPolicy::new(
        false,
        None,
        BTreeSet::from([NodeId::new("source")?]),
        BTreeSet::from([ContextSemanticRole::FailureEvidence]),
        BTreeSet::from([ContextCategory::Decision, ContextCategory::Artifact]),
        BTreeSet::from([ContextCategory::RawProgress]),
        Some(artifact_selector),
        ContextBudget::new(16, 4_096, 4_096, None)?,
        ContextOrdering::CausalKindSource,
        ContextTruncation::OmitOversized,
        ContextSessionPolicy::Fresh,
        true,
    )?;
    let revision = revision(policy.clone())?;
    let selected_node = candidate(
        "selected",
        ContextSemanticKind::SuccessfulOutput,
        Some(NodeId::new("source")?),
        1,
    )?;
    let mut selected_role = candidate(
        "failure",
        ContextSemanticKind::Failure,
        Some(NodeId::new("current")?),
        1,
    )?;
    selected_role.roles = BTreeSet::from([ContextSemanticRole::FailureEvidence]);
    let decision = candidate(
        "decision",
        ContextSemanticKind::Decision,
        Some(NodeId::new("current")?),
        1,
    )?;
    let raw = candidate(
        "raw",
        ContextSemanticKind::RawProgress,
        Some(NodeId::new("source")?),
        1,
    )?;
    let artifact = |name: &str| -> TestResult<ContextCandidate> {
        let mut value = candidate(
            name,
            ContextSemanticKind::Artifact,
            Some(NodeId::new("current")?),
            1,
        )?;
        value.selected_artifact_bytes = 10;
        value.artifact = Some(milkdrift_runtime::ContextCandidateArtifactFacts {
            name: name.to_owned(),
            media_type: "application/json".to_owned(),
            sensitivity: ContextArtifactSensitivity::Public,
            retention: ContextArtifactRetention::WhileReferenced,
            provenance: ContextProvenanceClass::Artifact,
        });
        Ok(value)
    };
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::new(),
        candidates: vec![
            raw,
            artifact("ignored")?,
            decision,
            selected_role,
            artifact("evidence")?,
            selected_node,
        ],
    })?;
    assert_eq!(manifest.entries().len(), 4);
    assert!(manifest.entries().iter().any(|entry| {
        entry.reason() == ContextInclusionReason::SelectedNode
            && entry.kind() == ContextSemanticKind::SuccessfulOutput
    }));
    assert!(manifest.entries().iter().any(|entry| {
        entry.reason() == ContextInclusionReason::SelectedRole
            && entry.kind() == ContextSemanticKind::Failure
    }));
    assert!(manifest.entries().iter().any(|entry| {
        entry.reason() == ContextInclusionReason::IncludedCategory
            && entry.kind() == ContextSemanticKind::Decision
    }));
    assert!(manifest.entries().iter().any(|entry| {
        entry.reason() == ContextInclusionReason::ArtifactSelector
            && entry.kind() == ContextSemanticKind::Artifact
    }));
    assert!(manifest.omissions().iter().any(|omission| {
        omission.kind == ContextSemanticKind::RawProgress
            && omission.reason == ContextOmissionReason::ExcludedCategory
    }));
    assert!(manifest.omissions().iter().any(|omission| {
        omission.kind == ContextSemanticKind::Artifact
            && omission.reason == ContextOmissionReason::NotSelected
    }));
    Ok(())
}

#[test]
fn exact_manifest_is_committed_before_dispatch_and_survives_restart() -> TestResult {
    let policy = policy(ContextBudget::new(1, 10, 1, Some(10))?)?;
    let revision = revision(policy.clone())?;
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity(&revision)?,
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::new(),
        candidates: vec![candidate(
            "prompt",
            ContextSemanticKind::DirectInput,
            None,
            10,
        )?],
    })?;
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let reference = persist_context_manifest(
        &store,
        &manifest,
        WorkspaceBudget::new(0, 0, 0, 1, 1_048_576, 1_048_576)?,
        WorkspaceUsage::EMPTY,
    )?;
    let artifact = ArtifactId::new(reference.identity())?;
    let durable = store
        .metadata(&artifact)?
        .ok_or("context manifest metadata was not committed")?;
    assert!(store.is_committed(durable.reference())?);
    drop(store);

    let reopened = RedbStore::open(root.path())?;
    let durable = reopened
        .metadata(&artifact)?
        .ok_or("context manifest did not survive restart")?;
    assert!(reopened.is_committed(durable.reference())?);
    let duplicate = persist_context_manifest(
        &reopened,
        &manifest,
        WorkspaceBudget::new(0, 0, 0, 1, 1_048_576, 1_048_576)?,
        WorkspaceUsage::EMPTY,
    )?;
    assert_eq!(duplicate, reference);
    Ok(())
}
