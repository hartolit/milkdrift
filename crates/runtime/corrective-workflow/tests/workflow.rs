//! Corrective workflow orchestration integration tests.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::NonZeroU16;
use std::rc::Rc;

use corrective_workflow::{
    Artifact, ArtifactContent, ArtifactContentKind, ArtifactId, ArtifactInputs, ArtifactKind,
    ArtifactReference, ArtifactRole, ArtifactStore, BoundedDiagnosticsSink, BoundedTextSink,
    CorrectiveWorkflowConfiguration, CorrectiveWorkflowExecutor, Diagnostic, DiagnosticLocation,
    DiagnosticSeverity, ModelPolicy, ModelTaskExecutor, ModelTaskRequest,
    NormalizedValidationReport, OutputSinkError, RawDiagnostic, TaskBudget, TaskId, TaskKind,
    ValidationReport, ValidationTaskExecutor, ValidationTaskRequest, ValidationVerdict,
    WorkflowArtifactLimits, WorkflowBudgetClass, WorkflowConfigurationError, WorkflowError,
    WorkflowEvent, WorkflowExecutorLimitError, WorkflowExecutorLimits, WorkflowId, WorkflowOutcome,
    WorkflowStage, WorkflowStatus, normalize_validation_report,
};
use domain_contracts::{BackendId, ModelId};

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedCall {
    Model {
        kind: TaskKind,
        policy: ModelPolicy,
        task: TaskId,
        attempt: u16,
        inputs: Vec<ArtifactId>,
    },
    Validation {
        kind: TaskKind,
        task: TaskId,
        attempt: u16,
        inputs: Vec<ArtifactId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordedEvent {
    Started(WorkflowStage),
    Committed(WorkflowStage),
    RetryScheduled(WorkflowStage),
    Completed(WorkflowStatus),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TestPortError(&'static str);

impl Display for TestPortError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for TestPortError {}

type CallLog = Rc<RefCell<Vec<RecordedCall>>>;
type RecordingExecutor = CorrectiveWorkflowExecutor<RecordingModel, RecordingValidator>;
type ExecutorFixture = (RecordingExecutor, CallLog);

struct RecordingModel {
    calls: CallLog,
    responses: VecDeque<Result<String, TestPortError>>,
}

impl ModelTaskExecutor for RecordingModel {
    type Error = TestPortError;

    fn execute_model_task(
        &mut self,
        request: ModelTaskRequest<'_>,
        artifacts: &ArtifactInputs<'_>,
        output: &mut BoundedTextSink,
    ) -> Result<(), Self::Error> {
        if request.input_artifacts != artifacts.ids()
            || request
                .input_artifacts
                .iter()
                .any(|id| artifacts.get(*id).is_none())
        {
            return Err(TestPortError("model input was not committed"));
        }
        self.calls.borrow_mut().push(RecordedCall::Model {
            kind: request.kind,
            policy: request.model_policy,
            task: request.attempt.task,
            attempt: request.attempt.number.get(),
            inputs: request.input_artifacts.to_vec(),
        });
        match self
            .responses
            .pop_front()
            .ok_or(TestPortError("missing model response"))?
        {
            Ok(response) => output
                .append(&response)
                .map_err(|_| TestPortError("model output sink failed")),
            Err(error) => Err(error),
        }
    }
}

struct RestrictedAccessModel {
    responses: VecDeque<Result<String, TestPortError>>,
    undeclared_probe: Rc<Cell<ArtifactId>>,
    denied_checks: Rc<RefCell<Vec<bool>>>,
}

impl ModelTaskExecutor for RestrictedAccessModel {
    type Error = TestPortError;

    fn execute_model_task(
        &mut self,
        request: ModelTaskRequest<'_>,
        artifacts: &ArtifactInputs<'_>,
        output: &mut BoundedTextSink,
    ) -> Result<(), Self::Error> {
        if request.input_artifacts != artifacts.ids()
            || request
                .input_artifacts
                .iter()
                .any(|id| artifacts.get(*id).is_none())
        {
            return Err(TestPortError("declared model input was unavailable"));
        }
        let undeclared = self.undeclared_probe.get();
        if undeclared.get() != 0 {
            self.denied_checks
                .borrow_mut()
                .push(artifacts.get(undeclared).is_none());
        }
        match self
            .responses
            .pop_front()
            .ok_or(TestPortError("missing restricted model response"))?
        {
            Ok(response) => output
                .append(&response)
                .map_err(|_| TestPortError("restricted model output sink failed")),
            Err(error) => Err(error),
        }
    }
}

struct RecordingValidator {
    calls: CallLog,
    responses: VecDeque<Result<ValidationReport, TestPortError>>,
}

impl ValidationTaskExecutor for RecordingValidator {
    type Error = TestPortError;

    fn execute_validation_task(
        &mut self,
        request: ValidationTaskRequest<'_>,
        artifacts: &ArtifactInputs<'_>,
        output: &mut BoundedDiagnosticsSink,
    ) -> Result<ValidationVerdict, Self::Error> {
        if request.input_artifacts != artifacts.ids()
            || request
                .input_artifacts
                .iter()
                .any(|id| artifacts.get(*id).is_none())
        {
            return Err(TestPortError("validation input was not committed"));
        }
        self.calls.borrow_mut().push(RecordedCall::Validation {
            kind: request.kind,
            task: request.attempt.task,
            attempt: request.attempt.number.get(),
            inputs: request.input_artifacts.to_vec(),
        });
        match self
            .responses
            .pop_front()
            .ok_or(TestPortError("missing validation response"))?
        {
            Ok(report) => {
                for diagnostic in &report.diagnostics {
                    output
                        .append(diagnostic)
                        .map_err(|_| TestPortError("validation output sink failed"))?;
                }
                Ok(report.verdict)
            }
            Err(error) => Err(error),
        }
    }
}

fn configuration(attempts: u16) -> Result<CorrectiveWorkflowConfiguration, TestPortError> {
    let attempts = NonZeroU16::new(attempts).ok_or(TestPortError("attempts must be nonzero"))?;
    let model_budget = TaskBudget::new(2048, 512, attempts);
    let validation_budget = TaskBudget::new(2048, 256, attempts);
    Ok(CorrectiveWorkflowConfiguration {
        initial_validation: TaskKind::CompileCheck,
        model_policy: ModelPolicy::AnyCompatible,
        model_budget,
        validation_budget,
        artifact_limits: WorkflowArtifactLimits {
            draft: 4096,
            raw_validation: 4096,
            normalized_diagnostics: 4096,
            review: 4096,
            revision: 4096,
            final_validation: 4096,
        },
    })
}

const fn report(verdict: ValidationVerdict) -> ValidationReport {
    ValidationReport {
        verdict,
        diagnostics: Vec::new(),
    }
}

fn executor_limits() -> Result<WorkflowExecutorLimits, WorkflowError> {
    WorkflowExecutorLimits::new(64, 256, 4096)
}

fn executor(
    model_responses: impl IntoIterator<Item = Result<String, TestPortError>>,
    validation_responses: impl IntoIterator<Item = Result<ValidationReport, TestPortError>>,
) -> Result<ExecutorFixture, WorkflowError> {
    Ok(executor_with_limits(
        model_responses,
        validation_responses,
        executor_limits()?,
    ))
}

fn executor_with_limits(
    model_responses: impl IntoIterator<Item = Result<String, TestPortError>>,
    validation_responses: impl IntoIterator<Item = Result<ValidationReport, TestPortError>>,
    limits: WorkflowExecutorLimits,
) -> ExecutorFixture {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let model = RecordingModel {
        calls: Rc::clone(&calls),
        responses: model_responses.into_iter().collect(),
    };
    let validator = RecordingValidator {
        calls: Rc::clone(&calls),
        responses: validation_responses.into_iter().collect(),
    };
    (
        CorrectiveWorkflowExecutor::new(model, validator, limits),
        calls,
    )
}

#[test]
fn bounded_text_sink_accepts_chunked_output_at_exact_limit() -> Result<(), Box<dyn Error>> {
    let mut output = BoundedTextSink::new(10);

    output.append("chunk")?;
    output.append("ed")?;
    output.append("✓")?;

    assert_eq!(output.as_str(), "chunked✓");
    assert_eq!(output.bytes_written(), 10);
    assert_eq!(output.maximum_bytes(), 10);
    assert_eq!(output.remaining_bytes(), 0);
    assert_eq!(output.failure(), None);
    Ok(())
}

#[test]
fn bounded_text_overflow_is_atomic_sticky_and_never_truncates() -> Result<(), Box<dyn Error>> {
    let mut output = BoundedTextSink::new(5);
    output.append("abc")?;

    let failure = OutputSinkError::CapacityExceeded {
        required: 6,
        maximum: 5,
    };
    assert_eq!(output.append("def"), Err(failure));
    assert_eq!(output.as_str(), "abc");
    assert_eq!(output.bytes_written(), 3);
    assert_eq!(output.remaining_bytes(), 2);
    assert_eq!(output.failure(), Some(failure));
    assert_eq!(output.append("d"), Err(failure));
    assert_eq!(output.as_str(), "abc");
    Ok(())
}

#[test]
fn bounded_diagnostics_account_for_verdict_structure_count_and_strings()
-> Result<(), Box<dyn Error>> {
    assert_eq!(
        BoundedDiagnosticsSink::new(0).err(),
        Some(OutputSinkError::CapacityExceeded {
            required: 1,
            maximum: 0,
        })
    );

    let diagnostic = RawDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: Some("E1".to_owned()),
        message: "bad".to_owned(),
        location: Some(DiagnosticLocation {
            path: Some("src".to_owned()),
            line: Some(2),
            column: Some(3),
        }),
    };
    let mut output = BoundedDiagnosticsSink::new(23)?;
    output.append(&diagnostic)?;

    assert_eq!(output.diagnostics(), [diagnostic]);
    assert_eq!(output.bytes_written(), 23);
    assert_eq!(output.remaining_bytes(), 0);

    let minimal = RawDiagnostic {
        severity: DiagnosticSeverity::Information,
        code: None,
        message: String::new(),
        location: None,
    };
    let failure = OutputSinkError::CapacityExceeded {
        required: 26,
        maximum: 23,
    };
    assert_eq!(output.append(&minimal), Err(failure));
    assert_eq!(output.diagnostics().len(), 1);
    assert_eq!(output.bytes_written(), 23);
    assert_eq!(output.failure(), Some(failure));
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn canonical_workflow_uses_exact_call_order_and_input_ids() -> Result<(), Box<dyn Error>> {
    let raw_diagnostic = RawDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: Some(" E002 ".to_owned()),
        message: "  broken\n   item  ".to_owned(),
        location: Some(DiagnosticLocation {
            path: Some(" src/main.rs ".to_owned()),
            line: Some(3),
            column: Some(7),
        }),
    };
    let (mut executor, calls) = executor(
        [
            Ok("draft transcript".to_owned()),
            Ok("review only".to_owned()),
            Ok("revision only".to_owned()),
        ],
        [
            Ok(ValidationReport {
                verdict: ValidationVerdict::Rejected,
                diagnostics: vec![raw_diagnostic],
            }),
            Ok(report(ValidationVerdict::Passed)),
        ],
    )?;
    let specification = executor.insert_specification("specification only".to_owned())?;

    let outcome = executor.execute(specification, configuration(2)?)?;

    assert_eq!(
        outcome,
        WorkflowOutcome::Accepted {
            workflow: WorkflowId::new(1),
            revision: ArtifactId::new(6),
            validation: ArtifactId::new(7),
        }
    );
    assert_eq!(
        calls.borrow().as_slice(),
        [
            RecordedCall::Model {
                kind: TaskKind::Draft,
                policy: ModelPolicy::AnyCompatible,
                task: TaskId::new(1),
                attempt: 1,
                inputs: vec![ArtifactId::new(1)],
            },
            RecordedCall::Validation {
                kind: TaskKind::CompileCheck,
                task: TaskId::new(2),
                attempt: 1,
                inputs: vec![ArtifactId::new(2)],
            },
            RecordedCall::Model {
                kind: TaskKind::Review,
                policy: ModelPolicy::AnyCompatible,
                task: TaskId::new(4),
                attempt: 1,
                inputs: vec![ArtifactId::new(1), ArtifactId::new(2), ArtifactId::new(4),],
            },
            RecordedCall::Model {
                kind: TaskKind::Revise,
                policy: ModelPolicy::AnyCompatible,
                task: TaskId::new(5),
                attempt: 1,
                inputs: vec![
                    ArtifactId::new(1),
                    ArtifactId::new(2),
                    ArtifactId::new(4),
                    ArtifactId::new(5),
                ],
            },
            RecordedCall::Validation {
                kind: TaskKind::Validate,
                task: TaskId::new(6),
                attempt: 1,
                inputs: vec![ArtifactId::new(6)],
            },
        ]
    );
    assert_eq!(executor.artifacts().len(), 7);
    assert_eq!(
        executor
            .artifacts()
            .get(specification)
            .and_then(Artifact::owner),
        None
    );
    for id in 2_u64..=7 {
        assert_eq!(
            executor
                .artifacts()
                .get(ArtifactId::new(id))
                .and_then(Artifact::owner),
            Some(WorkflowId::new(1))
        );
    }

    let normalized = executor
        .artifacts()
        .get(ArtifactId::new(4))
        .ok_or(TestPortError("missing normalized artifact"))?;
    assert_eq!(
        normalized.content(),
        &ArtifactContent::NormalizedDiagnostics(NormalizedValidationReport {
            verdict: ValidationVerdict::Rejected,
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Error,
                code: Some("E002".to_owned()),
                message: "broken item".to_owned(),
                location: Some(DiagnosticLocation {
                    path: Some("src/main.rs".to_owned()),
                    line: Some(3),
                    column: Some(7),
                }),
            }],
        })
    );
    assert_eq!(
        executor
            .artifacts()
            .get(ArtifactId::new(5))
            .map(Artifact::content),
        Some(&ArtifactContent::Review("review only".to_owned()))
    );
    assert_eq!(
        executor
            .artifacts()
            .get(ArtifactId::new(6))
            .map(Artifact::content),
        Some(&ArtifactContent::Revision("revision only".to_owned()))
    );

    let mut events = Vec::new();
    while let Some(event) = executor.poll_event() {
        assert_eq!(event.workflow(), WorkflowId::new(1));
        events.push(match event {
            WorkflowEvent::StageStarted { stage, .. } => RecordedEvent::Started(stage),
            WorkflowEvent::ArtifactCommitted { stage, .. } => RecordedEvent::Committed(stage),
            WorkflowEvent::RetryScheduled { stage, .. } => RecordedEvent::RetryScheduled(stage),
            WorkflowEvent::Completed { status, .. } => RecordedEvent::Completed(status),
        });
    }
    assert_eq!(
        events,
        [
            RecordedEvent::Started(WorkflowStage::Draft),
            RecordedEvent::Committed(WorkflowStage::Draft),
            RecordedEvent::Started(WorkflowStage::InitialValidation),
            RecordedEvent::Committed(WorkflowStage::InitialValidation),
            RecordedEvent::Started(WorkflowStage::NormalizeDiagnostics),
            RecordedEvent::Committed(WorkflowStage::NormalizeDiagnostics),
            RecordedEvent::Started(WorkflowStage::Review),
            RecordedEvent::Committed(WorkflowStage::Review),
            RecordedEvent::Started(WorkflowStage::Revise),
            RecordedEvent::Committed(WorkflowStage::Revise),
            RecordedEvent::Started(WorkflowStage::FinalValidation),
            RecordedEvent::Committed(WorkflowStage::FinalValidation),
            RecordedEvent::Completed(WorkflowStatus::Accepted),
        ]
    );
    Ok(())
}

#[test]
fn exact_preferred_and_any_model_policies_are_forwarded() -> Result<(), Box<dyn Error>> {
    for policy in [
        ModelPolicy::Exact(ModelId::new(41)),
        ModelPolicy::PreferredBackend(BackendId::new(17)),
        ModelPolicy::AnyCompatible,
    ] {
        let (mut executor, calls) = executor(
            [
                Ok("draft".to_owned()),
                Ok("review".to_owned()),
                Ok("revision".to_owned()),
            ],
            [
                Ok(report(ValidationVerdict::Passed)),
                Ok(report(ValidationVerdict::Passed)),
            ],
        )?;
        let specification = executor.insert_specification("spec".to_owned())?;
        let mut config = configuration(1)?;
        config.model_policy = policy;

        executor.execute(specification, config)?;

        let forwarded: Vec<ModelPolicy> = calls
            .borrow()
            .iter()
            .filter_map(|call| match call {
                RecordedCall::Model { policy, .. } => Some(*policy),
                RecordedCall::Validation { .. } => None,
            })
            .collect();
        assert_eq!(forwarded, [policy, policy, policy]);
    }
    Ok(())
}

#[test]
fn final_rejection_is_a_successful_terminal_outcome() -> Result<(), Box<dyn Error>> {
    let (mut executor, _) = executor(
        [
            Ok("draft".to_owned()),
            Ok("review".to_owned()),
            Ok("revision".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(ValidationReport {
                verdict: ValidationVerdict::Rejected,
                diagnostics: vec![RawDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: None,
                    message: "still invalid".to_owned(),
                    location: None,
                }],
            }),
        ],
    )?;
    let specification = executor.insert_specification("spec".to_owned())?;

    let outcome = executor.execute(specification, configuration(1)?)?;

    assert_eq!(outcome.status(), WorkflowStatus::Rejected);
    assert_eq!(outcome.revision(), ArtifactId::new(6));
    assert_eq!(outcome.validation(), ArtifactId::new(7));
    assert!(matches!(outcome, WorkflowOutcome::Rejected { .. }));
    assert!(matches!(
        executor
            .artifacts()
            .get(outcome.validation())
            .map(Artifact::content),
        Some(ArtifactContent::FinalValidation(ValidationReport {
            verdict: ValidationVerdict::Rejected,
            ..
        }))
    ));
    Ok(())
}

#[test]
fn normalization_sorts_deduplicates_and_collapses_whitespace() {
    let duplicate = RawDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: Some(" W1 ".to_owned()),
        message: " alpha\n\t beta ".to_owned(),
        location: Some(DiagnosticLocation {
            path: Some(" src/lib.rs ".to_owned()),
            line: Some(2),
            column: None,
        }),
    };
    let normalized = normalize_validation_report(&ValidationReport {
        verdict: ValidationVerdict::Rejected,
        diagnostics: vec![
            duplicate.clone(),
            RawDiagnostic {
                severity: DiagnosticSeverity::Information,
                code: Some("   ".to_owned()),
                message: " zeta ".to_owned(),
                location: Some(DiagnosticLocation {
                    path: Some("  ".to_owned()),
                    line: None,
                    column: None,
                }),
            },
            duplicate,
        ],
    });

    assert_eq!(normalized.verdict, ValidationVerdict::Rejected);
    assert_eq!(
        normalized.diagnostics,
        [
            Diagnostic {
                severity: DiagnosticSeverity::Information,
                code: None,
                message: "zeta".to_owned(),
                location: Some(DiagnosticLocation {
                    path: None,
                    line: None,
                    column: None,
                }),
            },
            Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: Some("W1".to_owned()),
                message: "alpha beta".to_owned(),
                location: Some(DiagnosticLocation {
                    path: Some("src/lib.rs".to_owned()),
                    line: Some(2),
                    column: None,
                }),
            },
        ]
    );
}

