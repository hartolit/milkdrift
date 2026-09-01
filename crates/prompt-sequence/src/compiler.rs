use std::collections::{BTreeMap, BTreeSet};

use milkdrift_blueprint::{
    AuthorRef, BindingSource, BlueprintMetadata, BlueprintRevision, BranchConfig, Condition,
    ContextCategory, ContextOrdering, ContextSemanticRole, ContextSessionPolicy, ContextTruncation,
    DataPort, Edge, EdgeId, EdgeKind, Mutation, MutationBatch, Node, NodeId, NodeKind,
    PathSelector, PortId, SchemaRef, TaskConfig, TaskContextPolicy, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{BoundedJson, CapabilityRequirement, ExtensionKey, SchemaId};
use milkdrift_workspace::{
    ArtifactId, ArtifactReference as WorkspaceArtifactReference, ContentDigest, MediaType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    CapabilityProfileRef, FailurePolicy, PromptSequenceDocument, PromptSequenceError, PromptSource,
    SessionPolicy, StageDefinition,
};

const CONTROL_IN: &str = "in";
const CONTROL_OUT: &str = "out";
const PASS: &str = "pass";
const FAIL: &str = "fail";
const APPROVED: &str = "approved";
const SEQUENCE_SUCCEEDED: &str = "sequence-succeeded";

/// Stable generated ordinary-node identities for one imported stage.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageBlueprintSummary {
    /// Imported stage identity.
    pub stage_id: String,
    /// Fresh coding task.
    pub coding_node: String,
    /// Distinct verification task.
    pub verification_node: String,
    /// Safe result-artifact branch.
    pub gate_node: String,
    /// Reviewer task on the failure route, when configured.
    pub reviewer_node: Option<String>,
    /// Durable approval wait on the failure route, when configured.
    pub approval_wait_node: Option<String>,
    /// Exact prompt digest or artifact digest.
    pub prompt_digest: String,
    /// Verification output used as the accepted checkpoint fact.
    pub checkpoint_artifact: String,
}

/// Validated import result ready for the ordinary revision mutation/storage path.
#[derive(Clone, Debug)]
pub struct CompiledPromptSequence {
    revision: BlueprintRevision,
    import_digest: String,
    repository_profile_digest: String,
    stages: Vec<StageBlueprintSummary>,
}

impl CompiledPromptSequence {
    /// Generated ordinary immutable blueprint revision.
    #[must_use]
    pub const fn revision(&self) -> &BlueprintRevision {
        &self.revision
    }

    /// Domain-separated digest of the canonical imported sequence document.
    #[must_use]
    pub fn import_digest(&self) -> &str {
        &self.import_digest
    }

    /// Domain-separated digest of the repository workspace profile.
    #[must_use]
    pub fn repository_profile_digest(&self) -> &str {
        &self.repository_profile_digest
    }

    /// Ordered stage-to-node mapping.
    #[must_use]
    pub fn stages(&self) -> &[StageBlueprintSummary] {
        &self.stages
    }
}

