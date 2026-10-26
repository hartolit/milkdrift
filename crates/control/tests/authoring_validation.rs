//! Saved review reports repairs against the content and project settings the submission will use.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityOperation, AuthorityRequest, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
};
use milkdrift_blueprint::authoring::{
    AuthoringConnection, AuthoringConnectionKind, AuthoringDocument, AuthoringSource,
};
use milkdrift_blueprint::{
    NewTask, ProjectEnvironment, ProjectSettings, TaskBatch, TaskChange, TaskExecution, TaskId,
    TaskInputBinding, TaskInputValue, TaskOutputDeclaration, TaskPathAccess, TaskProjectAccess,
    TaskProjectBinding, TaskProjectFolder, TaskTool, WorkflowCreate,
};
use milkdrift_capability::{
    ArtifactReference, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityRequirement, InvocationAdmissionEnvelope,
    InvocationEventKind, InvocationTerminal, OperationId, SideEffectClass, TaskAuthoringContract,
    TaskAuthoringPort, TaskInputMedia, TerminalStatus,
};
use milkdrift_control::{ActorAuthorityContext, AuthoringScope, ControlError, TaskControlService};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, DaemonDisplayName,
    ProjectMutation, ProjectOperation, ProjectOperationState, ProjectRecord, ProjectStore,
    WorkflowStore,
};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, ExecutionDispatch, ExecutorError,
    PreparedExecution, ResolvedCapability, SequentialIdGenerator, SystemBoundaryClock,
    TaskExecutor, TaskRuntime,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct Authority(AtomicBool, AtomicBool);
impl AuthorityEvaluator for Authority {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let denied = (request.operation == AuthorityOperation::InspectProject
            && self.0.load(Ordering::SeqCst))
            || (matches!(
                request.operation,
                AuthorityOperation::ReadArtifactMetadata | AuthorityOperation::ReadArtifactContent
            ) && self.1.load(Ordering::SeqCst));
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("authoring-validation")?,
            1,
            request.clone(),
            vec![if denied {
                DecisionReasonCode::Revoked
            } else {
                DecisionReasonCode::Allowed
            }],
            AuthorityBudget {
                invocations: Some(8),
                artifact_bytes: Some(16_777_216),
                concurrency: Some(8),
                ..AuthorityBudget::default()
            },
            SideEffectClass::Unknown,
        )
    }
}

struct Executor {
    descriptor: CapabilityDescriptor,
    inner: DeterministicExecutor,
    context_inputs: bool,
}
impl TaskExecutor for Executor {
    fn task_authoring(
        &self,
        requirement: &CapabilityRequirement,
    ) -> Result<Option<(CapabilityDescriptor, TaskAuthoringContract)>, ExecutorError> {
        if requirement.exact_capability() != Some(self.descriptor.identity()) {
            return Ok(None);
        }
        Ok(Some((
            self.descriptor.clone(),
            TaskAuthoringContract {
                label: "Evidence reader".into(),
                input_media: TaskInputMedia::TextJsonOrImage,
                inputs: if self.context_inputs {
                    Vec::new()
                } else {
                    vec![TaskAuthoringPort {
                        name: "brief".into(),
                        label: "Brief".into(),
                        required: false,
                        accepts_artifacts: true,
                        media_type: Some("text/plain".into()),
                        max_bytes: 1024,
                    }]
                },
                allows_additional_inputs: self.context_inputs,
                outputs: ["report", "diagnostic"]
                    .into_iter()
                    .map(|name| TaskAuthoringPort {
                        name: name.into(),
                        label: name.into(),
                        required: false,
                        accepts_artifacts: true,
                        media_type: Some("text/plain".into()),
                        max_bytes: milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES,
                    })
                    .collect(),
                default_output: Some("report".into()),
            },
        )))
    }
    fn pin_project_paths(
        &self,
        binding: &TaskProjectBinding,
    ) -> Result<Vec<TaskProjectFolder>, ExecutorError> {
        milkdrift_capability_host::verify_project_binding(binding)?;
        if !binding.paths.is_empty() {
            return Err(ExecutorError::BoundaryBeforeEntry(
                "review fixture does not request native file access".into(),
            ));
        }
        Ok(Vec::new())
    }
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        now: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.inner.resolve(requirement, now)
    }
    fn prepare_exact_entry<'a>(
        &'a self,
        dispatch: &ExecutionDispatch,
    ) -> Result<PreparedExecution<'a>, ExecutorError> {
        let prepared = self.inner.prepare_exact_entry(dispatch)?;
        // This fixture emits retained reports in process; it does not enter an external model
        // whose duration or spending would require a provider's enforced admission envelope.
        Ok(PreparedExecution::new(
            dispatch,
            InvocationAdmissionEnvelope::not_applicable(),
            move |dispatch, reporter| prepared.enter(dispatch, reporter),
        ))
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.inner.cancel(request)
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    store: Arc<RedbStore>,
    runtime: Arc<TaskRuntime>,
    executor: Arc<Executor>,
    control: TaskControlService,
    authority: Arc<Authority>,
    actor: ActorAuthorityContext,
}
impl Fixture {
    fn new() -> TestResult<Self> {
        Self::with_context_inputs(false)
    }

