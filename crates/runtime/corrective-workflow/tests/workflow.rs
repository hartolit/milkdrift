//! Data-defined corrective workflow integration tests.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::{NonZeroU16, NonZeroU32};
use std::rc::Rc;

use corrective_workflow::{
    ArtifactId, ArtifactKind, ArtifactRole, BoundedDiagnosticsSink, BoundedTextSink,
    CorrectiveOperation, CorrectiveTask, CorrectiveWorkflowDefinition, CorrectiveWorkflowExecutor,
    ModelOperation, ModelPolicy, ModelTaskContext, ModelTaskExecutor, OutputSinkError,
    ReferenceCorrectiveConfiguration, ReferenceCorrectiveTemplate, TokenBudget,
    ValidationOperation, ValidationReport, ValidationTaskContext, ValidationTaskExecutor,
    ValidationVerdict, WorkflowDefinitionError, WorkflowError, WorkflowEvent,
    WorkflowExecutorLimits, WorkflowId, WorkflowInputBinding, WorkflowShapeLimits, WorkflowStage,
    WorkflowStatus,
};
use task_graph::{TaskArtifactInput, TaskArtifactOutput, TaskDependency, TaskId, TaskNode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TestPortError(&'static str);

impl Display for TestPortError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for TestPortError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RecordedCall {
    Model {
        operation: ModelOperation,
        definition_task: TaskId,
        runtime_task: TaskId,
        attempt: u16,
        inputs: Vec<ArtifactId>,
    },
    Validation {
        operation: ValidationOperation,
        definition_task: TaskId,
        runtime_task: TaskId,
        attempt: u16,
        inputs: Vec<ArtifactId>,
    },
}

type CallLog = Rc<RefCell<Vec<RecordedCall>>>;

struct RecordingModel {
    calls: CallLog,
    responses: VecDeque<Result<String, TestPortError>>,
    undeclared_probe: Rc<Cell<ArtifactId>>,
    denied: Rc<RefCell<Vec<bool>>>,
}

impl ModelTaskExecutor for RecordingModel {
    type Error = TestPortError;

    fn execute_model_task(
        &mut self,
        context: ModelTaskContext<'_>,
        output: &mut BoundedTextSink,
    ) -> Result<(), Self::Error> {
        let inputs = context.artifacts.ids();
        if inputs.iter().any(|id| context.artifacts.get(*id).is_none()) {
            return Err(TestPortError("declared model input unavailable"));
        }
        let probe = self.undeclared_probe.get();
        if probe.get() != 0 {
            self.denied
                .borrow_mut()
                .push(context.artifacts.get(probe).is_none());
        }
        self.calls.borrow_mut().push(RecordedCall::Model {
            operation: context.operation,
            definition_task: context.definition_task,
            runtime_task: context.attempt.task,
            attempt: context.attempt.number.get(),
            inputs: inputs.to_vec(),
        });
        match self
            .responses
            .pop_front()
            .ok_or(TestPortError("missing model response"))?
        {
            Ok(response) => output
                .append(&response)
                .map_err(|_| TestPortError("model sink failed")),
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
        context: ValidationTaskContext<'_>,
        output: &mut BoundedDiagnosticsSink,
    ) -> Result<ValidationVerdict, Self::Error> {
        let inputs = context.artifacts.ids();
        if inputs.iter().any(|id| context.artifacts.get(*id).is_none()) {
            return Err(TestPortError("declared validator input unavailable"));
        }
        self.calls.borrow_mut().push(RecordedCall::Validation {
            operation: context.operation,
            definition_task: context.definition_task,
            runtime_task: context.attempt.task,
            attempt: context.attempt.number.get(),
            inputs: inputs.to_vec(),
        });
        match self
            .responses
            .pop_front()
            .ok_or(TestPortError("missing validator response"))?
        {
            Ok(report) => {
                for diagnostic in &report.diagnostics {
                    output
                        .append(diagnostic)
                        .map_err(|_| TestPortError("validator sink failed"))?;
                }
                Ok(report.verdict)
            }
            Err(error) => Err(error),
        }
    }
}

type TestExecutor = CorrectiveWorkflowExecutor<RecordingModel, RecordingValidator>;

struct Fixture {
    executor: TestExecutor,
    calls: CallLog,
    undeclared_probe: Rc<Cell<ArtifactId>>,
    denied: Rc<RefCell<Vec<bool>>>,
}

fn report(verdict: ValidationVerdict) -> ValidationReport {
    ValidationReport {
        verdict,
        diagnostics: Vec::new(),
    }
}

fn shape() -> Result<WorkflowShapeLimits, WorkflowError> {
    WorkflowShapeLimits::new(8, 16, 16, 20)
}

fn limits(artifacts: usize, events: usize) -> Result<WorkflowExecutorLimits, WorkflowError> {
    WorkflowExecutorLimits::new(artifacts, events, 4_096, shape()?)
}

fn fixture(
    model: impl IntoIterator<Item = Result<String, TestPortError>>,
    validation: impl IntoIterator<Item = Result<ValidationReport, TestPortError>>,
    limits: WorkflowExecutorLimits,
) -> Fixture {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let undeclared_probe = Rc::new(Cell::new(ArtifactId::new(0)));
    let denied = Rc::new(RefCell::new(Vec::new()));
    let model = RecordingModel {
        calls: Rc::clone(&calls),
        responses: model.into_iter().collect(),
        undeclared_probe: Rc::clone(&undeclared_probe),
        denied: Rc::clone(&denied),
    };
    let validator = RecordingValidator {
        calls: Rc::clone(&calls),
        responses: validation.into_iter().collect(),
    };
    Fixture {
        executor: CorrectiveWorkflowExecutor::new(model, validator, limits),
        calls,
        undeclared_probe,
        denied,
    }
}

fn configuration(attempts: u16) -> Result<ReferenceCorrectiveConfiguration, TestPortError> {
    let mut configuration = ReferenceCorrectiveConfiguration::default();
    let attempts = NonZeroU16::new(attempts).ok_or(TestPortError("attempts must be non-zero"))?;
    configuration.model_attempts = attempts;
    configuration.validation_attempts = attempts;
    Ok(configuration)
}

fn success_fixture(run_count: usize) -> Result<Fixture, WorkflowError> {
    let model = (0..run_count).flat_map(|run| {
        [
            Ok(format!("draft {run}")),
            Ok(format!("review {run}")),
            Ok(format!("revision {run}")),
        ]
    });
    let validation = (0..run_count).flat_map(|_| {
        [
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ]
    });
    Ok(fixture(model, validation, limits(64, 256)?))
}

#[test]
fn output_sinks_are_bounded_atomic_and_sticky() -> Result<(), Box<dyn Error>> {
    let mut text = BoundedTextSink::new(5);
    text.append("abc")?;
    let failure = OutputSinkError::CapacityExceeded {
        required: 6,
        maximum: 5,
    };
    assert_eq!(text.append("def"), Err(failure));
    assert_eq!(text.as_str(), "abc");
    assert_eq!(text.append("d"), Err(failure));

    assert_eq!(
        BoundedDiagnosticsSink::new(0).err(),
        Some(OutputSinkError::CapacityExceeded {
            required: 1,
            maximum: 0,
        })
    );
    Ok(())
}

#[test]
fn reference_template_constructs_and_validates_as_borrowed_data() -> Result<(), Box<dyn Error>> {
    let template = ReferenceCorrectiveTemplate::new(configuration(2)?)?;
    let definition = template.definition();
    let mut incoming = [0_u32; 6];
    let mut queue = [0_usize; 6];

    definition
        .validate(shape()?, &mut incoming, &mut queue)
        .map_err(WorkflowError::InvalidDefinition)?;

    assert_eq!(definition.nodes.len(), 6);
    assert_eq!(definition.dependencies.len(), 8);
    assert_eq!(definition.task_inputs.len(), 11);
    assert_eq!(definition.task_outputs.len(), 6);
    Ok(())
}

#[test]
fn reference_template_rejects_zero_output_limit() -> Result<(), Box<dyn Error>> {
    let mut configuration = configuration(1)?;
    configuration.artifact_limits.revision = 0;
    assert_eq!(
        ReferenceCorrectiveTemplate::new(configuration).err(),
        Some(WorkflowError::InvalidDefinition(
            WorkflowDefinitionError::ZeroOutputLimit(ArtifactId::new(6))
        ))
    );
    Ok(())
}

#[test]
fn reference_execution_uses_definition_order_and_committed_inputs() -> Result<(), Box<dyn Error>> {
    let mut fixture = success_fixture(1)?;
    let specification = fixture
        .executor
        .insert_specification("specification".to_owned())?;

    let outcome = fixture
        .executor
        .execute_reference(specification, configuration(1)?)?;

    assert_eq!(outcome.status(), WorkflowStatus::Accepted);
    assert_eq!(outcome.result(), ArtifactId::new(6));
    assert_eq!(outcome.validation(), ArtifactId::new(7));
    let operations: Vec<_> = fixture
        .calls
        .borrow()
        .iter()
        .map(|call| match call {
            RecordedCall::Model { operation, .. } => format!("model:{operation:?}"),
            RecordedCall::Validation { operation, .. } => format!("validation:{operation:?}"),
        })
        .collect();
    assert_eq!(
        operations,
        [
            "model:Draft",
            "validation:CompileCheck",
            "model:Review",
            "model:Revise",
            "validation:Validate",
        ]
    );
    assert_eq!(fixture.executor.artifacts().len(), 7);
    Ok(())
}

#[test]
fn structurally_different_definition_uses_same_executor_and_ready_order()
-> Result<(), Box<dyn Error>> {
    let mut fixture = fixture(
        [Ok("first ready".to_owned()), Ok("second ready".to_owned())],
        [Ok(report(ValidationVerdict::Passed))],
        limits(16, 16)?,
    );
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let definition = small_parallel_definition();
    let inputs = [WorkflowInputBinding {
        definition: ArtifactId::new(100),
        artifact: specification,
    }];

    let outcome = fixture.executor.execute(&definition, &inputs)?;

    assert_eq!(outcome.result(), ArtifactId::new(3));
    let definition_tasks: Vec<TaskId> = fixture
        .calls
        .borrow()
        .iter()
        .map(|call| match call {
            RecordedCall::Model {
                definition_task, ..
            }
            | RecordedCall::Validation {
                definition_task, ..
            } => *definition_task,
        })
        .collect();
    assert_eq!(
        definition_tasks,
        [TaskId::new(20), TaskId::new(10), TaskId::new(30)]
    );
    Ok(())
}

#[test]
fn artifact_and_event_capacity_preflight_before_port_side_effects() -> Result<(), Box<dyn Error>> {
    let mut artifact_fixture = fixture([Ok("unused".to_owned())], [], limits(6, 64)?);
    let specification = artifact_fixture
        .executor
        .insert_specification("spec".to_owned())?;
    assert_eq!(
        artifact_fixture
            .executor
            .execute_reference(specification, configuration(1)?),
        Err(WorkflowError::ArtifactCapacityExceeded {
            required: 6,
            available: 5,
        })
    );
    assert!(artifact_fixture.calls.borrow().is_empty());

    let mut event_fixture = fixture([Ok("unused".to_owned())], [], limits(8, 12)?);
    let specification = event_fixture
        .executor
        .insert_specification("spec".to_owned())?;
    assert_eq!(
        event_fixture
            .executor
            .execute_reference(specification, configuration(1)?),
        Err(WorkflowError::EventCapacityExceeded {
            required: 13,
            available: 12,
        })
    );
    assert!(event_fixture.calls.borrow().is_empty());
    Ok(())
}

#[test]
fn model_and_validator_failures_retry_then_complete() -> Result<(), Box<dyn Error>> {
    let mut fixture = fixture(
        [
            Err(TestPortError("draft retry")),
            Ok("draft".to_owned()),
            Ok("review".to_owned()),
            Ok("revision".to_owned()),
        ],
        [
            Err(TestPortError("validation retry")),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Rejected)),
        ],
        limits(16, 23)?,
    );
    let specification = fixture.executor.insert_specification("spec".to_owned())?;

    let outcome = fixture
        .executor
        .execute_reference(specification, configuration(2)?)?;

    assert_eq!(outcome.status(), WorkflowStatus::Rejected);
    let retry_count = std::iter::from_fn(|| fixture.executor.poll_event())
        .filter(|event| matches!(event, WorkflowEvent::RetryScheduled { .. }))
        .count();
    assert_eq!(retry_count, 2);
    Ok(())
}

#[test]
fn output_failure_is_non_retryable_and_rolls_back() -> Result<(), Box<dyn Error>> {
    let mut fixture = fixture([Ok("oversized".to_owned())], [], limits(16, 64)?);
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let mut configuration = configuration(2)?;
    configuration.artifact_limits.draft = 4;

    let error = fixture
        .executor
        .execute_reference(specification, configuration)
        .err()
        .ok_or(TestPortError("oversized output succeeded"))?;

    assert!(matches!(
        error,
        WorkflowError::OutputCapacityExceeded {
            stage: WorkflowStage::Draft,
            required: 9,
            maximum: 4,
            ..
        }
    ));
    assert_eq!(fixture.calls.borrow().len(), 1);
    assert_eq!(fixture.executor.artifacts().len(), 1);
    assert_eq!(fixture.executor.poll_event(), None);
    Ok(())
}

#[test]
fn late_failure_rolls_back_and_identifiers_are_not_reused() -> Result<(), Box<dyn Error>> {
    let mut fixture = fixture(
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
            Err(TestPortError("late failure")),
            Ok(report(ValidationVerdict::Passed)),
            Ok(report(ValidationVerdict::Passed)),
        ],
        limits(16, 64)?,
    );
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let error = fixture
        .executor
        .execute_reference(specification, configuration(1)?)
        .err()
        .ok_or(TestPortError("late failure succeeded"))?;
    assert!(matches!(error, WorkflowError::TaskExhausted { task, .. } if task == TaskId::new(6)));
    assert_eq!(fixture.executor.artifacts().len(), 1);
    assert_eq!(fixture.executor.poll_event(), None);

    let outcome = fixture
        .executor
        .execute_reference(specification, configuration(1)?)?;
    assert_eq!(outcome.workflow(), WorkflowId::new(2));
    assert_eq!(outcome.result(), ArtifactId::new(12));
    assert_eq!(outcome.validation(), ArtifactId::new(13));
    assert!(fixture.calls.borrow().iter().any(|call| matches!(
        call,
        RecordedCall::Model {
            definition_task,
            runtime_task,
            ..
        } if *definition_task == TaskId::new(1) && *runtime_task == TaskId::new(7)
    )));
    Ok(())
}

