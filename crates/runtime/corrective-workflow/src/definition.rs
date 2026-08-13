//! Borrowed corrective workflow definitions and validation.

use std::num::NonZeroU32;

use domain_contracts::{ArtifactId, BackendId, ModelId};
use task_graph::{
    ArtifactFlow, GraphValidationScratch, TaskArtifactInput, TaskArtifactOutput, TaskDependency,
    TaskGraph, TaskGraphError, TaskId, TaskNode, validate_artifact_flow, validate_graph,
};

use super::WorkflowShapeLimits;

/// Corrective artifact storage category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArtifactKind {
    /// UTF-8 text.
    Text,
    /// Structured validation diagnostics.
    Diagnostics,
    /// Capability-defined artifact category.
    Other(u16),
}

/// Corrective semantic role for an artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArtifactRole {
    /// External corrective specification.
    Specification,
    /// Initial generated draft.
    Draft,
    /// Raw validator findings.
    RawDiagnostics,
    /// Deterministically normalized findings.
    NormalizedDiagnostics,
    /// Model-produced review.
    Review,
    /// Model-produced revision.
    Revision,
    /// Terminal validation report.
    FinalValidation,
    /// Capability-defined semantic role.
    Other(u16),
}

/// Runtime identity and corrective metadata for a committed artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactReference {
    /// Runtime artifact identity.
    pub id: ArtifactId,
    /// Corrective storage category.
    pub kind: ArtifactKind,
    /// Corrective semantic role.
    pub role: ArtifactRole,
}

/// Definition-local artifact metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactDefinition {
    /// Definition-local identity used by graph bindings.
    pub id: ArtifactId,
    /// Corrective storage category.
    pub kind: ArtifactKind,
    /// Corrective semantic role.
    pub role: ArtifactRole,
    /// Maximum produced payload bytes; external inputs use zero.
    pub maximum_bytes: u64,
}

/// Model selection interpreted only by this corrective capability's model port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPolicy {
    /// Use one exact logical model.
    Exact(ModelId),
    /// Prefer a compatible model from one backend.
    PreferredBackend(BackendId),
    /// Use any compatible admitted model.
    AnyCompatible,
}

/// Non-zero token limits forwarded to a model or validator port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenBudget {
    /// Maximum admitted input tokens.
    pub maximum_input_tokens: NonZeroU32,
    /// Maximum admitted output tokens.
    pub maximum_output_tokens: NonZeroU32,
}

impl TokenBudget {
    /// Creates a token budget from validated non-zero bounds.
    #[must_use]
    pub const fn new(maximum_input_tokens: NonZeroU32, maximum_output_tokens: NonZeroU32) -> Self {
        Self {
            maximum_input_tokens,
            maximum_output_tokens,
        }
    }
}

/// Supported model-backed corrective operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelOperation {
    /// Produce an initial draft.
    Draft,
    /// Review prior corrective artifacts.
    Review,
    /// Produce a revised result.
    Revise,
}

/// Supported typed validation operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationOperation {
    /// Run a compiler or equivalent structural checker.
    CompileCheck,
    /// Run a deterministic validator.
    Validate,
}

/// Operation interpreted by the bounded corrective executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrectiveOperation {
    /// Invoke the typed model port.
    Model {
        /// Corrective model operation.
        operation: ModelOperation,
        /// Model-selection policy.
        policy: ModelPolicy,
        /// Port-enforced token bounds.
        token_budget: TokenBudget,
    },
    /// Invoke the typed validator port.
    Validate {
        /// Validator operation.
        operation: ValidationOperation,
        /// Port-enforced token bounds.
        token_budget: TokenBudget,
    },
    /// Normalize a raw validation report without a port call.
    NormalizeDiagnostics,
}

/// Stable human-meaningful stage label carried by workflow data and events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkflowStage {
    /// Reference draft stage.
    Draft,
    /// Reference initial validation stage.
    InitialValidation,
    /// Reference diagnostic normalization stage.
    NormalizeDiagnostics,
    /// Reference review stage.
    Review,
    /// Reference revision stage.
    Revise,
    /// Reference terminal validation stage.
    FinalValidation,
    /// Definition-specific stage label.
    Other(u16),
}