    fn with_context_inputs(context_inputs: bool) -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let store = Arc::new(RedbStore::open(root.path())?);
        let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/descriptor-v1.json"
        ))?
        .body()
        .clone();
        let executor = Arc::new(Executor {
            inner: DeterministicExecutor::new(descriptor.clone()),
            descriptor,
            context_inputs,
        });
        let authority = Arc::new(Authority::default());
        let clock = Arc::new(SystemBoundaryClock);
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            executor.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("authoring-validation", 1)?),
        ));
        let control =
            TaskControlService::new(runtime.clone(), authority.clone(), clock, store.clone())
                .with_authoring_store(store.clone())
                .with_projects(store.clone());
        let actor = ActorAuthorityContext::new(
            ActorRef::new("human:authoring-validation")?,
            CommandAuthorityClaim::new(
                GrantId::new("grant:authoring-validation")?,
                1,
                GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
                0,
            )?,
        );
        Ok(Self {
            _root: root,
            store,
            runtime,
            executor,
            control,
            authority,
            actor,
        })
    }
    fn operation(&self, key: &str) -> TestResult<ProjectOperation> {
        Ok(ProjectOperation {
            id: format!("project-operation:{key}"),
            actor: self.actor.actor().clone(),
            key: key.into(),
            canonical_request: serde_json::to_vec(&serde_json::json!({"key":key}))?,
            state: ProjectOperationState::Accepted {
                project: None,
                workflow: None,
            },
        })
    }
}

fn document() -> TestResult<AuthoringDocument> {
    Ok(serde_json::from_value(serde_json::json!({
        "tasks":[{
            "id":"candidate", "version":null, "title_edited":true,
            "contract_generation":3,
            "definition":{
                "title":"Read evidence", "description":"", "instructions":"Read the brief",
                "capability":{"id":"anthropic-primary","operation":"model.generate"},
                "inputs":{}, "outputs":{}, "dependencies":{}, "required":true,
                "corrective_of":null, "paths":[], "tools":[], "actions":null,
                "execution":{"kind":"legacy"}
            }
        }],
        "sources":[], "connections":[], "changes":[], "usual_model":null, "accepted":{}
    }))?)
}

fn scope(changes: Vec<u32>) -> AuthoringScope {
    AuthoringScope {
        tasks: vec!["candidate".into()],
        changes,
        use_current: vec![],
    }
}

#[test]
fn review_uses_the_selected_shared_input_change_for_media_checks() -> TestResult {
    let text = || TaskInputValue::Text {
        text: "Reviewed brief".into(),
    };
    let json = || TaskInputValue::Json {
        value: serde_json::json!({"brief":"structured"}),
    };
    for (original, replacement, compatible) in [(text(), json(), false), (json(), text(), true)] {
        let fixture = Fixture::new()?;
        let mut create: WorkflowCreate = serde_json::from_value(
            serde_json::json!({"title":"Prospective shared input","tasks":[]}),
        )?;
        create.inputs.insert("shared_brief".into(), original);
        let workflow = fixture
            .control
            .create(&fixture.actor, "empty", create)?
            .workflow;
        let mut document = document()?;
        document.sources.push(AuthoringSource {
            id: "brief_card".into(),
            label: "Brief".into(),
            value: Some(replacement.clone()),
            shared_name: Some("shared_brief".into()),
        });
        document.connections.push(AuthoringConnection {
            id: "brief_wire".into(),
            source: "brief_card".into(),
            target: "candidate".into(),
            input: Some("brief".into()),
            output: None,
            kind: AuthoringConnectionKind::Data,
            optional: false,
            outcome: false,
        });
        document.changes.push(TaskChange::BindInput {
            name: "shared_brief".into(),
            value: replacement.clone(),
        });
        let saved = fixture
            .control
            .checkpoint_authoring(&fixture.actor, &workflow, 0, document)?;
        let before = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let omitted = fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &scope(vec![]),
        )?;
        assert!(
            omitted
                .issues
                .iter()
                .any(|issue| issue.code == "source_change")
        );
        let review = fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &scope(vec![0]),
        )?;
        assert_eq!(fixture.runtime.workflow(&workflow)?, Some(before));
        if compatible {
            assert!(review.issues.is_empty(), "{review:?}");
            fixture.control.run_authoring(
                &fixture.actor,
                &workflow,
                "run",
                review.expected_generation,
                saved.generation,
                &scope(vec![0]),
                br#"{"key":"run"}"#,
            )?;
            let accepted = fixture
                .runtime
                .workflow(&workflow)?
                .ok_or("workflow absent")?;
            assert_eq!(accepted.inputs.get("shared_brief"), Some(&replacement));
            assert_eq!(accepted.tasks.len(), 1);
        } else {
            let issue = review
                .issues
                .iter()
                .find(|issue| issue.code == "media_mismatch")
                .ok_or("prospective incompatible media was not reported")?;
            assert_eq!(issue.task_id.as_deref(), Some("candidate"));
            assert_eq!(issue.connection_id.as_deref(), Some("brief_wire"));
            assert_eq!(issue.source_id.as_deref(), Some("brief_card"));
        }
    }
    Ok(())
}