#[test]
fn undeclared_prior_artifact_is_invisible_and_release_is_explicit() -> Result<(), Box<dyn Error>> {
    let mut fixture = success_fixture(2)?;
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let first = fixture
        .executor
        .execute_reference(specification, configuration(1)?)?;
    fixture.undeclared_probe.set(ArtifactId::new(2));
    let second = fixture
        .executor
        .execute_reference(specification, configuration(1)?)?;

    assert_eq!(fixture.denied.borrow().as_slice(), [true, true, true]);
    assert_eq!(fixture.executor.release_workflow(first.workflow()), 6);
    assert_eq!(fixture.executor.release_workflow(first.workflow()), 0);
    assert!(fixture.executor.artifacts().get(specification).is_some());
    assert!(fixture.executor.artifacts().get(first.result()).is_none());
    assert!(fixture.executor.artifacts().get(second.result()).is_some());
    Ok(())
}

#[test]
fn cancellation_marks_graph_state_then_rolls_back_workflow_visibility() -> Result<(), Box<dyn Error>>
{
    let mut fixture = success_fixture(1)?;
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let template = ReferenceCorrectiveTemplate::new(configuration(1)?)?;
    let inputs = [WorkflowInputBinding {
        definition: template.specification_input(),
        artifact: specification,
    }];

    let error = fixture
        .executor
        .execute_with_cancellation(&template.definition(), &inputs, |request| {
            request.stage == WorkflowStage::Review
        })
        .err()
        .ok_or(TestPortError("cancelled workflow succeeded"))?;

    assert!(matches!(
        error,
        WorkflowError::Cancelled {
            stage: WorkflowStage::Review,
            task,
            ..
        } if task == TaskId::new(4)
    ));
    assert_eq!(fixture.executor.artifacts().len(), 1);
    assert_eq!(fixture.executor.poll_event(), None);
    Ok(())
}

