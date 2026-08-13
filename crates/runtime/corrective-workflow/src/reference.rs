//! The current six-stage behavior expressed as ordinary corrective workflow data.

use std::num::{NonZeroU16, NonZeroU32};

use domain_contracts::ArtifactId;
use task_graph::{TaskArtifactInput, TaskArtifactOutput, TaskDependency, TaskId, TaskNode};

use super::{
    ArtifactDefinition, ArtifactKind, ArtifactRole, CorrectiveOperation, CorrectiveTask,
    CorrectiveWorkflowDefinition, ModelOperation, ModelPolicy, TokenBudget, ValidationOperation,
    WorkflowDefinitionError, WorkflowError, WorkflowStage,
};

const SPECIFICATION: ArtifactId = ArtifactId::new(1);
const DRAFT: ArtifactId = ArtifactId::new(2);
const RAW_VALIDATION: ArtifactId = ArtifactId::new(3);
const NORMALIZED_DIAGNOSTICS: ArtifactId = ArtifactId::new(4);
const REVIEW: ArtifactId = ArtifactId::new(5);
const REVISION: ArtifactId = ArtifactId::new(6);
const FINAL_VALIDATION: ArtifactId = ArtifactId::new(7);

const DRAFT_TASK: TaskId = TaskId::new(1);
const INITIAL_VALIDATION_TASK: TaskId = TaskId::new(2);
const NORMALIZE_TASK: TaskId = TaskId::new(3);
const REVIEW_TASK: TaskId = TaskId::new(4);
const REVISE_TASK: TaskId = TaskId::new(5);
const FINAL_VALIDATION_TASK: TaskId = TaskId::new(6);

/// Per-artifact byte limits for the six-stage reference template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceArtifactLimits {
    /// Draft text bytes.
    pub draft: u64,
    /// Raw validation report bytes.
    pub raw_validation: u64,
    /// Normalized diagnostics bytes.
    pub normalized_diagnostics: u64,
    /// Review text bytes.
    pub review: u64,
    /// Revision text bytes.
    pub revision: u64,
    /// Final validation report bytes.
    pub final_validation: u64,
}

/// Policy and bounds used to construct the six-stage reference template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceCorrectiveConfiguration {
    /// Initial validation operation.
    pub initial_validation: ValidationOperation,
    /// Model policy for draft, review, and revision.
    pub model_policy: ModelPolicy,
    /// Token budget for model operations.
    pub model_token_budget: TokenBudget,
    /// Token budget for validation operations.
    pub validation_token_budget: TokenBudget,
    /// Maximum attempts for each model operation.
    pub model_attempts: NonZeroU16,
    /// Maximum attempts for each validator operation.
    pub validation_attempts: NonZeroU16,
    /// Produced artifact byte limits.
    pub artifact_limits: ReferenceArtifactLimits,
}

impl Default for ReferenceCorrectiveConfiguration {
    fn default() -> Self {
        Self {
            initial_validation: ValidationOperation::CompileCheck,
            model_policy: ModelPolicy::AnyCompatible,
            model_token_budget: TokenBudget::new(
                NonZeroU32::new(2_048).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(512).unwrap_or(NonZeroU32::MIN),
            ),
            validation_token_budget: TokenBudget::new(
                NonZeroU32::new(2_048).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(256).unwrap_or(NonZeroU32::MIN),
            ),
            model_attempts: NonZeroU16::MIN,
            validation_attempts: NonZeroU16::MIN,
            artifact_limits: ReferenceArtifactLimits {
                draft: 4_096,
                raw_validation: 4_096,
                normalized_diagnostics: 4_096,
                review: 4_096,
                revision: 4_096,
                final_validation: 4_096,
            },
        }
    }
}

