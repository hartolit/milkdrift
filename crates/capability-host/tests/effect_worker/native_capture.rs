//! Native capture crosses the real host claim and artifact publication boundaries.
use super::*;
use milkdrift_blueprint::{TaskOutputDeclaration, TaskPathAccess, WorkflowId};
use milkdrift_capability_host::{
    InvocationDataAccess, MaterializationLimits, MaterializedExecution, StoreInvocationDataAccess,
};
use milkdrift_persistence::{ArtifactReadAuthority, NativePathClaimStore};
use milkdrift_runtime::TaskState;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

const FINAL_BYTES: &[u8] = b"completed native research, captured after settlement";

struct NativeRoot(PathBuf);
impl MaterializedExecution for NativeRoot {
    fn root(&self) -> &Path {
        &self.0
    }
    fn input_path(&self, _name: &str) -> Option<&Path> {
        None
    }
}

struct NativeAdapter {
    project: PathBuf,
    data: StoreInvocationDataAccess,
    writer_entered: SyncSender<()>,
    writer_result: Mutex<Receiver<bool>>,
    captures: AtomicUsize,
}

impl CapabilityAdapter for NativeAdapter {
    fn native_path_claims(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<Vec<milkdrift_capability_host::NativePathScope>, AdapterError> {
        let writer = invocation.request().inputs().iter().any(|input| {
            input.name() == "instructions"
                && input
                    .value()
                    .inline()
                    .and_then(|value| value.value().as_str())
                    == Some("writer")
        });
        Ok(if writer {
            vec![milkdrift_capability_host::NativePathScope {
                path: self.project.clone(),
                write: true,
            }]
        } else {
            Vec::new()
        })
    }

    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        CapabilityExecutionRequirements::default()
    }
    fn start(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        Ok(())
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            milkdrift_capability::CapabilityId::new("native-capture").map_err(error)?,
            now,
            true,
            0,
            "ready",
        )
        .map_err(error)
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            None,
        )
        .map_err(error)
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let mode = invocation
            .request()
            .inputs()
            .iter()
            .find(|input| input.name() == "instructions")
            .and_then(|input| input.value().inline())
            .and_then(|value| value.value().as_str())
            .ok_or_else(|| error("capture fixture mode absent"))?;
        let mut outputs = Vec::new();
        if mode == "writer" {
            fs::write(
                self.project.join("research.txt"),
                b"unfinished native bytes",
            )
            .map_err(error)?;
            self.writer_entered.send(()).map_err(error)?;
            if !self
                .writer_result
                .lock()
                .map_err(error)?
                .recv_timeout(Duration::from_secs(10))
                .map_err(error)?
            {
                return Err(error("writer lost its terminal evidence"));
            }
            fs::write(self.project.join("research.txt"), FINAL_BYTES).map_err(error)?;
        } else {
            self.captures.fetch_add(1, Ordering::SeqCst);
            let reference = self
                .data
                .publish_file(
                    invocation
                        .context()
                        .ok_or_else(|| error("capture context absent"))?,
                    invocation.request(),
                    &NativeRoot(self.project.clone()),
                    "capture",
                    Path::new("research.txt"),
                    "text/plain",
                    MaterializationLimits {
                        max_files: 1,
                        max_file_bytes: 1024,
                        max_total_bytes: 1024,
                        max_path_bytes: 4096,
                        max_directory_depth: 16,
                        chunk_bytes: 1024,
                    },
                )
                .map_err(error)?;
            reporter.invocation(
                InvocationEvent::new(
                    invocation.request().invocation().clone(),
                    1,
                    InvocationEventKind::Output {
                        name: "capture".into(),
                        reference: reference.clone(),
                    },
                )
                .map_err(error)?,
            )?;
            outputs.push(reference);
        }
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                if mode == "writer" { 1 } else { 2 },
                InvocationEventKind::Terminal {
                    terminal: InvocationTerminal::new(
                        TerminalStatus::Success,
                        outputs,
                        None,
                        None,
                        SideEffectClass::Unknown,
                    )
                    .map_err(error)?,
                },
            )
            .map_err(error)?,
        )
    }
}

fn error(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::external_failure(error.to_string())
}

struct Fixture {
    runtime: Arc<TaskRuntime>,
    store: Arc<RedbStore>,
    adapter: Arc<NativeAdapter>,
    entered: Receiver<()>,
    release: SyncSender<bool>,
}