#[test]
fn operational_failure_retries_then_succeeds() -> Result<(), Box<dyn Error>> {
    let (mut executor, calls) = executor(
        [
            Err(TestPortError("temporary model failure")),
            Ok("draft".to_owned()),
            Ok("review".to_owned()),
            Ok("revision".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ],
    )?;
    let specification = executor.insert_specification("spec".to_owned())?;

    let outcome = executor.execute(specification, configuration(2)?)?;

    assert_eq!(outcome.status(), WorkflowStatus::Accepted);
    let recorded = calls.borrow();
    assert!(matches!(
        recorded.first(),
        Some(RecordedCall::Model {
            kind: TaskKind::Draft,
            attempt: 1,
            ..
        })
    ));
    assert!(matches!(
        recorded.get(1),
        Some(RecordedCall::Model {
            kind: TaskKind::Draft,
            attempt: 2,
            ..
        })
    ));
    drop(recorded);

    let mut saw_retry = false;
    while let Some(event) = executor.poll_event() {
        if let WorkflowEvent::RetryScheduled {
            workflow,
            stage,
            failed_attempt,
            next_attempt,
        } = event
        {
            assert_eq!(workflow, WorkflowId::new(1));
            assert_eq!(stage, WorkflowStage::Draft);
            assert_eq!(failed_attempt.task, TaskId::new(1));
            assert_eq!(failed_attempt.number.get(), 1);
            assert_eq!(next_attempt.task, TaskId::new(1));
            assert_eq!(next_attempt.number.get(), 2);
            saw_retry = true;
        }
    }
    assert!(saw_retry);
    Ok(())
}

#[test]
fn terminal_port_exhaustion_returns_owned_diagnostic() -> Result<(), Box<dyn Error>> {
    let (mut executor, _) = executor(
        [
            Err(TestPortError("first failure")),
            Err(TestPortError("terminal failure")),
        ],
        [],
    )?;
    let specification = executor.insert_specification("spec".to_owned())?;

    let error = executor
        .execute(specification, configuration(2)?)
        .err()
        .ok_or(TestPortError("workflow unexpectedly succeeded"))?;

    assert_eq!(
        error,
        WorkflowError::TaskExhausted {
            workflow: WorkflowId::new(1),
            stage: WorkflowStage::Draft,
            task: TaskId::new(1),
            attempts: 2,
            diagnostic: "terminal failure".to_owned(),
        }
    );
    assert_eq!(executor.artifacts().len(), 1);
    Ok(())
}

#[test]
fn workflow_events_are_copyable_identity_only_values() {
    fn assert_copy<T: Copy>() {}
    assert_copy::<WorkflowEvent>();

    let event = WorkflowEvent::Completed {
        workflow: WorkflowId::new(9),
        status: WorkflowStatus::Accepted,
        revision: ArtifactId::new(10),
        validation: ArtifactId::new(11),
    };
    let copied = event;
    assert_eq!(event, copied);
}

#[test]
fn artifact_store_rejects_wrong_roles_and_duplicate_ids() -> Result<(), Box<dyn Error>> {
    let wrong_reference = ArtifactReference {
        id: ArtifactId::new(1),
        kind: ArtifactKind::Text,
        role: ArtifactRole::Draft,
    };
    assert_eq!(
        Artifact::new(
            wrong_reference,
            ArtifactContent::Specification("spec".to_owned())
        ),
        Err(WorkflowError::ArtifactContentMismatch {
            reference: wrong_reference,
            content: ArtifactContentKind::Specification,
        })
    );

    let generated_reference = ArtifactReference {
        id: ArtifactId::new(3),
        kind: ArtifactKind::Text,
        role: ArtifactRole::Draft,
    };
    assert_eq!(
        Artifact::new(
            generated_reference,
            ArtifactContent::Draft("draft".to_owned())
        ),
        Err(WorkflowError::ArtifactOwnershipMismatch {
            reference: generated_reference,
            owner: None,
        })
    );
    let specification_reference = ArtifactReference {
        id: ArtifactId::new(4),
        kind: ArtifactKind::Text,
        role: ArtifactRole::Specification,
    };
    assert_eq!(
        Artifact::new_owned(
            specification_reference,
            WorkflowId::new(1),
            ArtifactContent::Specification("spec".to_owned())
        ),
        Err(WorkflowError::ArtifactOwnershipMismatch {
            reference: specification_reference,
            owner: Some(WorkflowId::new(1)),
        })
    );

    let limits = WorkflowExecutorLimits::new(2, 1, 1)?;
    let mut store = ArtifactStore::new(limits.maximum_artifacts());
    let valid_reference = ArtifactReference {
        id: ArtifactId::new(2),
        kind: ArtifactKind::Text,
        role: ArtifactRole::Specification,
    };
    let artifact = Artifact::new(
        valid_reference,
        ArtifactContent::Specification("stored once".to_owned()),
    )?;
    store.insert(artifact.clone())?;
    assert_eq!(
        store.insert(artifact),
        Err(WorkflowError::DuplicateArtifact(ArtifactId::new(2)))
    );
    assert_eq!(store.len(), 1);
    assert_eq!(
        store.get(ArtifactId::new(2)).map(Artifact::content),
        Some(&ArtifactContent::Specification("stored once".to_owned()))
    );
    Ok(())
}

#[test]
fn invalid_configuration_is_rejected_by_typed_field() -> Result<(), Box<dyn Error>> {
    let base = configuration(1)?;
    let mut invalid_kind = base;
    invalid_kind.initial_validation = TaskKind::Draft;
    assert_eq!(
        invalid_kind.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::InvalidInitialValidation(TaskKind::Draft)
        ))
    );

    let mut deterministic_model = base;
    deterministic_model.model_policy = ModelPolicy::Deterministic;
    assert_eq!(
        deterministic_model.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::DeterministicModelPolicy
        ))
    );

    let mut zero_model_input = base;
    zero_model_input.model_budget.maximum_input_tokens = 0;
    assert_eq!(
        zero_model_input.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::ZeroMaximumInputTokens(WorkflowBudgetClass::Model)
        ))
    );

    let mut zero_validation_input = base;
    zero_validation_input.validation_budget.maximum_input_tokens = 0;
    assert_eq!(
        zero_validation_input.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::ZeroMaximumInputTokens(WorkflowBudgetClass::Validation)
        ))
    );

    let mut zero_tokens = base;
    zero_tokens.model_budget.maximum_output_tokens = 0;
    assert_eq!(
        zero_tokens.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::ZeroMaximumOutputTokens(WorkflowBudgetClass::Model)
        ))
    );

    let mut zero_bytes = base;
    zero_bytes.artifact_limits.revision = 0;
    assert_eq!(
        zero_bytes.validate(),
        Err(WorkflowError::InvalidConfiguration(
            WorkflowConfigurationError::ZeroArtifactLimit(ArtifactRole::Revision)
        ))
    );
    Ok(())
}

