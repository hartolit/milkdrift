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
    ArtifactReference, BoundedJson, CancellationRequest, CapabilityId, InputReference,
    InvocationEvent, InvocationEventKind, InvocationId, InvocationRequest,
    InvocationValueReference, OperationId, ProviderProfileRef, ResolvedCapabilitySnapshot,
    TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterExecutionContext, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    InMemorySecretResolver, InvocationDataAccess, InvocationDataError, MaterializationLimits,
    MaterializedExecution, SecretResolver,
};
use milkdrift_model::{
    AuthorityFact, ContentPart, ContextManifest, ContextManifestDocument, ContextSemanticKind,
    ContextSource, ContextTotals, MODEL_TASK_INPUT_NAME, Message, MessageRole, ModelTaskRequest,
    ModelTaskRequestDocument, SessionSelection, StructuredOutput, ToolDefinition,
};
use milkdrift_model_provider::{
    AuthMode, EndpointLimits, EndpointProfile, ModelEndpointAdapter, ModelFeature,
    ProviderProtocol, ProxyPolicy, RedirectPolicy, TlsPolicy, descriptor_for_profile,
};
use milkdrift_persistence::{ArtifactStore, AttemptId, NodeExecutionId};
use milkdrift_redb_store::RedbStore;
use milkdrift_runtime::{
    CausalContextBuilder, ContextBuildIdentity, ContextBuildRequest, ContextCandidate,
    persist_context_manifest,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactSensitivity, RunId, WorkspaceBudget, WorkspaceUsage,
};
use serde_json::{Value, json};

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

#[derive(Default)]
struct Reporter(Mutex<Vec<InvocationEvent>>);

impl AdapterReporter for Reporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.0
            .lock()
            .map_err(|_| AdapterError::external_failure("report lock"))?
            .push(event);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
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
    Ok(EndpointProfile::new(
        ProviderProfileRef::new(identity)?,
        1,
        protocol,
        format!("http://{address}"),
        "mock-model",
        auth,
        limits(),
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
        "application/vnd.milkdrift.context-manifest.v1+json",
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
    .with_context_manifest(manifest)?)
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
    execute_bound(profile, task, data, secrets, manifest, context)
}

fn execute_bound(
    profile: EndpointProfile,
    task: ModelTaskRequest,
    data: Arc<MockData>,
    secrets: Arc<dyn SecretResolver>,
    manifest: ArtifactReference,
    context: AdapterExecutionContext,
) -> TestResult<Vec<InvocationEvent>> {
    let capability = CapabilityId::new("model-mock")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let request = request(&capability, &profile, manifest, task)?;
    let adapter = ModelEndpointAdapter::new(capability, profile, secrets, data)?;
    adapter.start()?;
    let reporter = Reporter::default();
    adapter.execute(
        &AdapterInvocation::with_context(&snapshot, &request, &context),
        &reporter,
    )?;
    Ok(reporter.0.into_inner().map_err(|_| "reporter lock")?)
}

