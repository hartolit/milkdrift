//! Bounded synchronous reference engine for data-defined corrective workflows.
//!
//! This incubating capability engine is intentionally narrower than Milkdrift's
//! future general workflow runtime. It supports a small corrective operation set,
//! in-memory artifacts, static ports, and one synchronous bounded run.

#![forbid(unsafe_code)]

mod artifact;
mod definition;
mod diagnostics;
mod executor;
mod output;
mod reference;

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU64, NonZeroUsize};

pub use artifact::{Artifact, ArtifactContent, ArtifactContentKind, ArtifactInputs, ArtifactStore};
pub use definition::{
    ArtifactDefinition, ArtifactKind, ArtifactReference, ArtifactRole, CorrectiveNode,
    CorrectiveOperation, CorrectiveTask, CorrectiveWorkflowDefinition, ModelOperation, ModelPolicy,
    TokenBudget, ValidationOperation, WorkflowDefinitionError, WorkflowDefinitionResource,
    WorkflowInputBinding, WorkflowStage,
};
pub use diagnostics::{
    Diagnostic, DiagnosticLocation, DiagnosticSeverity, NormalizedValidationReport, RawDiagnostic,
    ValidationReport, ValidationVerdict, normalize_validation_report,
};
pub use domain_contracts::ArtifactId;
pub use executor::CorrectiveWorkflowExecutor;
pub use output::{BoundedDiagnosticsSink, BoundedTextSink, OutputSinkError};
pub use reference::{
    ReferenceArtifactLimits, ReferenceCorrectiveConfiguration, ReferenceCorrectiveTemplate,
};
pub use task_graph::{TaskAttempt, TaskGraphError, TaskId};

/// Stable identity of one corrective workflow execution.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkflowId(u64);

impl WorkflowId {
    /// Creates a workflow identity from its numeric representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stable numeric representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Terminal decision of a completed corrective workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowStatus {
    /// The configured terminal validation passed.
    Accepted,
    /// The configured terminal validation rejected the result.
    Rejected,
}

/// Identity-only terminal result of a corrective workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowOutcome {
    workflow: WorkflowId,
    status: WorkflowStatus,
    result: ArtifactId,
    validation: ArtifactId,
}

impl WorkflowOutcome {
    const fn new(
        workflow: WorkflowId,
        status: WorkflowStatus,
        result: ArtifactId,
        validation: ArtifactId,
    ) -> Self {
        Self {
            workflow,
            status,
            result,
            validation,
        }
    }

    /// Returns the workflow identity.
    #[must_use]
    pub const fn workflow(self) -> WorkflowId {
        self.workflow
    }

    /// Returns the terminal status.
    #[must_use]
    pub const fn status(self) -> WorkflowStatus {
        self.status
    }

    /// Returns the configured terminal result artifact identity.
    #[must_use]
    pub const fn result(self) -> ArtifactId {
        self.result
    }

    /// Returns the committed terminal-validation artifact identity.
    #[must_use]
    pub const fn validation(self) -> ArtifactId {
        self.validation
    }
}

/// Payload-free event emitted by the corrective executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowEvent {
    /// A graph task attempt started.
    StageStarted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Definition-owned stage label.
        stage: WorkflowStage,
        /// Run-unique attempt identity.
        attempt: TaskAttempt,
    },
    /// A task output was committed.
    ArtifactCommitted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Definition-owned stage label.
        stage: WorkflowStage,
        /// Run-unique attempt identity.
        attempt: TaskAttempt,
        /// Committed corrective artifact reference.
        artifact: ArtifactReference,
    },
    /// An operational port failure was admitted for another attempt.
    RetryScheduled {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Definition-owned stage label.
        stage: WorkflowStage,
        /// Failed run-unique attempt.
        failed_attempt: TaskAttempt,
        /// Next run-unique attempt.
        next_attempt: TaskAttempt,
    },
    /// A workflow reached its configured terminal validation.
    Completed {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Accepted or rejected status.
        status: WorkflowStatus,
        /// Committed terminal result artifact.
        result: ArtifactId,
        /// Committed terminal-validation artifact.
        validation: ArtifactId,
    },
}

impl WorkflowEvent {
    /// Returns the workflow that owns this event.
    #[must_use]
    pub const fn workflow(self) -> WorkflowId {
        match self {
            Self::StageStarted { workflow, .. }
            | Self::ArtifactCommitted { workflow, .. }
            | Self::RetryScheduled { workflow, .. }
            | Self::Completed { workflow, .. } => workflow,
        }
    }
}