#[test]
fn project_changes_are_local_repairs_but_project_denial_stays_global() -> TestResult {
    let fixture = Fixture::new()?;
    let folder = tempfile::tempdir()?;
    let mut project = ProjectRecord {
        id: "project:review".into(),
        name: DaemonDisplayName::new("Reviewed project")?,
        generation: 1,
        folder: milkdrift_capability_host::capture_native_project_root(folder.path(), false)?,
        settings: ProjectSettings {
            environment: ProjectEnvironment::Host,
            allow_changes: false,
        },
    };
    fixture.store.mutate_project(
        fixture.operation("attach")?,
        ProjectMutation::Attach(project.clone()),
    )?;
    let mut create: WorkflowCreate =
        serde_json::from_value(serde_json::json!({"title":"Project review","tasks":[]}))?;
    create.project_id = Some(project.id.clone());
    let workflow = fixture
        .control
        .create(&fixture.actor, "empty", create)?
        .workflow;
    let mut document = document()?;
    document
        .tasks
        .first_mut()
        .ok_or("candidate absent")?
        .definition
        .execution = TaskExecution::Explicit {
        expected_generation: 1,
        settings: project.settings.clone(),
        access: TaskProjectAccess::None,
        tools: vec![],
    };
    let saved =
        fixture
            .control
            .checkpoint_authoring(&fixture.actor, &workflow, 0, document.clone())?;
    project.generation = 2;
    fixture.store.mutate_project(
        fixture.operation("update")?,
        ProjectMutation::Update {
            project,
            expected_generation: 1,
        },
    )?;
    let before = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let review = fixture.control.review_authoring(
        &fixture.actor,
        &workflow,
        saved.generation,
        &scope(vec![]),
    )?;
    let issue = review
        .issues
        .iter()
        .find(|issue| issue.code == "project_changed")
        .ok_or("project change was not reported before Run")?;
    assert_eq!(issue.task_id.as_deref(), Some("candidate"));
    assert_eq!(fixture.runtime.workflow(&workflow)?, Some(before));
    let execution = &mut document
        .tasks
        .first_mut()
        .ok_or("candidate absent")?
        .definition
        .execution;
    let TaskExecution::Explicit {
        expected_generation,
        ..
    } = execution
    else {
        return Err("explicit settings absent".into());
    };
    *expected_generation = 2;
    let repaired = fixture.control.checkpoint_authoring(
        &fixture.actor,
        &workflow,
        saved.generation,
        document,
    )?;
    fixture.authority.0.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            repaired.generation,
            &scope(vec![])
        ),
        Err(ControlError::AuthorizationDenied { .. })
    ));
    fixture.authority.0.store(false, Ordering::SeqCst);
    let review = fixture.control.review_authoring(
        &fixture.actor,
        &workflow,
        repaired.generation,
        &scope(vec![]),
    )?;
    assert!(review.issues.is_empty(), "{review:?}");
    fixture.control.run_authoring(
        &fixture.actor,
        &workflow,
        "run",
        review.expected_generation,
        repaired.generation,
        &scope(vec![]),
        br#"{"key":"run"}"#,
    )?;
    let accepted = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let task = accepted
        .tasks
        .values()
        .next()
        .ok_or("accepted task absent")?;
    assert_eq!(
        task.definition
            .execution
            .binding()
            .ok_or("binding absent")?
            .generation,
        2
    );
    Ok(())
}