#[test]
fn aggregate_executor_limits_reject_zero_values() {
    assert_eq!(
        WorkflowExecutorLimits::new(0, 1, 1),
        Err(WorkflowError::InvalidExecutorLimits(
            WorkflowExecutorLimitError::ZeroMaximumArtifacts
        ))
    );
    assert_eq!(
        WorkflowExecutorLimits::new(1, 0, 1),
        Err(WorkflowError::InvalidExecutorLimits(
            WorkflowExecutorLimitError::ZeroMaximumPendingEvents
        ))
    );
    assert_eq!(
        WorkflowExecutorLimits::new(1, 1, 0),
        Err(WorkflowError::InvalidExecutorLimits(
            WorkflowExecutorLimitError::ZeroMaximumSpecificationBytes
        ))
    );
}

#[test]
fn partial_artifact_room_fails_before_execution_side_effects() -> Result<(), Box<dyn Error>> {
    let limits = WorkflowExecutorLimits::new(4, 13, 64)?;
    let (mut executor, calls) = executor_with_limits([Ok("unused draft".to_owned())], [], limits);
    let specification = executor.insert_specification("spec".to_owned())?;

    let error = executor
        .execute(specification, configuration(1)?)
        .err()
        .ok_or(TestPortError("artifact admission unexpectedly succeeded"))?;

    assert_eq!(
        error,
        WorkflowError::ArtifactCapacityExceeded {
            required: 6,
            available: 3,
        }
    );
    assert_eq!(executor.artifacts().capacity(), 4);
    assert_eq!(executor.artifacts().remaining_capacity(), 3);
    assert_eq!(executor.artifacts().len(), 1);
    assert!(executor.artifacts().get(ArtifactId::new(2)).is_none());
    assert!(calls.borrow().is_empty());
    assert_eq!(executor.poll_event(), None);
    Ok(())
}

