//! Actual runtime/host/provider entry with a bounded local HTTP observer.
use super::*;
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityRequest, DecisionReasonCode, GrantDigest, GrantId, PolicyId,
};
use milkdrift_blueprint::{
    BindingSource, DataPort, Edge, EdgeId, EdgeKind, PortId, SchemaRef, WorkflowInterface,
};
use milkdrift_capability::{CapabilityObservation, CapabilityRequirement, SchemaId};
use milkdrift_capability_host::{
    CapabilityHost, CapabilitySelectionPolicy, HostConfig, StoreInvocationDataAccess,
};
use milkdrift_persistence::{
    ArtifactReadAuthority, EvidenceId, PageSize, Reason, RevisionStore, RunEventKind, RunJournal,
    WorkerId,
};
use milkdrift_runtime::{
    CommandAuthorityClaim, RetryPolicy, RunCommand, RuntimeConfig, RuntimeService, SchedulerLimits,
    SequentialIdGenerator,
};
use milkdrift_workspace::{ScopeId, WorkspaceScope};

struct Allow;
struct Clock;
impl milkdrift_runtime::BoundaryClock for Clock {
    fn now(
        &self,
    ) -> Result<milkdrift_persistence::TimestampMillis, milkdrift_runtime::RuntimeError> {
        Ok(milkdrift_persistence::TimestampMillis::new(1000))
    }
}
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