/// Immutable model-attempt authority passed to a model port.
#[derive(Clone, Copy, Debug)]
pub struct ModelTaskContext<'a> {
    /// Workflow execution identity.
    pub workflow: WorkflowId,
    /// Run-unique attempt identity.
    pub attempt: TaskAttempt,
    /// Definition-local task identity.
    pub definition_task: TaskId,
    /// Supported model operation selected by workflow data.
    pub operation: ModelOperation,
    /// Validated model-selection policy selected by workflow data.
    pub model_policy: ModelPolicy,
    /// Non-zero token bounds enforced by the concrete port.
    pub token_budget: TokenBudget,
    /// Resolver restricted to the task's declared committed inputs.
    pub artifacts: ArtifactInputs<'a>,
}

/// Coarse capability boundary for model-backed corrective operations.
pub trait ModelTaskExecutor {
    /// Owned or borrowed implementation error convertible to a stable diagnostic.
    type Error: Display;

    /// Executes one supported model operation into executor-owned bounded output.
    ///
    /// # Errors
    ///
    /// Returns an implementation-defined operational failure eligible for retry.
    fn execute_model_task(
        &mut self,
        context: ModelTaskContext<'_>,
        output: &mut BoundedTextSink,
    ) -> Result<(), Self::Error>;
}

/// Immutable validator-attempt authority passed to a validator port.
#[derive(Clone, Copy, Debug)]
pub struct ValidationTaskContext<'a> {
    /// Workflow execution identity.
    pub workflow: WorkflowId,
    /// Run-unique attempt identity.
    pub attempt: TaskAttempt,
    /// Definition-local task identity.
    pub definition_task: TaskId,
    /// Supported validation operation selected by workflow data.
    pub operation: ValidationOperation,
    /// Non-zero token bounds enforced by the concrete port.
    pub token_budget: TokenBudget,
    /// Resolver restricted to the task's declared committed inputs.
    pub artifacts: ArtifactInputs<'a>,
}

/// Coarse capability boundary for typed deterministic validators.
pub trait ValidationTaskExecutor {
    /// Owned or borrowed implementation error convertible to a stable diagnostic.
    type Error: Display;

    /// Executes one validation operation into executor-owned bounded output.
    ///
    /// A rejected verdict is a successful operation rather than a port failure.
    ///
    /// # Errors
    ///
    /// Returns an implementation-defined operational failure eligible for retry.
    fn execute_validation_task(
        &mut self,
        context: ValidationTaskContext<'_>,
        output: &mut BoundedDiagnosticsSink,
    ) -> Result<ValidationVerdict, Self::Error>;
}

/// Read-only cancellation query issued before one task attempt starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancellationRequest {
    /// Workflow being executed.
    pub workflow: WorkflowId,
    /// Run-unique task identity that would start next.
    pub task: TaskId,
    /// Definition-owned stage label.
    pub stage: WorkflowStage,
}

/// Bounded workflow-definition shape accepted by an executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowShapeLimits {
    tasks: NonZeroUsize,
    dependencies: usize,
    artifacts: NonZeroUsize,
    task_inputs: usize,
}

impl WorkflowShapeLimits {
    /// Creates shape limits. Dependency and task-input limits may be zero.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidExecutorLimits`] when the task or
    /// definition-artifact maximum is zero.
    pub fn new(
        maximum_tasks: usize,
        maximum_dependencies: usize,
        maximum_artifacts: usize,
        maximum_task_inputs: usize,
    ) -> Result<Self, WorkflowError> {
        let tasks = NonZeroUsize::new(maximum_tasks).ok_or(
            WorkflowError::InvalidExecutorLimits(WorkflowExecutorLimitError::ZeroMaximumTasks),
        )?;
        let artifacts =
            NonZeroUsize::new(maximum_artifacts).ok_or(WorkflowError::InvalidExecutorLimits(
                WorkflowExecutorLimitError::ZeroMaximumDefinitionArtifacts,
            ))?;
        Ok(Self {
            tasks,
            dependencies: maximum_dependencies,
            artifacts,
            task_inputs: maximum_task_inputs,
        })
    }

    /// Returns the maximum accepted task count.
    #[must_use]
    pub const fn maximum_tasks(self) -> NonZeroUsize {
        self.tasks
    }

    /// Returns the maximum accepted dependency count.
    #[must_use]
    pub const fn maximum_dependencies(self) -> usize {
        self.dependencies
    }