#[test]
fn causal_context_is_persisted_sent_streamed_published_and_inspectable_after_restart() -> TestResult
{
    let body = [
        json!({"choices":[{"delta":{"content":"grounded "},"finish_reason":null}]}),
        json!({"choices":[{"delta":{"content":"answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":2}}),
    ]
    .into_iter()
    .map(|value| format!("data: {value}\n\n"))
    .chain(std::iter::once("data: [DONE]\n\n".to_owned()))
    .collect::<String>();
    let (address, server) = serve(body, "text/event-stream")?;
    let profile = profile(
        &address,
        "openai-e2e",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::Streaming]),
    )?;
    let revision = revision()?;
    let identity = ContextBuildIdentity {
        run: RunId::new("run-model-mock")?,
        revision: revision.id().clone(),
        node: NodeId::new("model")?,
        execution: NodeExecutionId::new("execution-model")?,
        attempt: AttemptId::new("attempt-model")?,
    };
    let policy = TaskContextPolicy::default();
    let manifest = CausalContextBuilder::build(ContextBuildRequest {
        identity: identity.clone(),
        semantic: revision.semantic(),
        policy: &policy,
        visible_scopes: BTreeSet::new(),
        candidates: vec![ContextCandidate {
            kind: ContextSemanticKind::DirectInput,
            source: Some(ContextSource::DirectInput {
                name: "prompt".to_owned(),
                reference: InvocationValueReference::Inline {
                    value: BoundedJson::new(json!("causal prompt"))?,
                },
            }),
            node: None,
            roles: BTreeSet::new(),
            scope: None,
            exposed_across_scope: false,
            required: true,
            available: true,
            selected_bytes: 13,
            selected_artifact_bytes: 0,
            estimated_model_input_units: Some(4),
            sensitivity: ArtifactSensitivity::Restricted,
            authority: AuthorityFact {
                required: false,
                authorized: true,
                authority_reference: None,
            },
            artifact: None,
            causal_parents: Vec::new(),
        }],
    })?;
    let manifest_digest = manifest.digest().as_str().to_owned();
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let manifest_ref = persist_context_manifest(
        &store,
        &manifest,
        WorkspaceBudget::new(0, 0, 0, 1, 1_048_576, 1_048_576)?,
        WorkspaceUsage::EMPTY,
    )?;
    let manifest_bytes = ContextManifestDocument::new(manifest).to_canonical_json()?;
    let data = Arc::new(MockData::default());
    data.artifacts
        .lock()
        .map_err(|_| "artifact lock")?
        .insert(manifest_ref.identity().to_owned(), manifest_bytes);
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "causal prompt".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        true,
        BTreeMap::new(),
    )?;
    let events = execute_bound(
        profile,
        task,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
        manifest_ref.clone(),
        AdapterExecutionContext::new(
            identity.run,
            identity.revision,
            identity.node,
            identity.execution,
            identity.attempt,
        ),
    )?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.contains("causal prompt"));
    assert!(captured.contains("Milkdrift causal context manifest"));
    assert!(captured.contains(&manifest_digest));
    assert!(events.iter().any(|event| {
        event
            .kind()
            .progress()
            .is_some_and(|(text, _, _)| text == "grounded ")
    }));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success)
    );
    assert!(
        !data
            .published
            .lock()
            .map_err(|_| "published lock")?
            .is_empty()
    );
    drop(store);
    let reopened = RedbStore::open(root.path())?;
    let durable = reopened
        .metadata(&ArtifactId::new(manifest_ref.identity())?)?
        .ok_or("manifest missing after restart")?;
    assert!(reopened.is_committed(durable.reference())?);
    Ok(())
}

#[test]
fn openai_compatible_non_streaming_preserves_tools_structure_usage_and_artifacts() -> TestResult {
    let response = json!({
        "id":"response-1","model":"mock-model",
        "choices":[{"message":{"content":"{\"answer\":42}","tool_calls":[{
            "id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"id\":\"x\"}"}
        }]},"finish_reason":"tool_calls"}],
        "usage":{"prompt_tokens":12,"completion_tokens":7,"prompt_tokens_details":{"cached_tokens":3}}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "openai-local",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([
            ModelFeature::SystemRole,
            ModelFeature::Tools,
            ModelFeature::StructuredOutput,
        ]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "answer".to_owned(),
            }],
            None,
        )?],
        vec![ToolDefinition::new(
            "lookup",
            "lookup",
            BoundedJson::new(json!({"type":"object"}))?,
        )?],
        Some(StructuredOutput::new(
            "answer",
            BoundedJson::new(json!({"type":"object"}))?,
            true,
        )?),
        SessionSelection::Fresh,
        None,
        128,
        false,
        BTreeMap::new(),
    )?;
    let data = Arc::new(MockData::default());
    let events = execute(
        profile,
        task,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(captured.contains("\"response_format\""));
    assert!(captured.contains("\"tools\""));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success),
        "{events:#?}"
    );
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "tool_calls")
    }));
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "structured_output")
    }));
    assert_eq!(
        data.published.lock().map_err(|_| "published lock")?.len(),
        5
    );
    Ok(())
}

#[test]
fn anthropic_native_streaming_maps_events_and_required_headers() -> TestResult {
    let body = [
        json!({"type":"message_start","message":{"usage":{"input_tokens":9}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}),
        json!({"type":"message_stop"}),
    ]
    .into_iter()
    .map(|value| format!("data: {value}\n\n"))
    .collect::<String>();
    let (address, server) = serve(body, "text/event-stream")?;
    let secret_ref = milkdrift_authority::SecretRef::new("secret:anthropic-test")?;
    let resolver = Arc::new(InMemorySecretResolver::new());
    resolver.insert(secret_ref.clone(), b"test-secret-value".to_vec())?;
    let profile = profile(
        &address,
        "anthropic-local",
        ProviderProtocol::Anthropic {
            version: "2023-06-01".to_owned(),
            path: "v1/messages".to_owned(),
        },
        AuthMode::AnthropicApiKey { secret: secret_ref },
        BTreeSet::from([ModelFeature::Streaming, ModelFeature::SystemRole]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "say hello".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        true,
        BTreeMap::new(),
    )?;
    let events = execute(profile, task, Arc::new(MockData::default()), resolver)?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.starts_with("POST /v1/messages HTTP/1.1"));
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("anthropic-version: 2023-06-01")
    );
    assert!(
        captured
            .to_ascii_lowercase()
            .contains("x-api-key: test-secret-value")
    );
    assert!(events.iter().any(|event| {
        event
            .kind()
            .progress()
            .is_some_and(|(text, _, _)| text == "hello")
    }));
    assert!(matches!(
        events.last().map(InvocationEvent::kind),
        Some(InvocationEventKind::Terminal { .. })
    ));
    Ok(())
}