impl Fixture {
    fn open(root: &Path, project: &Path, label: &str) -> TestResult<Self> {
        let store = Arc::new(RedbStore::open_with_config(
            RedbStoreConfig::new(root.join("store")).with_clock(Arc::new(FixtureClock)),
        )?);
        let host = CapabilityHost::new(
            HostConfig {
                max_registrations: 1,
                max_generations_per_capability: 1,
                max_concurrent_per_generation: 4,
                observation_stale_after_ms: 10_000,
            },
            CapabilitySelectionPolicy::priorities(BTreeMap::new()),
        )?;
        host.install_native_path_store(store.clone(), Vec::new())?;
        let (writer_entered, entered) = sync_channel(1);
        let (release, writer_result) = sync_channel(1);
        let adapter = Arc::new(NativeAdapter {
            project: project.to_path_buf(),
            data: StoreInvocationDataAccess::new(
                store.clone(),
                root.join("materialization"),
                ArtifactReadAuthority::PublicOnly,
            )?,
            writer_entered,
            writer_result: Mutex::new(writer_result),
            captures: AtomicUsize::new(0),
        });
        let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        *descriptor
            .pointer_mut("/descriptor/identity")
            .ok_or("descriptor identity absent")? = serde_json::json!("native-capture");
        *descriptor
            .pointer_mut("/descriptor/category")
            .ok_or("descriptor category absent")? = serde_json::json!({"type":"process"});
        *descriptor
            .pointer_mut("/descriptor/operations/model.generate/side_effect")
            .ok_or("descriptor operation absent")? = serde_json::json!("unknown");
        let descriptor =
            CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?;
        host.register(
            descriptor.body().clone(),
            adapter.clone(),
            Some(CapabilityObservation::new(
                descriptor.body().identity().clone(),
                1000,
                true,
                0,
                "ready",
            )?),
        )?;
        let runtime = Arc::new(TaskRuntime::new(
            store.clone(),
            Arc::new(host),
            Arc::new(AllowAuthority),
            Arc::new(ManualClock::new(1000)),
            Arc::new(SequentialIdGenerator::new(label, 1)?),
        ));
        Ok(Self {
            runtime,
            store,
            adapter,
            entered,
            release,
        })
    }

    fn create(&self, key: &str, write: bool) -> TestResult<WorkflowId> {
        use milkdrift_authority::{
            AuthorityExecutionProvenance, AuthorityOperation, AuthorityRequest, BoundaryTimeMillis,
            DecisionId, RequestedResourceFacts,
        };
        let actor = ActorRef::new("human:native-capture")?;
        let workflow = TaskRuntime::workflow_identity(actor.as_str(), key)?;
        let decision = AllowAuthority.evaluate(&AuthorityRequest {
            decision: DecisionId::new(format!("start-{key}"))?,
            actor: actor.clone(),
            grant: GrantId::new("grant:native-capture")?,
            grant_revision: 1,
            grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
            revocation_generation: 0,
            operation: AuthorityOperation::StartRun,
            resources: RequestedResourceFacts {
                workflow: Some(workflow.clone()),
                ..RequestedResourceFacts::empty()
            },
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(1000),
            provenance: AuthorityExecutionProvenance::default(),
        })?;
        self.runtime.create(
            actor.as_str(),
            key,
            WorkflowCreate {
                project_id: None,
                title: key.into(),
                description: String::new(),
                inputs: BTreeMap::new(),
                limits: WorkflowLimits::default(),
                finish_when_settled: true,
                tasks: vec![NewTask {
                    alias: TaskId::new("work")?,
                    definition: TaskDefinition {
                        execution: milkdrift_blueprint::TaskExecution::Legacy,
                        title: key.into(),
                        description: String::new(),
                        instructions: if write { "writer" } else { "capture" }.into(),
                        capability: CapabilityRequirement::new(OperationId::new("model.generate")?),
                        inputs: BTreeMap::new(),
                        dependencies: Default::default(),
                        outputs: if write {
                            BTreeMap::new()
                        } else {
                            BTreeMap::from([(
                                "capture".into(),
                                TaskOutputDeclaration {
                                    required: true,
                                    media_type: Some("text/plain".into()),
                                    max_bytes: 1024,
                                },
                            )])
                        },
                        required: true,
                        corrective_of: None,
                        // Earlier fixed-directory writers have no project field or explicit path.
                        paths: if write {
                            Vec::new()
                        } else {
                            vec![TaskPathAccess {
                                path: self.adapter.project.to_string_lossy().into_owned(),
                                write: false,
                            }]
                        },
                        tools: Vec::new(),
                        actions: None,
                    },
                }],
            },
            decision,
        )?;
        Ok(workflow)
    }

