//! Synchronous frontend-neutral corrective workflow orchestration.

#![forbid(unsafe_code)]

mod artifact;
mod diagnostics;
mod executor;
mod output;

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU64, NonZeroUsize};

pub use artifact::{Artifact, ArtifactContent, ArtifactContentKind, ArtifactInputs, ArtifactStore};
pub use diagnostics::{
    Diagnostic, DiagnosticLocation, DiagnosticSeverity, NormalizedValidationReport, RawDiagnostic,
    ValidationReport, ValidationVerdict, normalize_validation_report,
};
pub use domain_contracts::ArtifactId;
pub use executor::CorrectiveWorkflowExecutor;
pub use output::{BoundedDiagnosticsSink, BoundedTextSink, OutputSinkError};
pub use task_graph::{
    ArtifactKind, ArtifactReference, ArtifactRole, ModelPolicy, TaskAttempt, TaskBudget,
    TaskGraphError, TaskId, TaskKind,
};

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

/// Canonical stage in a corrective workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowStage {
    /// Generate an initial draft from the specification.
    Draft,
    /// Run the configured deterministic check over the draft.
    InitialValidation,
    /// Normalize raw validation findings.
    NormalizeDiagnostics,
    /// Review the draft against the specification and normalized findings.
    Review,
    /// Revise the draft using review and validation findings.
    Revise,
    /// Deterministically validate the revision.
    FinalValidation,
}

/// Terminal decision of a successfully executed corrective workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowStatus {
    /// The final validation passed.
    Accepted,
    /// The final validation completed and rejected the revision.
    Rejected,
}

/// Identity-only terminal result of a corrective workflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowOutcome {
    /// The revision passed final validation.
    Accepted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Committed revision artifact identity.
        revision: ArtifactId,
        /// Committed final-validation artifact identity.
        validation: ArtifactId,
    },
    /// The revision was rejected by final validation.
    Rejected {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Committed revision artifact identity.
        revision: ArtifactId,
        /// Committed final-validation artifact identity containing the findings.
        validation: ArtifactId,
    },
}

impl WorkflowOutcome {
    /// Returns the workflow identity.
    #[must_use]
    pub const fn workflow(self) -> WorkflowId {
        match self {
            Self::Accepted { workflow, .. } | Self::Rejected { workflow, .. } => workflow,
        }
    }

    /// Returns the terminal status.
    #[must_use]
    pub const fn status(self) -> WorkflowStatus {
        match self {
            Self::Accepted { .. } => WorkflowStatus::Accepted,
            Self::Rejected { .. } => WorkflowStatus::Rejected,
        }
    }

    /// Returns the committed revision artifact identity.
    #[must_use]
    pub const fn revision(self) -> ArtifactId {
        match self {
            Self::Accepted { revision, .. } | Self::Rejected { revision, .. } => revision,
        }
    }

    /// Returns the committed final-validation artifact identity.
    #[must_use]
    pub const fn validation(self) -> ArtifactId {
        match self {
            Self::Accepted { validation, .. } | Self::Rejected { validation, .. } => validation,
        }
    }
}

/// Payload-free event emitted by a corrective workflow executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowEvent {
    /// A graph task attempt started.
    StageStarted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Canonical stage being attempted.
        stage: WorkflowStage,
        /// Attempt-safe graph task identity.
        attempt: TaskAttempt,
    },
    /// A task output was committed before its graph transition succeeded.
    ArtifactCommitted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Stage that produced the artifact.
        stage: WorkflowStage,
        /// Attempt that produced the artifact.
        attempt: TaskAttempt,
        /// Kind-, role-, and identity-only artifact reference.
        artifact: ArtifactReference,
    },
    /// An operational port failure was admitted for another attempt.
    RetryScheduled {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Stage being retried.
        stage: WorkflowStage,
        /// Failed attempt identity.
        failed_attempt: TaskAttempt,
        /// Identity reserved for the next attempt.
        next_attempt: TaskAttempt,
    },
    /// A workflow reached a validation decision.
    Completed {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Accepted or rejected status.
        status: WorkflowStatus,
        /// Committed revision artifact identity.
        revision: ArtifactId,
        /// Committed final-validation artifact identity.
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

/// Identity-only model task invocation.
#[derive(Clone, Copy, Debug)]
pub struct ModelTaskRequest<'a> {
    /// Workflow execution identity.
    pub workflow: WorkflowId,
    /// Attempt-safe graph task identity.
    pub attempt: TaskAttempt,
    /// Model operation, always draft, review, or revise.
    pub kind: TaskKind,
    /// Validated model-selection policy for this task.
    pub model_policy: ModelPolicy,
    /// Token budget forwarded to the concrete model port.
    ///
    /// The concrete port is responsible for enforcing input and output token
    /// limits while tokenizing and generating. This engine enforces artifact-byte
    /// bounds, not token counts.
    pub budget: TaskBudget,
    /// Artifact identities available to this invocation.
    pub input_artifacts: &'a [ArtifactId],
}