#[test]
fn specification_byte_limit_is_checked_before_id_allocation() -> Result<(), Box<dyn Error>> {
    let limits = WorkflowExecutorLimits::new(4, 4, 3)?;
    let (mut executor, _) = executor_with_limits([], [], limits);

    assert_eq!(
        executor.insert_specification("four".to_owned()),
        Err(WorkflowError::SpecificationCapacityExceeded {
            required: 4,
            maximum: 3,
        })
    );
    assert!(executor.artifacts().is_empty());
    assert_eq!(
        executor.insert_specification("ok".to_owned())?,
        ArtifactId::new(1)
    );
    Ok(())
}

#[test]
fn one_less_than_required_event_capacity_fails_before_side_effects() -> Result<(), Box<dyn Error>> {
    let limits = WorkflowExecutorLimits::new(8, 22, 64)?;
    let (mut executor, calls) = executor_with_limits([Ok("unused draft".to_owned())], [], limits);
    let specification = executor.insert_specification("spec".to_owned())?;

    let error = executor
        .execute(specification, configuration(2)?)
        .err()
        .ok_or(TestPortError("event admission unexpectedly succeeded"))?;

    assert_eq!(
        error,
        WorkflowError::EventCapacityExceeded {
            required: 23,
            available: 22,
        }
    );
    assert_eq!(executor.artifacts().len(), 1);
    assert!(calls.borrow().is_empty());
    assert_eq!(executor.poll_event(), None);
    Ok(())
}

