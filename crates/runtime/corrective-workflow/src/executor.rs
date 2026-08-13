//! Validated data-driven corrective scheduling and checked state transitions.

use std::collections::VecDeque;
use std::num::NonZeroU16;

use domain_contracts::ArtifactId;
use task_graph::{TaskAttempt, TaskId, TaskRuntimeState, TaskStateTable, TaskStatus};

use super::output::{normalized_report_size, raw_report_size};
use super::{
    Artifact, ArtifactContent, ArtifactInputs, ArtifactKind, ArtifactReference, ArtifactRole,
    ArtifactStore, BoundedDiagnosticsSink, BoundedTextSink, CancellationRequest,
    CorrectiveOperation, CorrectiveTask, CorrectiveWorkflowDefinition, ModelOperation,
    ModelTaskContext, ModelTaskExecutor, OutputSinkError, ReferenceCorrectiveConfiguration,
    ReferenceCorrectiveTemplate, RunAllocationResource, ValidationOperation, ValidationTaskContext,
    ValidationTaskExecutor, ValidationVerdict, WorkflowError, WorkflowEvent,
    WorkflowExecutorLimits, WorkflowId, WorkflowIdentifierKind, WorkflowInputBinding,
    WorkflowOutcome, WorkflowStage, WorkflowStatus,
    diagnostics::normalize_validation_report_bounded,
};

const EVENTS_PER_MAXIMUM_ATTEMPT: usize = 2;
const COMPLETION_EVENT_COUNT: usize = 1;

/// Synchronous owner of bounded corrective ports, artifacts, events, and IDs.
///
/// The executor validates borrowed workflow data before invoking either port.
/// Ready tasks are selected in generic graph node order and dispatched only by
/// their declared supported corrective operation.
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

    /// Returns immutable committed artifacts for application-side inspection.
    #[must_use]
    pub const fn artifacts(&self) -> &ArtifactStore {
        &self.artifacts
    }

    /// Releases generated artifacts owned by one workflow.
    pub fn release_workflow(&mut self, workflow: WorkflowId) -> usize {
        self.artifacts.remove_owned_by(workflow)
    }

    /// Allocates and commits one shared root specification.
    ///
    /// # Errors
    ///
    /// Returns a typed capacity, allocation, identity, or artifact-store error.
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
        let reference = ArtifactReference {
            id,
            kind: ArtifactKind::Text,
            role: ArtifactRole::Specification,
        };
        self.artifacts.insert(Artifact::new(
            reference,
            ArtifactContent::Specification(specification),
        )?)?;
        Ok(id)
    }

    /// Removes and returns the oldest pending identity-only event.
    pub fn poll_event(&mut self) -> Option<WorkflowEvent> {
        self.events.pop_front()
    }

    fn remaining_event_capacity(&self) -> usize {
        self.limits
            .maximum_pending_events()
            .get()
            .saturating_sub(self.events.len())
    }

    fn enqueue_event(&mut self, event: WorkflowEvent) -> Result<(), WorkflowError> {
        let available = self.remaining_event_capacity();
        if available == 0 {
            return Err(WorkflowError::EventCapacityExceeded {
                required: 1,
                available,
            });
        }
        self.events.push_back(event);
        Ok(())
    }

    fn rollback_failed_workflow(&mut self, workflow: WorkflowId) {
        self.artifacts.remove_owned_by(workflow);
        self.events.retain(|event| event.workflow() != workflow);
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
}