/// Coarse application-service boundary for model-backed workflow tasks.
pub trait ModelTaskExecutor {
    /// Owned or borrowed implementation error convertible to a stable diagnostic.
    type Error: Display;

    /// Executes one model-backed task using only the declared input identities.
    ///
    /// The restricted [`ArtifactInputs`] resolver exposes only the identities in
    /// [`ModelTaskRequest::input_artifacts`] without copying artifact payloads.
    /// Generated chunks must be appended to the executor-owned
    /// [`BoundedTextSink`]. The concrete model port must enforce the request's
    /// token limits during tokenization and generation; the sink independently
    /// enforces the stage's artifact-byte contract.
    ///
    /// # Errors
    ///
    /// Returns an implementation-defined operational failure eligible for retry.
    /// A sink failure is non-retryable and takes precedence even if the port also
    /// returns an operational error.
    fn execute_model_task(
        &mut self,
        request: ModelTaskRequest<'_>,
        artifacts: &ArtifactInputs<'_>,
        output: &mut BoundedTextSink,
    ) -> Result<(), Self::Error>;
}

/// Identity-only deterministic validation invocation.
#[derive(Clone, Copy, Debug)]
pub struct ValidationTaskRequest<'a> {
    /// Workflow execution identity.
    pub workflow: WorkflowId,
    /// Attempt-safe graph task identity.
    pub attempt: TaskAttempt,
    /// Validation operation, either compile-check or validate.
    pub kind: TaskKind,
    /// Token budget forwarded to the concrete validation port.
    ///
    /// The concrete port is responsible for enforcing input and output token
    /// limits while tokenizing and validating. This engine enforces artifact-byte
    /// bounds, not token counts.
    pub budget: TaskBudget,
    /// Artifact identities available to this invocation.
    pub input_artifacts: &'a [ArtifactId],
}

/// Coarse application-service boundary for typed deterministic validators.
pub trait ValidationTaskExecutor {
    /// Owned or borrowed implementation error convertible to a stable diagnostic.
    type Error: Display;

    /// Executes one validator task using only the declared input identities.
    ///
    /// A returned rejected verdict is a successful operation, not an error. The
    /// restricted [`ArtifactInputs`] resolver exposes only declared identities.
    /// Findings must be appended to the executor-owned
    /// [`BoundedDiagnosticsSink`], which accounts for the verdict and all
    /// structured diagnostic bytes. The concrete validator must enforce request
    /// token limits during tokenization and validation.
    ///
    /// # Errors
    ///
    /// Returns an implementation-defined operational failure eligible for retry.
    /// A sink failure is non-retryable and takes precedence even if the port also
    /// returns an operational error.
    fn execute_validation_task(
        &mut self,
        request: ValidationTaskRequest<'_>,
        artifacts: &ArtifactInputs<'_>,
        output: &mut BoundedDiagnosticsSink,
    ) -> Result<ValidationVerdict, Self::Error>;
}

/// Validated aggregate storage bounds owned by one workflow executor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowExecutorLimits {
    artifacts: NonZeroUsize,
    pending_events: NonZeroUsize,
    specification_bytes: NonZeroU64,
}

