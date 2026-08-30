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

    for (index, stage) in stages.iter().enumerate() {
        let ids = StageIds::new(stage);
        let repository_profile =
            serde_json::to_value(&document.sequence().repository).map_err(json_error)?;
        let coding = coding_node(stage, &ids, index != 0, repository_profile.clone())?;
        let verification = verification_node(stage, &ids, repository_profile)?;
        let gate = gate_node(stage, &ids)?;
        operations.extend([
            Mutation::AddNode { node: coding },
            Mutation::AddNode { node: verification },
            Mutation::AddNode { node: gate },
        ]);
        operations.push(control_edge(
            &format!("{}-coding-verification", stage.id),
            &ids.coding,
            CONTROL_OUT,
            &ids.verification,
            CONTROL_IN,
        )?);
        operations.push(control_edge(
            &format!("{}-verification-gate", stage.id),
            &ids.verification,
            CONTROL_OUT,
            &ids.gate,
            CONTROL_IN,
        )?);
        operations.push(data_edge(
            &format!("{}-verification-result-gate", stage.id),
            &ids.verification,
            &stage.verification.success_artifact,
            &ids.gate,
            &stage.verification.success_artifact,
        )?);

        let success_target = stages
            .get(index + 1)
            .map(|next| format!("stage-{}-coding", next.id))
            .unwrap_or_else(|| "sequence-succeeded".to_owned());
        operations.push(control_edge(
            &format!("{}-gate-pass", stage.id),
            &ids.gate,
            PASS,
            &success_target,
            CONTROL_IN,
        )?);

        let (reviewer_node, approval_wait_node) = match stage.failure {
            FailurePolicy::PauseForReview => {
                let reviewer = reviewer_node(stage, &ids)?;
                let hold = approval_wait_node(&ids)?;
                let failure = failure_terminal(&ids)?;
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
                (Some(ids.reviewer.clone()), Some(ids.hold.clone()))
            }
            FailurePolicy::FailRun => {
                let failure = failure_terminal(&ids)?;
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
            coding_node: ids.coding,
            verification_node: ids.verification,
            gate_node: ids.gate,
            reviewer_node,
            approval_wait_node,
            prompt_digest: prompt_digest(&stage.prompt)?,
            checkpoint_artifact: stage.verification.success_artifact.clone(),
        });
    }

    operations.push(Mutation::AddNode {
        node: Node::new(
            NodeId::new("sequence-succeeded").map_err(|error| compilation(error.to_string()))?,
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

struct StageIds {
    coding: String,
    verification: String,
    gate: String,
    reviewer: String,
    hold: String,
    failure_terminal: String,
}

impl StageIds {
    fn new(stage: &StageDefinition) -> Self {
        Self {
            coding: format!("stage-{}-coding", stage.id),
            verification: format!("stage-{}-verification", stage.id),
            gate: format!("stage-{}-gate", stage.id),
            reviewer: format!("stage-{}-review", stage.id),
            hold: format!("stage-{}-approval", stage.id),
            failure_terminal: format!("stage-{}-failed", stage.id),
        }
    }

    fn remediation(stage: &StageDefinition, generation: u16) -> Self {
        let prefix = format!("stage-{}-remediation-{generation}", stage.id);
        Self {
            coding: format!("{prefix}-coding"),
            verification: format!("{prefix}-verification"),
            gate: format!("{prefix}-gate"),
            reviewer: format!("{prefix}-review"),
            hold: format!("{prefix}-approval"),
            failure_terminal: format!("{prefix}-failed"),
        }
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
    let initial = StageIds::new(stage);
    let ids = StageIds::remediation(stage, generation);
    let failure_review = format!("stage-{}-remediation-{generation}-failure-review", stage.id);
    let superseded_pass = format!(
        "stage-{}-remediation-{generation}-superseded-pass",
        stage.id
    );
    let repository = serde_json::to_value(&document.sequence().repository).map_err(json_error)?;
    let coding = coding_node(&remediation_stage, &ids, true, repository.clone())?;
    let verification = verification_node(&remediation_stage, &ids, repository)?;
    let gate = gate_node(&remediation_stage, &ids)?;
    let success_review = reviewer_node_named(&remediation_stage, &ids.reviewer)?;
    let failure_review_node = reviewer_node_named(&remediation_stage, &failure_review)?;
    let hold = approval_wait_node(&ids)?;
    let failure = failure_terminal(&ids)?;
    let next = document
        .sequence()
        .stages
        .iter()
        .position(|candidate| candidate.id == stage.id)
        .and_then(|index| document.sequence().stages.get(index + 1))
        .map(|next| format!("stage-{}-coding", next.id))
        .unwrap_or_else(|| "sequence-succeeded".to_owned());

    let mut operations = vec![
        Mutation::RemoveEdge {
            edge: EdgeId::new(format!("{}-hold-failure", stage.id))
                .map_err(|error| compilation(error.to_string()))?,
        },
        Mutation::ReplaceEdge {
            edge: Edge::new(
                EdgeId::new(format!("{}-gate-pass", stage.id))
                    .map_err(|error| compilation(error.to_string()))?,
                EdgeKind::Control,
                node_id(&initial.gate)?,
                port(PASS)?,
                node_id(&superseded_pass)?,
                port(CONTROL_IN)?,
            ),
        },
        Mutation::RemoveNode {
            node: node_id(&initial.failure_terminal)?,
        },
        Mutation::AddNode { node: coding },
        Mutation::AddNode { node: verification },
        Mutation::AddNode { node: gate },
        Mutation::AddNode {
            node: success_review,
        },
        Mutation::AddNode {
            node: failure_review_node,
        },
        Mutation::AddNode { node: hold },
        Mutation::AddNode { node: failure },
        Mutation::AddNode {
            node: Node::new(
                node_id(&superseded_pass)?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?
            .with_control_input(port(CONTROL_IN)?)?,
        },
    ];
    operations.extend([
        control_edge(
            &format!("{}-approved-remediation-{generation}", stage.id),
            &initial.hold,
            APPROVED,
            &ids.coding,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{}-coding-verification", ids.coding),
            &ids.coding,
            CONTROL_OUT,
            &ids.verification,
            CONTROL_IN,
        )?,
        control_edge(
            &format!("{}-verification-gate", ids.verification),
            &ids.verification,
            CONTROL_OUT,
            &ids.gate,
            CONTROL_IN,
        )?,
        data_edge(
            &format!("{}-verification-result-gate", ids.verification),
            &ids.verification,
            &stage.verification.success_artifact,
            &ids.gate,
            &stage.verification.success_artifact,
        )?,
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

fn coding_node(
    stage: &StageDefinition,
    ids: &StageIds,
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
    let mut node = Node::new(node_id(&ids.coding)?, NodeKind::Task { config })?
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
    ids: &StageIds,
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
    let mut node = Node::new(node_id(&ids.verification)?, NodeKind::Task { config })?
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

fn gate_node(stage: &StageDefinition, ids: &StageIds) -> Result<Node, PromptSequenceError> {
    let success = port(PASS)?;
    let failure = port(FAIL)?;
    let source = BindingSource::NodeOutput {
        node: node_id(&ids.verification)?,
        port: port(&stage.verification.success_artifact)?,
        path: PathSelector::new(Vec::new()).map_err(|error| compilation(error.to_string()))?,
    };
    Ok(Node::new(
        node_id(&ids.gate)?,
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

fn reviewer_node(stage: &StageDefinition, ids: &StageIds) -> Result<Node, PromptSequenceError> {
    reviewer_node_named(stage, &ids.reviewer)
}

fn reviewer_node_named(
    stage: &StageDefinition,
    identity: &str,
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
    Ok(Node::new(node_id(identity)?, NodeKind::Task { config })?
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

fn approval_wait_node(ids: &StageIds) -> Result<Node, PromptSequenceError> {
    Ok(Node::new(
        node_id(&ids.hold)?,
        NodeKind::SignalWait {
            signal: milkdrift_capability::OperationId::new("sequence.approved")
                .map_err(|error| compilation(error.to_string()))?,
        },
    )?
    .with_control_input(port(CONTROL_IN)?)?
    .with_control_output(port(APPROVED)?)?)
}

fn failure_terminal(ids: &StageIds) -> Result<Node, PromptSequenceError> {
    Ok(Node::new(
        node_id(&ids.failure_terminal)?,
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
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> Result<Mutation, PromptSequenceError> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(identity).map_err(|error| compilation(error.to_string()))?,
            EdgeKind::Control,
            node_id(source)?,
            port(source_port)?,
            node_id(target)?,
            port(target_port)?,
        ),
    })
}

fn data_edge(
    identity: &str,
    source: &str,
    source_port: &str,
    target: &str,
    target_port: &str,
) -> Result<Mutation, PromptSequenceError> {
    Ok(Mutation::AddEdge {
        edge: Edge::new(
            EdgeId::new(identity).map_err(|error| compilation(error.to_string()))?,
            EdgeKind::Data,
            node_id(source)?,
            port(source_port)?,
            node_id(target)?,
            port(target_port)?,
        ),
    })
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

fn node_id(value: &str) -> Result<NodeId, PromptSequenceError> {
    NodeId::new(value).map_err(|error| compilation(error.to_string()))
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