#[test]
fn saved_incomplete_project_bindings_require_current_read_scope_but_can_be_removed() -> TestResult {
    let fixture = Fixture::new()?;
    let folder = tempfile::tempdir()?;
    let captured = milkdrift_capability_host::capture_native_project_root(folder.path(), false)?;
    let folder = TaskProjectFolder {
        requested: captured.requested.to_string_lossy().into_owned(),
        physical: captured.physical.to_string_lossy().into_owned(),
        identity: captured.identity,
    };
    let paths = vec![TaskPathAccess {
        path: folder.requested.clone(),
        write: false,
    }];
    let binding = TaskProjectBinding {
        project_id: "project:private-review".into(),
        generation: 1,
        folder: folder.clone(),
        settings: ProjectSettings {
            environment: ProjectEnvironment::Host,
            allow_changes: false,
        },
        access: TaskProjectAccess::Read,
        paths: paths.clone(),
        scopes: vec![folder],
        tools: vec![TaskTool::ReadFile],
    };
    let create = serde_json::from_value(
        serde_json::json!({"title":"Private unfinished project work","tasks":[]}),
    )?;
    let workflow = fixture
        .control
        .create(&fixture.actor, "empty", create)?
        .workflow;
    let mut pending = document()?;
    let task = pending.tasks.first_mut().ok_or("candidate absent")?;
    task.definition.capability.id.clear();
    task.definition.capability.operation.clear();
    task.definition.instructions.clear();
    task.definition.paths = paths;
    task.definition.tools = binding.tools.clone();
    task.definition.execution = TaskExecution::Bound { binding };
    let saved =
        fixture
            .control
            .checkpoint_authoring(&fixture.actor, &workflow, 0, pending.clone())?;
    assert_eq!(fixture.control.authoring(&fixture.actor, &workflow)?, saved);
    let collaborator = ActorAuthorityContext::new(
        ActorRef::new("human:project-collaborator")?,
        CommandAuthorityClaim::new(
            GrantId::new("grant:project-collaborator")?,
            1,
            GrantDigest::new(format!("b3_{}", "1".repeat(64)))?,
            0,
        )?,
    );
    fixture.authority.0.store(true, Ordering::SeqCst);
    for actor in [&fixture.actor, &collaborator] {
        assert!(matches!(
            fixture.control.authoring(actor, &workflow),
            Err(ControlError::AuthorizationDenied { .. })
        ));
        assert!(matches!(
            fixture.control.checkpoint_authoring(
                actor,
                &workflow,
                saved.generation,
                pending.clone()
            ),
            Err(ControlError::AuthorizationDenied { .. })
        ));
        assert_eq!(
            fixture.store.workflow_authoring(&workflow)?,
            Some(saved.clone())
        );
    }
    // Replacement authorization does not require reading the removed private content.
    pending.tasks.clear();
    let removed = fixture.control.checkpoint_authoring(
        &collaborator,
        &workflow,
        saved.generation,
        pending,
    )?;
    assert_eq!(
        fixture.control.authoring(&collaborator, &workflow)?,
        removed
    );
    assert!(
        fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?
            .tasks
            .is_empty()
    );
    Ok(())
}

#[test]
fn newly_published_output_is_authorized_and_its_actual_size_is_reviewed() -> TestResult {
    review_new_publication(false, false)
}

#[test]
fn newly_published_outcome_requires_current_authority_for_named_and_terminal_results() -> TestResult
{
    review_new_publication(true, false)?;
    review_new_publication(true, true)
}