/// Corrective metadata carried opaquely by the generic task graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorrectiveTask {
    /// Event-facing stage label.
    pub stage: WorkflowStage,
    /// Supported operation and its port policy.
    pub operation: CorrectiveOperation,
}

/// One corrective task node using generic graph mechanics.
pub type CorrectiveNode = TaskNode<CorrectiveTask>;

/// Definition-external artifact binding to a committed runtime artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowInputBinding {
    /// Definition-local external artifact identity.
    pub definition: ArtifactId,
    /// Committed runtime artifact identity.
    pub artifact: ArtifactId,
}

/// Borrowed, data-defined corrective workflow accepted by the executor.
#[derive(Clone, Copy, Debug)]
pub struct CorrectiveWorkflowDefinition<'a> {
    /// Corrective nodes in deterministic scheduling order.
    pub nodes: &'a [CorrectiveNode],
    /// Generic success dependencies.
    pub dependencies: &'a [TaskDependency],
    /// Corrective metadata for every external and produced artifact identity.
    pub artifacts: &'a [ArtifactDefinition],
    /// Definition-local external artifact identities.
    pub external_inputs: &'a [ArtifactId],
    /// Definition-local task input bindings.
    pub task_inputs: &'a [TaskArtifactInput],
    /// Definition-local task output provenance.
    pub task_outputs: &'a [TaskArtifactOutput],
    /// Definition-local terminal result artifact.
    pub terminal_result: ArtifactId,
    /// Definition-local terminal validation artifact.
    pub terminal_validation: ArtifactId,
}

impl CorrectiveWorkflowDefinition<'_> {
    /// Returns the generic graph view interpreted by graph mechanics.
    #[must_use]
    pub const fn graph(&self) -> TaskGraph<'_, CorrectiveTask> {
        TaskGraph::new(self.nodes, self.dependencies)
    }

    /// Returns the generic identity-only artifact flow.
    #[must_use]
    pub const fn artifact_flow(&self) -> ArtifactFlow<'_> {
        ArtifactFlow::new(self.external_inputs, self.task_inputs, self.task_outputs)
    }

    /// Returns corrective metadata for one definition-local artifact.
    #[must_use]
    pub fn artifact(&self, id: ArtifactId) -> Option<&ArtifactDefinition> {
        self.artifacts.iter().find(|artifact| artifact.id == id)
    }

    /// Validates graph mechanics, bounded shape, provenance, operations, and terminal artifacts.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowDefinitionError`] without invoking a model or validator port.
    pub fn validate(
        &self,
        shape: WorkflowShapeLimits,
        incoming_counts: &mut [u32],
        queue: &mut [usize],
    ) -> Result<(), WorkflowDefinitionError> {
        self.validate_shape(shape)?;
        let graph = self.graph();
        validate_graph(
            &graph,
            GraphValidationScratch {
                incoming_counts,
                queue,
            },
        )
        .map_err(WorkflowDefinitionError::TaskGraph)?;
        validate_artifact_flow(&graph, &self.artifact_flow())
            .map_err(WorkflowDefinitionError::TaskGraph)?;
        validate_artifact_definitions(self)?;
        validate_tasks(self)?;
        validate_terminal(self)
    }

    pub(crate) fn validate_shape(
        &self,
        shape: WorkflowShapeLimits,
    ) -> Result<(), WorkflowDefinitionError> {
        validate_shape(self, shape)
    }
}

/// Definition collection whose configured maximum was exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowDefinitionResource {
    /// Task nodes.
    Tasks,
    /// Dependency edges.
    Dependencies,
    /// Task input bindings.
    TaskInputs,
    /// Corrective artifact metadata declarations.
    Artifacts,
    /// Task output bindings.
    TaskOutputs,
    /// External input identities.
    ExternalInputs,
}