/// Compiles one bounded sequence into ordinary task, branch, signal-wait, and terminal nodes.
pub fn compile(
    document: &PromptSequenceDocument,
    author: AuthorRef,
) -> Result<CompiledPromptSequence, PromptSequenceError> {
    let canonical = document.to_canonical_json()?;
    let import_digest = domain_digest("milkdrift.prompt-sequence-import.v1", &canonical);
    let repository = serde_json::to_vec(&document.sequence().repository)
        .map_err(|error| PromptSequenceError::Compilation(error.to_string()))?;
    let repository_profile_digest =
        domain_digest("milkdrift.repository-workspace-profile.v1", &repository);
    let mut operations = Vec::new();
    let mut summaries = Vec::new();
    let stages = &document.sequence().stages;
    let repository_profile =
        serde_json::to_value(&document.sequence().repository).map_err(json_error)?;

    for (index, stage) in stages.iter().enumerate() {
        let ids = StageTopologyIds::initial(stage)?;
        let pipeline = stage_pipeline(stage, &ids, index != 0, repository_profile.clone())?;
        operations.extend(pipeline.nodes);
        operations.extend(pipeline.edges);

        let success_target = stages
            .get(index + 1)
            .map(initial_coding_id)
            .transpose()?
            .unwrap_or(sequence_success_id()?);
        operations.push(control_edge(
            &format!("{}-gate-pass", stage.id),
            &ids.gate,
            PASS,
            &success_target,
            CONTROL_IN,
        )?);

        let (reviewer_node, approval_wait_node) = match stage.failure {
            FailurePolicy::PauseForReview => {
                let reviewer = reviewer_node(stage, &ids.reviewer)?;
                let hold = approval_wait_node(&ids.hold)?;
                let failure = failure_terminal(&ids.failure_terminal)?;
                operations.push(Mutation::AddNode { node: reviewer });
                operations.push(Mutation::AddNode { node: hold });
                operations.push(Mutation::AddNode { node: failure });
                operations.push(control_edge(
                    &format!("{}-gate-review", stage.id),
                    &ids.gate,
                    FAIL,
                    &ids.reviewer,
                    CONTROL_IN,
                )?);
                operations.push(control_edge(
                    &format!("{}-review-hold", stage.id),
                    &ids.reviewer,
                    CONTROL_OUT,
                    &ids.hold,
                    CONTROL_IN,
                )?);
                operations.push(control_edge(
                    &format!("{}-hold-failure", stage.id),
                    &ids.hold,
                    APPROVED,
                    &ids.failure_terminal,
                    CONTROL_IN,
                )?);
                (Some(ids.reviewer.to_string()), Some(ids.hold.to_string()))
            }
            FailurePolicy::FailRun => {
                let failure = failure_terminal(&ids.failure_terminal)?;
                operations.push(Mutation::AddNode { node: failure });
                operations.push(control_edge(
                    &format!("{}-gate-failure", stage.id),
                    &ids.gate,
                    FAIL,
                    &ids.failure_terminal,
                    CONTROL_IN,
                )?);
                (None, None)
            }
        };

        summaries.push(StageBlueprintSummary {
            stage_id: stage.id.clone(),
            coding_node: ids.coding.to_string(),
            verification_node: ids.verification.to_string(),
            gate_node: ids.gate.to_string(),
            reviewer_node,
            approval_wait_node,
            prompt_digest: prompt_digest(&stage.prompt)?,
            checkpoint_artifact: stage.verification.success_artifact.clone(),
        });
    }

    operations.push(Mutation::AddNode {
        node: Node::new(
            sequence_success_id()?,
            NodeKind::Terminal {
                outcome: TerminalOutcome::Success,
            },
        )?
        .with_control_input(port(CONTROL_IN)?)?,
    });
    operations.push(Mutation::SetMetadata {
        metadata: metadata(
            document,
            &import_digest,
            &repository_profile_digest,
            &summaries,
        )?,
    });

    let batch = MutationBatch::new(operations).map_err(|error| compilation(error.to_string()))?;
    let revision = BlueprintRevision::genesis(
        WorkflowId::new(document.sequence().workflow_id.clone())
            .map_err(|error| compilation(error.to_string()))?,
        batch,
        author,
        format!(
            "import prompt sequence {} schema v1",
            document.sequence().id
        ),
    )
    .map_err(|error| compilation(format!("{error:?}")))?;
    Ok(CompiledPromptSequence {
        revision,
        import_digest,
        repository_profile_digest,
        stages: summaries,
    })
}

struct StageTopologyIds {
    coding: NodeId,
    verification: NodeId,
    gate: NodeId,
    reviewer: NodeId,
    hold: NodeId,
    failure_terminal: NodeId,
    coding_verification: EdgeId,
    verification_gate: EdgeId,
    verification_result_gate: EdgeId,
}

impl StageTopologyIds {
    fn initial(stage: &StageDefinition) -> Result<Self, PromptSequenceError> {
        Self::from_prefix(
            &format!("stage-{}", stage.id),
            edge_id(format!("{}-coding-verification", stage.id))?,
            edge_id(format!("{}-verification-gate", stage.id))?,
            edge_id(format!("{}-verification-result-gate", stage.id))?,
        )
    }