fn review_new_publication(outcome: bool, terminal_only: bool) -> TestResult {
    use milkdrift_workspace::{
        ArtifactId, ArtifactMetadata, ArtifactOwner, ArtifactProvenance, ArtifactRetention,
        ArtifactSensitivity, CausalId, CausalReference, ContentDigest, MediaType, RunId,
        WorkspaceBudget,
    };
    let fixture = Fixture::with_context_inputs(outcome)?;
    let mut producer = document()?
        .tasks
        .pop()
        .ok_or("producer absent")?
        .definition
        .task_definition()?;
    producer.outputs.insert(
        "report".into(),
        TaskOutputDeclaration {
            required: !terminal_only,
            media_type: Some("text/plain".into()),
            max_bytes: 4096,
        },
    );
    let mut create: WorkflowCreate = serde_json::from_value(
        serde_json::json!({"title":"Future output review","tasks":[],"finish_when_settled":false}),
    )?;
    create.tasks.push(NewTask {
        alias: TaskId::new("producer")?,
        definition: producer,
    });
    let workflow = fixture
        .control
        .create(&fixture.actor, "create", create)?
        .workflow;
    let record = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let producer = record.tasks.keys().next().ok_or("producer absent")?.clone();
    let mut pending = document()?;
    pending.connections.push(AuthoringConnection {
        id: "report_wire".into(),
        source: producer.to_string(),
        target: "candidate".into(),
        input: Some("brief".into()),
        output: (!outcome).then(|| "report".into()),
        kind: AuthoringConnectionKind::Data,
        optional: false,
        outcome,
    });
    let saved = fixture
        .control
        .checkpoint_authoring(&fixture.actor, &workflow, 0, pending)?;
    assert!(saved.protected_sources.is_empty());
    let before = fixture.control.review_authoring(
        &fixture.actor,
        &workflow,
        saved.generation,
        &scope(vec![]),
    )?;
    assert!(
        before.issues.is_empty(),
        "an unpublished maximum is not actual content size: {before:?}"
    );

    let bytes = vec![b'x'; 2048];
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new("published-review-report")?,
            ContentDigest::for_bytes(&bytes),
            MediaType::new("text/plain")?,
            u64::try_from(bytes.len())?,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("review-output-fixture")?,
            },
            vec![],
        )?,
    )?;
    let run = RunId::new(workflow.as_str())?;
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication:review-report")?,
        run.clone(),
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 16, 2_097_152, 16_777_216)?,
        fixture
            .store
            .artifact_usage(&ArtifactOwner::Workflow { run })?,
    )?;
    fixture.store.begin_publication(&publication)?;
    fixture
        .store
        .write_chunk(publication.publication(), 0, &bytes)?;
    fixture
        .store
        .commit_publication(publication.publication())?;
    let reference = ArtifactReference::new(
        metadata.reference().artifact().as_str(),
        metadata.reference().digest().to_hex(),
        Some("text/plain".into()),
        Some(u64::try_from(bytes.len())?),
    )?;
    let mut events = Vec::new();
    if !terminal_only {
        events.push(InvocationEventKind::Output {
            name: "report".into(),
            reference: reference.clone(),
        });
    }
    events.push(InvocationEventKind::Terminal {
        terminal: InvocationTerminal::new(
            TerminalStatus::Success,
            vec![reference],
            None,
            None,
            SideEffectClass::None,
        )?,
    });
    fixture
        .executor
        .inner
        .set_script(OperationId::new("model.generate")?, events)?;
    fixture
        .runtime
        .execute(fixture.runtime.tick(1)?.pop().ok_or("producer not ready")?)?;
    let record = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let completed = record.tasks.get(&producer).ok_or("producer absent")?;
    assert_eq!(
        completed.state,
        milkdrift_runtime::TaskState::Completed,
        "{completed:?}"
    );
    assert_eq!(
        fixture.store.workflow_authoring(&workflow)?,
        Some(saved.clone()),
        "publication does not rewrite the saved sketch's source list"
    );

    fixture.authority.1.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &scope(vec![])
        ),
        Err(ControlError::AuthorizationDenied { .. })
    ));
    assert!(matches!(
        fixture.control.run_authoring(
            &fixture.actor,
            &workflow,
            "denied-run",
            record.generation,
            saved.generation,
            &scope(vec![]),
            br#"{"key":"denied-run"}"#,
        ),
        Err(ControlError::AuthorizationDenied { .. })
    ));
    assert_eq!(fixture.runtime.workflow(&workflow)?, Some(record.clone()));
    assert_eq!(
        fixture.store.workflow_authoring(&workflow)?,
        Some(saved.clone())
    );
    assert!(
        fixture
            .store
            .workflow_receipt(fixture.actor.actor().as_str(), "denied-run")?
            .is_none()
    );
    let mut direct = document()?
        .tasks
        .pop()
        .ok_or("consumer absent")?
        .definition
        .task_definition()?;
    direct.inputs.insert(
        "brief".into(),
        if outcome {
            TaskInputBinding::SettledOutcome { task: producer }
        } else {
            TaskInputBinding::TaskOutput {
                task: producer,
                output: "report".into(),
                optional: false,
            }
        },
    );
    assert!(matches!(
        fixture.control.edit(
            &fixture.actor,
            "denied-published-source",
            &workflow,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![TaskChange::Add {
                    alias: TaskId::new("direct-consumer")?,
                    definition: direct
                }],
            }
        ),
        Err(ControlError::AuthorizationDenied { .. })
    ));
    fixture.authority.1.store(false, Ordering::SeqCst);
    let review = fixture.control.review_authoring(
        &fixture.actor,
        &workflow,
        saved.generation,
        &scope(vec![]),
    )?;
    if outcome {
        assert!(review.issues.is_empty(), "{review:?}");
        fixture.control.run_authoring(
            &fixture.actor,
            &workflow,
            "allowed-outcome",
            review.expected_generation,
            saved.generation,
            &scope(vec![]),
            br#"{"key":"allowed-outcome"}"#,
        )?;
        let accepted = fixture
            .store
            .workflow_authoring(&workflow)?
            .ok_or("authoring absent")?;
        assert_eq!(
            accepted.protected_sources,
            vec!["published-review-report".to_owned()]
        );
        assert!(
            accepted
                .document
                .get("tasks")
                .and_then(serde_json::Value::as_array)
                .is_some_and(Vec::is_empty)
        );
        return Ok(());
    }
    let issue = review
        .issues
        .iter()
        .find(|issue| issue.code == "media_mismatch")
        .ok_or("actual oversized publication was not reported")?;
    assert_eq!(issue.task_id.as_deref(), Some("candidate"));
    assert_eq!(issue.connection_id.as_deref(), Some("report_wire"));
    let refused = fixture.control.run_authoring(
        &fixture.actor,
        &workflow,
        "run",
        review.expected_generation,
        saved.generation,
        &scope(vec![]),
        br#"{"key":"run"}"#,
    );
    assert!(
        matches!(refused, Err(ControlError::AuthoringIssues(issues)) if issues.iter().any(|issue| issue.code == "media_mismatch"))
    );
    assert_eq!(fixture.runtime.workflow(&workflow)?, Some(record));
    Ok(())
}