#[test]
fn exactly_sufficient_event_capacity_completes_without_event_error() -> Result<(), Box<dyn Error>> {
    let limits = WorkflowExecutorLimits::new(8, 23, 64)?;
    let (mut executor, _) = executor_with_limits(
        [
            Err(TestPortError("draft retry")),
            Ok("draft".to_owned()),
            Err(TestPortError("review retry")),
            Ok("review".to_owned()),
            Err(TestPortError("revision retry")),
            Ok("revision".to_owned()),
        ],
        [
            Err(TestPortError("initial validation retry")),
            Ok(report(ValidationVerdict::Passed)),
            Err(TestPortError("final validation retry")),
            Ok(report(ValidationVerdict::Passed)),
        ],
        limits,
    );
    let specification = executor.insert_specification("spec".to_owned())?;

    let outcome = executor.execute(specification, configuration(2)?)?;

    assert_eq!(outcome.status(), WorkflowStatus::Accepted);
    let mut event_count = 0_usize;
    while executor.poll_event().is_some() {
        event_count += 1;
    }
    assert_eq!(event_count, 23);
    Ok(())
}

#[test]
fn queued_events_reduce_admission_capacity_before_new_side_effects() -> Result<(), Box<dyn Error>> {
    let limits = WorkflowExecutorLimits::new(16, 35, 64)?;
    let (mut executor, calls) = executor_with_limits(
        [
            Ok("draft".to_owned()),
            Ok("review".to_owned()),
            Ok("revision".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ],
        limits,
    );
    let specification = executor.insert_specification("spec".to_owned())?;
    executor.execute(specification, configuration(1)?)?;
    let artifacts_before = executor.artifacts().len();
    let calls_before = calls.borrow().len();

    let error = executor
        .execute(specification, configuration(2)?)
        .err()
        .ok_or(TestPortError(
            "queued-event admission unexpectedly succeeded",
        ))?;

    assert_eq!(
        error,
        WorkflowError::EventCapacityExceeded {
            required: 23,
            available: 22,
        }
    );
    assert_eq!(executor.artifacts().len(), artifacts_before);
    assert_eq!(calls.borrow().len(), calls_before);
    let mut queued_events = 0_usize;
    while executor.poll_event().is_some() {
        queued_events += 1;
    }
    assert_eq!(queued_events, 13);
    Ok(())
}

#[test]
fn output_contract_is_enforced_without_commit_or_truncation() -> Result<(), Box<dyn Error>> {
    let (mut executor, calls) = executor([Ok("oversized".to_owned())], [])?;
    let specification = executor.insert_specification("spec".to_owned())?;
    let mut config = configuration(2)?;
    config.artifact_limits.draft = 4;

    let error = executor
        .execute(specification, config)
        .err()
        .ok_or(TestPortError("capacity violation unexpectedly succeeded"))?;

    assert_eq!(
        error,
        WorkflowError::OutputCapacityExceeded {
            workflow: WorkflowId::new(1),
            stage: WorkflowStage::Draft,
            task: TaskId::new(1),
            artifact: ArtifactId::new(2),
            required: 9,
            maximum: 4,
        }
    );
    assert_eq!(executor.artifacts().len(), 1);
    assert!(executor.artifacts().get(ArtifactId::new(2)).is_none());
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(executor.poll_event(), None);
    Ok(())
}

#[test]
fn validation_sink_failure_is_non_retryable_and_rolls_back_prior_output()
-> Result<(), Box<dyn Error>> {
    let (mut executor, calls) = executor(
        [Ok("draft".to_owned())],
        [Ok(ValidationReport {
            verdict: ValidationVerdict::Rejected,
            diagnostics: vec![RawDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: None,
                message: "too big".to_owned(),
                location: None,
            }],
        })],
    )?;
    let specification = executor.insert_specification("spec".to_owned())?;
    let mut config = configuration(2)?;
    config.artifact_limits.raw_validation = 4;

    let error = executor
        .execute(specification, config)
        .err()
        .ok_or(TestPortError(
            "validation capacity violation unexpectedly succeeded",
        ))?;

    assert_eq!(
        error,
        WorkflowError::OutputCapacityExceeded {
            workflow: WorkflowId::new(1),
            stage: WorkflowStage::InitialValidation,
            task: TaskId::new(2),
            artifact: ArtifactId::new(3),
            required: 11,
            maximum: 4,
        }
    );
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(executor.artifacts().len(), 1);
    assert_eq!(executor.poll_event(), None);
    Ok(())
}

