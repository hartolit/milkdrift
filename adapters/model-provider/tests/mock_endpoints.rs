//! Local mock-server coverage for both model protocol families.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use milkdrift_blueprint::{
    AuthorRef, BlueprintRevision, Mutation, MutationBatch, Node, NodeId, NodeKind,
    TaskContextPolicy, TerminalOutcome, WorkflowId,
};
use milkdrift_capability::{
    AdmissionBound, ArtifactReference, BoundedJson, CancellationBehavior, CancellationRequest,
    CapabilityId, IdempotencyBehavior, InputReference, InvocationEvent, InvocationEventKind,
    InvocationId, InvocationRequest, InvocationValueReference, OperationId, ProviderProfileRef,
    ResolvedCapabilitySnapshot, SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterExecutionContext, AdapterInvocation, CapabilityAdapter, InMemorySecretResolver,
    InvocationDataAccess, InvocationDataError, MaterializationLimits, MaterializedExecution,
    SecretResolver,
    conformance::{
        AdapterConformanceCase, AdapterConformanceExpectations, ConformanceScenario,
        StartReplayExpectation, UnknownCancellationExpectation, run_adapter_conformance,
    },
};
use milkdrift_model::{
    AuthorityFact, ContentPart, ContextManifest, ContextManifestDocument, ContextProducerFact,
    ContextSemanticKind, ContextSource, ContextTotals, MODEL_TASK_INPUT_NAME, Message, MessageRole,
    ModelTaskRequest, ModelTaskRequestDocument, SessionSelection, StructuredOutput, ToolDefinition,
};
use milkdrift_model_provider::{
    AuthMode, EndpointLimits, EndpointProfile, ModelEndpointAdapter, ModelFeature,
    ProviderProtocol, ProxyPolicy, RedirectPolicy, TlsPolicy, descriptor_for_profile,
};
use milkdrift_persistence::{ArtifactStore, AttemptId, NodeExecutionId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CausalContextBuilder, ContextBuildIdentity, ContextBuildRequest, ContextCandidate,
    ContextCandidateAvailability, persist_context_manifest,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactSensitivity, ContentDigest, RunId, WorkspaceBudget, WorkspaceUsage,
};
use serde_json::{Value, json};

#[path = "mock_endpoints/runtime_session.rs"]
mod runtime_session;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct MockData {
    artifacts: Mutex<BTreeMap<String, Vec<u8>>>,
    published: Mutex<Vec<(String, String, Vec<u8>)>>,
    fail_publication: AtomicBool,
}

impl MockData {
    fn install(&self, name: &str, media: &str, bytes: Vec<u8>) -> TestResult<ArtifactReference> {
        let reference = ArtifactReference::new(
            name,
            blake3::hash(&bytes).to_hex().to_string(),
            Some(media.to_owned()),
            Some(bytes.len() as u64),
        )?;
        self.artifacts
            .lock()
            .map_err(|_| "artifact lock")?
            .insert(name.to_owned(), bytes);
        Ok(reference)
    }
}

impl InvocationDataAccess for MockData {
    fn read_input_bytes(
        &self,
        context: &AdapterExecutionContext,
        input: &InputReference,
        limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        match input.value() {
            InvocationValueReference::Artifact { reference } => {
                self.read_artifact_bytes(context, reference, limits)
            }
            InvocationValueReference::Inline { value } => serde_json::to_vec(value.value())
                .map_err(|error| InvocationDataError::Integrity(error.to_string())),
            InvocationValueReference::WorkspaceValue { .. } => Err(InvocationDataError::Rejected(
                "workspace input is unused in this mock".to_owned(),
            )),
        }
    }