#[test]
fn generated_context_inputs_keep_safe_names_and_map_colon_and_long_wire_ids() -> TestResult {
    let fixture = Fixture::with_context_inputs(true)?;
    let create =
        serde_json::from_value(serde_json::json!({"title":"Context wire identities","tasks":[]}))?;
    let workflow = fixture
        .control
        .create(&fixture.actor, "empty", create)?
        .workflow;
    let wire_ids = [
        "ordinary_wire".to_owned(),
        "wire:brief".to_owned(),
        "w".repeat(128),
    ];
    let mut pending = document()?;
    for (index, wire_id) in wire_ids.iter().enumerate() {
        let source = format!("source_{index}");
        pending.sources.push(AuthoringSource {
            id: source.clone(),
            label: format!("Brief {index}"),
            value: Some(TaskInputValue::Text {
                text: format!("Context {index}"),
            }),
            shared_name: None,
        });
        pending.connections.push(AuthoringConnection {
            id: wire_id.clone(),
            source,
            target: "candidate".into(),
            input: None,
            output: None,
            kind: AuthoringConnectionKind::Data,
            optional: false,
            outcome: false,
        });
    }
    let saved = fixture
        .control
        .checkpoint_authoring(&fixture.actor, &workflow, 0, pending)?;
    let review = fixture.control.review_authoring(
        &fixture.actor,
        &workflow,
        saved.generation,
        &scope(vec![]),
    )?;
    assert!(review.issues.is_empty(), "{review:?}");
    let canonical = br#"{"key":"run-context"}"#;
    let receipt = fixture.control.run_authoring(
        &fixture.actor,
        &workflow,
        "run-context",
        review.expected_generation,
        saved.generation,
        &scope(vec![]),
        canonical,
    )?;
    let record = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let task = record.tasks.values().next().ok_or("accepted task absent")?;
    task.definition.validate()?;
    assert_eq!(task.definition.inputs.len(), wire_ids.len());
    assert!(task.definition.inputs.contains_key("context_ordinary_wire"));
    let snapshot = fixture.control.authoring(&fixture.actor, &workflow)?;
    let accepted: AuthoringDocument = serde_json::from_value(snapshot.document.clone())?;
    assert!(accepted.tasks.is_empty());
    assert_eq!(
        accepted.accepted.get("candidate").map(String::as_str),
        Some(task.id.as_str())
    );
    for (index, wire_id) in wire_ids.iter().enumerate() {
        let wire = accepted
            .connections
            .iter()
            .find(|wire| &wire.id == wire_id)
            .ok_or("wire absent")?;
        let input = wire.input.as_ref().ok_or("resolved input absent")?;
        assert!(input.len() <= 128);
        assert!(
            input
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        );
        assert_eq!(
            task.definition.inputs.get(input),
            Some(&milkdrift_blueprint::TaskInputBinding::Inline {
                value: TaskInputValue::Text {
                    text: format!("Context {index}")
                },
            })
        );
    }
    // Exact replay retains the already admitted keys and mapping after the pending cards disappear.
    assert_eq!(
        fixture.control.run_authoring(
            &fixture.actor,
            &workflow,
            "run-context",
            review.expected_generation,
            saved.generation,
            &scope(vec![]),
            canonical
        )?,
        receipt
    );
    assert_eq!(
        fixture.control.authoring(&fixture.actor, &workflow)?,
        snapshot
    );
    Ok(())
}