    fn dispatch(&self) -> TestResult<milkdrift_runtime::ExecutionDispatch> {
        self.runtime
            .tick(200)?
            .pop()
            .ok_or_else(|| "native task was not ready".into())
    }
    fn state(&self, id: &WorkflowId) -> TestResult<TaskState> {
        Ok(self
            .runtime
            .workflow(id)?
            .ok_or("workflow absent")?
            .tasks
            .values()
            .next()
            .ok_or("task absent")?
            .state)
    }
}

#[test]
fn native_capture_cannot_read_live_writer_and_publishes_exact_settled_bytes() -> TestResult {
    let root = tempfile::tempdir()?;
    let project = tempfile::tempdir()?;
    let fixture = Fixture::open(root.path(), project.path(), "native-live")?;
    let writer = fixture.create("writer", true)?;
    let dispatch = fixture.dispatch()?;
    let owner = dispatch.attempt().as_str().to_owned();
    let runtime = fixture.runtime.clone();
    let active = std::thread::spawn(move || runtime.execute(dispatch));
    fixture.entered.recv_timeout(Duration::from_secs(5))?;
    assert_eq!(fixture.state(&writer)?, TaskState::Running);
    let captured = fixture.create("capture-live", false)?;
    let waiting_dispatch = fixture.dispatch()?;
    fixture.runtime.execute(waiting_dispatch.clone())?;
    assert_eq!(fixture.state(&captured)?, TaskState::Queued);
    assert_eq!(fixture.adapter.captures.load(Ordering::SeqCst), 0);
    assert!(fixture.store.native_path_claim(&owner)?.is_some());
    fixture.release.send(true)?;
    active.join().map_err(|_| "writer thread panicked")??;
    assert_eq!(fixture.state(&writer)?, TaskState::Completed);
    assert!(fixture.store.native_path_claim(&owner)?.is_none());
    let resumed = fixture.dispatch()?;
    assert_eq!(resumed, waiting_dispatch);
    fixture.runtime.execute(resumed)?;
    assert_eq!(fixture.state(&captured)?, TaskState::Completed);
    assert_eq!(fixture.adapter.captures.load(Ordering::SeqCst), 1);
    let record = fixture
        .runtime
        .workflow(&captured)?
        .ok_or("workflow absent")?;
    let output = record
        .tasks
        .values()
        .next()
        .and_then(|task| task.result.as_ref())
        .and_then(|result| result.outputs.get("capture"))
        .ok_or("capture output absent")?;
    assert_eq!(output.digest(), blake3::hash(FINAL_BYTES).to_hex().as_str());
    assert_eq!(output.size_bytes(), Some(FINAL_BYTES.len() as u64));
    Ok(())
}

#[test]
fn uncertain_native_writer_survives_reopen_and_keeps_capture_outside_entry() -> TestResult {
    let root = tempfile::tempdir()?;
    let project = tempfile::tempdir()?;
    let (writer, owner);
    {
        let fixture = Fixture::open(root.path(), project.path(), "native-before")?;
        writer = fixture.create("writer", true)?;
        let dispatch = fixture.dispatch()?;
        owner = dispatch.attempt().as_str().to_owned();
        let runtime = fixture.runtime.clone();
        let active = std::thread::spawn(move || runtime.execute(dispatch));
        fixture.entered.recv_timeout(Duration::from_secs(5))?;
        fixture.release.send(false)?;
        active.join().map_err(|_| "writer thread panicked")??;
        assert_eq!(fixture.state(&writer)?, TaskState::Uncertain);
    }
    let fixture = Fixture::open(root.path(), project.path(), "native-after")?;
    fixture.runtime.recover()?;
    assert_eq!(fixture.state(&writer)?, TaskState::Uncertain);
    assert!(fixture.store.native_path_claim(&owner)?.is_some());
    let captured = fixture.create("capture-after-restart", false)?;
    fixture.runtime.execute(fixture.dispatch()?)?;
    assert_eq!(fixture.state(&captured)?, TaskState::Queued);
    assert_eq!(fixture.adapter.captures.load(Ordering::SeqCst), 0);
    assert!(fixture.store.native_path_claim(&owner)?.is_some());
    assert_eq!(
        fs::read(project.path().join("research.txt"))?,
        b"unfinished native bytes"
    );
    Ok(())
}
