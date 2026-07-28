//! Canonical corrective workflow construction and synchronous execution.

use std::collections::VecDeque;
use std::num::NonZeroU16;

use task_graph::{
    ArtifactFlow, GraphValidationScratch, TaskArtifactInput, TaskArtifactOutput, TaskDependency,
    TaskGraph, TaskNode, TaskOutputContract, TaskRuntimeState, TaskStateTable,
    validate_artifact_flow, validate_graph,
};

use super::{
    Artifact, ArtifactContent, ArtifactId, ArtifactInputs, ArtifactKind, ArtifactReference,
    ArtifactRole, ArtifactStore, CorrectiveWorkflowConfiguration, Diagnostic, DiagnosticLocation,
    ModelPolicy, ModelTaskExecutor, ModelTaskRequest, NormalizedValidationReport, RawDiagnostic,
    TaskAttempt, TaskId, TaskKind, ValidationReport, ValidationTaskExecutor, ValidationTaskRequest,
    ValidationVerdict, WorkflowError, WorkflowEvent, WorkflowExecutorLimits, WorkflowId,
    WorkflowIdentifierKind, WorkflowOutcome, WorkflowStage, WorkflowStatus,
    normalize_validation_report,
};

const TASK_COUNT: usize = 6;
const WORKFLOW_OUTPUT_COUNT: usize = TASK_COUNT;
const MODEL_TASK_COUNT: usize = 3;
const VALIDATION_TASK_COUNT: usize = 2;
const EVENTS_PER_RETRYABLE_ATTEMPT: usize = 2;
const NORMALIZATION_EVENT_COUNT: usize = 2;
const COMPLETION_EVENT_COUNT: usize = 1;
const NON_TOKENIZED_TOKEN_BOUND: u32 = 0;

/// Normalization does not tokenize, so token bounds are inapplicable and it runs once.
const NORMALIZATION_TASK_BUDGET: task_graph::TaskBudget = task_graph::TaskBudget::new(
    NON_TOKENIZED_TOKEN_BOUND,
    NON_TOKENIZED_TOKEN_BOUND,
    NonZeroU16::MIN,
);

/// Synchronous, statically dispatched owner of corrective workflow orchestration.
///
/// The executor is independent from the hosted model lifecycle. It owns its
/// ports, immutable artifact store, event queue, and checked identity sequences,
/// and can execute multiple workflows sequentially without identity reuse.
pub struct CorrectiveWorkflowExecutor<M, V> {
    model: M,
    validator: V,
    artifacts: ArtifactStore,
    events: VecDeque<WorkflowEvent>,
    limits: WorkflowExecutorLimits,
    next_workflow_id: u64,
    next_task_id: u64,
    next_artifact_id: u64,
}

impl<M, V> CorrectiveWorkflowExecutor<M, V> {
    /// Creates an executor with empty fixed-capacity artifact and event storage.
    #[must_use]
    pub const fn new(model: M, validator: V, limits: WorkflowExecutorLimits) -> Self {
        Self {
            model,
            validator,
            artifacts: ArtifactStore::new(limits.maximum_artifacts()),
            events: VecDeque::new(),
            limits,
            next_workflow_id: 1,
            next_task_id: 1,
            next_artifact_id: 1,
        }
    }

    /// Returns the immutable artifact store for application-side inspection.
    #[must_use]
    pub const fn artifacts(&self) -> &ArtifactStore {
        &self.artifacts
    }

    /// Allocates and commits one root specification artifact.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::SpecificationCapacityExceeded`] when the UTF-8
    /// payload exceeds its configured limit,
    /// [`WorkflowError::SpecificationSizeOverflow`] when its length cannot be
    /// represented, [`WorkflowError::IdentifierExhausted`] if artifact identity
    /// space is exhausted, or an artifact-store error if insertion fails.
    pub fn insert_specification(
        &mut self,
        specification: String,
    ) -> Result<ArtifactId, WorkflowError> {
        let required = u64::try_from(specification.len())
            .map_err(|_| WorkflowError::SpecificationSizeOverflow)?;
        let maximum = self.limits.maximum_specification_bytes().get();
        if required > maximum {
            return Err(WorkflowError::SpecificationCapacityExceeded { required, maximum });
        }
        let id = self.allocate_artifact_id()?;
        let reference = artifact_reference(id, ArtifactKind::Text, ArtifactRole::Specification);
        self.artifacts.insert(Artifact::new(
            reference,
            ArtifactContent::Specification(specification),
        )?)?;
        Ok(id)
    }