    fn read_artifact_bytes(
        &self,
        _context: &AdapterExecutionContext,
        reference: &ArtifactReference,
        _limits: MaterializationLimits,
    ) -> Result<Vec<u8>, InvocationDataError> {
        let bytes = self
            .artifacts
            .lock()
            .map_err(|_| InvocationDataError::Integrity("artifact lock".to_owned()))?
            .get(reference.identity())
            .cloned()
            .ok_or_else(|| InvocationDataError::Integrity("artifact missing".to_owned()))?;
        if reference.digest() != blake3::hash(&bytes).to_hex().as_str()
            || reference.size_bytes() != Some(bytes.len() as u64)
        {
            return Err(InvocationDataError::Integrity(
                "artifact reference mismatch".to_owned(),
            ));
        }
        Ok(bytes)
    }

    fn materialize(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        _inputs: &[milkdrift_capability_host::InputMaterialization],
        _limits: MaterializationLimits,
    ) -> Result<Box<dyn MaterializedExecution>, InvocationDataError> {
        Err(InvocationDataError::Rejected("unused".to_owned()))
    }

    fn publish_file(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        _workspace: &dyn MaterializedExecution,
        _output_name: &str,
        _relative_path: &Path,
        _media_type: &str,
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        Err(InvocationDataError::Rejected("unused".to_owned()))
    }

    fn publish_bytes(
        &self,
        _context: &AdapterExecutionContext,
        _request: &InvocationRequest,
        output_name: &str,
        media_type: &str,
        bytes: &[u8],
        _limits: MaterializationLimits,
    ) -> Result<ArtifactReference, InvocationDataError> {
        if self.fail_publication.load(Ordering::SeqCst) {
            return Err(InvocationDataError::Publication(
                "injected publication failure".to_owned(),
            ));
        }
        let digest = blake3::hash(bytes).to_hex().to_string();
        self.published
            .lock()
            .map_err(|_| InvocationDataError::Publication("publication lock".to_owned()))?
            .push((
                output_name.to_owned(),
                media_type.to_owned(),
                bytes.to_vec(),
            ));
        ArtifactReference::new(
            format!("model:{output_name}:{digest}"),
            digest,
            Some(media_type.to_owned()),
            Some(bytes.len() as u64),
        )
        .map_err(|error| InvocationDataError::Publication(error.to_string()))
    }
}

fn limits() -> EndpointLimits {
    EndpointLimits {
        connect_timeout_ms: 2_000,
        request_timeout_ms: 5_000,
        idle_timeout_ms: 2_000,
        max_headers: 64,
        max_header_bytes: 16_384,
        max_request_bytes: 1_048_576,
        max_response_bytes: 1_048_576,
        max_stream_line_bytes: 65_536,
        max_stream_event_bytes: 131_072,
        max_fragment_bytes: 4_096,
    }
}

fn profile(
    address: &str,
    identity: &str,
    protocol: ProviderProtocol,
    auth: AuthMode,
    features: BTreeSet<ModelFeature>,
) -> TestResult<EndpointProfile> {
    profile_with_limits(address, identity, protocol, auth, features, limits())
}

fn profile_with_limits(
    address: &str,
    identity: &str,
    protocol: ProviderProtocol,
    auth: AuthMode,
    features: BTreeSet<ModelFeature>,
    limits: EndpointLimits,
) -> TestResult<EndpointProfile> {
    Ok(EndpointProfile::new(
        ProviderProfileRef::new(identity)?,
        1,
        protocol,
        format!("http://{address}"),
        "mock-model",
        auth,
        limits,
        RedirectPolicy::Deny,
        TlsPolicy::WebPkiRoots,
        ProxyPolicy::Disabled,
        features,
        2,
        true,
        BTreeSet::from([address.split(':').next().ok_or("host")?.to_owned()]),
        BTreeSet::from(["local-test".to_owned()]),
        BTreeMap::new(),
    )?)
}

fn serve_stalled_body() -> TestResult<(String, thread::JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let _request = read_request(&mut stream)?;
        stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
        )?;
        stream.flush()?;
        thread::sleep(Duration::from_millis(250));
        Ok(())
    });
    Ok((address, handle))
}

fn serve_drop_after_request() -> TestResult<(String, thread::JoinHandle<std::io::Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let _request = read_request(&mut stream)?;
        Ok(())
    });
    Ok((address, handle))
}