#[test]
fn normalized_diagnostics_are_bounded_before_commit() -> Result<(), Box<dyn Error>> {
    let (mut executor, calls) = executor(
        [Ok("draft".to_owned())],
        [Ok(ValidationReport {
            verdict: ValidationVerdict::Rejected,
            diagnostics: vec![RawDiagnostic {
                severity: DiagnosticSeverity::Error,
                code: None,
                message: "x".to_owned(),
                location: None,
            }],
        })],
    )?;
    let specification = executor.insert_specification("spec".to_owned())?;
    let mut config = configuration(1)?;
    config.artifact_limits.normalized_diagnostics = 4;

    let error = executor
        .execute(specification, config)
        .err()
        .ok_or(TestPortError(
            "normalized capacity violation unexpectedly succeeded",
        ))?;

    assert_eq!(
        error,
        WorkflowError::OutputCapacityExceeded {
            workflow: WorkflowId::new(1),
            stage: WorkflowStage::NormalizeDiagnostics,
            task: TaskId::new(3),
            artifact: ArtifactId::new(4),
            required: 5,
            maximum: 4,
        }
    );
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(executor.artifacts().len(), 1);
    assert_eq!(executor.poll_event(), None);
    Ok(())
}

#[test]
fn artifact_inputs_hide_undeclared_prior_workflow_artifacts() -> Result<(), Box<dyn Error>> {
    let undeclared_probe = Rc::new(Cell::new(ArtifactId::new(0)));
    let denied_checks = Rc::new(RefCell::new(Vec::new()));
    let model = RestrictedAccessModel {
        responses: [
            Ok("draft one".to_owned()),
            Ok("review one".to_owned()),
            Ok("revision one".to_owned()),
            Ok("draft two".to_owned()),
            Ok("review two".to_owned()),
            Ok("revision two".to_owned()),
        ]
        .into_iter()
        .collect(),
        undeclared_probe: Rc::clone(&undeclared_probe),
        denied_checks: Rc::clone(&denied_checks),
    };
    let validator = RecordingValidator {
        calls: Rc::new(RefCell::new(Vec::new())),
        responses: [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ]
        .into_iter()
        .collect(),
    };
    let mut executor = CorrectiveWorkflowExecutor::new(model, validator, executor_limits()?);
    let specification = executor.insert_specification("shared spec".to_owned())?;
    executor.execute(specification, configuration(1)?)?;
    undeclared_probe.set(ArtifactId::new(2));

    executor.execute(specification, configuration(1)?)?;

    assert_eq!(denied_checks.borrow().as_slice(), [true, true, true]);
    Ok(())
}