/// Owned six-stage reference template.
///
/// [`Self::definition`] returns the same borrowed definition type accepted by
/// the generic corrective executor; the scheduler contains no six-stage branch.
pub struct ReferenceCorrectiveTemplate {
    nodes: [TaskNode<CorrectiveTask>; 6],
    dependencies: [TaskDependency; 8],
    artifacts: [ArtifactDefinition; 7],
    external_inputs: [ArtifactId; 1],
    task_inputs: [TaskArtifactInput; 11],
    task_outputs: [TaskArtifactOutput; 6],
}

impl ReferenceCorrectiveTemplate {
    /// Constructs the reference template as data.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidDefinition`] when a produced artifact has
    /// a zero byte limit.
    pub fn new(configuration: ReferenceCorrectiveConfiguration) -> Result<Self, WorkflowError> {
        let limits = configuration.artifact_limits;
        validate_limits(limits)?;
        Ok(Self {
            nodes: reference_nodes(configuration),
            dependencies: reference_dependencies(),
            artifacts: reference_artifacts(limits),
            external_inputs: [SPECIFICATION],
            task_inputs: reference_inputs(),
            task_outputs: reference_outputs(),
        })
    }

    /// Borrows this template through the executor's ordinary definition path.
    #[must_use]
    pub const fn definition(&self) -> CorrectiveWorkflowDefinition<'_> {
        CorrectiveWorkflowDefinition {
            nodes: &self.nodes,
            dependencies: &self.dependencies,
            artifacts: &self.artifacts,
            external_inputs: &self.external_inputs,
            task_inputs: &self.task_inputs,
            task_outputs: &self.task_outputs,
            terminal_result: REVISION,
            terminal_validation: FINAL_VALIDATION,
        }
    }

    /// Returns the definition-local specification identity used for input binding.
    #[must_use]
    pub const fn specification_input(&self) -> ArtifactId {
        SPECIFICATION
    }
}

fn validate_limits(limits: ReferenceArtifactLimits) -> Result<(), WorkflowError> {
    for (artifact, maximum) in [
        (DRAFT, limits.draft),
        (RAW_VALIDATION, limits.raw_validation),
        (NORMALIZED_DIAGNOSTICS, limits.normalized_diagnostics),
        (REVIEW, limits.review),
        (REVISION, limits.revision),
        (FINAL_VALIDATION, limits.final_validation),
    ] {
        if maximum == 0 {
            return Err(WorkflowError::InvalidDefinition(
                WorkflowDefinitionError::ZeroOutputLimit(artifact),
            ));
        }
    }

    Ok(())
}

fn reference_nodes(
    configuration: ReferenceCorrectiveConfiguration,
) -> [TaskNode<CorrectiveTask>; 6] {
    let model = |operation| CorrectiveOperation::Model {
        operation,
        policy: configuration.model_policy,
        token_budget: configuration.model_token_budget,
    };
    let validate = |operation| CorrectiveOperation::Validate {
        operation,
        token_budget: configuration.validation_token_budget,
    };
    [
        node(
            DRAFT_TASK,
            WorkflowStage::Draft,
            model(ModelOperation::Draft),
            configuration.model_attempts,
        ),
        node(
            INITIAL_VALIDATION_TASK,
            WorkflowStage::InitialValidation,
            validate(configuration.initial_validation),
            configuration.validation_attempts,
        ),
        node(
            NORMALIZE_TASK,
            WorkflowStage::NormalizeDiagnostics,
            CorrectiveOperation::NormalizeDiagnostics,
            NonZeroU16::MIN,
        ),
        node(
            REVIEW_TASK,
            WorkflowStage::Review,
            model(ModelOperation::Review),
            configuration.model_attempts,
        ),
        node(
            REVISE_TASK,
            WorkflowStage::Revise,
            model(ModelOperation::Revise),
            configuration.model_attempts,
        ),
        node(
            FINAL_VALIDATION_TASK,
            WorkflowStage::FinalValidation,
            validate(ValidationOperation::Validate),
            configuration.validation_attempts,
        ),
    ]
}