/// Stable corrective workflow-definition failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowDefinitionError {
    /// A corrective definition contains no tasks.
    Empty,
    /// A definition collection exceeds executor shape limits.
    CapacityExceeded {
        /// Bounded collection.
        resource: WorkflowDefinitionResource,
        /// Required entries.
        required: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// Generic topology, provenance, or validation scratch failed.
    TaskGraph(TaskGraphError),
    /// Two corrective metadata declarations share one artifact identity.
    DuplicateArtifactDefinition(ArtifactId),
    /// Graph flow references an artifact with no corrective metadata.
    UnknownArtifactDefinition(ArtifactId),
    /// Corrective metadata is not used by the graph flow.
    UnusedArtifactDefinition(ArtifactId),
    /// An external artifact is not a zero-limit text specification.
    InvalidExternalArtifact(ArtifactId),
    /// A corrective task does not declare exactly one output.
    InvalidTaskOutputCount {
        /// Definition-local task identity.
        task: TaskId,
        /// Number of declared outputs.
        outputs: usize,
    },
    /// A produced corrective artifact has no output capacity.
    ZeroOutputLimit(ArtifactId),
    /// The operation's output kind or role is inconsistent.
    OperationOutputMismatch(TaskId),
    /// Diagnostic normalization does not have exactly one raw-diagnostics input.
    InvalidNormalizationInput(TaskId),
    /// Deterministic normalization is configured for more than one attempt.
    InvalidNormalizationAttempts(TaskId),
    /// Terminal result is not a produced draft or revision text artifact.
    InvalidTerminalResult(ArtifactId),
    /// Terminal validation is not a produced final-validation report.
    InvalidTerminalValidation(ArtifactId),
}

fn validate_shape(
    definition: &CorrectiveWorkflowDefinition<'_>,
    shape: WorkflowShapeLimits,
) -> Result<(), WorkflowDefinitionError> {
    if definition.nodes.is_empty() {
        return Err(WorkflowDefinitionError::Empty);
    }
    for (resource, required, maximum) in [
        (
            WorkflowDefinitionResource::Tasks,
            definition.nodes.len(),
            shape.maximum_tasks().get(),
        ),
        (
            WorkflowDefinitionResource::Dependencies,
            definition.dependencies.len(),
            shape.maximum_dependencies(),
        ),
        (
            WorkflowDefinitionResource::TaskInputs,
            definition.task_inputs.len(),
            shape.maximum_task_inputs(),
        ),
        (
            WorkflowDefinitionResource::Artifacts,
            definition.artifacts.len(),
            shape.maximum_artifacts().get(),
        ),
        (
            WorkflowDefinitionResource::TaskOutputs,
            definition.task_outputs.len(),
            shape.maximum_artifacts().get(),
        ),
        (
            WorkflowDefinitionResource::ExternalInputs,
            definition.external_inputs.len(),
            shape.maximum_artifacts().get(),
        ),
    ] {
        if required > maximum {
            return Err(WorkflowDefinitionError::CapacityExceeded {
                resource,
                required,
                maximum,
            });
        }
    }
    Ok(())
}

fn validate_artifact_definitions(
    definition: &CorrectiveWorkflowDefinition<'_>,
) -> Result<(), WorkflowDefinitionError> {
    for (index, artifact) in definition.artifacts.iter().enumerate() {
        if definition
            .artifacts
            .get(index.saturating_add(1)..)
            .is_some_and(|tail| tail.iter().any(|other| other.id == artifact.id))
        {
            return Err(WorkflowDefinitionError::DuplicateArtifactDefinition(
                artifact.id,
            ));
        }
        let external = definition.external_inputs.contains(&artifact.id);
        let produced = definition
            .task_outputs
            .iter()
            .any(|output| output.artifact == artifact.id);
        if !external && !produced {
            return Err(WorkflowDefinitionError::UnusedArtifactDefinition(
                artifact.id,
            ));
        }
        if external
            && (artifact.kind != ArtifactKind::Text
                || artifact.role != ArtifactRole::Specification
                || artifact.maximum_bytes != 0)
        {
            return Err(WorkflowDefinitionError::InvalidExternalArtifact(
                artifact.id,
            ));
        }
        if produced && artifact.maximum_bytes == 0 {
            return Err(WorkflowDefinitionError::ZeroOutputLimit(artifact.id));
        }
    }
    for id in definition
        .external_inputs
        .iter()
        .copied()
        .chain(definition.task_outputs.iter().map(|output| output.artifact))
    {
        if definition.artifact(id).is_none() {
            return Err(WorkflowDefinitionError::UnknownArtifactDefinition(id));
        }
    }
    Ok(())
}