fn revision() -> TestResult<BlueprintRevision> {
    Ok(BlueprintRevision::genesis(
        WorkflowId::new("model-mock")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("model")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:test")?,
        "mock endpoint",
    )?)
}

fn manifest(
    data: &MockData,
    revision: &BlueprintRevision,
) -> TestResult<(ArtifactReference, AdapterExecutionContext)> {
    let run = RunId::new("run-model-mock")?;
    let execution = NodeExecutionId::new("execution-model")?;
    let attempt = AttemptId::new("attempt-model")?;
    let policy = TaskContextPolicy::default();
    let manifest = ContextManifest::new(
        run.clone(),
        revision.id().clone(),
        NodeId::new("model")?,
        execution.clone(),
        attempt.clone(),
        1,
        policy.digest()?,
        Vec::new(),
        Vec::new(),
        ContextTotals::default(),
        policy.budget(),
    )?;
    let bytes = ContextManifestDocument::new(manifest).to_canonical_json()?;
    let reference = data.install(
        "context-manifest",
        "application/vnd.milkdrift.context-manifest.v2+json",
        bytes,
    )?;
    Ok((
        reference,
        AdapterExecutionContext::new(
            run,
            revision.id().clone(),
            NodeId::new("model")?,
            execution,
            attempt,
        ),
    ))
}

fn request(
    capability: &CapabilityId,
    profile: &EndpointProfile,
    manifest: ArtifactReference,
    task: ModelTaskRequest,
    context_inputs: Vec<InputReference>,
) -> TestResult<InvocationRequest> {
    let task_value: Value =
        serde_json::from_slice(&ModelTaskRequestDocument::new(task).to_canonical_json()?)?;
    Ok(InvocationRequest::new(
        InvocationId::new("invocation-model")?,
        capability.clone(),
        OperationId::new("model.generate")?,
        Some(profile.identity().clone()),
        None,
        vec![InputReference::new(
            MODEL_TASK_INPUT_NAME,
            InvocationValueReference::Inline {
                value: BoundedJson::new(task_value)?,
            },
        )?],
        BTreeMap::new(),
    )?
    .with_context_materialization(manifest, context_inputs)?)
}

fn serve(
    response_body: String,
    content_type: &'static str,
) -> TestResult<(String, thread::JoinHandle<std::io::Result<String>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let request = read_request(&mut stream)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes())?;
        Ok(request)
    });
    Ok((address, handle))
}

fn serve_delayed_stream(
    first_event: String,
) -> TestResult<(
    String,
    mpsc::Receiver<()>,
    thread::JoinHandle<std::io::Result<()>>,
)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let _request = read_request(&mut stream)?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first_event}"
        );
        stream.write_all(response.as_bytes())?;
        stream.flush()?;
        ready_tx
            .send(())
            .map_err(|_| std::io::Error::other("test receiver dropped"))?;
        thread::sleep(Duration::from_millis(250));
        Ok(())
    });
    Ok((address, ready_rx, handle))
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..header_end + 4])
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= header_end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn execute(
    profile: EndpointProfile,
    task: ModelTaskRequest,
    data: Arc<MockData>,
    secrets: Arc<dyn SecretResolver>,
) -> TestResult<Vec<InvocationEvent>> {
    let revision = revision()?;
    let (manifest, context) = manifest(&data, &revision)?;
    execute_bound(profile, task, data, secrets, manifest, context, Vec::new())
}