impl WorkflowExecutorLimits {
    /// Validates raw aggregate executor bounds.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidExecutorLimits`] when any bound is zero.
    pub fn new(
        maximum_artifacts: usize,
        maximum_pending_events: usize,
        maximum_specification_bytes: u64,
    ) -> Result<Self, WorkflowError> {
        let maximum_artifacts = NonZeroUsize::new(maximum_artifacts).ok_or(
            WorkflowError::InvalidExecutorLimits(WorkflowExecutorLimitError::ZeroMaximumArtifacts),
        )?;
        let maximum_pending_events = NonZeroUsize::new(maximum_pending_events).ok_or(
            WorkflowError::InvalidExecutorLimits(
                WorkflowExecutorLimitError::ZeroMaximumPendingEvents,
            ),
        )?;
        let maximum_specification_bytes = NonZeroU64::new(maximum_specification_bytes).ok_or(
            WorkflowError::InvalidExecutorLimits(
                WorkflowExecutorLimitError::ZeroMaximumSpecificationBytes,
            ),
        )?;
        Ok(Self {
            artifacts: maximum_artifacts,
            pending_events: maximum_pending_events,
            specification_bytes: maximum_specification_bytes,
        })
    }

    /// Returns the fixed artifact-store capacity.
    #[must_use]
    pub const fn maximum_artifacts(self) -> NonZeroUsize {
        self.artifacts
    }

    /// Returns the fixed pending-event capacity.
    #[must_use]
    pub const fn maximum_pending_events(self) -> NonZeroUsize {
        self.pending_events
    }

    /// Returns the maximum accepted root-specification size in UTF-8 bytes.
    #[must_use]
    pub const fn maximum_specification_bytes(self) -> NonZeroU64 {
        self.specification_bytes
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
}

/// Per-role hard limits for committed workflow output payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkflowArtifactLimits {
    /// Maximum UTF-8 and structured payload bytes for the draft.
    pub draft: u64,
    /// Maximum payload bytes for raw initial-validation output.
    pub raw_validation: u64,
    /// Maximum payload bytes for normalized diagnostics.
    pub normalized_diagnostics: u64,
    /// Maximum UTF-8 bytes for the review.
    pub review: u64,
    /// Maximum UTF-8 bytes for the revision.
    pub revision: u64,
    /// Maximum payload bytes for final validation.
    pub final_validation: u64,
}

/// Validated policies and bounds used to construct one canonical task graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CorrectiveWorkflowConfiguration {
    /// Initial deterministic operation; restricted to compile-check or validate.
    pub initial_validation: TaskKind,
    /// Model policy applied to draft, review, and revise tasks.
    pub model_policy: ModelPolicy,
    /// Budget applied to model-backed stages.
    pub model_budget: TaskBudget,
    /// Budget applied to initial and final validation stages.
    pub validation_budget: TaskBudget,
    /// Hard persisted-output byte limits by artifact role.
    pub artifact_limits: WorkflowArtifactLimits,
}

impl CorrectiveWorkflowConfiguration {
    /// Creates and validates a corrective workflow configuration.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidConfiguration`] for an unsupported initial
    /// validator kind, deterministic model policy, zero input/output-token budget,
    /// or zero artifact output limit.
    pub fn new(
        initial_validation: TaskKind,
        model_policy: ModelPolicy,
        model_budget: TaskBudget,
        validation_budget: TaskBudget,
        artifact_limits: WorkflowArtifactLimits,
    ) -> Result<Self, WorkflowError> {
        let configuration = Self {
            initial_validation,
            model_policy,
            model_budget,
            validation_budget,
            artifact_limits,
        };
        configuration.validate()?;
        Ok(configuration)
    }