#[test]
fn sequential_workflows_do_not_reuse_workflow_task_or_artifact_ids() -> Result<(), Box<dyn Error>> {
    let (mut executor, calls) = executor(
        [
            Ok("draft one".to_owned()),
            Ok("review one".to_owned()),
            Ok("revision one".to_owned()),
            Ok("draft two".to_owned()),
            Ok("review two".to_owned()),
            Ok("revision two".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Rejected)),
        ],
    )?;
    let specification = executor.insert_specification("shared spec".to_owned())?;

    let first = executor.execute(specification, configuration(1)?)?;
    let second = executor.execute(specification, configuration(1)?)?;

    assert_eq!(first.workflow(), WorkflowId::new(1));
    assert_eq!(second.workflow(), WorkflowId::new(2));
    assert_eq!(first.revision(), ArtifactId::new(6));
    assert_eq!(second.revision(), ArtifactId::new(12));
    assert_eq!(executor.artifacts().len(), 13);

    let recorded = calls.borrow();
    let mut task_ids: Vec<TaskId> = recorded
        .iter()
        .map(|call| match call {
            RecordedCall::Model { task, .. } | RecordedCall::Validation { task, .. } => *task,
        })
        .collect();
    task_ids.sort();
    task_ids.dedup();
    assert_eq!(
        task_ids,
        [
            TaskId::new(1),
            TaskId::new(2),
            TaskId::new(4),
            TaskId::new(5),
            TaskId::new(6),
            TaskId::new(7),
            TaskId::new(8),
            TaskId::new(10),
            TaskId::new(11),
            TaskId::new(12),
        ]
    );
    Ok(())
}