#[test]
fn queued_parent_admitted_alone_keeps_outputs_for_current_and_candidate_connections() -> TestResult
{
    for apply_candidate in [false, true] {
        let fixture = Fixture::new()?;
        let create = serde_json::from_value(
            serde_json::json!({"title":"Queued parent connections","tasks":[]}),
        )?;
        let workflow = fixture
            .control
            .create(&fixture.actor, "empty", create)?
            .workflow;
        let mut pending = document()?;
        pending.sources.push(AuthoringSource {
            id: "missing_brief".into(),
            label: "Future brief".into(),
            value: None,
            shared_name: Some("future_brief".into()),
        });
        pending.connections.push(AuthoringConnection {
            id: "brief_wire".into(),
            source: "missing_brief".into(),
            target: "candidate".into(),
            input: Some("brief".into()),
            output: None,
            kind: AuthoringConnectionKind::Data,
            optional: false,
            outcome: false,
        });
        let saved = fixture
            .control
            .checkpoint_authoring(&fixture.actor, &workflow, 0, pending)?;
        let review = fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &scope(vec![]),
        )?;
        assert!(review.issues.is_empty(), "{review:?}");
        fixture.control.run_authoring(
            &fixture.actor,
            &workflow,
            "parent-alone",
            review.expected_generation,
            saved.generation,
            &scope(vec![]),
            br#"{"key":"parent-alone"}"#,
        )?;
        assert!(fixture.runtime.tick(1)?.is_empty());
        let record = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let parent = record.tasks.values().next().ok_or("parent absent")?;
        assert!(parent.frozen.is_none());
        assert_eq!(parent.definition.outputs.len(), 2);
        assert!(
            parent
                .definition
                .outputs
                .values()
                .all(|output| output.max_bytes == milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES)
        );
        let parent_id = parent.id.clone();

        // A prior explicit tighter promise is retained by both future choices, even if the
        // new editable sketch omits output declarations altogether.
        let mut narrower = parent.definition.clone();
        narrower
            .outputs
            .get_mut("report")
            .ok_or("report promise absent")?
            .max_bytes = 512;
        fixture.control.edit(
            &fixture.actor,
            "narrow-promise",
            &workflow,
            TaskBatch {
                expected_generation: record.generation,
                changes: vec![TaskChange::Revise {
                    task: parent_id.clone(),
                    expected_version: parent.version,
                    definition: narrower,
                }],
            },
        )?;
        let record = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let accepted_parent = record.tasks.get(&parent_id).ok_or("parent absent")?.clone();
        let saved = fixture.control.authoring(&fixture.actor, &workflow)?;
        let mut pending: AuthoringDocument = serde_json::from_value(saved.document)?;
        pending
            .connections
            .first_mut()
            .ok_or("brief wire absent")?
            .target = parent_id.to_string();
        let mut candidate = document()?.tasks.pop().ok_or("candidate absent")?;
        candidate.id = parent_id.to_string();
        candidate.version = Some(accepted_parent.version);
        candidate.definition.instructions = "A revised evidence-reading approach".into();
        assert!(candidate.definition.outputs.is_empty());
        let mut consumer = document()?.tasks.pop().ok_or("consumer absent")?;
        consumer.id = "consumer".into();
        pending.tasks = vec![candidate, consumer];
        pending.connections.push(AuthoringConnection {
            id: "report_wire".into(),
            source: parent_id.to_string(),
            target: "consumer".into(),
            input: Some("brief".into()),
            output: Some("report".into()),
            kind: AuthoringConnectionKind::Data,
            optional: false,
            outcome: false,
        });
        let saved = fixture.control.checkpoint_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            pending,
        )?;
        let mut selected = AuthoringScope {
            tasks: vec!["consumer".into()],
            changes: vec![],
            use_current: vec![],
        };
        let choice = fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &selected,
        )?;
        assert!(
            choice
                .issues
                .iter()
                .any(|issue| issue.code == "predecessor_choice"
                    && issue.task_id.as_deref() == Some(parent_id.as_str()))
        );
        if apply_candidate {
            selected.tasks.push(parent_id.to_string());
        } else {
            selected.use_current.push(parent_id.to_string());
        }
        let review = fixture.control.review_authoring(
            &fixture.actor,
            &workflow,
            saved.generation,
            &selected,
        )?;
        assert!(review.issues.is_empty(), "{review:?}");
        fixture.control.run_authoring(
            &fixture.actor,
            &workflow,
            "connect-consumer",
            review.expected_generation,
            saved.generation,
            &selected,
            br#"{"key":"connect-consumer"}"#,
        )?;
        let record = fixture
            .runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?;
        let parent = record.tasks.get(&parent_id).ok_or("parent absent")?;
        assert_eq!(
            parent
                .definition
                .outputs
                .get("report")
                .ok_or("report absent")?
                .max_bytes,
            512
        );
        assert_eq!(
            parent
                .definition
                .outputs
                .get("diagnostic")
                .ok_or("diagnostic absent")?
                .max_bytes,
            milkdrift_blueprint::MAX_TASK_OUTPUT_BYTES
        );
        if apply_candidate {
            assert_eq!(parent.version, accepted_parent.version + 1);
            assert_eq!(
                parent.definition.instructions,
                "A revised evidence-reading approach"
            );
            assert_eq!(
                parent.previous_definitions.last(),
                Some(&accepted_parent.definition)
            );
        } else {
            assert_eq!(parent.definition, accepted_parent.definition);
            assert_eq!(parent.version, accepted_parent.version);
        }
        let consumer = record
            .tasks
            .values()
            .find(|task| task.id != parent_id)
            .ok_or("consumer absent")?;
        assert_eq!(
            consumer.definition.inputs.get("brief"),
            Some(&TaskInputBinding::TaskOutput {
                task: parent_id,
                output: "report".into(),
                optional: false,
            })
        );
        assert!(fixture.runtime.tick(2)?.is_empty());
    }
    Ok(())
}

