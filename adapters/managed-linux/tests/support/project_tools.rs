//! A controlled model proposes one real managed process tool through ordinary task execution.
use super::{Result, support};
use milkdrift_authority::CapabilityExecutionRequirements;
use milkdrift_blueprint::*;
use milkdrift_capability::*;
use milkdrift_capability_host::*;
use milkdrift_managed_linux::*;
use milkdrift_model::*;
use milkdrift_persistence::{managed::*, *};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{ManualClock, SequentialIdGenerator, TaskRuntime};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn failure(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::rejected(error.to_string())
}
fn envelope() -> InvocationAdmissionEnvelope {
    InvocationAdmissionEnvelope::new(
        AdmissionUnit::ModelTokens,
        AdmissionBound::Bounded(4096),
        AdmissionBound::Bounded(4096),
        AdmissionBound::Bounded(65_536),
        AdmissionBound::NotApplicable,
    )
    .with_execution_duration_ms(AdmissionBound::Bounded(1000))
}

struct ControlledModel {
    descriptor: CapabilityDescriptor,
    data: Arc<StoreInvocationDataAccess>,
    calls: AtomicUsize,
    store: Arc<RedbStore>,
    installation: milkdrift_capability::managed::ManagedName,
    expect_refusal: bool,
}

struct ModelAuthority;
impl milkdrift_authority::AuthorityEvaluator for ModelAuthority {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> std::result::Result<
        milkdrift_authority::AuthorityDecisionSnapshot,
        milkdrift_authority::AuthorityError,
    > {
        use milkdrift_authority::{
            AuthorityBudget, AuthorityDecisionSnapshot, DecisionReasonCode, PolicyId,
        };
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("managed-project-test")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                cost_minor: Some(1000),
                duration_ms: Some(10_000_000),
                invocations: Some(64),
                artifact_bytes: Some(67_108_864),
                units: Some(100_000),
                concurrency: Some(4),
            },
            SideEffectClass::Unknown,
        )
    }
}
impl CapabilityAdapter for ControlledModel {
    fn admission_envelope(
        &self,
        _: &AdapterInvocation<'_>,
    ) -> std::result::Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(envelope())
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }
    fn start(&self) -> std::result::Result<(), AdapterError> {
        Ok(())
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> std::result::Result<(), AdapterError> {
        let second = self.calls.fetch_add(1, Ordering::SeqCst) != 0;
        if second
            && self
                .store
                .managed_installation(&self.installation)
                .map_err(failure)?
                .is_none_or(|record| !record.uses.is_empty())
        {
            return Err(failure(
                "completed child kept its managed generation lifetime",
            ));
        }
        if second {
            let context = invocation
                .context()
                .ok_or_else(|| failure("context missing"))?;
            let input = invocation
                .request()
                .inputs()
                .iter()
                .find(|input| input.name() == MODEL_TASK_INPUT_NAME)
                .ok_or_else(|| failure("model request missing"))?;
            let bytes = self
                .data
                .read_input_bytes(
                    context,
                    input,
                    MaterializationLimits {
                        max_files: 256,
                        max_file_bytes: 2_097_152,
                        max_total_bytes: 2_097_152,
                        max_path_bytes: 4096,
                        max_directory_depth: 64,
                        chunk_bytes: 65_536,
                    },
                )
                .map_err(failure)?;
            let task = ModelTaskRequestDocument::from_json(&bytes).map_err(failure)?;
            let text = task
                .body()
                .messages()
                .iter()
                .filter(|message| message.role() == MessageRole::ToolResult)
                .flat_map(|message| message.parts())
                .find_map(|part| match part {
                    ContentPart::Text { text } => Some(text),
                    _ => None,
                })
                .ok_or_else(|| failure("managed process tool feedback absent"))?;
            let feedback: serde_json::Value = serde_json::from_str(text).map_err(failure)?;
            let result: serde_json::Value = serde_json::from_str(
                feedback
                    .pointer("/outputs/worker_result/text")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| failure("worker output absent"))?,
            )
            .map_err(failure)?;
            if result.get("success").and_then(serde_json::Value::as_bool)
                != Some(!self.expect_refusal)
            {
                return Err(failure(format!(
                    "unexpected physical project tool result: {result}"
                )));
            }
            if self.expect_refusal
                && !result
                    .get("stderr")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|text| {
                        text.contains("error mounting")
                            && text.contains("/proc/")
                            && text.contains("invalid argument")
                    })
            {
                return Err(failure(
                    "expected exact runtime fd-mount refusal was not observed",
                ));
            }
        }
        let calls = if second {
            Vec::new()
        } else {
            vec![ToolCall::new("write-project", "process_0", BoundedJson::new(serde_json::json!({
                "inputs": {"command":{"argv":["/bin/sh","-c","set -eu; test \"$PWD\" = /project; test \"$(cat retained)\" = retained; printf model-tool > from-model"]}}
            })).map_err(failure)?).map_err(failure)?]
        };
        let response = ModelResponse::new(
            if second {
                if self.expect_refusal {
                    "Managed runtime refusal observed".into()
                } else {
                    "Managed project edit completed".into()
                }
            } else {
                String::new()
            },
            None,
            calls,
            if second {
                FinishReason::Stop
            } else {
                FinishReason::ToolCalls
            },
            Usage {
                input_units: Some(1),
                output_units: Some(1),
                cached_input_units: None,
                cost_micros: None,
                currency: None,
            },
            BTreeMap::new(),
        )
        .map_err(failure)?;
        let bytes = ModelResponseDocument::new(response)
            .to_canonical_json()
            .map_err(failure)?;
        let reference = self
            .data
            .publish_bytes(
                invocation
                    .context()
                    .ok_or_else(|| failure("task context missing"))?,
                invocation.request(),
                "model_response",
                "application/vnd.milkdrift.model-response+json",
                &bytes,
                MaterializationLimits {
                    max_files: 1,
                    max_file_bytes: 65_536,
                    max_total_bytes: 65_536,
                    max_path_bytes: 256,
                    max_directory_depth: 8,
                    chunk_bytes: 65_536,
                },
            )
            .map_err(failure)?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: "model_response".into(),
                    reference: reference.clone(),
                },
            )
            .map_err(failure)?,
        )?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                2,
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        vec![reference],
                        None,
                        Some(
                            UsageObservation::new(
                                Some(1),
                                Some(1),
                                Some(1),
                                None,
                                None,
                                BTreeMap::new(),
                            )
                            .map_err(failure)?,
                        ),
                        SideEffectClass::ReadOnly,
                    )
                    .map_err(failure)?,
                },
            )
            .map_err(failure)?,
        )
    }
    fn health(&self, now: u64) -> std::result::Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.descriptor.identity().clone(),
            now,
            true,
            0,
            "controlled model ready",
        )
        .map_err(failure)
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> std::result::Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            false,
            false,
            None,
        )
        .map_err(failure)
    }
    fn begin_drain(&self) -> std::result::Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> std::result::Result<(), AdapterError> {
        Ok(())
    }
}