    fn remediation(stage: &StageDefinition, generation: u16) -> Result<Self, PromptSequenceError> {
        let prefix = format!("stage-{}-remediation-{generation}", stage.id);
        Self::from_prefix(
            &prefix,
            edge_id(format!("{prefix}-coding-coding-verification"))?,
            edge_id(format!("{prefix}-verification-verification-gate"))?,
            edge_id(format!("{prefix}-verification-verification-result-gate"))?,
        )
    }

    fn from_prefix(
        prefix: &str,
        coding_verification: EdgeId,
        verification_gate: EdgeId,
        verification_result_gate: EdgeId,
    ) -> Result<Self, PromptSequenceError> {
        Ok(Self {
            coding: node_id(format!("{prefix}-coding"))?,
            verification: node_id(format!("{prefix}-verification"))?,
            gate: node_id(format!("{prefix}-gate"))?,
            reviewer: node_id(format!("{prefix}-review"))?,
            hold: node_id(format!("{prefix}-approval"))?,
            failure_terminal: node_id(format!("{prefix}-failed"))?,
            coding_verification,
            verification_gate,
            verification_result_gate,
        })
    }
}

pub(crate) fn remediation_mutation(
    document: &PromptSequenceDocument,
    stage: &StageDefinition,
    generation: u16,
    prompt: PromptSource,
) -> Result<MutationBatch, PromptSequenceError> {
    if generation == 0 {
        return Err(PromptSequenceError::Invalid(
            "remediation generation must be nonzero".to_owned(),
        ));
    }
    if stage.failure != FailurePolicy::PauseForReview {
        return Err(PromptSequenceError::Invalid(
            "remediation requires pause_for_review on the selected stage".to_owned(),
        ));
    }
    let mut remediation_stage = stage.clone();
    remediation_stage.prompt = prompt;
    let initial = StageTopologyIds::initial(stage)?;
    let ids = StageTopologyIds::remediation(stage, generation)?;
    let failure_review = node_id(format!(
        "stage-{}-remediation-{generation}-failure-review",
        stage.id
    ))?;
    let superseded_pass = node_id(format!(
        "stage-{}-remediation-{generation}-superseded-pass",
        stage.id
    ))?;
    let repository = serde_json::to_value(&document.sequence().repository).map_err(json_error)?;
    let pipeline = stage_pipeline(&remediation_stage, &ids, true, repository)?;
    let next = document
        .sequence()
        .stages
        .iter()
        .position(|candidate| candidate.id == stage.id)
        .and_then(|index| document.sequence().stages.get(index + 1))
        .map(initial_coding_id)
        .transpose()?
        .unwrap_or(sequence_success_id()?);

    let mut operations = vec![
        Mutation::RemoveEdge {
            edge: edge_id(format!("{}-hold-failure", stage.id))?,
        },
        Mutation::ReplaceEdge {
            edge: build_edge(
                &format!("{}-gate-pass", stage.id),
                EdgeKind::Control,
                &initial.gate,
                PASS,
                &superseded_pass,
                CONTROL_IN,
            )?,
        },
        Mutation::RemoveNode {
            node: initial.failure_terminal,
        },
    ];
    operations.extend(pipeline.nodes);
    operations.extend([
        Mutation::AddNode {
            node: reviewer_node(&remediation_stage, &ids.reviewer)?,
        },
        Mutation::AddNode {
            node: reviewer_node_named(&remediation_stage, &failure_review)?,
        },
        Mutation::AddNode {
            node: approval_wait_node(&ids.hold)?,
        },
        Mutation::AddNode {
            node: failure_terminal(&ids.failure_terminal)?,
        },
        Mutation::AddNode {
            node: Node::new(
                superseded_pass.clone(),
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?
            .with_control_input(port(CONTROL_IN)?)?,
        },
    ]);
    operations.push(control_edge(
        &format!("{}-approved-remediation-{generation}", stage.id),
        &initial.hold,
        APPROVED,
        &ids.coding,
        CONTROL_IN,
    )?);
    operations.extend(pipeline.edges);
    operations.extend([
        control_edge(
            &format!("{}-gate-review", ids.gate),
            &ids.gate,
            PASS,
            &ids.reviewer,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{}-review-continue", ids.reviewer),
            &ids.reviewer,
            CONTROL_OUT,
            &next,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{}-gate-failure-review", ids.gate),
            &ids.gate,
            FAIL,
            &failure_review,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{failure_review}-hold"),
            &failure_review,
            CONTROL_OUT,
            &ids.hold,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{}-hold-failure", ids.hold),
            &ids.hold,
            APPROVED,
            &ids.failure_terminal,
            CONTROL_IN,
        )?,
    ]);
    MutationBatch::new(operations).map_err(|error| compilation(format!("{error:?}")))
}

struct StagePipelineMutations {
    nodes: [Mutation; 3],
    edges: [Mutation; 3],
}

fn stage_pipeline(
    stage: &StageDefinition,
    ids: &StageTopologyIds,
    has_predecessor: bool,
    repository_profile: Value,
) -> Result<StagePipelineMutations, PromptSequenceError> {
    Ok(StagePipelineMutations {
        nodes: [
            Mutation::AddNode {
                node: coding_node(
                    stage,
                    &ids.coding,
                    has_predecessor,
                    repository_profile.clone(),
                )?,
            },
            Mutation::AddNode {
                node: verification_node(stage, &ids.verification, repository_profile)?,
            },
            Mutation::AddNode {
                node: gate_node(stage, &ids.verification, &ids.gate)?,
            },
        ],
        edges: [
            control_edge(
                ids.coding_verification.as_str(),
                &ids.coding,
                CONTROL_OUT,
                &ids.verification,
                CONTROL_IN,
            )?,
            control_edge(
                ids.verification_gate.as_str(),
                &ids.verification,
                CONTROL_OUT,
                &ids.gate,
                CONTROL_IN,
            )?,
            add_edge(
                ids.verification_result_gate.as_str(),
                EdgeKind::Data,
                &ids.verification,
                &stage.verification.success_artifact,
                &ids.gate,
                &stage.verification.success_artifact,
            )?,
        ],
    })
}

fn coding_node(
    stage: &StageDefinition,
    identity: &NodeId,
    has_predecessor: bool,
    repository_profile: Value,
) -> Result<Node, PromptSequenceError> {
    let roles = BTreeSet::from([
        ContextSemanticRole::Implementation,
        ContextSemanticRole::Requirement,
    ]);
    let policy = causal_policy(stage.session, 8, roles.clone())?;
    let config = TaskConfig::new(requirement(&stage.coding), policy)
        .and_then(|config| config.with_output_context_roles(roles))
        .map_err(|error| compilation(error.to_string()))?;
    let mut node = Node::new(identity.clone(), NodeKind::Task { config })?
        .with_control_output(port(CONTROL_OUT)?)?
        .with_data_input(
            port("prompt")?,
            DataPort::input(prompt_schema()?, true, Some(prompt_binding(&stage.prompt)?))?,
        )?
        .with_data_input(
            port("repository_profile")?,
            literal_input(repository_schema()?, repository_profile)?,
        )?
        .with_data_input(
            port("stage_contract")?,
            literal_input(
                stage_schema()?,
                serde_json::to_value(stage).map_err(json_error)?,
            )?,
        )?;
    if has_predecessor {
        node = node.with_control_input(port(CONTROL_IN)?)?;
    }
    for output in &stage.outputs {
        node = node.with_data_output(port(&output.name)?, DataPort::output(artifact_schema()?))?;
    }
    Ok(node)
}

fn verification_node(
    stage: &StageDefinition,
    identity: &NodeId,
    repository_profile: Value,
) -> Result<Node, PromptSequenceError> {
    let roles = BTreeSet::from([
        ContextSemanticRole::Verification,
        ContextSemanticRole::FailureEvidence,
    ]);
    let config = TaskConfig::new(
        requirement(&stage.verification.profile),
        causal_policy(
            SessionPolicy::Fresh,
            2,
            BTreeSet::from([
                ContextSemanticRole::Implementation,
                ContextSemanticRole::Requirement,
            ]),
        )?,
    )
    .and_then(|config| config.with_output_context_roles(roles))
    .map_err(|error| compilation(error.to_string()))?;
    let mut node = Node::new(identity.clone(), NodeKind::Task { config })?
        .with_control_input(port(CONTROL_IN)?)?
        .with_control_output(port(CONTROL_OUT)?)?
        .with_data_input(
            port("repository_profile")?,
            literal_input(repository_schema()?, repository_profile)?,
        )?
        .with_data_input(
            port("verification_contract")?,
            literal_input(
                verification_schema()?,
                serde_json::to_value(&stage.verification).map_err(json_error)?,
            )?,
        )?
        .with_data_output(
            port(&stage.verification.success_artifact)?,
            DataPort::output(artifact_schema()?),
        )?
        .with_data_output(
            port(&stage.verification.result_artifact)?,
            DataPort::output(artifact_schema()?),
        )?;
    if let Some(log) = &stage.verification.log_artifact {
        node = node.with_data_output(port(log)?, DataPort::output(artifact_schema()?))?;
    }
    Ok(node)
}

fn gate_node(
    stage: &StageDefinition,
    verification: &NodeId,
    identity: &NodeId,
) -> Result<Node, PromptSequenceError> {
    let success = port(PASS)?;
    let failure = port(FAIL)?;
    let source = BindingSource::NodeOutput {
        node: verification.clone(),
        port: port(&stage.verification.success_artifact)?,
        path: PathSelector::new(Vec::new()).map_err(|error| compilation(error.to_string()))?,
    };
    Ok(Node::new(
        identity.clone(),
        NodeKind::Branch {
            config: BranchConfig::new(
                BTreeMap::from([(
                    success.clone(),
                    Condition::Exists {
                        source: source.clone(),
                    },
                )]),
                Some(failure.clone()),
            )?,
        },
    )?
    .with_control_input(port(CONTROL_IN)?)?
    .with_control_output(success)?
    .with_control_output(failure)?
    .with_data_input(
        port(&stage.verification.success_artifact)?,
        DataPort::input(artifact_schema()?, false, Some(source))?,
    )?)
}

fn reviewer_node(stage: &StageDefinition, identity: &NodeId) -> Result<Node, PromptSequenceError> {
    reviewer_node_named(stage, identity)
}

fn reviewer_node_named(
    stage: &StageDefinition,
    identity: &NodeId,
) -> Result<Node, PromptSequenceError> {
    let selected_roles = BTreeSet::from([
        ContextSemanticRole::FailureEvidence,
        ContextSemanticRole::Implementation,
        ContextSemanticRole::Requirement,
        ContextSemanticRole::Review,
        ContextSemanticRole::Verification,
    ]);
    let config = TaskConfig::new(
        requirement(&stage.reviewer),
        causal_policy(SessionPolicy::Fresh, 8, selected_roles)?,
    )
    .and_then(|config| {
        config.with_output_context_roles(BTreeSet::from([ContextSemanticRole::Review]))
    })
    .map_err(|error| compilation(error.to_string()))?;
    Ok(Node::new(identity.clone(), NodeKind::Task { config })?
        .with_control_input(port(CONTROL_IN)?)?
        .with_control_output(port(CONTROL_OUT)?)?
        .with_data_input(
            port("review_contract")?,
            literal_input(
                review_schema()?,
                json!({
                    "schema_version": 1,
                    "stage_id": stage.id,
                    "approval": stage.approval,
                    "failure": stage.failure,
                }),
            )?,
        )?
        .with_data_output(port("review")?, DataPort::output(artifact_schema()?))?
        .with_data_output(
            port("remediation_proposal")?,
            DataPort::output(artifact_schema()?),
        )?)
}

fn approval_wait_node(identity: &NodeId) -> Result<Node, PromptSequenceError> {
    Ok(Node::new(
        identity.clone(),
        NodeKind::SignalWait {
            signal: milkdrift_capability::OperationId::new("sequence.approved")
                .map_err(|error| compilation(error.to_string()))?,
        },
    )?
    .with_control_input(port(CONTROL_IN)?)?
    .with_control_output(port(APPROVED)?)?)
}

fn failure_terminal(identity: &NodeId) -> Result<Node, PromptSequenceError> {
    Ok(Node::new(
        identity.clone(),
        NodeKind::Terminal {
            outcome: TerminalOutcome::Failure,
        },
    )?
    .with_control_input(port(CONTROL_IN)?)?)
}

fn requirement(profile: &CapabilityProfileRef) -> CapabilityRequirement {
    let mut requirement = CapabilityRequirement::new(profile.operation.clone())
        .exact(profile.capability.clone())
        .execution_trust(profile.execution_trust)
        .maximum_side_effect(profile.maximum_side_effect);
    if let Some(provider) = &profile.provider_profile {
        requirement = requirement.provider_profile(provider.clone());
    }
    requirement
}

fn causal_policy(
    session: SessionPolicy,
    ancestor_depth: u16,
    selected_roles: BTreeSet<ContextSemanticRole>,
) -> Result<TaskContextPolicy, PromptSequenceError> {
    TaskContextPolicy::new(
        true,
        Some(ancestor_depth),
        BTreeSet::new(),
        selected_roles,
        BTreeSet::from([
            ContextCategory::Artifact,
            ContextCategory::Decision,
            ContextCategory::DirectInput,
            ContextCategory::Failure,
            ContextCategory::SuccessfulOutput,
        ]),
        BTreeSet::from([
            ContextCategory::FinalOutput,
            ContextCategory::PriorPrompt,
            ContextCategory::RawProgress,
            ContextCategory::ToolTrace,
            ContextCategory::VerboseCommandOutput,
        ]),
        None,
        milkdrift_blueprint::ContextBudget::default(),
        ContextOrdering::CausalKindSource,
        ContextTruncation::OmitOversized,
        match session {
            SessionPolicy::Fresh => ContextSessionPolicy::Fresh,
            SessionPolicy::ExplicitContinuation => ContextSessionPolicy::ExplicitContinuation,
        },
        true,
    )
    .map_err(|error| compilation(error.to_string()))
}

fn prompt_binding(prompt: &PromptSource) -> Result<BindingSource, PromptSequenceError> {
    match prompt {
        PromptSource::InlineMarkdown { content } => Ok(BindingSource::Literal {
            value: BoundedJson::new(Value::String(content.clone()))
                .map_err(|error| compilation(error.to_string()))?,
        }),
        PromptSource::Artifact { reference } => {
            let digest = ContentDigest::from_hex(reference.digest())
                .map_err(|error| compilation(error.to_string()))?;
            let artifact = WorkspaceArtifactReference::new(
                ArtifactId::new(reference.identity().to_owned())
                    .map_err(|error| compilation(error.to_string()))?,
                digest,
                MediaType::new(
                    reference
                        .media_type()
                        .ok_or_else(|| compilation("prompt artifact requires media_type"))?
                        .to_owned(),
                )
                .map_err(|error| compilation(error.to_string()))?,
                reference
                    .size_bytes()
                    .ok_or_else(|| compilation("prompt artifact requires size_bytes"))?,
            );
            Ok(BindingSource::Artifact {
                reference: serde_json::to_string(&artifact).map_err(json_error)?,
                contract: prompt_schema()?,
            })
        }
    }
}

fn literal_input(schema: SchemaRef, value: Value) -> Result<DataPort, PromptSequenceError> {
    DataPort::input(
        schema,
        true,
        Some(BindingSource::Literal {
            value: BoundedJson::new(value).map_err(|error| compilation(error.to_string()))?,
        }),
    )
    .map_err(|error| compilation(error.to_string()))
}

fn metadata(
    document: &PromptSequenceDocument,
    import_digest: &str,
    repository_profile_digest: &str,
    stages: &[StageBlueprintSummary],
) -> Result<BlueprintMetadata, PromptSequenceError> {
    let extension = BoundedJson::new(json!({
        "schema_version": 2,
        "sequence_id": document.sequence().id,
        "import_digest": import_digest,
        "repository_profile_id": document.sequence().repository.id,
        "repository_profile_digest": repository_profile_digest,
        "repository_root_ref": document.sequence().repository.root_ref,
        "budget": document.sequence().budget,
        "stages": stages,
    }))
    .map_err(|error| compilation(error.to_string()))?;
    BlueprintMetadata::new(
        document.sequence().title.clone(),
        "Imported ordered implementation sequence compiled from ordinary Milkdrift primitives",
        BTreeSet::from([
            "headless".to_owned(),
            "prompt-sequence".to_owned(),
            "schema-v2".to_owned(),
        ]),
        BTreeMap::from([(
            ExtensionKey::new("org.milkdrift/prompt-sequence")
                .map_err(|error| compilation(error.to_string()))?,
            extension,
        )]),
    )
    .map_err(|error| compilation(error.to_string()))
}

fn prompt_digest(prompt: &PromptSource) -> Result<String, PromptSequenceError> {
    match prompt {
        PromptSource::InlineMarkdown { content } => Ok(domain_digest(
            "milkdrift.prompt-sequence-inline-prompt.v1",
            content.as_bytes(),
        )),
        PromptSource::Artifact { reference } => Ok(reference.digest().to_owned()),
    }
}

fn domain_digest(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&[0]);
    hasher.update(bytes);
    format!("b3_{}", hasher.finalize())
}