impl<M, V> CorrectiveWorkflowExecutor<M, V>
where
    M: ModelTaskExecutor,
    V: ValidationTaskExecutor,
{
    /// Executes one validated borrowed corrective definition.
    ///
    /// # Errors
    ///
    /// Returns a typed preflight, graph, output, port, cancellation, or terminal
    /// failure. Failures after workflow allocation roll back artifacts and events
    /// without rewinding workflow, task, or artifact identity sequences.
    pub fn execute(
        &mut self,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
    ) -> Result<WorkflowOutcome, WorkflowError> {
        self.execute_with_cancellation(definition, inputs, |_| false)
    }

    /// Executes the six-stage reference template through [`Self::execute`].
    ///
    /// # Errors
    ///
    /// Returns the same typed failures as reference construction and execution.
    pub fn execute_reference(
        &mut self,
        specification: ArtifactId,
        configuration: ReferenceCorrectiveConfiguration,
    ) -> Result<WorkflowOutcome, WorkflowError> {
        let template = ReferenceCorrectiveTemplate::new(configuration)?;
        let inputs = [WorkflowInputBinding {
            definition: template.specification_input(),
            artifact: specification,
        }];
        self.execute(&template.definition(), &inputs)
    }

    /// Executes a definition while checking cancellation before every attempt.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::Cancelled`] after marking the selected generic
    /// task cancelled and its pending descendants blocked. The workflow's
    /// artifacts and events are then rolled back transactionally.
    pub fn execute_with_cancellation<F>(
        &mut self,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
        mut is_cancelled: F,
    ) -> Result<WorkflowOutcome, WorkflowError>
    where
        F: FnMut(CancellationRequest) -> bool,
    {
        self.preflight(definition, inputs)?;
        let workflow = self.allocate_workflow_id()?;
        let result = self.execute_allocated(workflow, definition, inputs, &mut is_cancelled);
        if result.is_err() {
            self.rollback_failed_workflow(workflow);
        }
        result
    }

    fn preflight(
        &self,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
    ) -> Result<(), WorkflowError> {
        definition
            .validate_shape(self.limits.shape())
            .map_err(WorkflowError::InvalidDefinition)?;
        let node_count = definition.nodes.len();
        let mut incoming =
            reserved_default_vec(node_count, RunAllocationResource::GraphState, 0_u32)?;
        let mut queue =
            reserved_default_vec(node_count, RunAllocationResource::GraphState, 0_usize)?;
        definition
            .validate(self.limits.shape(), &mut incoming, &mut queue)
            .map_err(WorkflowError::InvalidDefinition)?;
        self.validate_input_bindings(definition, inputs)?;

        let output_count = definition.task_outputs.len();
        let artifact_capacity = self.artifacts.remaining_capacity();
        if output_count > artifact_capacity {
            return Err(WorkflowError::ArtifactCapacityExceeded {
                required: output_count,
                available: artifact_capacity,
            });
        }
        let required_events = required_event_capacity(definition)?;
        let available_events = self.remaining_event_capacity();
        if required_events > available_events {
            return Err(WorkflowError::EventCapacityExceeded {
                required: required_events,
                available: available_events,
            });
        }
        Ok(())
    }

    fn validate_input_bindings(
        &self,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
    ) -> Result<(), WorkflowError> {
        for binding in inputs {
            if !definition.external_inputs.contains(&binding.definition) {
                return Err(WorkflowError::UnknownWorkflowInput(binding.definition));
            }
            if inputs
                .iter()
                .filter(|other| other.definition == binding.definition)
                .count()
                != 1
            {
                return Err(WorkflowError::DuplicateWorkflowInputBinding(
                    binding.definition,
                ));
            }
            let expected = definition
                .artifact(binding.definition)
                .ok_or(WorkflowError::UnknownWorkflowInput(binding.definition))?;
            let Some(actual) = self.artifacts.get(binding.artifact) else {
                return Err(WorkflowError::InvalidWorkflowInput {
                    definition: binding.definition,
                    artifact: binding.artifact,
                });
            };
            let reference = actual.reference();
            if reference.kind != expected.kind || reference.role != expected.role {
                return Err(WorkflowError::InvalidWorkflowInput {
                    definition: binding.definition,
                    artifact: binding.artifact,
                });
            }
        }
        for external in definition.external_inputs {
            if !inputs.iter().any(|binding| binding.definition == *external) {
                return Err(WorkflowError::MissingWorkflowInput(*external));
            }
        }
        Ok(())
    }

    fn execute_allocated<F>(
        &mut self,
        workflow: WorkflowId,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
        is_cancelled: &mut F,
    ) -> Result<WorkflowOutcome, WorkflowError>
    where
        F: FnMut(CancellationRequest) -> bool,
    {
        let mut run = self.prepare_run(definition, inputs)?;
        let graph = definition.graph();

        loop {
            let ready_count = {
                let table = TaskStateTable::new(&graph, &mut run.states)?;
                table.ready_tasks(&graph, &mut run.ready)?
            };
            if ready_count == 0 {
                break;
            }
            let definition_task =
                run.ready
                    .first()
                    .copied()
                    .ok_or(WorkflowError::RunAllocationFailed(
                        RunAllocationResource::GraphState,
                    ))?;
            let index = graph
                .node_index(definition_task)
                .ok_or(task_graph::TaskGraphError::UnknownTask(definition_task))?;
            let task = run
                .tasks
                .get(index)
                .ok_or(WorkflowError::RunAllocationFailed(
                    RunAllocationResource::Tasks,
                ))?;
            let cancellation = CancellationRequest {
                workflow,
                task: task.runtime_id,
                stage: task.definition.stage,
            };
            if is_cancelled(cancellation) {
                let mut table = TaskStateTable::new(&graph, &mut run.states)?;
                table.cancel(&graph, definition_task)?;
                table.propagate_blocked(&graph)?;
                return Err(WorkflowError::Cancelled {
                    workflow,
                    stage: task.definition.stage,
                    task: task.runtime_id,
                });
            }
            self.execute_ready_task(workflow, definition, &mut run, index)?;
        }

        if !run
            .states
            .iter()
            .all(|state| state.status == TaskStatus::Succeeded)
        {
            return Err(WorkflowError::InvalidTerminalArtifact(
                definition.terminal_validation,
            ));
        }
        self.terminalize(workflow, definition, &run)
    }

    fn prepare_run(
        &mut self,
        definition: &CorrectiveWorkflowDefinition<'_>,
        inputs: &[WorkflowInputBinding],
    ) -> Result<PreparedRun, WorkflowError> {
        let mut outputs = reserved_vec(
            definition.task_outputs.len(),
            RunAllocationResource::Artifacts,
        )?;
        for output in definition.task_outputs {
            let metadata = definition
                .artifact(output.artifact)
                .ok_or(WorkflowError::InvalidTerminalArtifact(output.artifact))?;
            outputs.push(RuntimeArtifact {
                definition_id: output.artifact,
                reference: ArtifactReference {
                    id: self.allocate_artifact_id()?,
                    kind: metadata.kind,
                    role: metadata.role,
                },
            });
        }

        let mut tasks = reserved_vec(definition.nodes.len(), RunAllocationResource::Tasks)?;
        for node in definition.nodes {
            let output_definition = definition
                .task_outputs
                .iter()
                .find(|output| output.producer == node.id)
                .map(|output| output.artifact)
                .ok_or(WorkflowError::InvalidTerminalArtifact(
                    definition.terminal_result,
                ))?;
            let output = runtime_artifact(&outputs, output_definition)
                .ok_or(WorkflowError::InvalidTerminalArtifact(output_definition))?;
            let input_count = definition
                .task_inputs
                .iter()
                .filter(|input| input.consumer == node.id)
                .count();
            let mut task_inputs = reserved_vec(input_count, RunAllocationResource::TaskInputs)?;
            for input in definition
                .task_inputs
                .iter()
                .filter(|input| input.consumer == node.id)
            {
                task_inputs.push(resolve_runtime_artifact(inputs, &outputs, input.artifact)?);
            }
            tasks.push(RuntimeTask {
                definition: node.operation,
                definition_id: node.id,
                runtime_id: self.allocate_task_id()?,
                output: output.reference,
                maximum_bytes: definition
                    .artifact(output_definition)
                    .map(|artifact| artifact.maximum_bytes)
                    .ok_or(WorkflowError::InvalidTerminalArtifact(output_definition))?,
                inputs: task_inputs,
            });
        }
        let states = reserved_default_vec(
            definition.nodes.len(),
            RunAllocationResource::GraphState,
            TaskRuntimeState::default(),
        )?;
        let ready = reserved_default_vec(
            definition.nodes.len(),
            RunAllocationResource::GraphState,
            TaskId::new(0),
        )?;
        Ok(PreparedRun {
            outputs,
            tasks,
            states,
            ready,
        })
    }

    fn execute_ready_task(
        &mut self,
        workflow: WorkflowId,
        definition: &CorrectiveWorkflowDefinition<'_>,
        run: &mut PreparedRun,
        index: usize,
    ) -> Result<(), WorkflowError> {
        let graph = definition.graph();
        let task = run
            .tasks
            .get(index)
            .ok_or(WorkflowError::RunAllocationFailed(
                RunAllocationResource::Tasks,
            ))?;
        let local_attempt = {
            let mut table = TaskStateTable::new(&graph, &mut run.states)?;
            table.start(&graph, task.definition_id)?
        };
        let attempt = TaskAttempt::new(task.runtime_id, local_attempt.number);
        self.enqueue_event(WorkflowEvent::StageStarted {
            workflow,
            stage: task.definition.stage,
            attempt,
        })?;

        let attempt_spec = AttemptSpec {
            workflow,
            definition_task: task.definition_id,
            attempt,
            definition: task.definition,
            inputs: &task.inputs,
            output: task.output,
            maximum_bytes: task.maximum_bytes,
        };
        match self.dispatch_attempt(attempt_spec)? {
            AttemptResult::Succeeded(content) => {
                self.commit_output(CommitSpec {
                    workflow,
                    stage: task.definition.stage,
                    attempt,
                    reference: task.output,
                    maximum_bytes: task.maximum_bytes,
                    content,
                })?;
                let mut table = TaskStateTable::new(&graph, &mut run.states)?;
                table.succeed_attempt(&graph, local_attempt)?;
                Ok(())
            }
            AttemptResult::OperationalFailure(diagnostic) => self.handle_operational_failure(
                OperationalFailure {
                    workflow,
                    stage: task.definition.stage,
                    local_attempt,
                    runtime_attempt: attempt,
                    diagnostic,
                },
                &graph,
                &mut run.states,
            ),
        }
    }

    fn dispatch_attempt(&mut self, spec: AttemptSpec<'_>) -> Result<AttemptResult, WorkflowError> {
        match spec.definition.operation {
            CorrectiveOperation::Model {
                operation,
                policy,
                token_budget,
            } => self.execute_model_attempt(spec, operation, policy, token_budget),
            CorrectiveOperation::Validate {
                operation,
                token_budget,
            } => self.execute_validation_attempt(spec, operation, token_budget),
            CorrectiveOperation::NormalizeDiagnostics => self.execute_normalization(spec),
        }
    }

    fn execute_model_attempt(
        &mut self,
        spec: AttemptSpec<'_>,
        operation: ModelOperation,
        policy: super::ModelPolicy,
        token_budget: super::TokenBudget,
    ) -> Result<AttemptResult, WorkflowError> {
        let artifacts = ArtifactInputs::new(&self.artifacts, spec.inputs);
        let context = ModelTaskContext {
            workflow: spec.workflow,
            attempt: spec.attempt,
            definition_task: spec.definition_task,
            operation,
            model_policy: policy,
            token_budget,
            artifacts,
        };
        let mut output = BoundedTextSink::new(spec.maximum_bytes);
        let port_result = self.model.execute_model_task(context, &mut output);
        if let Some(failure) = output.failure() {
            return Err(output_sink_error(spec.error_context(), failure));
        }
        match port_result {
            Ok(()) => {
                let text = output
                    .finish()
                    .map_err(|failure| output_sink_error(spec.error_context(), failure))?;
                Ok(AttemptResult::Succeeded(match operation {
                    ModelOperation::Draft => ArtifactContent::Draft(text),
                    ModelOperation::Review => ArtifactContent::Review(text),
                    ModelOperation::Revise => ArtifactContent::Revision(text),
                }))
            }
            Err(error) => Ok(AttemptResult::OperationalFailure(error.to_string())),
        }
    }

    fn execute_validation_attempt(
        &mut self,
        spec: AttemptSpec<'_>,
        operation: ValidationOperation,
        token_budget: super::TokenBudget,
    ) -> Result<AttemptResult, WorkflowError> {
        let artifacts = ArtifactInputs::new(&self.artifacts, spec.inputs);
        let context = ValidationTaskContext {
            workflow: spec.workflow,
            attempt: spec.attempt,
            definition_task: spec.definition_task,
            operation,
            token_budget,
            artifacts,
        };
        let mut output = BoundedDiagnosticsSink::new(spec.maximum_bytes)
            .map_err(|failure| output_sink_error(spec.error_context(), failure))?;
        let port_result = self.validator.execute_validation_task(context, &mut output);
        if let Some(failure) = output.failure() {
            return Err(output_sink_error(spec.error_context(), failure));
        }
        match port_result {
            Ok(verdict) => {
                let report = output
                    .finish(verdict)
                    .map_err(|failure| output_sink_error(spec.error_context(), failure))?;
                let artifact_content = if spec.output.role == ArtifactRole::FinalValidation {
                    ArtifactContent::FinalValidation(report)
                } else {
                    ArtifactContent::RawValidation(report)
                };
                Ok(AttemptResult::Succeeded(artifact_content))
            }
            Err(error) => Ok(AttemptResult::OperationalFailure(error.to_string())),
        }
    }

    fn execute_normalization(&self, spec: AttemptSpec<'_>) -> Result<AttemptResult, WorkflowError> {
        let input =
            spec.inputs
                .first()
                .copied()
                .ok_or(WorkflowError::InvalidCommittedArtifact {
                    artifact: spec.output.id,
                    expected_role: ArtifactRole::RawDiagnostics,
                })?;
        let report = self.require_raw_validation(input)?;
        let normalized = normalize_validation_report_bounded(report, spec.maximum_bytes)
            .map_err(|failure| output_sink_error(spec.error_context(), failure))?;
        Ok(AttemptResult::Succeeded(
            ArtifactContent::NormalizedDiagnostics(normalized),
        ))
    }

    fn handle_operational_failure(
        &mut self,
        failure: OperationalFailure,
        graph: &task_graph::TaskGraph<'_, CorrectiveTask>,
        states: &mut [TaskRuntimeState],
    ) -> Result<(), WorkflowError> {
        let task_status = {
            let mut table = TaskStateTable::new(graph, states)?;
            table.fail_attempt(graph, failure.local_attempt)?;
            table
                .state(graph, failure.local_attempt.task)
                .map(|state| state.status)
                .ok_or(task_graph::TaskGraphError::UnknownTask(
                    failure.local_attempt.task,
                ))?
        };
        if task_status == TaskStatus::Failed {
            let next_number = failure
                .runtime_attempt
                .number
                .get()
                .checked_add(1)
                .and_then(NonZeroU16::new)
                .ok_or(WorkflowError::IdentifierExhausted(
                    WorkflowIdentifierKind::Task,
                ))?;
            self.enqueue_event(WorkflowEvent::RetryScheduled {
                workflow: failure.workflow,
                stage: failure.stage,
                failed_attempt: failure.runtime_attempt,
                next_attempt: TaskAttempt::new(failure.runtime_attempt.task, next_number),
            })
        } else {
            let mut table = TaskStateTable::new(graph, states)?;
            table.propagate_blocked(graph)?;
            Err(WorkflowError::TaskExhausted {
                workflow: failure.workflow,
                stage: failure.stage,
                task: failure.runtime_attempt.task,
                attempts: failure.runtime_attempt.number.get(),
                diagnostic: failure.diagnostic,
            })
        }
    }

    fn commit_output(&mut self, spec: CommitSpec) -> Result<(), WorkflowError> {
        let required =
            artifact_content_size(&spec.content).ok_or(WorkflowError::ArtifactSizeOverflow {
                workflow: spec.workflow,
                stage: spec.stage,
                task: spec.attempt.task,
                artifact: spec.reference.id,
            })?;
        if required > spec.maximum_bytes {
            return Err(WorkflowError::OutputCapacityExceeded {
                workflow: spec.workflow,
                stage: spec.stage,
                task: spec.attempt.task,
                artifact: spec.reference.id,
                required,
                maximum: spec.maximum_bytes,
            });
        }
        self.artifacts.insert(Artifact::new_owned(
            spec.reference,
            spec.workflow,
            spec.content,
        )?)?;
        self.enqueue_event(WorkflowEvent::ArtifactCommitted {
            workflow: spec.workflow,
            stage: spec.stage,
            attempt: spec.attempt,
            artifact: spec.reference,
        })
    }

    fn terminalize(
        &mut self,
        workflow: WorkflowId,
        definition: &CorrectiveWorkflowDefinition<'_>,
        run: &PreparedRun,
    ) -> Result<WorkflowOutcome, WorkflowError> {
        let result = run.runtime_artifact(definition.terminal_result).ok_or(
            WorkflowError::InvalidTerminalArtifact(definition.terminal_result),
        )?;
        let validation = run.runtime_artifact(definition.terminal_validation).ok_or(
            WorkflowError::InvalidTerminalArtifact(definition.terminal_validation),
        )?;
        let status = match self.artifacts.get(validation).map(Artifact::content) {
            Some(ArtifactContent::FinalValidation(report)) => match report.verdict {
                ValidationVerdict::Passed => WorkflowStatus::Accepted,
                ValidationVerdict::Rejected => WorkflowStatus::Rejected,
            },
            _ => return Err(WorkflowError::InvalidTerminalArtifact(validation)),
        };
        self.enqueue_event(WorkflowEvent::Completed {
            workflow,
            status,
            result,
            validation,
        })?;
        Ok(WorkflowOutcome::new(workflow, status, result, validation))
    }

    fn require_raw_validation(
        &self,
        artifact_id: ArtifactId,
    ) -> Result<&super::ValidationReport, WorkflowError> {
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

struct PreparedRun {
    outputs: Vec<RuntimeArtifact>,
    tasks: Vec<RuntimeTask>,
    states: Vec<TaskRuntimeState>,
    ready: Vec<TaskId>,
}

impl PreparedRun {
    fn runtime_artifact(&self, definition_id: ArtifactId) -> Option<ArtifactId> {
        runtime_artifact(&self.outputs, definition_id).map(|value| value.reference.id)
    }
}

struct RuntimeArtifact {
    definition_id: ArtifactId,
    reference: ArtifactReference,
}

struct RuntimeTask {
    definition: CorrectiveTask,
    definition_id: TaskId,
    runtime_id: TaskId,
    output: ArtifactReference,
    maximum_bytes: u64,
    inputs: Vec<ArtifactId>,
}

#[derive(Clone, Copy)]
struct AttemptSpec<'a> {
    workflow: WorkflowId,
    definition_task: TaskId,
    attempt: TaskAttempt,
    definition: CorrectiveTask,
    inputs: &'a [ArtifactId],
    output: ArtifactReference,
    maximum_bytes: u64,
}

impl AttemptSpec<'_> {
    const fn error_context(self) -> OutputErrorContext {
        OutputErrorContext {
            workflow: self.workflow,
            stage: self.definition.stage,
            task: self.attempt.task,
            artifact: self.output.id,
        }
    }
}