    /// Returns the maximum accepted definition artifact count.
    #[must_use]
    pub const fn maximum_artifacts(self) -> NonZeroUsize {
        self.artifacts
    }

    /// Returns the maximum accepted task-input binding count.
    #[must_use]
    pub const fn maximum_task_inputs(self) -> usize {
        self.task_inputs
    }
}

/// Validated aggregate storage and workflow-shape bounds for one executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowExecutorLimits {
    artifacts: NonZeroUsize,
    pending_events: NonZeroUsize,
    specification_bytes: NonZeroU64,
    shape: WorkflowShapeLimits,
}

impl WorkflowExecutorLimits {
    /// Validates aggregate executor bounds.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidExecutorLimits`] when a required bound is zero.
    pub fn new(
        maximum_artifacts: usize,
        maximum_pending_events: usize,
        maximum_specification_bytes: u64,
        shape: WorkflowShapeLimits,
    ) -> Result<Self, WorkflowError> {
        let artifacts = NonZeroUsize::new(maximum_artifacts).ok_or(
            WorkflowError::InvalidExecutorLimits(WorkflowExecutorLimitError::ZeroMaximumArtifacts),
        )?;
        let pending_events = NonZeroUsize::new(maximum_pending_events).ok_or(
            WorkflowError::InvalidExecutorLimits(
                WorkflowExecutorLimitError::ZeroMaximumPendingEvents,
            ),
        )?;
        let specification_bytes = NonZeroU64::new(maximum_specification_bytes).ok_or(
            WorkflowError::InvalidExecutorLimits(
                WorkflowExecutorLimitError::ZeroMaximumSpecificationBytes,
            ),
        )?;
        Ok(Self {
            artifacts,
            pending_events,
            specification_bytes,
            shape,
        })
    }

    /// Returns fixed artifact-store capacity.
    #[must_use]
    pub const fn maximum_artifacts(self) -> NonZeroUsize {
        self.artifacts
    }

    /// Returns fixed pending-event capacity.
    #[must_use]
    pub const fn maximum_pending_events(self) -> NonZeroUsize {
        self.pending_events
    }

    /// Returns maximum accepted root-specification bytes.
    #[must_use]
    pub const fn maximum_specification_bytes(self) -> NonZeroU64 {
        self.specification_bytes
    }

    /// Returns accepted workflow-definition shape.
    #[must_use]
    pub const fn shape(self) -> WorkflowShapeLimits {
        self.shape
    }
}

/// Stable aggregate executor-limit validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowExecutorLimitError {
    /// Artifact storage capacity is zero.
    ZeroMaximumArtifacts,
    /// Pending event capacity is zero.
    ZeroMaximumPendingEvents,
    /// Root specification byte capacity is zero.
    ZeroMaximumSpecificationBytes,
    /// Accepted task count is zero.
    ZeroMaximumTasks,
    /// Accepted definition artifact count is zero.
    ZeroMaximumDefinitionArtifacts,
}

/// Identifier sequence that exhausted checked allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowIdentifierKind {
    /// Workflow identity sequence.
    Workflow,
    /// Run-unique task identity sequence.
    Task,
    /// Artifact identity sequence.
    Artifact,
}

/// Fallible executor-owned run preparation resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunAllocationResource {
    /// Run-unique task mapping.
    Tasks,
    /// Run artifact mapping.
    Artifacts,
    /// Per-task input identity mapping.
    TaskInputs,
    /// Generic graph state and ready-task scratch.
    GraphState,
}