fn control_edge(
    identity: &str,
    source: &NodeId,
    source_port: &str,
    target: &NodeId,
    target_port: &str,
) -> Result<Mutation, PromptSequenceError> {
    add_edge(
        identity,
        EdgeKind::Control,
        source,
        source_port,
        target,
        target_port,
    )
}

fn add_edge(
    identity: &str,
    kind: EdgeKind,
    source: &NodeId,
    source_port: &str,
    target: &NodeId,
    target_port: &str,
) -> Result<Mutation, PromptSequenceError> {
    Ok(Mutation::AddEdge {
        edge: build_edge(identity, kind, source, source_port, target, target_port)?,
    })
}

fn build_edge(
    identity: &str,
    kind: EdgeKind,
    source: &NodeId,
    source_port: &str,
    target: &NodeId,
    target_port: &str,
) -> Result<Edge, PromptSequenceError> {
    Ok(Edge::new(
        edge_id(identity)?,
        kind,
        source.clone(),
        port(source_port)?,
        target.clone(),
        port(target_port)?,
    ))
}

fn schema(identity: &str) -> Result<SchemaRef, PromptSequenceError> {
    SchemaRef::new(
        SchemaId::new(identity).map_err(|error| compilation(error.to_string()))?,
        1,
    )
    .map_err(|error| compilation(error.to_string()))
}