#[test]
fn late_stage_failure_rolls_back_artifacts_and_events_without_reusing_ids()
-> Result<(), Box<dyn Error>> {
    let (mut executor, _) = executor(
        [
            Ok("failed draft".to_owned()),
            Ok("failed review".to_owned()),
            Ok("failed revision".to_owned()),
            Ok("next draft".to_owned()),
            Ok("next review".to_owned()),
            Ok("next revision".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Err(TestPortError("late final failure")),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ],
    )?;
    let specification = executor.insert_specification("shared spec".to_owned())?;

    let error = executor
        .execute(specification, configuration(1)?)
        .err()
        .ok_or(TestPortError("late-stage workflow unexpectedly succeeded"))?;

    assert_eq!(
        error,
        WorkflowError::TaskExhausted {
            workflow: WorkflowId::new(1),
            stage: WorkflowStage::FinalValidation,
            task: TaskId::new(6),
            attempts: 1,
            diagnostic: "late final failure".to_owned(),
        }
    );
    assert_eq!(executor.artifacts().len(), 1);
    assert_eq!(
        executor
            .artifacts()
            .get(specification)
            .and_then(Artifact::owner),
        None
    );
    for id in 2_u64..=7 {
        assert!(executor.artifacts().get(ArtifactId::new(id)).is_none());
    }
    assert_eq!(executor.poll_event(), None);

    let outcome = executor.execute(specification, configuration(1)?)?;

    assert_eq!(outcome.workflow(), WorkflowId::new(2));
    assert_eq!(outcome.revision(), ArtifactId::new(12));
    assert_eq!(outcome.validation(), ArtifactId::new(13));
    assert_eq!(executor.artifacts().len(), 7);
    for id in 8_u64..=13 {
        assert_eq!(
            executor
                .artifacts()
                .get(ArtifactId::new(id))
                .and_then(Artifact::owner),
            Some(WorkflowId::new(2))
        );
    }
    let mut event_count = 0_usize;
    while let Some(event) = executor.poll_event() {
        assert_eq!(event.workflow(), WorkflowId::new(2));
        event_count += 1;
    }
    assert_eq!(event_count, 13);
    Ok(())
}

#[test]
fn explicit_release_preserves_specification_other_workflow_and_events() -> Result<(), Box<dyn Error>>
{
    let (mut executor, _) = executor(
        [
            Ok("draft one".to_owned()),
            Ok("review one".to_owned()),
            Ok("revision one".to_owned()),
            Ok("draft two".to_owned()),
            Ok("review two".to_owned()),
            Ok("revision two".to_owned()),
        ],
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ],
    )?;
    let specification = executor.insert_specification("shared spec".to_owned())?;
    let first = executor.execute(specification, configuration(1)?)?;
    let second = executor.execute(specification, configuration(1)?)?;

    assert_eq!(executor.artifacts().len(), 13);
    assert_eq!(executor.release_workflow(first.workflow()), 6);
    assert_eq!(executor.release_workflow(first.workflow()), 0);
    assert_eq!(executor.release_workflow(WorkflowId::new(99)), 0);
    assert_eq!(executor.artifacts().len(), 7);
    assert_eq!(
        executor
            .artifacts()
            .get(specification)
            .and_then(Artifact::owner),
        None
    );
    for id in 2_u64..=7 {
        assert!(executor.artifacts().get(ArtifactId::new(id)).is_none());
    }
    for id in 8_u64..=13 {
        assert_eq!(
            executor
                .artifacts()
                .get(ArtifactId::new(id))
                .and_then(Artifact::owner),
            Some(second.workflow())
        );
    }

    let mut first_events = 0_usize;
    let mut second_events = 0_usize;
    while let Some(event) = executor.poll_event() {
        if event.workflow() == first.workflow() {
            first_events += 1;
        } else if event.workflow() == second.workflow() {
            second_events += 1;
        }
    }
    assert_eq!(first_events, 13);
    assert_eq!(second_events, 13);

    assert_eq!(executor.release_workflow(second.workflow()), 6);
    assert_eq!(executor.artifacts().len(), 1);
    assert!(executor.artifacts().get(specification).is_some());
    Ok(())
}
