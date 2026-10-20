//! Ordinary task entry keeps selected bytes and provider effects in their production owners.
use super::*;
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityRequest, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
};
use milkdrift_blueprint::{
    NewTask, TaskDefinition, TaskId, TaskInputBinding, TaskInputValue, WorkflowCreate, WorkflowId,
    WorkflowLimits,
};
use milkdrift_capability::{CapabilityObservation, CapabilityRequirement};
use milkdrift_capability_host::{
    CapabilityHost, CapabilitySelectionPolicy, HostConfig, StoreInvocationDataAccess,
};
use milkdrift_control::{ActorAuthorityContext, TaskControlService};
use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig, StoreClock};
use milkdrift_runtime::{
    BoundaryClock, CommandAuthorityClaim, ManualClock, SequentialIdGenerator, TaskRecord,
    TaskRuntime,
};

pub(super) struct Allow;
impl AuthorityEvaluator for Allow {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test.model-session")?,
            1,
            request.clone(),
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget {
                cost_minor: Some(u64::MAX),
                duration_ms: Some(u64::MAX),
                invocations: Some(u64::MAX),
                artifact_bytes: Some(u64::MAX),
                units: Some(u64::MAX),
                concurrency: Some(u32::MAX),
            },
            SideEffectClass::Unknown,
        )
    }
}
pub(super) struct NoFaults;
impl milkdrift_redb_store::FaultInjector for NoFaults {
    fn check(
        &self,
        _: milkdrift_redb_store::FaultPoint,
    ) -> Result<(), milkdrift_persistence::PersistenceError> {
        Ok(())
    }
}
struct TaskStoreClock(Arc<ManualClock>);
impl StoreClock for TaskStoreClock {
    fn now(
        &self,
    ) -> Result<milkdrift_persistence::TimestampMillis, milkdrift_persistence::PersistenceError>
    {
        self.0.now().map_err(|error| {
            milkdrift_persistence::PersistenceError::InvalidDocument(error.to_string())
        })
    }
}