    /// Removes and returns the oldest pending identity-only workflow event.
    pub fn poll_event(&mut self) -> Option<WorkflowEvent> {
        self.events.pop_front()
    }

    fn remaining_event_capacity(&self) -> usize {
        let capacity = self.limits.maximum_pending_events().get();
        if self.events.len() >= capacity {
            0
        } else {
            capacity - self.events.len()
        }
    }

    fn ensure_event_capacity(&self) -> Result<(), WorkflowError> {
        let available = self.remaining_event_capacity();
        if available == 0 {
            Err(WorkflowError::EventCapacityExceeded {
                required: 1,
                available,
            })
        } else {
            Ok(())
        }
    }

    fn admit_execution(
        &self,
        configuration: &CorrectiveWorkflowConfiguration,
    ) -> Result<(), WorkflowError> {
        let artifact_capacity = self.artifacts.remaining_capacity();
        if artifact_capacity < WORKFLOW_OUTPUT_COUNT {
            return Err(WorkflowError::ArtifactCapacityExceeded {
                required: WORKFLOW_OUTPUT_COUNT,
                available: artifact_capacity,
            });
        }
        let required_events = required_event_capacity(configuration)?;
        let available_events = self.remaining_event_capacity();
        if required_events > available_events {
            return Err(WorkflowError::EventCapacityExceeded {
                required: required_events,
                available: available_events,
            });
        }
        Ok(())
    }

    fn enqueue_event(&mut self, event: WorkflowEvent) -> Result<(), WorkflowError> {
        self.ensure_event_capacity()?;
        self.events.push_back(event);
        Ok(())
    }

    fn allocate_workflow_id(&mut self) -> Result<WorkflowId, WorkflowError> {
        let value = allocate_id(&mut self.next_workflow_id, WorkflowIdentifierKind::Workflow)?;
        Ok(WorkflowId::new(value))
    }

    fn allocate_task_id(&mut self) -> Result<TaskId, WorkflowError> {
        let value = allocate_id(&mut self.next_task_id, WorkflowIdentifierKind::Task)?;
        Ok(TaskId::new(value))
    }

    fn allocate_artifact_id(&mut self) -> Result<ArtifactId, WorkflowError> {
        let value = allocate_id(&mut self.next_artifact_id, WorkflowIdentifierKind::Artifact)?;
        Ok(ArtifactId::new(value))
    }

    fn reserve_task_ids(&mut self) -> Result<TaskIdentities, WorkflowError> {
        Ok(TaskIdentities {
            draft: self.allocate_task_id()?,
            initial_validation: self.allocate_task_id()?,
            normalize: self.allocate_task_id()?,
            review: self.allocate_task_id()?,
            revise: self.allocate_task_id()?,
            final_validation: self.allocate_task_id()?,
        })
    }

    fn reserve_output_references(&mut self) -> Result<ArtifactReferences, WorkflowError> {
        Ok(ArtifactReferences {
            draft: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Text,
                ArtifactRole::Draft,
            ),
            raw_validation: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Diagnostics,
                ArtifactRole::RawDiagnostics,
            ),
            normalized_diagnostics: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Diagnostics,
                ArtifactRole::NormalizedDiagnostics,
            ),
            review: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Text,
                ArtifactRole::Review,
            ),
            revision: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Text,
                ArtifactRole::Revision,
            ),
            final_validation: artifact_reference(
                self.allocate_artifact_id()?,
                ArtifactKind::Diagnostics,
                ArtifactRole::FinalValidation,
            ),
        })
    }
}