#[test]
fn anthropic_native_non_streaming_preserves_tool_calls_usage_and_finish() -> TestResult {
    let response = json!({
        "id":"message-1",
        "model":"mock-model",
        "content":[
            {"type":"text","text":"checking"},
            {"type":"tool_use","id":"tool-1","name":"lookup","input":{"id":"x"}}
        ],
        "stop_reason":"tool_use",
        "usage":{"input_tokens":8,"output_tokens":3,"cache_read_input_tokens":2}
    })
    .to_string();
    let (address, server) = serve(response, "application/json")?;
    let profile = profile(
        &address,
        "anthropic-local-tools",
        ProviderProtocol::Anthropic {
            version: "2023-06-01".to_owned(),
            path: "v1/messages".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::Tools]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "look up x".to_owned(),
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
    let data = Arc::new(MockData::default());
    let events = execute(
        profile,
        task,
        data.clone(),
        Arc::new(InMemorySecretResolver::new()),
    )?;
    let captured = server.join().map_err(|_| "server panicked")??;
    assert!(captured.contains("\"tools\""));
    assert!(events.iter().any(|event| {
        event
            .kind()
            .output()
            .is_some_and(|(name, _)| name == "tool_calls")
    }));
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Success)
    );
    assert_eq!(
        data.published.lock().map_err(|_| "published lock")?.len(),
        4
    );
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
        BTreeSet::new(),
    )?;
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
        BTreeSet::new(),
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
        BTreeSet::new(),
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
        BTreeSet::new(),
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

#[test]
fn streaming_cancellation_is_cooperative_and_does_not_claim_remote_termination() -> TestResult {
    let (address, ready, server) = serve_delayed_stream(format!(
        "data: {}\n\n",
        json!({"choices":[{"delta":{"content":"partial"},"finish_reason":null}]})
    ))?;
    let profile = profile(
        &address,
        "openai-cancel",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::Streaming]),
    )?;
    let task = ModelTaskRequest::new(
        vec![Message::new(
            MessageRole::User,
            vec![ContentPart::Text {
                text: "wait".to_owned(),
            }],
            None,
        )?],
        Vec::new(),
        None,
        SessionSelection::Fresh,
        None,
        64,
        true,
        BTreeMap::new(),
    )?;
    let capability = CapabilityId::new("model-mock")?;
    let descriptor = descriptor_for_profile(capability.clone(), &profile)?;
    let snapshot = ResolvedCapabilitySnapshot::from_descriptor(
        &descriptor,
        &OperationId::new("model.generate")?,
    )?;
    let revision = revision()?;
    let data = Arc::new(MockData::default());
    let (manifest, context) = manifest(&data, &revision)?;
    let request = request(&capability, &profile, manifest, task)?;
    let invocation = request.invocation().clone();
    let adapter = Arc::new(ModelEndpointAdapter::new(
        capability,
        profile,
        Arc::new(InMemorySecretResolver::new()),
        data,
    )?);
    adapter.start()?;
    let reporter = Arc::new(Reporter::default());
    let worker_adapter = adapter.clone();
    let worker_reporter = reporter.clone();
    let worker = thread::spawn(move || {
        worker_adapter.execute(
            &AdapterInvocation::with_context(&snapshot, &request, &context),
            worker_reporter.as_ref(),
        )
    });
    ready.recv_timeout(Duration::from_secs(2))?;
    let acknowledgement = adapter.cancel(&CancellationRequest::new(invocation, 1, "stop")?)?;
    assert!(acknowledgement.accepted());
    assert!(!acknowledgement.terminal_boundary());
    worker.join().map_err(|_| "adapter worker panicked")??;
    server.join().map_err(|_| "server panicked")??;
    let events = reporter.0.lock().map_err(|_| "reporter lock")?;
    assert_eq!(
        events
            .last()
            .and_then(|event| event.kind().terminal())
            .map(|terminal| terminal.status()),
        Some(TerminalStatus::Cancelled)
    );
    Ok(())
}