fn validate_tasks(
    definition: &CorrectiveWorkflowDefinition<'_>,
) -> Result<(), WorkflowDefinitionError> {
    for node in definition.nodes {
        let outputs = definition
            .task_outputs
            .iter()
            .filter(|output| output.producer == node.id)
            .count();
        if outputs != 1 {
            return Err(WorkflowDefinitionError::InvalidTaskOutputCount {
                task: node.id,
                outputs,
            });
        }
        let output_id = definition
            .task_outputs
            .iter()
            .find(|output| output.producer == node.id)
            .map(|output| output.artifact)
            .ok_or(WorkflowDefinitionError::InvalidTaskOutputCount {
                task: node.id,
                outputs,
            })?;
        let output = definition.artifact(output_id).ok_or(
            WorkflowDefinitionError::UnknownArtifactDefinition(output_id),
        )?;
        if !operation_matches_output(node.operation.operation, output) {
            return Err(WorkflowDefinitionError::OperationOutputMismatch(node.id));
        }
        if node.operation.operation == CorrectiveOperation::NormalizeDiagnostics {
            validate_normalization(definition, node)?;
        }
    }
    Ok(())
}

fn operation_matches_output(operation: CorrectiveOperation, output: &ArtifactDefinition) -> bool {
    match operation {
        CorrectiveOperation::Model { operation, .. } => {
            let role = match operation {
                ModelOperation::Draft => ArtifactRole::Draft,
                ModelOperation::Review => ArtifactRole::Review,
                ModelOperation::Revise => ArtifactRole::Revision,
            };
            output.kind == ArtifactKind::Text && output.role == role
        }
        CorrectiveOperation::Validate { .. } => {
            output.kind == ArtifactKind::Diagnostics
                && matches!(
                    output.role,
                    ArtifactRole::RawDiagnostics | ArtifactRole::FinalValidation
                )
        }
        CorrectiveOperation::NormalizeDiagnostics => {
            output.kind == ArtifactKind::Diagnostics
                && output.role == ArtifactRole::NormalizedDiagnostics
        }
    }
}

fn validate_normalization(
    definition: &CorrectiveWorkflowDefinition<'_>,
    node: &CorrectiveNode,
) -> Result<(), WorkflowDefinitionError> {
    if node.maximum_attempts != core::num::NonZeroU16::MIN {
        return Err(WorkflowDefinitionError::InvalidNormalizationAttempts(
            node.id,
        ));
    }
    let mut inputs = definition
        .task_inputs
        .iter()
        .filter(|input| input.consumer == node.id);
    let Some(input) = inputs.next() else {
        return Err(WorkflowDefinitionError::InvalidNormalizationInput(node.id));
    };
    if inputs.next().is_some()
        || definition.artifact(input.artifact).map(|value| value.role)
            != Some(ArtifactRole::RawDiagnostics)
    {
        return Err(WorkflowDefinitionError::InvalidNormalizationInput(node.id));
    }
    Ok(())
}

fn validate_terminal(
    definition: &CorrectiveWorkflowDefinition<'_>,
) -> Result<(), WorkflowDefinitionError> {
    let produced = |id| {
        definition
            .task_outputs
            .iter()
            .any(|output| output.artifact == id)
    };
    let result_valid = definition
        .artifact(definition.terminal_result)
        .is_some_and(|artifact| {
            produced(artifact.id)
                && artifact.kind == ArtifactKind::Text
                && matches!(artifact.role, ArtifactRole::Draft | ArtifactRole::Revision)
        });
    if !result_valid {
        return Err(WorkflowDefinitionError::InvalidTerminalResult(
            definition.terminal_result,
        ));
    }
    let validation_valid = definition
        .artifact(definition.terminal_validation)
        .is_some_and(|artifact| {
            produced(artifact.id)
                && artifact.kind == ArtifactKind::Diagnostics
                && artifact.role == ArtifactRole::FinalValidation
        });
    if validation_valid {
        Ok(())
    } else {
        Err(WorkflowDefinitionError::InvalidTerminalValidation(
            definition.terminal_validation,
        ))
    }
}