/// Typed synchronous corrective workflow failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowError {
    /// Workflow data is invalid or exceeds configured shape bounds.
    InvalidDefinition(WorkflowDefinitionError),
    /// Aggregate executor storage limits are invalid.
    InvalidExecutorLimits(WorkflowExecutorLimitError),
    /// A required external definition artifact has no binding.
    MissingWorkflowInput(ArtifactId),
    /// An input binding names no external definition artifact.
    UnknownWorkflowInput(ArtifactId),
    /// An external definition artifact was bound more than once.
    DuplicateWorkflowInputBinding(ArtifactId),
    /// A bound artifact is missing or has incompatible corrective metadata.
    InvalidWorkflowInput {
        /// Definition-local external artifact identity.
        definition: ArtifactId,
        /// Bound runtime artifact identity.
        artifact: ArtifactId,
    },
    /// A checked identity sequence cannot allocate another value.
    IdentifierExhausted(WorkflowIdentifierKind),
    /// Fallible bounded run preparation could not reserve storage.
    RunAllocationFailed(RunAllocationResource),
    /// An artifact reference does not agree with its typed payload.
    ArtifactContentMismatch {
        /// Rejected artifact reference.
        reference: ArtifactReference,
        /// Actual payload discriminator.
        content: ArtifactContentKind,
    },
    /// Artifact ownership does not match root/generated lifecycle rules.
    ArtifactOwnershipMismatch {
        /// Rejected artifact reference.
        reference: ArtifactReference,
        /// Supplied workflow owner.
        owner: Option<WorkflowId>,
    },
    /// An immutable artifact identity was already committed.
    DuplicateArtifact(ArtifactId),
    /// Fixed artifact storage has insufficient entry capacity.
    ArtifactCapacityExceeded {
        /// Entries required by the operation.
        required: usize,
        /// Entries currently available.
        available: usize,
    },
    /// Root specification exceeds its configured UTF-8 byte limit.
    SpecificationCapacityExceeded {
        /// Required UTF-8 bytes.
        required: u64,
        /// Configured maximum UTF-8 bytes.
        maximum: u64,
    },
    /// Root specification byte count cannot be represented as `u64`.
    SpecificationSizeOverflow,
    /// Fixed pending-event storage has insufficient capacity.
    EventCapacityExceeded {
        /// Entries required by the operation.
        required: usize,
        /// Entries currently available.
        available: usize,
    },
    /// Worst-case event admission arithmetic overflowed `usize`.
    EventCapacityOverflow,
    /// Generic graph or graph-state validation failed.
    TaskGraph(TaskGraphError),
    /// Structured payload byte accounting overflowed.
    ArtifactSizeOverflow {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing run-unique task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
    },
    /// A completed task exceeded its declared output limit.
    OutputCapacityExceeded {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing run-unique task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
        /// Accounted payload bytes.
        required: u64,
        /// Declared maximum payload bytes.
        maximum: u64,
    },
    /// Fallible reservation for contract-admissible output failed.
    OutputAllocationFailed {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing run-unique task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
        /// Accounted bytes requiring storage.
        required: u64,
    },
    /// An operational port failure exhausted the task attempt budget.
    TaskExhausted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Failed stage.
        stage: WorkflowStage,
        /// Failed run-unique task identity.
        task: TaskId,
        /// Attempts executed.
        attempts: u16,
        /// Owned display diagnostic from the final port failure.
        diagnostic: String,
    },
    /// Cancellation was observed before a task attempt started.
    Cancelled {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Cancelled stage.
        stage: WorkflowStage,
        /// Cancelled run-unique task identity.
        task: TaskId,
    },
    /// A committed prerequisite artifact was unavailable or mistyped.
    InvalidCommittedArtifact {
        /// Required artifact identity.
        artifact: ArtifactId,
        /// Required corrective semantic role.
        expected_role: ArtifactRole,
    },
    /// All graph tasks succeeded but terminal artifacts were unavailable.
    InvalidTerminalArtifact(ArtifactId),
}

impl From<TaskGraphError> for WorkflowError {
    fn from(value: TaskGraphError) -> Self {
        Self::TaskGraph(value)
    }
}

impl Display for WorkflowError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            error @ (Self::InvalidDefinition(_)
            | Self::InvalidExecutorLimits(_)
            | Self::MissingWorkflowInput(_)
            | Self::UnknownWorkflowInput(_)
            | Self::DuplicateWorkflowInputBinding(_)
            | Self::InvalidWorkflowInput { .. }
            | Self::IdentifierExhausted(_)
            | Self::RunAllocationFailed(_)
            | Self::ArtifactContentMismatch { .. }
            | Self::ArtifactOwnershipMismatch { .. }
            | Self::DuplicateArtifact(_)
            | Self::ArtifactCapacityExceeded { .. }
            | Self::SpecificationCapacityExceeded { .. }
            | Self::SpecificationSizeOverflow
            | Self::EventCapacityExceeded { .. }
            | Self::EventCapacityOverflow
            | Self::TaskGraph(_)) => format_admission_error(error, formatter),
            error => format_execution_error(error, formatter),
        }
    }
}