pub(super) struct ModelFixture {
    pub(super) runtime: Arc<TaskRuntime>,
    pub(super) host: Arc<CapabilityHost>,
    pub(super) store: Arc<RedbStore>,
    pub(super) workflow: WorkflowId,
    pub(super) task: TaskId,
    pub(super) directory: tempfile::TempDir,
    pub(super) read_denial: Arc<AtomicBool>,
    pub(super) adapter: Arc<ModelEndpointAdapter>,
}
impl ModelFixture {
    pub(super) fn new(
        profile: EndpointProfile,
        authority: Arc<dyn AuthorityEvaluator>,
        faults: Arc<dyn milkdrift_redb_store::FaultInjector>,
    ) -> TestResult<Self> {
        Self::with_instructions(
            profile,
            "bounded hostile endpoint: inspect the supplied project",
            authority,
            faults,
        )
    }
    pub(super) fn with_instructions(
        profile: EndpointProfile,
        instructions: &str,
        authority: Arc<dyn AuthorityEvaluator>,
        faults: Arc<dyn milkdrift_redb_store::FaultInjector>,
    ) -> TestResult<Self> {
        Self::with_tools(profile, instructions, authority, faults, vec![], vec![])
    }
    pub(super) fn with_tools(
        profile: EndpointProfile,
        instructions: &str,
        authority: Arc<dyn AuthorityEvaluator>,
        faults: Arc<dyn milkdrift_redb_store::FaultInjector>,
        tools: Vec<milkdrift_blueprint::TaskTool>,
        paths: Vec<milkdrift_blueprint::TaskPathAccess>,
    ) -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let clock = Arc::new(ManualClock::new(1000));
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path().join("store"))
                .with_fault_injector(faults)
                .with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let (host, adapter, read_denial) = model_host_with_tools(
            store.clone(),
            directory.path(),
            profile,
            (!tools.is_empty()).then(|| authority.clone()),
        )?;
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            host.clone(),
            authority.clone(),
            clock.clone(),
            Arc::new(SequentialIdGenerator::new("session-http", 1)?),
        ));
        let control = TaskControlService::new(runtime.clone(), authority, clock, store.clone());
        let actor = ActorAuthorityContext::new(
            ActorRef::new("human:session")?,
            CommandAuthorityClaim::new(
                GrantId::new("grant:session")?,
                1,
                GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
                0,
            )?,
        );
        let request = WorkflowCreate {
            project_id: None,
            title: "Model session task".into(),
            description: "Actual ordinary runtime to provider".into(),
            tasks: vec![NewTask {
                alias: TaskId::new("model")?,
                definition: TaskDefinition {
                    execution: milkdrift_blueprint::TaskExecution::Legacy,
                    title: "Inspect the captured project".into(),
                    description: String::new(),
                    instructions: instructions.into(),
                    capability: CapabilityRequirement::new(OperationId::new("model.generate")?),
                    inputs: BTreeMap::from([(
                        "file".into(),
                        TaskInputBinding::Supplied {
                            name: "file".into(),
                            optional: false,
                        },
                    )]),
                    dependencies: BTreeSet::new(),
                    outputs: BTreeMap::new(),
                    required: true,
                    corrective_of: None,
                    paths,
                    tools,
                    actions: None,
                },
            }],
            inputs: BTreeMap::from([(
                "file".into(),
                TaskInputValue::text_file(
                    "project.txt".into(),
                    "captured project evidence".into(),
                )?,
            )]),
            limits: WorkflowLimits::default(),
            finish_when_settled: true,
        };
        let receipt = control.create(&actor, "session-task", request)?;
        let workflow = receipt.workflow;
        let task = runtime
            .workflow(&workflow)?
            .ok_or("workflow absent")?
            .tasks
            .keys()
            .next()
            .ok_or("task absent")?
            .clone();
        Ok(Self {
            runtime,
            host,
            store,
            workflow,
            task,
            directory,
            read_denial,
            adapter,
        })
    }
    pub(super) fn state(&self) -> TestResult<TaskRecord> {
        Ok(self
            .runtime
            .workflow(&self.workflow)?
            .ok_or("workflow absent")?
            .tasks
            .get(&self.task)
            .ok_or("task absent")?
            .clone())
    }
    pub(super) fn reopen(
        self,
        profile: EndpointProfile,
        authority: Arc<dyn AuthorityEvaluator>,
        now: u64,
    ) -> TestResult<Self> {
        let Self {
            runtime,
            host,
            store,
            workflow,
            task,
            directory,
            adapter,
            read_denial,
        } = self;
        drop(runtime);
        drop(adapter);
        drop(host);
        drop(store);
        drop(read_denial);
        let clock = Arc::new(ManualClock::new(now));
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path().join("store"))
                .with_clock(Arc::new(TaskStoreClock(clock.clone()))),
        )?);
        let (host, adapter, read_denial) = model_host(store.clone(), directory.path(), profile)?;
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            host.clone(),
            authority,
            clock,
            Arc::new(SequentialIdGenerator::new("session-reopen", 1)?),
        ));
        runtime.recover()?;
        Ok(Self {
            runtime,
            host,
            store,
            workflow,
            task,
            directory,
            adapter,
            read_denial,
        })
    }
}

// Proves that entering prepared model work does not reload or re-encode any input.
struct ReadGuard {
    denied: Arc<AtomicBool>,
    inner: StoreInvocationDataAccess,
}
impl InvocationDataAccess for ReadGuard {
    fn read_input_bytes(
        &self,
        context: &AdapterExecutionContext,
        input: &InputReference,
        limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(InvocationDataError::Rejected(
                "read after preparation".to_owned(),
            ));
        }
        self.inner.read_input_bytes(context, input, limits)
    }
    fn read_artifact_bytes(
        &self,
        context: &AdapterExecutionContext,
        reference: &ArtifactReference,
        limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(InvocationDataError::Rejected(
                "read after preparation".to_owned(),
            ));
        }
        self.inner.read_artifact_bytes(context, reference, limits)
    }
    fn materialize(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        inputs: &[milkdrift_capability_host::InputMaterialization],
        limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError> {
        self.inner.materialize(context, request, inputs, limits)
    }
    fn publish_file(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        workspace: &dyn MaterializedExecution,
        name: &str,
        path: &Path,
        media: &str,
        limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        self.inner
            .publish_file(context, request, workspace, name, path, media, limits)
    }
    fn publish_bytes(
        &self,
        context: &AdapterExecutionContext,
        request: &InvocationRequest,
        name: &str,
        media: &str,
        bytes: &[u8],
        limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        self.inner
            .publish_bytes(context, request, name, media, bytes, limits)
    }
}