enum AttemptResult {
    Succeeded(ArtifactContent),
    OperationalFailure(String),
}

struct OperationalFailure {
    workflow: WorkflowId,
    stage: WorkflowStage,
    local_attempt: TaskAttempt,
    runtime_attempt: TaskAttempt,
    diagnostic: String,
}

struct CommitSpec {
    workflow: WorkflowId,
    stage: WorkflowStage,
    attempt: TaskAttempt,
    reference: ArtifactReference,
    maximum_bytes: u64,
    content: ArtifactContent,
}

#[derive(Clone, Copy)]
struct OutputErrorContext {
    workflow: WorkflowId,
    stage: WorkflowStage,
    task: TaskId,
    artifact: ArtifactId,
}

fn runtime_artifact(
    outputs: &[RuntimeArtifact],
    definition_id: ArtifactId,
) -> Option<&RuntimeArtifact> {
    outputs
        .iter()
        .find(|artifact| artifact.definition_id == definition_id)
}

fn resolve_runtime_artifact(
    inputs: &[WorkflowInputBinding],
    outputs: &[RuntimeArtifact],
    definition_id: ArtifactId,
) -> Result<ArtifactId, WorkflowError> {
    if let Some(binding) = inputs
        .iter()
        .find(|binding| binding.definition == definition_id)
    {
        return Ok(binding.artifact);
    }
    runtime_artifact(outputs, definition_id)
        .map(|artifact| artifact.reference.id)
        .ok_or(WorkflowError::InvalidTerminalArtifact(definition_id))
}