fn format_admission_error(error: &WorkflowError, formatter: &mut Formatter<'_>) -> fmt::Result {
    match error {
        WorkflowError::InvalidDefinition(value) => {
            write!(formatter, "invalid workflow definition: {value:?}")
        }
        WorkflowError::InvalidExecutorLimits(value) => {
            write!(formatter, "invalid executor limits: {value:?}")
        }
        WorkflowError::MissingWorkflowInput(id) => {
            write!(formatter, "definition input {} is unbound", id.get())
        }
        WorkflowError::UnknownWorkflowInput(id) => {
            write!(formatter, "definition input {} is unknown", id.get())
        }
        WorkflowError::DuplicateWorkflowInputBinding(id) => write!(
            formatter,
            "definition input {} is bound more than once",
            id.get()
        ),
        WorkflowError::InvalidWorkflowInput {
            definition,
            artifact,
        } => write!(
            formatter,
            "definition input {} has invalid artifact {}",
            definition.get(),
            artifact.get()
        ),
        WorkflowError::IdentifierExhausted(kind) => {
            write!(formatter, "{kind:?} identifier sequence exhausted")
        }
        WorkflowError::RunAllocationFailed(resource) => {
            write!(formatter, "run preparation could not reserve {resource:?}")
        }
        WorkflowError::ArtifactContentMismatch { reference, content } => write!(
            formatter,
            "artifact {} reference does not match {content:?} content",
            reference.id.get()
        ),
        WorkflowError::ArtifactOwnershipMismatch { reference, owner } => write!(
            formatter,
            "artifact {} has invalid owner {owner:?}",
            reference.id.get()
        ),
        WorkflowError::DuplicateArtifact(id) => {
            write!(formatter, "artifact {} is already committed", id.get())
        }
        WorkflowError::ArtifactCapacityExceeded {
            required,
            available,
        } => write!(
            formatter,
            "artifact operation requires {required} entries but {available} are available"
        ),
        WorkflowError::SpecificationCapacityExceeded { required, maximum } => write!(
            formatter,
            "specification requires {required} bytes but permits {maximum}"
        ),
        WorkflowError::SpecificationSizeOverflow => {
            formatter.write_str("specification byte count cannot be represented")
        }
        WorkflowError::EventCapacityExceeded {
            required,
            available,
        } => write!(
            formatter,
            "event operation requires {required} entries but {available} are available"
        ),
        WorkflowError::EventCapacityOverflow => {
            formatter.write_str("worst-case workflow event count overflowed")
        }
        WorkflowError::TaskGraph(value) => write!(formatter, "task graph failure: {value:?}"),
        _ => formatter.write_str("workflow admission failed"),
    }
}

fn format_execution_error(error: &WorkflowError, formatter: &mut Formatter<'_>) -> fmt::Result {
    match error {
        WorkflowError::ArtifactSizeOverflow {
            workflow,
            stage,
            artifact,
            ..
        } => write!(
            formatter,
            "workflow {} {stage:?} artifact {} size overflowed",
            workflow.get(),
            artifact.get()
        ),
        WorkflowError::OutputCapacityExceeded {
            workflow,
            stage,
            artifact,
            required,
            maximum,
            ..
        } => write!(
            formatter,
            "workflow {} {stage:?} artifact {} requires {required} bytes but permits {maximum}",
            workflow.get(),
            artifact.get()
        ),
        WorkflowError::OutputAllocationFailed {
            workflow,
            stage,
            artifact,
            required,
            ..
        } => write!(
            formatter,
            "workflow {} {stage:?} artifact {} could not reserve {required} bytes",
            workflow.get(),
            artifact.get()
        ),
        WorkflowError::TaskExhausted {
            workflow,
            stage,
            task,
            attempts,
            diagnostic,
        } => write!(
            formatter,
            "workflow {} {stage:?} task {} exhausted after {attempts} attempts: {diagnostic}",
            workflow.get(),
            task.get()
        ),
        WorkflowError::Cancelled {
            workflow,
            stage,
            task,
        } => write!(
            formatter,
            "workflow {} {stage:?} task {} was cancelled",
            workflow.get(),
            task.get()
        ),
        WorkflowError::InvalidCommittedArtifact {
            artifact,
            expected_role,
        } => write!(
            formatter,
            "artifact {} is not committed as {expected_role:?}",
            artifact.get()
        ),
        WorkflowError::InvalidTerminalArtifact(artifact) => write!(
            formatter,
            "terminal artifact {} is unavailable or invalid",
            artifact.get()
        ),
        _ => formatter.write_str("workflow execution failed"),
    }
}

impl Error for WorkflowError {}