const fn reference_dependencies() -> [TaskDependency; 8] {
    [
        dependency(DRAFT_TASK, INITIAL_VALIDATION_TASK),
        dependency(INITIAL_VALIDATION_TASK, NORMALIZE_TASK),
        dependency(DRAFT_TASK, REVIEW_TASK),
        dependency(NORMALIZE_TASK, REVIEW_TASK),
        dependency(DRAFT_TASK, REVISE_TASK),
        dependency(NORMALIZE_TASK, REVISE_TASK),
        dependency(REVIEW_TASK, REVISE_TASK),
        dependency(REVISE_TASK, FINAL_VALIDATION_TASK),
    ]
}

const fn reference_artifacts(limits: ReferenceArtifactLimits) -> [ArtifactDefinition; 7] {
    [
        artifact(
            SPECIFICATION,
            ArtifactKind::Text,
            ArtifactRole::Specification,
            0,
        ),
        artifact(DRAFT, ArtifactKind::Text, ArtifactRole::Draft, limits.draft),
        artifact(
            RAW_VALIDATION,
            ArtifactKind::Diagnostics,
            ArtifactRole::RawDiagnostics,
            limits.raw_validation,
        ),
        artifact(
            NORMALIZED_DIAGNOSTICS,
            ArtifactKind::Diagnostics,
            ArtifactRole::NormalizedDiagnostics,
            limits.normalized_diagnostics,
        ),
        artifact(
            REVIEW,
            ArtifactKind::Text,
            ArtifactRole::Review,
            limits.review,
        ),
        artifact(
            REVISION,
            ArtifactKind::Text,
            ArtifactRole::Revision,
            limits.revision,
        ),
        artifact(
            FINAL_VALIDATION,
            ArtifactKind::Diagnostics,
            ArtifactRole::FinalValidation,
            limits.final_validation,
        ),
    ]
}

const fn reference_inputs() -> [TaskArtifactInput; 11] {
    [
        input(DRAFT_TASK, SPECIFICATION),
        input(INITIAL_VALIDATION_TASK, DRAFT),
        input(NORMALIZE_TASK, RAW_VALIDATION),
        input(REVIEW_TASK, SPECIFICATION),
        input(REVIEW_TASK, DRAFT),
        input(REVIEW_TASK, NORMALIZED_DIAGNOSTICS),
        input(REVISE_TASK, SPECIFICATION),
        input(REVISE_TASK, DRAFT),
        input(REVISE_TASK, NORMALIZED_DIAGNOSTICS),
        input(REVISE_TASK, REVIEW),
        input(FINAL_VALIDATION_TASK, REVISION),
    ]
}

const fn reference_outputs() -> [TaskArtifactOutput; 6] {
    [
        output(DRAFT_TASK, DRAFT),
        output(INITIAL_VALIDATION_TASK, RAW_VALIDATION),
        output(NORMALIZE_TASK, NORMALIZED_DIAGNOSTICS),
        output(REVIEW_TASK, REVIEW),
        output(REVISE_TASK, REVISION),
        output(FINAL_VALIDATION_TASK, FINAL_VALIDATION),
    ]
}

const fn node(
    id: TaskId,
    stage: WorkflowStage,
    operation: CorrectiveOperation,
    maximum_attempts: NonZeroU16,
) -> TaskNode<CorrectiveTask> {
    TaskNode {
        id,
        operation: CorrectiveTask { stage, operation },
        maximum_attempts,
    }
}

const fn dependency(prerequisite: TaskId, dependent: TaskId) -> TaskDependency {
    TaskDependency {
        prerequisite,
        dependent,
    }
}

const fn artifact(
    id: ArtifactId,
    kind: ArtifactKind,
    role: ArtifactRole,
    maximum_bytes: u64,
) -> ArtifactDefinition {
    ArtifactDefinition {
        id,
        kind,
        role,
        maximum_bytes,
    }
}

const fn input(consumer: TaskId, artifact: ArtifactId) -> TaskArtifactInput {
    TaskArtifactInput { consumer, artifact }
}

const fn output(producer: TaskId, artifact: ArtifactId) -> TaskArtifactOutput {
    TaskArtifactOutput { producer, artifact }
}