fn required_event_capacity(
    definition: &CorrectiveWorkflowDefinition<'_>,
) -> Result<usize, WorkflowError> {
    definition
        .nodes
        .iter()
        .try_fold(0_usize, |total, node| {
            usize::from(node.maximum_attempts.get())
                .checked_mul(EVENTS_PER_MAXIMUM_ATTEMPT)
                .and_then(|node_events| total.checked_add(node_events))
        })
        .and_then(|total| total.checked_add(COMPLETION_EVENT_COUNT))
        .ok_or(WorkflowError::EventCapacityOverflow)
}

fn allocate_id(next: &mut u64, kind: WorkflowIdentifierKind) -> Result<u64, WorkflowError> {
    let current = *next;
    *next = current
        .checked_add(1)
        .ok_or(WorkflowError::IdentifierExhausted(kind))?;
    Ok(current)
}

fn reserved_vec<T>(
    capacity: usize,
    resource: RunAllocationResource,
) -> Result<Vec<T>, WorkflowError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| WorkflowError::RunAllocationFailed(resource))?;
    Ok(values)
}

fn reserved_default_vec<T: Clone>(
    capacity: usize,
    resource: RunAllocationResource,
    value: T,
) -> Result<Vec<T>, WorkflowError> {
    let mut values = reserved_vec(capacity, resource)?;
    values.resize(capacity, value);
    Ok(values)
}