fn execute_bound(
    profile: EndpointProfile,
    task: ModelTaskRequest,
    data: Arc<MockData>,
    secrets: Arc<dyn SecretResolver>,
    manifest: ArtifactReference,
    context: AdapterExecutionContext,
    context_inputs: Vec<InputReference>,
) -> TestResult<Vec<InvocationEvent>> {
    let capability = CapabilityId::new("model-mock")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let request = request(&capability, &profile, manifest, task, context_inputs)?;
    let expected_artifact_bytes = serde_json::to_value(&profile)?["limits"]["max_response_bytes"]
        .as_u64()
        .ok_or("profile omitted its response byte bound")?
        .saturating_mul(4);
    let adapter = ModelEndpointAdapter::new(capability, profile, secrets, data)?;
    adapter.start()?;
    let reporter = Reporter::default();
    let invocation = AdapterInvocation::with_context(&snapshot, &request, &context);
    let first_envelope = adapter.admission_envelope(&invocation)?;
    let second_envelope = adapter.admission_envelope(&invocation)?;
    assert_eq!(first_envelope, second_envelope);
    assert!(matches!(
        first_envelope.input_units(),
        AdmissionBound::Unknown
    ));
    assert!(matches!(
        first_envelope.output_units(),
        AdmissionBound::Unknown
    ));
    assert!(matches!(
        first_envelope.monetary_cost(),
        AdmissionBound::Unknown
    ));
    assert_eq!(
        first_envelope.artifact_bytes().bounded(),
        Some(&expected_artifact_bytes)
    );
    adapter.execute(&invocation, &reporter)?;
    Ok(reporter.events()?)
}