impl<M, V> CorrectiveWorkflowExecutor<M, V>
where
    M: ModelTaskExecutor,
    V: ValidationTaskExecutor,
{
    /// Builds, validates, and synchronously executes the canonical six-task graph.
    ///
    /// A rejected validator verdict is committed as successful task output and
    /// does not stop execution. Port errors retry within the corresponding task
    /// budget. Every successful output is size-checked and committed before the
    /// graph attempt is marked successful.
    ///
    /// # Errors
    ///
    /// Returns a typed [`WorkflowError`] for invalid configuration or root input,
    /// checked identity exhaustion, graph/state failures, output capacity
    /// violations, invalid committed artifacts, or terminal port exhaustion.
    #[allow(clippy::too_many_lines)]
    pub fn execute(
        &mut self,
        specification_id: ArtifactId,
        configuration: CorrectiveWorkflowConfiguration,
    ) -> Result<WorkflowOutcome, WorkflowError> {
        configuration.validate()?;
        let specification = self.require_specification(specification_id)?;
        self.admit_execution(&configuration)?;
        let workflow = self.allocate_workflow_id()?;
        let tasks = self.reserve_task_ids()?;
        let artifacts = self.reserve_output_references()?;
        let plan = CanonicalPlan::new(specification, tasks, artifacts, configuration);
        let graph = TaskGraph::new(&plan.nodes, &plan.dependencies);
        let flow = ArtifactFlow::new(&plan.workflow_inputs, &plan.task_inputs, &plan.task_outputs);
        let mut incoming_counts = [0_u32; TASK_COUNT];
        let mut queue = [0_usize; TASK_COUNT];
        validate_graph(
            &graph,
            GraphValidationScratch {
                incoming_counts: &mut incoming_counts,
                queue: &mut queue,
            },
        )?;
        validate_artifact_flow(&graph, &flow)?;

        let mut states = [TaskRuntimeState::default(); TASK_COUNT];
        let mut state_table = TaskStateTable::new(&graph, &mut states)?;

        self.run_model_stage(
            workflow,
            WorkflowStage::Draft,
            tasks.draft,
            TaskKind::Draft,
            configuration.model_policy,
            configuration.model_budget,
            &[specification.id],
            artifacts.draft,
            configuration.artifact_limits.draft,
            &graph,
            &mut state_table,
            ArtifactContent::Draft,
        )?;
        self.run_validation_stage(
            workflow,
            WorkflowStage::InitialValidation,
            tasks.initial_validation,
            configuration.initial_validation,
            configuration.validation_budget,
            &[artifacts.draft.id],
            artifacts.raw_validation,
            configuration.artifact_limits.raw_validation,
            &graph,
            &mut state_table,
            ArtifactContent::RawValidation,
        )?;
        self.run_normalization_stage(
            workflow,
            tasks.normalize,
            artifacts.raw_validation.id,
            artifacts.normalized_diagnostics,
            configuration.artifact_limits.normalized_diagnostics,
            &graph,
            &mut state_table,
        )?;
        self.run_model_stage(
            workflow,
            WorkflowStage::Review,
            tasks.review,
            TaskKind::Review,
            configuration.model_policy,
            configuration.model_budget,
            &[
                specification.id,
                artifacts.draft.id,
                artifacts.normalized_diagnostics.id,
            ],
            artifacts.review,
            configuration.artifact_limits.review,
            &graph,
            &mut state_table,
            ArtifactContent::Review,
        )?;
        self.run_model_stage(
            workflow,
            WorkflowStage::Revise,
            tasks.revise,
            TaskKind::Revise,
            configuration.model_policy,
            configuration.model_budget,
            &[
                specification.id,
                artifacts.draft.id,
                artifacts.normalized_diagnostics.id,
                artifacts.review.id,
            ],
            artifacts.revision,
            configuration.artifact_limits.revision,
            &graph,
            &mut state_table,
            ArtifactContent::Revision,
        )?;
        let final_verdict = self.run_validation_stage(
            workflow,
            WorkflowStage::FinalValidation,
            tasks.final_validation,
            TaskKind::Validate,
            configuration.validation_budget,
            &[artifacts.revision.id],
            artifacts.final_validation,
            configuration.artifact_limits.final_validation,
            &graph,
            &mut state_table,
            ArtifactContent::FinalValidation,
        )?;

        let terminal_status = match final_verdict {
            ValidationVerdict::Passed => WorkflowStatus::Accepted,
            ValidationVerdict::Rejected => WorkflowStatus::Rejected,
        };
        let outcome = match terminal_status {
            WorkflowStatus::Accepted => WorkflowOutcome::Accepted {
                workflow,
                revision: artifacts.revision.id,
                validation: artifacts.final_validation.id,
            },
            WorkflowStatus::Rejected => WorkflowOutcome::Rejected {
                workflow,
                revision: artifacts.revision.id,
                validation: artifacts.final_validation.id,
            },
        };
        self.enqueue_event(WorkflowEvent::Completed {
            workflow,
            status: terminal_status,
            revision: artifacts.revision.id,
            validation: artifacts.final_validation.id,
        })?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_model_stage<F>(
        &mut self,
        workflow: WorkflowId,
        stage: WorkflowStage,
        task: TaskId,
        kind: TaskKind,
        model_policy: ModelPolicy,
        budget: task_graph::TaskBudget,
        input_artifacts: &[ArtifactId],
        output: ArtifactReference,
        maximum_bytes: u64,
        graph: &TaskGraph<'_>,
        state_table: &mut TaskStateTable<'_>,
        make_content: F,
    ) -> Result<(), WorkflowError>
    where
        F: Fn(String) -> ArtifactContent,
    {
        loop {
            self.ensure_event_capacity()?;
            let attempt = state_table.start(graph, task)?;
            self.enqueue_event(WorkflowEvent::StageStarted {
                workflow,
                stage,
                attempt,
            })?;
            let request = ModelTaskRequest {
                workflow,
                attempt,
                kind,
                model_policy,
                budget,
                input_artifacts,
            };
            let artifacts = ArtifactInputs::new(&self.artifacts, input_artifacts);
            match self.model.execute_model_task(request, &artifacts) {
                Ok(value) => {
                    self.commit_output(
                        workflow,
                        stage,
                        attempt,
                        output,
                        make_content(value),
                        maximum_bytes,
                    )?;
                    state_table.succeed_attempt(graph, attempt)?;
                    return Ok(());
                }
                Err(error) => {
                    self.handle_operational_failure(
                        workflow,
                        stage,
                        attempt,
                        budget,
                        error.to_string(),
                        graph,
                        state_table,
                    )?;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_validation_stage<F>(
        &mut self,
        workflow: WorkflowId,
        stage: WorkflowStage,
        task: TaskId,
        kind: TaskKind,
        budget: task_graph::TaskBudget,
        input_artifacts: &[ArtifactId],
        output: ArtifactReference,
        maximum_bytes: u64,
        graph: &TaskGraph<'_>,
        state_table: &mut TaskStateTable<'_>,
        make_content: F,
    ) -> Result<ValidationVerdict, WorkflowError>
    where
        F: Fn(ValidationReport) -> ArtifactContent,
    {
        loop {
            self.ensure_event_capacity()?;
            let attempt = state_table.start(graph, task)?;
            self.enqueue_event(WorkflowEvent::StageStarted {
                workflow,
                stage,
                attempt,
            })?;
            let request = ValidationTaskRequest {
                workflow,
                attempt,
                kind,
                budget,
                input_artifacts,
            };
            let artifacts = ArtifactInputs::new(&self.artifacts, input_artifacts);
            match self.validator.execute_validation_task(request, &artifacts) {
                Ok(report) => {
                    let verdict = report.verdict;
                    self.commit_output(
                        workflow,
                        stage,
                        attempt,
                        output,
                        make_content(report),
                        maximum_bytes,
                    )?;
                    state_table.succeed_attempt(graph, attempt)?;
                    return Ok(verdict);
                }
                Err(error) => {
                    self.handle_operational_failure(
                        workflow,
                        stage,
                        attempt,
                        budget,
                        error.to_string(),
                        graph,
                        state_table,
                    )?;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_normalization_stage(
        &mut self,
        workflow: WorkflowId,
        task: TaskId,
        input: ArtifactId,
        output: ArtifactReference,
        maximum_bytes: u64,
        graph: &TaskGraph<'_>,
        state_table: &mut TaskStateTable<'_>,
    ) -> Result<(), WorkflowError> {
        self.ensure_event_capacity()?;
        let attempt = state_table.start(graph, task)?;
        self.enqueue_event(WorkflowEvent::StageStarted {
            workflow,
            stage: WorkflowStage::NormalizeDiagnostics,
            attempt,
        })?;
        let report = self.require_raw_validation(input)?;
        let normalized = normalize_validation_report(report);
        self.commit_output(
            workflow,
            WorkflowStage::NormalizeDiagnostics,
            attempt,
            output,
            ArtifactContent::NormalizedDiagnostics(normalized),
            maximum_bytes,
        )?;
        state_table.succeed_attempt(graph, attempt)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_operational_failure(
        &mut self,
        workflow: WorkflowId,
        stage: WorkflowStage,
        attempt: TaskAttempt,
        budget: task_graph::TaskBudget,
        diagnostic: String,
        graph: &TaskGraph<'_>,
        state_table: &mut TaskStateTable<'_>,
    ) -> Result<(), WorkflowError> {
        state_table.fail_attempt(graph, attempt)?;
        if attempt.number.get() < budget.maximum_attempts.get() {
            let next_number = attempt
                .number
                .get()
                .checked_add(1)
                .and_then(NonZeroU16::new)
                .ok_or(WorkflowError::IdentifierExhausted(
                    WorkflowIdentifierKind::Task,
                ))?;
            self.enqueue_event(WorkflowEvent::RetryScheduled {
                workflow,
                stage,
                failed_attempt: attempt,
                next_attempt: TaskAttempt::new(attempt.task, next_number),
            })
        } else {
            Err(WorkflowError::TaskExhausted {
                workflow,
                stage,
                task: attempt.task,
                attempts: attempt.number.get(),
                diagnostic,
            })
        }
    }

    fn commit_output(
        &mut self,
        workflow: WorkflowId,
        stage: WorkflowStage,
        attempt: TaskAttempt,
        reference: ArtifactReference,
        content: ArtifactContent,
        maximum_bytes: u64,
    ) -> Result<(), WorkflowError> {
        let required =
            artifact_content_size(&content).ok_or(WorkflowError::ArtifactSizeOverflow {
                workflow,
                stage,
                task: attempt.task,
                artifact: reference.id,
            })?;
        if required > maximum_bytes {
            return Err(WorkflowError::OutputCapacityExceeded {
                workflow,
                stage,
                task: attempt.task,
                artifact: reference.id,
                required,
                maximum: maximum_bytes,
            });
        }
        self.ensure_event_capacity()?;
        self.artifacts.insert(Artifact::new(reference, content)?)?;
        self.enqueue_event(WorkflowEvent::ArtifactCommitted {
            workflow,
            stage,
            attempt,
            artifact: reference,
        })
    }

    fn require_specification(
        &self,
        specification_id: ArtifactId,
    ) -> Result<ArtifactReference, WorkflowError> {
        let artifact = self
            .artifacts
            .get(specification_id)
            .ok_or(WorkflowError::UnknownSpecification(specification_id))?;
        let reference = artifact.reference();
        if reference.kind != ArtifactKind::Text
            || reference.role != ArtifactRole::Specification
            || !matches!(artifact.content(), ArtifactContent::Specification(_))
        {
            return Err(WorkflowError::InvalidSpecification(reference));
        }
        Ok(reference)
    }

    fn require_raw_validation(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<&ValidationReport, WorkflowError> {
        let artifact =
            self.artifacts
                .get(artifact_id)
                .ok_or(WorkflowError::InvalidCommittedArtifact {
                    artifact: artifact_id,
                    expected_role: ArtifactRole::RawDiagnostics,
                })?;
        match artifact.content() {
            ArtifactContent::RawValidation(report) => Ok(report),
            _ => Err(WorkflowError::InvalidCommittedArtifact {
                artifact: artifact_id,
                expected_role: ArtifactRole::RawDiagnostics,
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct TaskIdentities {
    draft: TaskId,
    initial_validation: TaskId,
    normalize: TaskId,
    review: TaskId,
    revise: TaskId,
    final_validation: TaskId,
}

#[derive(Clone, Copy)]
struct ArtifactReferences {
    draft: ArtifactReference,
    raw_validation: ArtifactReference,
    normalized_diagnostics: ArtifactReference,
    review: ArtifactReference,
    revision: ArtifactReference,
    final_validation: ArtifactReference,
}

struct CanonicalPlan {
    nodes: [TaskNode; TASK_COUNT],
    dependencies: [TaskDependency; 8],
    workflow_inputs: [ArtifactReference; 1],
    task_inputs: [TaskArtifactInput; 11],
    task_outputs: [TaskArtifactOutput; TASK_COUNT],
}

impl CanonicalPlan {
    const fn new(
        specification: ArtifactReference,
        tasks: TaskIdentities,
        artifacts: ArtifactReferences,
        configuration: CorrectiveWorkflowConfiguration,
    ) -> Self {
        let nodes = [
            task_node(
                tasks.draft,
                TaskKind::Draft,
                configuration.model_policy,
                configuration.model_budget,
                ArtifactKind::Text,
                configuration.artifact_limits.draft,
            ),
            task_node(
                tasks.initial_validation,
                configuration.initial_validation,
                task_graph::ModelPolicy::Deterministic,
                configuration.validation_budget,
                ArtifactKind::Diagnostics,
                configuration.artifact_limits.raw_validation,
            ),
            task_node(
                tasks.normalize,
                TaskKind::NormalizeDiagnostics,
                task_graph::ModelPolicy::Deterministic,
                NORMALIZATION_TASK_BUDGET,
                ArtifactKind::Diagnostics,
                configuration.artifact_limits.normalized_diagnostics,
            ),
            task_node(
                tasks.review,
                TaskKind::Review,
                configuration.model_policy,
                configuration.model_budget,
                ArtifactKind::Text,
                configuration.artifact_limits.review,
            ),
            task_node(
                tasks.revise,
                TaskKind::Revise,
                configuration.model_policy,
                configuration.model_budget,
                ArtifactKind::Text,
                configuration.artifact_limits.revision,
            ),
            task_node(
                tasks.final_validation,
                TaskKind::Validate,
                task_graph::ModelPolicy::Deterministic,
                configuration.validation_budget,
                ArtifactKind::Diagnostics,
                configuration.artifact_limits.final_validation,
            ),
        ];
        let dependencies = [
            dependency(tasks.draft, tasks.initial_validation),
            dependency(tasks.initial_validation, tasks.normalize),
            dependency(tasks.draft, tasks.review),
            dependency(tasks.normalize, tasks.review),
            dependency(tasks.draft, tasks.revise),
            dependency(tasks.normalize, tasks.revise),
            dependency(tasks.review, tasks.revise),
            dependency(tasks.revise, tasks.final_validation),
        ];
        let task_inputs = [
            task_input(tasks.draft, specification),
            task_input(tasks.initial_validation, artifacts.draft),
            task_input(tasks.normalize, artifacts.raw_validation),
            task_input(tasks.review, specification),
            task_input(tasks.review, artifacts.draft),
            task_input(tasks.review, artifacts.normalized_diagnostics),
            task_input(tasks.revise, specification),
            task_input(tasks.revise, artifacts.draft),
            task_input(tasks.revise, artifacts.normalized_diagnostics),
            task_input(tasks.revise, artifacts.review),
            task_input(tasks.final_validation, artifacts.revision),
        ];
        let task_outputs = [
            task_output(tasks.draft, artifacts.draft),
            task_output(tasks.initial_validation, artifacts.raw_validation),
            task_output(tasks.normalize, artifacts.normalized_diagnostics),
            task_output(tasks.review, artifacts.review),
            task_output(tasks.revise, artifacts.revision),
            task_output(tasks.final_validation, artifacts.final_validation),
        ];
        Self {
            nodes,
            dependencies,
            workflow_inputs: [specification],
            task_inputs,
            task_outputs,
        }
    }
}

const fn task_node(
    id: TaskId,
    kind: TaskKind,
    model_policy: task_graph::ModelPolicy,
    budget: task_graph::TaskBudget,
    output_kind: ArtifactKind,
    maximum_bytes: u64,
) -> TaskNode {
    TaskNode {
        id,
        kind,
        model_policy,
        budget,
        output: TaskOutputContract {
            kind: output_kind,
            maximum_bytes,
        },
    }
}

const fn dependency(prerequisite: TaskId, dependent: TaskId) -> TaskDependency {
    TaskDependency {
        prerequisite,
        dependent,
    }
}

const fn task_input(consumer: TaskId, artifact: ArtifactReference) -> TaskArtifactInput {
    TaskArtifactInput { consumer, artifact }
}

const fn task_output(producer: TaskId, artifact: ArtifactReference) -> TaskArtifactOutput {
    TaskArtifactOutput { producer, artifact }
}

const fn artifact_reference(
    id: ArtifactId,
    kind: ArtifactKind,
    role: ArtifactRole,
) -> ArtifactReference {
    ArtifactReference { id, kind, role }
}

fn required_event_capacity(
    configuration: &CorrectiveWorkflowConfiguration,
) -> Result<usize, WorkflowError> {
    let model_events = retryable_task_event_capacity(
        configuration.model_budget.maximum_attempts,
        MODEL_TASK_COUNT,
    )?;
    let validation_events = retryable_task_event_capacity(
        configuration.validation_budget.maximum_attempts,
        VALIDATION_TASK_COUNT,
    )?;
    model_events
        .checked_add(validation_events)
        .and_then(|total| total.checked_add(NORMALIZATION_EVENT_COUNT))
        .and_then(|total| total.checked_add(COMPLETION_EVENT_COUNT))
        .ok_or(WorkflowError::EventCapacityOverflow)
}

fn retryable_task_event_capacity(
    maximum_attempts: NonZeroU16,
    task_count: usize,
) -> Result<usize, WorkflowError> {
    usize::from(maximum_attempts.get())
        .checked_mul(EVENTS_PER_RETRYABLE_ATTEMPT)
        .and_then(|per_task| per_task.checked_mul(task_count))
        .ok_or(WorkflowError::EventCapacityOverflow)
}

fn allocate_id(next: &mut u64, kind: WorkflowIdentifierKind) -> Result<u64, WorkflowError> {
    let current = *next;
    *next = current
        .checked_add(1)
        .ok_or(WorkflowError::IdentifierExhausted(kind))?;
    Ok(current)
}

fn artifact_content_size(content: &ArtifactContent) -> Option<u64> {
    match content {
        ArtifactContent::Specification(value)
        | ArtifactContent::Draft(value)
        | ArtifactContent::Review(value)
        | ArtifactContent::Revision(value) => string_size(value),
        ArtifactContent::RawValidation(report) | ArtifactContent::FinalValidation(report) => {
            raw_report_size(report)
        }
        ArtifactContent::NormalizedDiagnostics(report) => normalized_report_size(report),
    }
}

fn raw_report_size(report: &ValidationReport) -> Option<u64> {
    report
        .diagnostics
        .iter()
        .try_fold(1_u64, |total, diagnostic| {
            checked_add(total, raw_diagnostic_size(diagnostic)?)
        })
}

fn normalized_report_size(report: &NormalizedValidationReport) -> Option<u64> {
    report
        .diagnostics
        .iter()
        .try_fold(1_u64, |total, diagnostic| {
            checked_add(total, diagnostic_size(diagnostic)?)
        })
}

fn raw_diagnostic_size(diagnostic: &RawDiagnostic) -> Option<u64> {
    let total = checked_add(1, optional_string_size(diagnostic.code.as_deref())?)?;
    let total = checked_add(total, string_size(&diagnostic.message)?)?;
    checked_add(total, optional_location_size(diagnostic.location.as_ref())?)
}

fn diagnostic_size(diagnostic: &Diagnostic) -> Option<u64> {
    let total = checked_add(1, optional_string_size(diagnostic.code.as_deref())?)?;
    let total = checked_add(total, string_size(&diagnostic.message)?)?;
    checked_add(total, optional_location_size(diagnostic.location.as_ref())?)
}

fn optional_string_size(value: Option<&str>) -> Option<u64> {
    let payload = value.map_or(Some(0), string_size)?;
    checked_add(1, payload)
}

fn optional_location_size(location: Option<&DiagnosticLocation>) -> Option<u64> {
    let Some(location) = location else {
        return Some(1);
    };
    let total = checked_add(1, optional_string_size(location.path.as_deref())?)?;
    let total = checked_add(total, optional_u32_size(location.line))?;
    checked_add(total, optional_u32_size(location.column))
}

const fn optional_u32_size(value: Option<u32>) -> u64 {
    if value.is_some() { 5 } else { 1 }
}

fn string_size(value: &str) -> Option<u64> {
    u64::try_from(value.len()).ok()
}

const fn checked_add(left: u64, right: u64) -> Option<u64> {
    left.checked_add(right)
}