const fn output_sink_error(context: OutputErrorContext, failure: OutputSinkError) -> WorkflowError {
    match failure {
        OutputSinkError::CapacityExceeded { required, maximum } => {
            WorkflowError::OutputCapacityExceeded {
                workflow: context.workflow,
                stage: context.stage,
                task: context.task,
                artifact: context.artifact,
                required,
                maximum,
            }
        }
        OutputSinkError::SizeOverflow => WorkflowError::ArtifactSizeOverflow {
            workflow: context.workflow,
            stage: context.stage,
            task: context.task,
            artifact: context.artifact,
        },
        OutputSinkError::AllocationFailed { required } => WorkflowError::OutputAllocationFailed {
            workflow: context.workflow,
            stage: context.stage,
            task: context.task,
            artifact: context.artifact,
            required,
        },
    }
}

fn artifact_content_size(content: &ArtifactContent) -> Option<u64> {
    match content {
        ArtifactContent::Specification(value)
        | ArtifactContent::Draft(value)
        | ArtifactContent::Review(value)
        | ArtifactContent::Revision(value) => u64::try_from(value.len()).ok(),
        ArtifactContent::RawValidation(report) | ArtifactContent::FinalValidation(report) => {
            raw_report_size(report)
        }
        ArtifactContent::NormalizedDiagnostics(report) => normalized_report_size(report),
    }
}