fn model_conformance_case(scenario: ConformanceScenario) -> TestResult<AdapterConformanceCase> {
    let (address, server) = if scenario.executes() {
        let response = json!({
            "id": "response-conformance",
            "model": "mock-model",
            "choices": [{"message": {"content": "complete"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        })
        .to_string();
        let (address, server) = serve(response, "application/json")?;
        (address, Some(server))
    } else {
        ("127.0.0.1:9".to_owned(), None)
    };
    let profile = profile(
        &address,
        "model-conformance",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "adapter conformance".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        32,
        false,
        BTreeMap::new(),
    )?;
    let data = Arc::new(MockData::default());
    let revision = revision()?;
    let (manifest, context) = manifest(data.as_ref(), &revision)?;
    let capability = CapabilityId::new("model-conformance")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let request = request(&capability, &profile, manifest, task, Vec::new())?;
    let adapter = Arc::new(ModelEndpointAdapter::new(
        capability,
        profile,
        Arc::new(InMemorySecretResolver::new()),
        data,
    )?);
    let case = AdapterConformanceCase::new(
        adapter,
        descriptor,
        request,
        context,
        AdapterConformanceExpectations {
            start_replay: StartReplayExpectation::Idempotent,
            available_while_draining: false,
            available_after_shutdown: false,
            unknown_cancellation: UnknownCancellationExpectation::NegativeAcknowledgement,
        },
    )?;
    Ok(match server {
        Some(server) => case.with_cleanup(move || {
            server
                .join()
                .map_err(|_| "model conformance server panicked".to_owned())?
                .map(|_request| ())
                .map_err(|error| error.to_string())
        }),
        None => case,
    })
}

#[test]
fn model_endpoint_adapter_passes_shared_conformance() -> TestResult {
    run_adapter_conformance(model_conformance_case)?;
    Ok(())
}

#[test]
fn endpoint_policy_rejects_remote_plaintext_and_cross_origin_redirects_by_default() -> TestResult {
    assert!(
        EndpointProfile::new(
            ProviderProfileRef::new("remote-http")?,
            1,
            ProviderProtocol::OpenAiCompatible {
                path: "v1/chat/completions".to_owned()
            },
            "http://example.com",
            "model",
            AuthMode::NoAuth,
            limits(),
            RedirectPolicy::Deny,
            TlsPolicy::WebPkiRoots,
            ProxyPolicy::Disabled,
            BTreeSet::new(),
            1,
            false,
            BTreeSet::from(["example.com".to_owned()]),
            BTreeSet::new(),
            BTreeMap::new(),
        )
        .is_err()
    );
    assert!(
        EndpointProfile::new(
            ProviderProfileRef::new("bad-session")?,
            1,
            ProviderProtocol::OpenAiCompatible {
                path: "v1/chat/completions".to_owned()
            },
            "https://example.com",
            "model",
            AuthMode::NoAuth,
            limits(),
            RedirectPolicy::Deny,
            TlsPolicy::WebPkiRoots,
            ProxyPolicy::Disabled,
            BTreeSet::from([ModelFeature::ProviderSessions]),
            1,
            false,
            BTreeSet::from(["example.com".to_owned()]),
            BTreeSet::new(),
            BTreeMap::new(),
        )
        .is_err()
    );
    let canonical = profile(
        "127.0.0.1:8080",
        "canonical-local",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let descriptor = descriptor_for_profile(CapabilityId::new("canonical-model")?, &canonical)?;
    let contract = descriptor
        .operation(&OperationId::new("model.generate")?)
        .ok_or("model descriptor omitted model.generate")?;
    assert_eq!(contract.side_effect(), SideEffectClass::Unknown);
    assert_eq!(contract.idempotency(), IdempotencyBehavior::Unsupported);
    assert_eq!(contract.cancellation(), CancellationBehavior::BestEffort);
    let bytes = canonical.to_canonical_json()?;
    assert_eq!(EndpointProfile::from_json(&bytes)?, canonical);
    let mut hostile: Value = serde_json::from_slice(&bytes)?;
    hostile["schema_version"] = json!(2);
    assert!(EndpointProfile::from_json(&serde_json::to_vec(&hostile)?).is_err());
    hostile["schema_version"] = json!(1);
    hostile["surprise"] = json!(true);
    assert!(EndpointProfile::from_json(&serde_json::to_vec(&hostile)?).is_err());

    let mut tiny_limits = limits();
    tiny_limits.max_request_bytes = 1;
    let bounded = EndpointProfile::new(
        ProviderProfileRef::new("bounded-local")?,
        1,
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        "http://127.0.0.1:9",
        "model",
        AuthMode::NoAuth,
        tiny_limits,
        RedirectPolicy::Deny,
        TlsPolicy::WebPkiRoots,
        ProxyPolicy::Disabled,
        BTreeSet::from([ModelFeature::SystemRole]),
        1,
        true,
        BTreeSet::from(["127.0.0.1".to_owned()]),
        BTreeSet::new(),
        BTreeMap::new(),
    )?;
    let bounded_task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "bounded".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        8,
        false,
        BTreeMap::new(),
    )?;
    let error = execute(
        bounded,
        bounded_task,
        Arc::new(MockData::default()),
        Arc::new(InMemorySecretResolver::new()),
    )
    .err()
    .ok_or("bounded request unexpectedly reached the endpoint")?;
    assert!(error.to_string().contains("request-body bound"));
    Ok(())
}

#[test]
fn unsupported_features_reject_before_entry_and_publication_failure_cannot_succeed() -> TestResult {
    let unused = TcpListener::bind("127.0.0.1:0")?;
    let address = unused.local_addr()?.to_string();
    let profile_without_tools = profile(
        &address,
        "no-tools",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let tool_task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "call a tool".to_owned(),
            }],
            None,
        )?],
        vec![ToolDefinition::new(
            "lookup",
            "lookup",
            BoundedJson::new(json!({"type":"object"}))?,
        )?],
        None,
        SessionSelection::Fresh,
        None,
        64,
        false,
        BTreeMap::new(),
    )?;
    assert!(
        execute(
            profile_without_tools,
            tool_task,
            Arc::new(MockData::default()),
            Arc::new(InMemorySecretResolver::new())
        )
        .is_err()
    );
    drop(unused);

    let response = json!({
        "id":"response-publication",
        "model":"mock-model",
        "choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":1,"completion_tokens":1}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "publication-failure",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "answer".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        false,
        BTreeMap::new(),
    )?;
    let data = Arc::new(MockData::default());
    data.fail_publication.store(true, Ordering::SeqCst);
    let events = execute(profile, task, data, Arc::new(InMemorySecretResolver::new()))?;
    server.join().map_err(|_| "server panicked")??;
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Failure)
    );
    assert!(events.iter().all(|event| event.kind().output().is_none()));
    Ok(())
}

#[path = "mock_endpoints/context.rs"]
mod context;

#[path = "mock_endpoints/providers.rs"]
mod providers;

#[path = "mock_endpoints/uncertainty.rs"]
mod uncertainty;

use milkdrift_capability_host::conformance::RecordingReporter as Reporter;