#[test]
fn governing_session_is_enforced_before_http_for_both_request_forms() -> TestResult {
    for artifact_backed in [false, true] {
        for declared in ["fresh", "explicit_continuation", "provider_managed"] {
            for requested in ["fresh", "explicit_continuation", "provider_managed"] {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let address = listener.local_addr()?.to_string();
                listener.set_nonblocking(true)?;
                let (stop, stopped) = mpsc::channel();
                let server = thread::spawn(move || -> std::io::Result<Option<String>> {
                    for _ in 0..600 {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                stream.set_nonblocking(false)?;
                                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                                let request = read_request(&mut stream)?;
                                let body = r#"{"id":"response-session","model":"mock-model","choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
                                write!(
                                    stream,
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                )?;
                                return Ok(Some(request));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                            Err(error) => return Err(error),
                        }
                        if stopped.recv_timeout(Duration::from_millis(5)).is_ok() {
                            return Ok(None);
                        }
                    }
                    Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "bounded endpoint observation expired",
                    ))
                });
                let directory = tempfile::tempdir()?;
                let store = Arc::new(RedbStore::open(directory.path().join("store"))?);
                let data = Arc::new(StoreInvocationDataAccess::new(
                    store.clone(),
                    directory.path().join("materialized"),
                    ArtifactReadAuthority::Authorized {
                        actor: ActorRef::new("human:session")?,
                        evidence: EvidenceId::new("session-read")?,
                    },
                )?);
                let profile = profile(
                    &address,
                    "session-profile",
                    ProviderProtocol::OpenAiCompatible {
                        path: "v1/chat/completions".to_owned(),
                    },
                    AuthMode::NoAuth,
                    BTreeSet::from([ModelFeature::SystemRole]),
                )?;
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
                        max_registrations: 1,
                        max_generations_per_capability: 1,
                        max_concurrent_per_generation: 1,
                        observation_stale_after_ms: 10_000,
                    },
                    CapabilitySelectionPolicy::priorities(BTreeMap::new()),
                )?);
                host.register(
                    descriptor.clone(),
                    adapter,
                    Some(CapabilityObservation::new(
                        descriptor.identity().clone(),
                        1000,
                        true,
                        0,
                        "ready",
                    )?),
                )?;
                let reference = ArtifactReference::new(
                    "prior",
                    "0".repeat(64),
                    Some("application/json".to_owned()),
                    Some(1),
                )?;
                let session = match requested {
                    "fresh" => SessionSelection::Fresh,
                    "explicit_continuation" => SessionSelection::ExplicitContinuation {
                        manifest: reference.clone(),
                        response: reference,
                    },
                    _ => SessionSelection::ProviderManaged {
                        session_id: "session-1".to_owned(),
                    },
                };
                let task = ModelTaskRequest::new(
                    vec![Message::new(
                        MessageRole::User,
                        vec![ContentPart::Text {
                            text: "hello".to_owned(),
                        }],
                        None,
                    )?],
                    Vec::new(),
                    None,
                    session,
                    None,
                    32,
                    false,
                    BTreeMap::new(),
                )?;
                let document = ModelTaskRequestDocument::new(task).to_canonical_json()?;
                let mut policy = serde_json::to_value(TaskContextPolicy::default())?;
                policy["session"] = json!(declared);
                let mut node = Node::new(
                    NodeId::new("model")?,
                    NodeKind::task(
                        CapabilityRequirement::new(OperationId::new("model.generate")?),
                        serde_json::from_value(policy)?,
                    )?,
                )?
                .with_control_output(PortId::new("out")?)?;
                let schema = SchemaRef::new(SchemaId::new("test.model-request")?, 1)?;
                for output in ["model_response", "final_text", "provider_metadata"] {
                    node = node
                        .with_data_output(PortId::new(output)?, DataPort::output(schema.clone()))?;
                }
                let binding = if artifact_backed {
                    use milkdrift_persistence::{ArtifactPublicationId, BeginArtifactPublication};
                    use milkdrift_workspace::{
                        ArtifactMetadata, ArtifactProvenance, ArtifactRetention, CausalId,
                        CausalReference, MediaType,
                    };
                    let durable = milkdrift_workspace::ArtifactReference::new(
                        ArtifactId::new("model-request")?,
                        ContentDigest::for_bytes(&document),
                        MediaType::new("application/json")?,
                        document.len() as u64,
                    );
                    let metadata = ArtifactMetadata::new(
                        durable.clone(),
                        ArtifactSensitivity::Restricted,
                        ArtifactRetention::WhileReferenced,
                        ArtifactProvenance::new(
                            CausalReference::External {
                                source: CausalId::new("model-request")?,
                            },
                            Vec::new(),
                        )?,
                    )?;
                    let publication = ArtifactPublicationId::new("model-request")?;
                    store.begin_publication(&BeginArtifactPublication::new(
                        publication.clone(),
                        RunId::new("request-owner")?,
                        metadata,
                        WorkspaceBudget::new(0, 0, 0, 8, 16_777_216, 16_777_216)?,
                        WorkspaceUsage::EMPTY,
                    )?)?;
                    store.write_chunk(&publication, 0, &document)?;
                    store.commit_publication(&publication)?;
                    BindingSource::Artifact {
                        reference: serde_json::to_string(&durable)?,
                        contract: schema.clone(),
                    }
                } else {
                    BindingSource::Literal {
                        value: BoundedJson::new(serde_json::from_slice(&document)?)?,
                    }
                };
                node = node.with_data_input(
                    PortId::new(MODEL_TASK_INPUT_NAME)?,
                    DataPort::input(schema, true, Some(binding))?,
                )?;
                let interface = WorkflowInterface::new([], [])?;
                let revision = BlueprintRevision::genesis(
                    WorkflowId::new("session-http")?,
                    MutationBatch::new(vec![
                        Mutation::SetInterface { interface },
                        Mutation::AddNode { node },
                        Mutation::AddNode {
                            node: Node::new(
                                NodeId::new("done")?,
                                NodeKind::Terminal {
                                    outcome: TerminalOutcome::Success,
                                },
                            )?
                            .with_control_input(PortId::new("in")?)?,
                        },
                        Mutation::AddEdge {
                            edge: Edge::new(
                                EdgeId::new("model-done")?,
                                EdgeKind::Control,
                                NodeId::new("model")?,
                                PortId::new("out")?,
                                NodeId::new("done")?,
                                PortId::new("in")?,
                            ),
                        },
                    ])?,
                    AuthorRef::new("human:session")?,
                    "session request",
                )?;
                store.put_revision(&revision)?;
                let runtime = RuntimeService::new_with_authority(
                    store.clone(),
                    host,
                    Arc::new(Allow),
                    Arc::new(Clock),
                    Arc::new(SequentialIdGenerator::new("session-http", 1)?),
                    RuntimeConfig::new(
                        WorkerId::new("session-worker")?,
                        ActorRef::new("controller:session")?,
                        30_000,
                        16,
                        SchedulerLimits::new(8, 4, 2, 4)?,
                        RetryPolicy::new(1, Vec::new(), 1, 1000, 0)?,
                    )?,
                )?;
                let run = RunId::new("session-http")?;
                let root_scope =
                    WorkspaceScope::run_root(run.clone(), ScopeId::new("session-root")?);
                let claim = CommandAuthorityClaim::new(
                    GrantId::new("grant:session")?,
                    1,
                    GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
                    0,
                )?;
                for command in [
                    RunCommand::CreateRun {
                        workflow: revision.semantic().workflow().clone(),
                        revision: revision.id().clone(),
                        root_scope,
                        workspace_budget: WorkspaceBudget::new(
                            128, 16_777_216, 16_777_216, 64, 16_777_216, 67_108_864,
                        )?,
                        inputs: Vec::new(),
                    },
                    RunCommand::StartRun,
                ] {
                    let document = runtime.command(
                        run.clone(),
                        ActorRef::new("human:session")?,
                        store.head(&run)?,
                        Reason::new("session check")?,
                        Vec::new(),
                        command,
                    )?;
                    runtime.handle_authorized_command(&document, &claim)?;
                }
                runtime.scheduler_tick()?;
                for action in runtime.claim_execution_effects(PageSize::new(8)?)? {
                    runtime.execute_effect(action)?;
                }
                let _ = stop.send(());
                let captured = server.join().map_err(|_| "endpoint observer panicked")??;
                let history = runtime.history(&run)?;
                assert_eq!(
                    captured.is_some(),
                    declared == "fresh" && requested == "fresh",
                    "{declared}/{requested}, artifact={artifact_backed}: {history:#?}"
                );
                assert_eq!(
                    history.iter().any(|event| matches!(
                        event.kind(),
                        RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
                    )),
                    declared == requested
                );
                if declared == requested && declared != "fresh" {
                    assert!(
                        history.iter().any(|event| matches!(
                            event.kind(),
                            RunEventKind::ExternalOutcomeUncertain { reason, .. }
                                if reason.as_str().contains("no configured protocol mapping")
                        )),
                        "{history:#?}"
                    );
                }
                if declared == "fresh" && requested == "fresh" {
                    assert!(
                        history.iter().any(|event| matches!(
                            event.kind(),
                            RunEventKind::NodeTerminal {
                                outcome: milkdrift_persistence::NodeOutcome::Succeeded,
                                ..
                            }
                        )),
                        "{history:#?}"
                    );
                }
            }
        }
    }
    Ok(())
}