#[test]
fn programmatic_revision_can_declare_a_queued_output_and_connect_it_in_one_batch() -> TestResult {
    let fixture = Fixture::new()?;
    let mut parent = document()?
        .tasks
        .pop()
        .ok_or("parent absent")?
        .definition
        .task_definition()?;
    parent.inputs.insert(
        "brief".into(),
        TaskInputBinding::Supplied {
            name: "future_brief".into(),
            optional: false,
        },
    );
    assert!(parent.outputs.is_empty());
    let mut create: WorkflowCreate = serde_json::from_value(
        serde_json::json!({"title":"Explicit prospective output","tasks":[]}),
    )?;
    create.tasks.push(NewTask {
        alias: TaskId::new("parent")?,
        definition: parent,
    });
    let workflow = fixture
        .control
        .create(&fixture.actor, "create", create)?
        .workflow;
    let before = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let parent = before.tasks.values().next().ok_or("parent absent")?;
    let mut revised = parent.definition.clone();
    revised.outputs.insert(
        "report".into(),
        TaskOutputDeclaration {
            required: false,
            media_type: Some("text/plain".into()),
            max_bytes: 512,
        },
    );
    let mut consumer = document()?
        .tasks
        .pop()
        .ok_or("consumer absent")?
        .definition
        .task_definition()?;
    consumer.inputs.insert(
        "brief".into(),
        TaskInputBinding::TaskOutput {
            task: parent.id.clone(),
            output: "report".into(),
            optional: false,
        },
    );
    let batch = TaskBatch {
        expected_generation: before.generation,
        changes: vec![
            // Source authorization must evaluate the batch's prospective definitions regardless
            // of whether the consumer appears before the producer revision in the request.
            TaskChange::Add {
                alias: TaskId::new("consumer")?,
                definition: consumer,
            },
            TaskChange::Revise {
                task: parent.id.clone(),
                expected_version: parent.version,
                definition: revised,
            },
        ],
    };
    let receipt = fixture.control.edit(
        &fixture.actor,
        "revise-and-connect",
        &workflow,
        batch.clone(),
    )?;
    let after = fixture
        .runtime
        .workflow(&workflow)?
        .ok_or("workflow absent")?;
    let accepted = after.tasks.get(&parent.id).ok_or("parent absent")?;
    assert_eq!(accepted.version, 2);
    assert_eq!(
        accepted.previous_definitions.last(),
        Some(&parent.definition)
    );
    assert_eq!(after.tasks.len(), 2);
    assert!(fixture.runtime.tick(2)?.is_empty());
    assert_eq!(
        fixture
            .control
            .edit(&fixture.actor, "revise-and-connect", &workflow, batch)?,
        receipt
    );
    Ok(())
}