pub(super) fn run(
    store: Arc<RedbStore>,
    platform: Arc<LinuxManagedPlatform>,
    setup: &ApprovedSetup,
    evidence: &Path,
    expect_refusal: bool,
) -> Result {
    let helper = std::path::PathBuf::from(std::env::var("MILKDRIFT_PROJECT_PIN_HELPER")?);
    let project_dir = tempfile::Builder::new()
        .prefix("milkdrift-model-project-")
        .tempdir()?
        .keep();
    std::fs::write(project_dir.join("retained"), b"retained")?;
    std::fs::set_permissions(&project_dir, std::fs::Permissions::from_mode(0o750))?;
    std::fs::set_permissions(
        project_dir.join("retained"),
        std::fs::Permissions::from_mode(0o640),
    )?;
    let before = project_dir.join("retained").metadata()?;
    let folder = capture_native_project_root(&project_dir, true)?;
    let worker = setup
        .capabilities
        .iter()
        .find(|descriptor| descriptor.category() == &CapabilityCategory::Process)
        .ok_or("worker absent")?
        .clone();
    let operation = OperationId::new("workspace.execute")?;
    let tool = TaskTool::Process {
        capability: CapabilityRequirement::new(operation.clone()).exact(worker.identity().clone()),
    };
    let settings = ProjectSettings {
        environment: ProjectEnvironment::Podman {
            capability: worker.identity().as_str().into(),
            operation: operation.as_str().into(),
            generation: worker.descriptor_revision(),
        },
        allow_changes: true,
    };
    let registration = ProjectRecord {
        id: "project:managed-tool".into(),
        name: DaemonDisplayName::new("Managed tool project")?,
        generation: 1,
        folder: folder.clone(),
        settings: settings.clone(),
    };
    store.mutate_project(
        ProjectOperation {
            id: "project-operation:managed-tool".into(),
            actor: support::caller()?.actor,
            key: "managed-tool-project".into(),
            canonical_request: br#"{"attach":"managed-tool-project"}"#.to_vec(),
            state: ProjectOperationState::Accepted {
                project: None,
                workflow: None,
            },
        },
        ProjectMutation::Attach(registration.clone()),
    )?;
    let data = Arc::new(
        StoreInvocationDataAccess::new(
            store.clone(),
            evidence.join("model-project-publications"),
            ArtifactReadAuthority::PublicOnly,
        )?
        .with_task_authority(Arc::new(ModelAuthority), Arc::new(ManualClock::new(1000))),
    );
    let host = Arc::new(CapabilityHost::new(
        HostConfig {
            max_registrations: 2,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?);
    host.install_native_path_store(store.clone(), Vec::new())?;
    host.register(
        worker.clone(),
        Arc::new(
            ManagedWorkerAdapter::new(platform, store.clone(), setup.clone(), data.clone())?
                .with_project_pin_helper(helper),
        ),
        Some(CapabilityObservation::new(
            worker.identity().clone(),
            1000,
            true,
            0,
            "verified physical worker",
        )?),
    )?;
    let schema = SchemaContract::new(
        SchemaId::new("milkdrift.controlled-model")?,
        1,
        BoundedJson::new(serde_json::json!({"type":"object"}))?,
    )?;
    let model = DescriptorBuilder::new(
        CapabilityId::new("controlled-model")?,
        1,
        CapabilityCategory::Model,
        AdmissionConstraints::new(1, 0)?,
        Locality::Local,
    )
    .operations(BTreeMap::from([(
        OperationId::new("model.generate")?,
        OperationContract::new(
            schema.clone(),
            schema,
            BTreeSet::from([StreamingMode::None]),
            CancellationBehavior::BestEffort,
            IdempotencyBehavior::Unsupported,
            SideEffectClass::ReadOnly,
            BTreeMap::new(),
        )?,
    )]))
    .build()?;
    let authority = Arc::new(ModelAuthority);
    let managed: milkdrift_capability::managed::ManagedBinding = serde_json::from_value(
        worker
            .extensions()
            .get(&ExtensionKey::new(
                milkdrift_capability::managed::MANAGED_BINDING_EXTENSION,
            )?)
            .ok_or("worker binding missing")?
            .value()
            .clone(),
    )?;
    let controlled = Arc::new(ControlledModel {
        descriptor: model.clone(),
        data: data.clone(),
        calls: AtomicUsize::new(0),
        store: store.clone(),
        installation: managed.installation,
        expect_refusal,
    });
    host.register(
        model.clone(),
        Arc::new(ModelToolLoopAdapter::new(
            model.clone(),
            controlled.clone(),
            envelope(),
            host.as_ref().clone(),
            data,
            authority.clone(),
            Arc::new(|| Ok(1000)),
        )),
        Some(CapabilityObservation::new(
            model.identity().clone(),
            1000,
            true,
            0,
            "controlled model ready",
        )?),
    )?;
    let runtime = TaskRuntime::new(
        store.clone(),
        host,
        authority,
        Arc::new(ManualClock::new(1000)),
        Arc::new(SequentialIdGenerator::new("managed-tool", 1)?),
    );
    let paths = vec![TaskPathAccess {
        path: project_dir.to_string_lossy().into_owned(),
        write: true,
    }];
    let pin = TaskProjectFolder {
        requested: folder.requested.to_string_lossy().into_owned(),
        physical: folder.physical.to_string_lossy().into_owned(),
        identity: folder.identity,
    };
    let binding = TaskProjectBinding {
        paths: paths.clone(),
        scopes: vec![pin.clone()],
        project_id: registration.id.clone(),
        generation: 1,
        folder: pin,
        settings,
        access: TaskProjectAccess::Write,
        tools: vec![tool.clone()],
    };
    let mut start = support::caller()?;
    start.operation = milkdrift_authority::AuthorityOperation::StartRun;
    start.resources.workflow = Some(TaskRuntime::workflow_identity(
        "human:managed",
        "managed-project-model-tool",
    )?);
    let receipt = runtime.create(
        "human:managed",
        "managed-project-model-tool",
        WorkflowCreate {
            project_id: Some(registration.id),
            title: "Managed project model tool".into(),
            description: String::new(),
            tasks: vec![NewTask {
                alias: TaskId::new("model")?,
                definition: TaskDefinition {
                    capability_generation: None,
                    execution: TaskExecution::Bound { binding },
                    title: "Edit attached project".into(),
                    description: String::new(),
                    instructions: "Use the configured tool once, then finish.".into(),
                    capability: CapabilityRequirement::new(OperationId::new("model.generate")?),
                    inputs: BTreeMap::new(),
                    dependencies: Default::default(),
                    outputs: BTreeMap::new(),
                    required: true,
                    corrective_of: None,
                    paths,
                    tools: vec![tool],
                    actions: None,
                },
            }],
            inputs: BTreeMap::new(),
            limits: WorkflowLimits {
                tool_calls_per_task: 1,
                ..WorkflowLimits::default()
            },
            finish_when_settled: true,
        },
        milkdrift_authority::AuthorityEvaluator::evaluate(&ModelAuthority, &start)?,
    )?;
    let dispatches = runtime.tick(1)?;
    assert_eq!(dispatches.len(), 1);
    for dispatch in dispatches {
        runtime.execute(dispatch)?;
    }
    let workflow = runtime
        .workflow(&receipt.workflow)?
        .ok_or("workflow disappeared")?;
    assert_eq!(
        controlled.calls.load(Ordering::SeqCst),
        2,
        "workflow {:?}, question {:?}, task reasons {:?}",
        workflow.state,
        workflow.wait_question,
        workflow
            .tasks
            .values()
            .map(|task| &task.reason)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        workflow.state,
        milkdrift_runtime::WorkflowState::Completed,
        "{:?}",
        workflow.wait_question
    );
    assert!(
        workflow
            .tasks
            .values()
            .all(|task| task.state == milkdrift_runtime::TaskState::Completed)
    );
    if expect_refusal {
        assert!(!project_dir.join("from-model").exists());
    } else {
        assert_eq!(
            std::fs::read(project_dir.join("from-model"))?,
            b"model-tool"
        );
    }
    assert_eq!(std::fs::read(project_dir.join("retained"))?, b"retained");
    let after = project_dir.join("retained").metadata()?;
    assert_eq!(
        (before.uid(), before.gid(), before.mode()),
        (after.uid(), after.gid(), after.mode())
    );
    store.verify_managed_integrity()?;
    std::fs::write(
        evidence.join("project-model-tool.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "workflow": receipt.workflow, "project": project_dir, "model_rounds": 2,
            "worker_capability": worker.identity(), "worker_generation": worker.descriptor_revision(),
        "host_file": "from-model", "host_file_created": !expect_refusal,
        "retained_uid":after.uid(),"retained_gid":after.gid(),"retained_mode":after.mode(),"retained_bytes":"retained",
        "runtime_fd_mount_refused": expect_refusal, "child_generation_hold_released_after_physical_absence": true
        }))?,
    )?;
    Ok(())
}