#[test]
fn undeclared_input_binding_is_rejected_before_calls() -> Result<(), Box<dyn Error>> {
    let mut fixture = success_fixture(1)?;
    let specification = fixture.executor.insert_specification("spec".to_owned())?;
    let template = ReferenceCorrectiveTemplate::new(configuration(1)?)?;
    let inputs = [WorkflowInputBinding {
        definition: ArtifactId::new(999),
        artifact: specification,
    }];

    assert_eq!(
        fixture.executor.execute(&template.definition(), &inputs),
        Err(WorkflowError::UnknownWorkflowInput(ArtifactId::new(999)))
    );
    assert!(fixture.calls.borrow().is_empty());
    Ok(())
}

#[test]
fn token_budget_and_policy_are_corrective_owned() {
    let budget = TokenBudget::new(NonZeroU32::MIN, NonZeroU32::MIN);
    let operation = CorrectiveOperation::Model {
        operation: ModelOperation::Draft,
        policy: ModelPolicy::AnyCompatible,
        token_budget: budget,
    };
    assert!(matches!(operation, CorrectiveOperation::Model { .. }));
}

fn small_parallel_definition<'a>() -> CorrectiveWorkflowDefinition<'a> {
    static NODES: [TaskNode<CorrectiveTask>; 3] = [
        model_node(20, WorkflowStage::Other(20)),
        model_node(10, WorkflowStage::Other(10)),
        validation_node(30),
    ];
    static DEPENDENCIES: [TaskDependency; 1] = [TaskDependency {
        prerequisite: TaskId::new(10),
        dependent: TaskId::new(30),
    }];
    static ARTIFACTS: [corrective_workflow::ArtifactDefinition; 4] = [
        artifact(100, ArtifactKind::Text, ArtifactRole::Specification, 0),
        artifact(101, ArtifactKind::Text, ArtifactRole::Draft, 64),
        artifact(102, ArtifactKind::Text, ArtifactRole::Draft, 64),
        artifact(
            103,
            ArtifactKind::Diagnostics,
            ArtifactRole::FinalValidation,
            64,
        ),
    ];
    static EXTERNAL: [ArtifactId; 1] = [ArtifactId::new(100)];
    static INPUTS: [TaskArtifactInput; 3] = [
        TaskArtifactInput {
            consumer: TaskId::new(20),
            artifact: ArtifactId::new(100),
        },
        TaskArtifactInput {
            consumer: TaskId::new(10),
            artifact: ArtifactId::new(100),
        },
        TaskArtifactInput {
            consumer: TaskId::new(30),
            artifact: ArtifactId::new(102),
        },
    ];
    static OUTPUTS: [TaskArtifactOutput; 3] = [
        TaskArtifactOutput {
            producer: TaskId::new(20),
            artifact: ArtifactId::new(101),
        },
        TaskArtifactOutput {
            producer: TaskId::new(10),
            artifact: ArtifactId::new(102),
        },
        TaskArtifactOutput {
            producer: TaskId::new(30),
            artifact: ArtifactId::new(103),
        },
    ];
    CorrectiveWorkflowDefinition {
        nodes: &NODES,
        dependencies: &DEPENDENCIES,
        artifacts: &ARTIFACTS,
        external_inputs: &EXTERNAL,
        task_inputs: &INPUTS,
        task_outputs: &OUTPUTS,
        terminal_result: ArtifactId::new(102),
        terminal_validation: ArtifactId::new(103),
    }
}