fn prompt_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.sequence_prompt")
}
fn stage_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.sequence_stage")
}
fn verification_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.verification_contract")
}
fn repository_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.repository_profile")
}
fn review_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.review_contract")
}
fn artifact_schema() -> Result<SchemaRef, PromptSequenceError> {
    schema("milkdrift.artifact_reference")
}

fn initial_coding_id(stage: &StageDefinition) -> Result<NodeId, PromptSequenceError> {
    node_id(format!("stage-{}-coding", stage.id))
}

fn sequence_success_id() -> Result<NodeId, PromptSequenceError> {
    node_id(SEQUENCE_SUCCEEDED)
}

fn node_id(value: impl Into<String>) -> Result<NodeId, PromptSequenceError> {
    NodeId::new(value).map_err(|error| compilation(error.to_string()))
}

fn edge_id(value: impl Into<String>) -> Result<EdgeId, PromptSequenceError> {
    EdgeId::new(value).map_err(|error| compilation(error.to_string()))
}

fn port(value: &str) -> Result<PortId, PromptSequenceError> {
    PortId::new(value).map_err(|error| compilation(error.to_string()))
}

fn compilation(message: impl Into<String>) -> PromptSequenceError {
    PromptSequenceError::Compilation(message.into())
}

fn json_error(error: serde_json::Error) -> PromptSequenceError {
    compilation(error.to_string())
}

impl From<milkdrift_blueprint::ModelError> for PromptSequenceError {
    fn from(error: milkdrift_blueprint::ModelError) -> Self {
        compilation(error.to_string())
    }
}