    /// Validates all graph-shaping policies and output bounds.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::InvalidConfiguration`] when any invariant is not
    /// satisfied.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if !matches!(
            self.initial_validation,
            TaskKind::CompileCheck | TaskKind::Validate
        ) {
            return Err(WorkflowError::InvalidConfiguration(
                WorkflowConfigurationError::InvalidInitialValidation(self.initial_validation),
            ));
        }
        if self.model_policy == ModelPolicy::Deterministic {
            return Err(WorkflowError::InvalidConfiguration(
                WorkflowConfigurationError::DeterministicModelPolicy,
            ));
        }
        for (class, budget) in [
            (WorkflowBudgetClass::Model, self.model_budget),
            (WorkflowBudgetClass::Validation, self.validation_budget),
        ] {
            if budget.maximum_input_tokens == 0 {
                return Err(WorkflowError::InvalidConfiguration(
                    WorkflowConfigurationError::ZeroMaximumInputTokens(class),
                ));
            }
            if budget.maximum_output_tokens == 0 {
                return Err(WorkflowError::InvalidConfiguration(
                    WorkflowConfigurationError::ZeroMaximumOutputTokens(class),
                ));
            }
        }
        for (role, maximum) in [
            (ArtifactRole::Draft, self.artifact_limits.draft),
            (
                ArtifactRole::RawDiagnostics,
                self.artifact_limits.raw_validation,
            ),
            (
                ArtifactRole::NormalizedDiagnostics,
                self.artifact_limits.normalized_diagnostics,
            ),
            (ArtifactRole::Review, self.artifact_limits.review),
            (ArtifactRole::Revision, self.artifact_limits.revision),
            (
                ArtifactRole::FinalValidation,
                self.artifact_limits.final_validation,
            ),
        ] {
            if maximum == 0 {
                return Err(WorkflowError::InvalidConfiguration(
                    WorkflowConfigurationError::ZeroArtifactLimit(role),
                ));
            }
        }
        Ok(())
    }
}

/// Budget category identifying an invalid input- or output-token limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowBudgetClass {
    /// Model-backed task budget.
    Model,
    /// Validator task budget.
    Validation,
}

/// Stable configuration validation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowConfigurationError {
    /// The initial validator kind is neither compile-check nor validate.
    InvalidInitialValidation(TaskKind),
    /// Model-backed stages cannot use the deterministic policy.
    DeterministicModelPolicy,
    /// One task class has no meaningful input-token allowance.
    ZeroMaximumInputTokens(WorkflowBudgetClass),
    /// One task class has no meaningful output-token allowance.
    ZeroMaximumOutputTokens(WorkflowBudgetClass),
    /// One produced artifact role has no byte allowance.
    ZeroArtifactLimit(ArtifactRole),
}

/// Identifier sequence that exhausted checked allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowIdentifierKind {
    /// Workflow identity sequence.
    Workflow,
    /// Graph task identity sequence.
    Task,
    /// Artifact identity sequence.
    Artifact,
}

/// Typed synchronous workflow failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowError {
    /// Static workflow configuration is invalid.
    InvalidConfiguration(WorkflowConfigurationError),
    /// Aggregate executor storage limits are invalid.
    InvalidExecutorLimits(WorkflowExecutorLimitError),
    /// The requested root artifact does not exist.
    UnknownSpecification(ArtifactId),
    /// The requested root artifact is not a specification artifact.
    InvalidSpecification(ArtifactReference),
    /// A checked identity sequence cannot allocate another value.
    IdentifierExhausted(WorkflowIdentifierKind),
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
        /// Supplied owner; specifications require `None`, generated outputs require `Some`.
        owner: Option<WorkflowId>,
    },
    /// An immutable artifact identity was already committed.
    DuplicateArtifact(ArtifactId),
    /// The fixed artifact store has insufficient remaining entry capacity.
    ArtifactCapacityExceeded {
        /// Artifact entries required by the operation.
        required: usize,
        /// Artifact entries currently available.
        available: usize,
    },
    /// A root specification exceeds its configured UTF-8 byte limit.
    SpecificationCapacityExceeded {
        /// Required UTF-8 bytes.
        required: u64,
        /// Configured maximum UTF-8 bytes.
        maximum: u64,
    },
    /// A root specification byte count cannot be represented as `u64`.
    SpecificationSizeOverflow,
    /// The fixed pending-event queue has insufficient remaining capacity.
    EventCapacityExceeded {
        /// Event entries required by the operation.
        required: usize,
        /// Event entries currently available.
        available: usize,
    },
    /// Worst-case event admission arithmetic overflowed `usize`.
    EventCapacityOverflow,
    /// Graph or graph-state validation failed.
    TaskGraph(TaskGraphError),
    /// Structured payload byte accounting overflowed.
    ArtifactSizeOverflow {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing graph task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
    },
    /// A completed task produced more bytes than its declared output contract.
    OutputCapacityExceeded {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing graph task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
        /// Accounted payload bytes.
        required: u64,
        /// Declared maximum payload bytes.
        maximum: u64,
    },
    /// Fallible reservation for contract-admissible output storage failed.
    OutputAllocationFailed {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Producing stage.
        stage: WorkflowStage,
        /// Producing graph task.
        task: TaskId,
        /// Reserved output artifact identity.
        artifact: ArtifactId,
        /// Accounted payload bytes requiring storage.
        required: u64,
    },
    /// An operational port failure exhausted the task attempt budget.
    TaskExhausted {
        /// Workflow execution identity.
        workflow: WorkflowId,
        /// Failed stage.
        stage: WorkflowStage,
        /// Failed graph task identity.
        task: TaskId,
        /// Number of attempts executed.
        attempts: u16,
        /// Owned display diagnostic from the final port failure.
        diagnostic: String,
    },
    /// A committed prerequisite artifact was unexpectedly unavailable or mistyped.
    InvalidCommittedArtifact {
        /// Required artifact identity.
        artifact: ArtifactId,
        /// Required semantic role.
        expected_role: ArtifactRole,
    },
}