fn model_host(
    store: Arc<RedbStore>,
    directory: &Path,
    profile: EndpointProfile,
) -> TestResult<(
    Arc<CapabilityHost>,
    Arc<ModelEndpointAdapter>,
    Arc<AtomicBool>,
)> {
    model_host_with_tools(store, directory, profile, None)
}
fn model_host_with_tools(
    store: Arc<RedbStore>,
    directory: &Path,
    profile: EndpointProfile,
    authority: Option<Arc<dyn AuthorityEvaluator>>,
) -> TestResult<(
    Arc<CapabilityHost>,
    Arc<ModelEndpointAdapter>,
    Arc<AtomicBool>,
)> {
    let read_denial = Arc::new(AtomicBool::new(false));
    let data = Arc::new(ReadGuard {
        denied: read_denial.clone(),
        inner: StoreInvocationDataAccess::new(
            store.clone(),
            directory.join("materialized"),
            ArtifactReadAuthority::Authorized {
                actor: ActorRef::new("human:session")?,
                evidence: EvidenceId::new("session-read")?,
            },
        )?,
    });
    let capability = CapabilityId::new("session-model")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let adapter = Arc::new(ModelEndpointAdapter::new(
        capability,
        profile,
        Arc::new(InMemorySecretResolver::new()),
        data.clone(),
    )?);
    let host = Arc::new(CapabilityHost::new(
        HostConfig {
            max_registrations: 2,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?);
    host.install_native_path_store(store, Vec::new())?;
    #[cfg(target_os = "linux")]
    if authority.is_some() {
        let process_directory = directory.join("process-project");
        std::fs::create_dir_all(&process_directory)?;
        let executable = std::path::Path::new("/bin/echo").canonicalize()?;
        let bytes = std::fs::read(&executable)?;
        let mut document: Value = serde_json::from_str(include_str!(
            "../../../local-process/tests/fixtures/process-profile-v2.json"
        ))?;
        *document
            .pointer_mut("/profile/capability")
            .ok_or("profile capability absent")? = json!("loop-process");
        *document
            .pointer_mut("/profile/executable")
            .ok_or("profile executable absent")? = json!(executable);
        *document
            .pointer_mut("/profile/implementation/content_digest")
            .ok_or("profile digest absent")? = json!(format!("b3_{}", blake3::hash(&bytes)));
        *document
            .pointer_mut("/profile/implementation/size_bytes")
            .ok_or("profile size absent")? = json!(bytes.len());
        *document
            .pointer_mut("/profile/arguments")
            .ok_or("profile arguments absent")? = json!(["real process tool result"]);
        *document
            .pointer_mut("/profile/working_directory")
            .ok_or("profile working directory absent")? =
            json!({"type":"authorized_host_path","path":process_directory});
        *document
            .pointer_mut("/profile/stdout")
            .ok_or("profile stdout absent")? = json!({"max_capture_bytes":1024,"stream_progress":false,"max_progress_events":0,"overflow_action":"terminate","artifact_name":"stdout"});
        let profile = milkdrift_local_process::ProcessProfileDocument::from_json(
            &serde_json::to_vec(&document)?,
        )?
        .into_profile();
        let process = Arc::new(milkdrift_local_process::LocalProcessAdapter::new(
            profile,
            data.clone(),
            Arc::new(InMemorySecretResolver::new()),
        )?);
        let descriptor = process.descriptor().clone();
        host.register(
            descriptor.clone(),
            process,
            Some(CapabilityObservation::new(
                descriptor.identity().clone(),
                1000,
                true,
                0,
                "ready",
            )?),
        )?;
    }
    let registered: Arc<dyn CapabilityAdapter> = match authority {
        Some(authority) => Arc::new(milkdrift_capability_host::ModelToolLoopAdapter::new(
            descriptor.clone(),
            adapter.clone(),
            adapter.maximum_admission_envelope()?,
            host.as_ref().clone(),
            data,
            authority,
            Arc::new(|| Ok(1000)),
        )),
        None => adapter.clone(),
    };
    host.register(
        descriptor.clone(),
        registered,
        Some(CapabilityObservation::new(
            descriptor.identity().clone(),
            1000,
            true,
            0,
            "ready",
        )?),
    )?;
    Ok((host, adapter, read_denial))
}

#[test]
fn direct_sessions_and_unadvertised_streaming_refuse_before_http() -> TestResult {
    for (session, streaming) in [
        (
            SessionSelection::ProviderManaged {
                session_id: "opaque-session".into(),
            },
            false,
        ),
        (SessionSelection::Fresh, true),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let endpoint = profile(
            &listener.local_addr()?.to_string(),
            "direct-refusal",
            ProviderProtocol::OpenAiCompatible {
                path: "v1/chat/completions".into(),
            },
            AuthMode::NoAuth,
            BTreeSet::from([ModelFeature::SystemRole]),
        )?;
        let task = ModelTaskRequest::new(
            vec![Message::new(
                MessageRole::User,
                vec![ContentPart::Text {
                    text: "inspect captured input".into(),
                }],
                None,
            )?],
            vec![],
            None,
            session,
            None,
            64,
            streaming,
            BTreeMap::new(),
        )?;
        let error = execute(
            endpoint,
            task,
            Arc::new(MockData::default()),
            Arc::new(InMemorySecretResolver::new()),
        )
        .err()
        .ok_or("unsupported request must refuse")?;
        assert!(
            error.to_string().contains(if streaming {
                "streaming"
            } else {
                "fresh sessions only"
            }),
            "{error}"
        );
        listener.set_nonblocking(true)?;
        assert!(
            matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock)
        );
    }
    Ok(())
}

#[test]
fn malformed_inline_and_artifact_model_documents_refuse_before_http() -> TestResult {
    for artifact in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let profile = profile(
            &listener.local_addr()?.to_string(),
            "malformed-model",
            ProviderProtocol::OpenAiCompatible {
                path: "v1/chat/completions".into(),
            },
            AuthMode::NoAuth,
            BTreeSet::from([ModelFeature::SystemRole]),
        )?;
        let capability = CapabilityId::new("malformed-model")?;
        let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
        let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
            &descriptor,
            &OperationId::new("model.generate")?,
        )?;
        let data = Arc::new(MockData::default());
        let malformed = json!({"schema_version":999});
        let value = if artifact {
            InvocationValueReference::Artifact {
                reference: data.install(
                    "malformed-task",
                    "application/json",
                    serde_json::to_vec(&malformed)?,
                )?,
            }
        } else {
            InvocationValueReference::Inline {
                value: BoundedJson::new(malformed)?,
            }
        };
        let request = InvocationRequest::new(
            InvocationId::new("malformed-invocation")?,
            capability.clone(),
            OperationId::new("model.generate")?,
            Some(profile.identity().clone()),
            None,
            vec![InputReference::new(MODEL_TASK_INPUT_NAME, value)?],
            BTreeMap::new(),
        )?;
        let context = AdapterExecutionContext::direct(DirectInputSelection::new(&request, 16_384)?);
        let adapter = ModelEndpointAdapter::new(
            capability,
            profile,
            Arc::new(InMemorySecretResolver::new()),
            data.clone(),
        )?;
        adapter.start()?;
        let reporter = Reporter::default();
        assert!(
            adapter
                .execute(
                    &AdapterInvocation::with_context(&snapshot, &request, &context),
                    &reporter
                )
                .is_err()
        );
        assert!(reporter.events()?.is_empty());
        assert!(
            data.published
                .lock()
                .map_err(|_| "publication lock")?
                .is_empty()
        );
        listener.set_nonblocking(true)?;
        assert!(
            matches!(listener.accept(),Err(error)if error.kind()==std::io::ErrorKind::WouldBlock)
        );
    }
    Ok(())
}