const fn model_node(id: u64, stage: WorkflowStage) -> TaskNode<CorrectiveTask> {
    TaskNode {
        id: TaskId::new(id),
        operation: CorrectiveTask {
            stage,
            operation: CorrectiveOperation::Model {
                operation: ModelOperation::Draft,
                policy: ModelPolicy::AnyCompatible,
                token_budget: TokenBudget::new(NonZeroU32::MIN, NonZeroU32::MIN),
            },
        },
        maximum_attempts: NonZeroU16::MIN,
    }
}

const fn validation_node(id: u64) -> TaskNode<CorrectiveTask> {
    TaskNode {
        id: TaskId::new(id),
        operation: CorrectiveTask {
            stage: WorkflowStage::FinalValidation,
            operation: CorrectiveOperation::Validate {
                operation: ValidationOperation::Validate,
                token_budget: TokenBudget::new(NonZeroU32::MIN, NonZeroU32::MIN),
            },
        },
        maximum_attempts: NonZeroU16::MIN,
    }
}

const fn artifact(
    id: u64,
    kind: ArtifactKind,
    role: ArtifactRole,
    maximum_bytes: u64,
) -> corrective_workflow::ArtifactDefinition {
    corrective_workflow::ArtifactDefinition {
        id: ArtifactId::new(id),
        kind,
        role,
        maximum_bytes,
    }
}