impl From<TaskGraphError> for WorkflowError {
    fn from(value: TaskGraphError) -> Self {
        Self::TaskGraph(value)
    }
}

impl Display for WorkflowError {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(error) => {
                write!(
                    formatter,
                    "invalid corrective workflow configuration: {error:?}"
                )
            }
            Self::InvalidExecutorLimits(error) => {
                write!(formatter, "invalid workflow executor limits: {error:?}")
            }
            Self::UnknownSpecification(id) => {
                write!(formatter, "unknown specification artifact {}", id.get())
            }
            Self::InvalidSpecification(reference) => write!(
                formatter,
                "artifact {} is not a specification artifact",
                reference.id.get()
            ),
            Self::IdentifierExhausted(kind) => {
                write!(formatter, "{kind:?} identifier sequence exhausted")
            }
            Self::ArtifactContentMismatch { reference, content } => write!(
                formatter,
                "artifact {} reference does not match {content:?} content",
                reference.id.get()
            ),
            Self::ArtifactOwnershipMismatch { reference, owner } => write!(
                formatter,
                "artifact {} has invalid workflow owner {owner:?}",
                reference.id.get()
            ),
            Self::DuplicateArtifact(id) => {
                write!(formatter, "artifact {} is already committed", id.get())
            }
            Self::ArtifactCapacityExceeded {
                required,
                available,
            } => write!(
                formatter,
                "artifact operation requires {required} entries but {available} are available"
            ),
            Self::SpecificationCapacityExceeded { required, maximum } => write!(
                formatter,
                "specification requires {required} bytes but permits {maximum}"
            ),
            Self::SpecificationSizeOverflow => {
                formatter.write_str("specification byte count cannot be represented")
            }
            Self::EventCapacityExceeded {
                required,
                available,
            } => write!(
                formatter,
                "event operation requires {required} entries but {available} are available"
            ),
            Self::EventCapacityOverflow => {
                formatter.write_str("worst-case workflow event count overflowed")
            }
            Self::TaskGraph(error) => write!(formatter, "task graph failure: {error:?}"),
            Self::ArtifactSizeOverflow {
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
            Self::OutputCapacityExceeded {
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
            Self::OutputAllocationFailed {
                workflow,
                stage,
                artifact,
                required,
                ..
            } => write!(
                formatter,
                "workflow {} {stage:?} artifact {} could not reserve storage for {required} bytes",
                workflow.get(),
                artifact.get()
            ),
            Self::TaskExhausted {
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
            Self::InvalidCommittedArtifact {
                artifact,
                expected_role,
            } => write!(
                formatter,
                "artifact {} is not a committed {expected_role:?} artifact",
                artifact.get()
            ),
        }
    }
}

impl Error for WorkflowError {}
