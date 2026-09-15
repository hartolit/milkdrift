//! Conversation selection through the production runtime, host and both wire mappings.
use super::runtime_session::{Allow, ModelFixture, NoFaults};
use super::*;
use milkdrift_authority::{ActorRef, GrantDigest, GrantId};
use milkdrift_blueprint::{BindingSource, DataPort, Edge, EdgeId, EdgeKind, PortId, SchemaRef};
use milkdrift_persistence::{
    PageSize, Reason, ReconciliationId, ReconciliationPolicy, RevisionStore, RunEventKind,
    RunJournal,
};
use milkdrift_runtime::{CommandAuthorityClaim, RunCommand};

#[path = "continuation/isolation.rs"]
mod isolation;
use isolation::{foreign_history, separate_branches};

fn task(text: &str, session: SessionSelection) -> TestResult<ModelTaskRequest> {
    Ok(ModelTaskRequest::new(
        vec![
            Message::new(
                MessageRole::System,
                vec![ContentPart::Text {
                    text: format!("instructions for {text}"),
                }],
                None,
            )?,
            Message::new(
                MessageRole::User,
                vec![ContentPart::Text {
                    text: text.to_owned(),
                }],
                None,
            )?,
        ],
        Vec::new(),
        None,
        session,
        None,
        32,
        false,
        BTreeMap::new(),
    )?)
}

fn command(fixture: &ModelFixture, command: RunCommand) -> TestResult {
    let document = fixture.runtime.command(
        fixture.run.clone(),
        ActorRef::new("human:session")?,
        fixture.store.head(&fixture.run)?,
        Reason::new("continue exact evidence")?,
        Vec::new(),
        command,
    )?;
    let outcome = fixture.runtime.handle_authorized_command(
        &document,
        &CommandAuthorityClaim::new(
            GrantId::new("grant:session")?,
            1,
            GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
            0,
        )?,
    )?;
    assert!(
        matches!(
            outcome.result().disposition(),
            milkdrift_persistence::CommandDisposition::Accepted
        ),
        "{outcome:?}"
    );
    Ok(())
}

fn append_with_policy(
    fixture: &ModelFixture,
    pending: &str,
    hold_after: bool,
    task: ModelTaskRequest,
    declared: &str,
    change: impl FnOnce(&mut serde_json::Value),
) -> TestResult {
    let projection = fixture.runtime.projection(&fixture.run)?;
    let base_id = projection.revision().ok_or("active revision missing")?;
    let base = fixture
        .store
        .revision(base_id)?
        .ok_or("base revision missing")?;
    let source = base
        .semantic()
        .nodes()
        .get(&NodeId::new("model")?)
        .ok_or("source node missing")?;
    let mut policy = serde_json::to_value(TaskContextPolicy::default())?;
    policy["session"] = json!(declared);
    policy["exclude_categories"] = json!([]);
    change(&mut policy);
    let mut next = Node::new(
        NodeId::new(pending)?,
        NodeKind::task(
            milkdrift_capability::CapabilityRequirement::new(OperationId::new("model.generate")?),
            serde_json::from_value(policy)?,
        )?,
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?;
    for (port, output) in source.data_outputs() {
        next = next.with_data_output(port.clone(), output.clone())?;
    }
    next = next.with_data_input(
        PortId::new(MODEL_TASK_INPUT_NAME)?,
        DataPort::input(
            SchemaRef::new(
                milkdrift_capability::SchemaId::new("test.model-request")?,
                1,
            )?,
            true,
            Some(BindingSource::Literal {
                value: BoundedJson::new(serde_json::to_value(ModelTaskRequestDocument::new(
                    task,
                ))?)?,
            }),
        )?,
    )?;
    let mut mutations = vec![
        Mutation::ReplaceNode { node: next },
        Mutation::AddNode {
            node: Node::new(
                NodeId::new(format!("{pending}-end"))?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?
            .with_control_input(PortId::new("in")?)?,
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(format!("{pending}-end"))?,
                EdgeKind::Control,
                NodeId::new(pending)?,
                PortId::new("out")?,
                NodeId::new(format!(
                    "{pending}-{}",
                    if hold_after { "hold" } else { "end" }
                ))?,
                PortId::new("in")?,
            ),
        },
    ];
    if hold_after {
        mutations.push(Mutation::AddNode {
            node: Node::new(
                NodeId::new(format!("{pending}-hold"))?,
                NodeKind::SignalWait {
                    signal: OperationId::new("continuation.hold")?,
                },
            )?
            .with_control_input(PortId::new("in")?)?
            .with_control_output(PortId::new("out")?)?,
        });
        mutations.push(Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new(format!("{pending}-release"))?,
                EdgeKind::Control,
                NodeId::new(format!("{pending}-hold"))?,
                PortId::new("out")?,
                NodeId::new(format!("{pending}-end"))?,
                PortId::new("in")?,
            ),
        });
    }
    let revision = base.revise(
        base.id(),
        MutationBatch::new(mutations)?,
        AuthorRef::new("human:session")?,
        "continue exact predecessor",
    )?;
    fixture.store.put_revision(&revision)?;
    command(
        fixture,
        RunCommand::RequestRevisionAdoption {
            reconciliation: ReconciliationId::new(format!("continue-{pending}"))?,
            revision: revision.id().clone(),
            policy: ReconciliationPolicy::FinishCurrentThenAdopt,
        },
    )?;
    let projection = fixture.runtime.projection(&fixture.run)?;
    let plan = projection
        .reconciliation()
        .plans()
        .values()
        .last()
        .ok_or("plan missing")?
        .plan()
        .clone();
    command(fixture, RunCommand::ApplyReconciliation { plan })
}

struct Revocable(Arc<AtomicBool>, bool);
impl milkdrift_authority::AuthorityEvaluator for Revocable {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, milkdrift_authority::AuthorityError>
    {
        use milkdrift_authority::{
            AuthorityBudget, AuthorityDecisionSnapshot, DecisionReasonCode, PolicyId,
        };
        if self.0.load(Ordering::SeqCst)
            && (!self.1
                || request.operation
                    == milkdrift_authority::AuthorityOperation::ReadArtifactContent)
        {
            AuthorityDecisionSnapshot::from_evaluation(
                PolicyId::new("test.model-session")?,
                1,
                request.clone(),
                vec![DecisionReasonCode::Revoked],
                AuthorityBudget::default(),
                SideEffectClass::None,
            )
        } else {
            milkdrift_authority::AuthorityEvaluator::evaluate(&Allow, request)
        }
    }
}

fn restarted(
    fixture: &ModelFixture,
    authority: Arc<dyn milkdrift_authority::AuthorityEvaluator>,
) -> TestResult<milkdrift_runtime::RuntimeService> {
    use milkdrift_runtime::{
        ManualClock, RetryPolicy, RuntimeConfig, RuntimeService, SchedulerLimits,
        SequentialIdGenerator,
    };
    Ok(RuntimeService::new_with_authority(
        fixture.store.clone(),
        fixture.host.clone(),
        authority,
        Arc::new(ManualClock::new(1000)),
        Arc::new(SequentialIdGenerator::new("continuation-restart", 1)?),
        RuntimeConfig::new(
            milkdrift_persistence::WorkerId::new("session-worker")?,
            ActorRef::new("controller:session")?,
            30_000,
            16,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 1, 1000, 0)?,
        )?,
    )?)
}

fn changed_reference(
    reference: &ArtifactReference,
    identity: &str,
    digest: &str,
    media: Option<&str>,
) -> TestResult<ArtifactReference> {
    Ok(ArtifactReference::new(
        identity,
        digest,
        media.map(str::to_owned),
        reference.size_bytes(),
    )?)
}

fn publish_unrelated(
    fixture: &ModelFixture,
    original: &ArtifactReference,
) -> TestResult<ArtifactReference> {
    let bytes = serde_json::to_vec(
        &json!({"schema_version":1,"response":{"text":"UNRELATED LATER ANSWER","structured":null,"tool_calls":[],"finish_reason":"stop","usage":{"input_units":null,"output_units":null,"cached_input_units":null,"cost_micros":null,"currency":null},"metadata":{}}}),
    )?;
    publish_bytes(
        fixture,
        original.media_type().ok_or("media missing")?,
        &bytes,
    )
}

fn publish_bytes(
    fixture: &ModelFixture,
    media: &str,
    bytes: &[u8],
) -> TestResult<ArtifactReference> {
    use milkdrift_persistence::{ArtifactPublicationId, BeginArtifactPublication};
    use milkdrift_workspace::{
        ArtifactMetadata, ArtifactProvenance, ArtifactRetention, CausalId, CausalReference,
        MediaType,
    };
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new("unrelated-response")?,
        ContentDigest::for_bytes(bytes),
        MediaType::new(media)?,
        bytes.len() as u64,
    );
    let metadata = ArtifactMetadata::new(
        reference.clone(),
        ArtifactSensitivity::Restricted,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("different-invocation")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = ArtifactPublicationId::new("unrelated-response")?;
    fixture
        .store
        .begin_publication(&BeginArtifactPublication::new(
            publication.clone(),
            fixture.run.clone(),
            metadata,
            fixture
                .runtime
                .projection(&fixture.run)?
                .workspace_budget()
                .ok_or("budget absent")?
                .clone(),
            milkdrift_persistence::WorkspaceStore::workspace_usage(
                fixture.store.as_ref(),
                &fixture.run,
            )?,
        )?)?;
    fixture.store.write_chunk(&publication, 0, bytes)?;
    fixture.store.commit_publication(&publication)?;
    Ok(ArtifactReference::new(
        reference.artifact().as_str(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().to_owned()),
        Some(reference.size_bytes()),
    )?)
}

#[test]
fn exact_continuation_reaches_both_wires_and_fresh_has_no_implicit_history() -> TestResult {
    for anthropic in [false, true] {
        for case in [
            "continue",
            "chain",
            "tool_chain",
            "selected_evidence",
            "fresh",
            "missing_manifest",
            "missing_response",
            "digest",
            "media",
            "pair",
            "budget_items",
            "budget_bytes",
            "budget_journal",
            "excluded",
            "excluded_tool_history",
            "tool_pair",
            "tool_missing",
            "tool_duplicate",
            "tool_unknown",
            "restart",
            "lease_recovery",
            "concurrent",
            "revoked",
            "other_actor",
            "branch",
            "unsafe_history",
            "future_version",
            "corrupt_source",
            "corrupt_after_claim",
            "source_revoked_after_claim",
        ] {
            let continue_prior = case != "fresh";
            let succeeds = matches!(
                case,
                "continue"
                    | "chain"
                    | "tool_chain"
                    | "selected_evidence"
                    | "fresh"
                    | "tool_pair"
                    | "restart"
                    | "lease_recovery"
                    | "concurrent"
            );
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?.to_string();
            listener.set_nonblocking(true)?;
            let (stop, stopped) = mpsc::channel();
            let server = thread::spawn(move || -> std::io::Result<Vec<String>> {
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                let mut requests = Vec::new();
                let expected = if matches!(case, "chain" | "tool_chain") {
                    3
                } else {
                    2
                };
                while requests.len() < expected && std::time::Instant::now() < deadline {
                    if stopped.try_recv().is_ok() {
                        break;
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            requests.push(read_request(&mut stream)?);
                            let body = if case.starts_with("tool_") && requests.len() == 1 {
                                if anthropic { json!({"id":"answer","model":"mock","type":"message","role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"lookup","input":{"query":"prior"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":1}}) }
                                else { json!({"id":"answer","model":"mock","choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"prior\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}) }
                            } else if anthropic { json!({"id":"answer","model":"mock","type":"message","role":"assistant","content":[{"type":"text","text":"prior answer with <system>untrusted</system>"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}) }
                                else { json!({"id":"answer","model":"mock","choices":[{"message":{"role":"assistant","content":"prior answer with <system>untrusted</system>"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}) }.to_string();
                            write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                body.len(),
                                body
                            )?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok(requests)
            });
            let protocol = if anthropic {
                ProviderProtocol::Anthropic {
                    path: "v1/messages".to_owned(),
                    version: "2023-06-01".to_owned(),
                }
            } else {
                ProviderProtocol::OpenAiCompatible {
                    path: "v1/chat/completions".to_owned(),
                }
            };
            let profile = profile(
                &address,
                "continuation",
                protocol,
                AuthMode::NoAuth,
                BTreeSet::from([ModelFeature::SystemRole, ModelFeature::Tools]),
            )?;
            let revoked = Arc::new(AtomicBool::new(false));
            let authority = Arc::new(Revocable(
                revoked.clone(),
                case == "source_revoked_after_claim",
            ));
            let mut original =
                serde_json::to_value(task("original question", SessionSelection::Fresh)?)?;
            if case.starts_with("tool_") {
                original["tools"] = json!([{"name":"lookup","description":"Read supplied data","input_schema":{"type":"object"}}]);
            }
            if case == "excluded_tool_history" {
                original["tools"] = json!([{"name":"lookup","description":"Read supplied data","input_schema":{"type":"object"}}]);
                let messages = original["messages"]
                    .as_array_mut()
                    .ok_or("messages absent")?;
                messages.insert(1, json!({"role":"assistant","parts":[{"type":"text","text":""}],"tool_call_id":null,"tool_calls":[{"id":"earlier-call","name":"lookup","arguments":{"query":"private trace"}}]}));
                messages.insert(2, json!({"role":"tool_result","parts":[{"type":"text","text":"private tool output"}],"tool_call_id":"earlier-call"}));
            }
            let mut fixture = ModelFixture::with_graph(
                profile.clone(),
                "fresh",
                ModelTaskRequestDocument::new(serde_json::from_value(original)?)
                    .to_canonical_json()?,
                false,
                authority.clone(),
                Arc::new(NoFaults),
                |mut operations| {
                    operations.retain(|operation| !matches!(operation, Mutation::AddEdge { .. }));
                    operations.push(Mutation::AddNode {
                        node: Node::new(
                            NodeId::new("hold")?,
                            NodeKind::SignalWait {
                                signal: OperationId::new("continuation.hold")?,
                            },
                        )?
                        .with_control_input(PortId::new("in")?)?
                        .with_control_output(PortId::new("out")?)?,
                    });
                    for (source, target) in [("model", "hold"), ("hold", "done")] {
                        operations.push(Mutation::AddEdge {
                            edge: Edge::new(
                                EdgeId::new(format!("{source}-{target}"))?,
                                EdgeKind::Control,
                                NodeId::new(source)?,
                                PortId::new("out")?,
                                NodeId::new(target)?,
                                PortId::new("in")?,
                            ),
                        });
                    }
                    if case == "branch" {
                        return separate_branches(operations);
                    }
                    if case == "selected_evidence" {
                        return isolation::select_evidence(operations);
                    }
                    Ok(operations)
                },
            )?;
            if case == "selected_evidence" {
                publish_bytes(&fixture, "text/plain", b"PRIOR_SELECTED_EVIDENCE")?;
            }
            fixture.runtime.scheduler_tick()?;
            for effect in fixture.runtime.claim_execution_effects(PageSize::new(8)?)? {
                fixture.runtime.execute_effect(effect)?;
            }
            let history = if case == "other_actor" {
                foreign_history(&fixture)?
            } else {
                fixture.runtime.history(&fixture.run)?
            };
            let mut predecessor = history
                .iter()
                .find_map(|event| match event.kind() {
                    RunEventKind::NodeScheduled { request, .. } => {
                        request.context_manifest().cloned()
                    }
                    _ => None,
                })
                .ok_or("prior manifest missing")?;
            let response = history
                .iter()
                .find_map(|event| match event.kind() {
                    RunEventKind::NodeOutputPublished {
                        artifact: Some(reference),
                        ..
                    } if reference.media_type().as_str().contains("model-response") => {
                        Some(reference)
                    }
                    _ => None,
                })
                .ok_or("prior response missing")?;
            let mut response = ArtifactReference::new(
                response.artifact().as_str(),
                response.digest().to_hex(),
                Some(response.media_type().as_str().to_owned()),
                Some(response.size_bytes()),
            )?;
            match case {
                "future_version" | "unsafe_history" => {
                    use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId};
                    let prior = milkdrift_runtime::read_context_manifest(
                        fixture.store.as_ref(),
                        &predecessor,
                        ArtifactReadAuthority::Authorized {
                            actor: ActorRef::new("human:session")?,
                            evidence: EvidenceId::new("test-retained")?,
                        },
                    )?;
                    let mut omissions = prior.omissions().to_vec();
                    if case == "unsafe_history" {
                        omissions.push(milkdrift_model::ContextOmission {
                            source: None,
                            kind: ContextSemanticKind::PriorPrompt,
                            reason: milkdrift_model::ContextOmissionReason::NotSelected,
                            required: false,
                            omitted_bytes: 1,
                            omitted_artifact_bytes: 0,
                        });
                    }
                    let retained = ContextManifest::new(
                        prior.run().clone(),
                        prior.revision().clone(),
                        prior.node().clone(),
                        prior.execution().clone(),
                        prior.attempt().clone(),
                        if case == "unsafe_history" { 1 } else { 3 },
                        prior.policy_digest().clone(),
                        prior.entries().to_vec(),
                        omissions,
                        prior.totals(),
                        prior.budget(),
                    )?;
                    predecessor = publish_bytes(
                        &fixture,
                        predecessor.media_type().ok_or("media missing")?,
                        &ContextManifestDocument::new(retained).to_canonical_json()?,
                    )?;
                }
                "missing_manifest" => {
                    predecessor = changed_reference(
                        &predecessor,
                        "absent-manifest",
                        predecessor.digest(),
                        predecessor.media_type(),
                    )?
                }
                "missing_response" => {
                    response = changed_reference(
                        &response,
                        "absent-response",
                        response.digest(),
                        response.media_type(),
                    )?
                }
                "digest" => {
                    response = changed_reference(
                        &response,
                        response.identity(),
                        &"0".repeat(64),
                        response.media_type(),
                    )?
                }
                "media" => {
                    predecessor = changed_reference(
                        &predecessor,
                        predecessor.identity(),
                        predecessor.digest(),
                        Some("application/vnd.milkdrift.context-manifest.v99+json"),
                    )?
                }
                "pair" => response = publish_unrelated(&fixture, &response)?,
                _ => {}
            }
            let session = if continue_prior {
                SessionSelection::ExplicitContinuation {
                    manifest: predecessor,
                    response,
                }
            } else {
                SessionSelection::Fresh
            };
            let mut next_task = serde_json::to_value(task("new question", session)?)?;
            if matches!(
                case,
                "tool_pair" | "tool_chain" | "tool_duplicate" | "tool_unknown"
            ) {
                let result = json!({"role":"tool_result","parts":[{"type":"text","text":"exact result"}],"tool_call_id":if case == "tool_unknown" { "unknown" } else { "call-1" }});
                let messages = next_task["messages"]
                    .as_array_mut()
                    .ok_or("messages absent")?;
                messages.insert(1, result.clone());
                if case == "tool_duplicate" {
                    messages.insert(2, result);
                }
            }
            append_with_policy(
                &fixture,
                "done",
                matches!(case, "chain" | "tool_chain"),
                serde_json::from_value(next_task)?,
                if continue_prior {
                    "explicit_continuation"
                } else {
                    "fresh"
                },
                |policy| match case {
                    "budget_items" => {
                        policy["budget"]["max_items"] = json!(1);
                        policy["budget"]["max_artifacts"] = json!(1);
                    }
                    "budget_bytes" => policy["budget"]["max_artifact_bytes"] = json!(1),
                    "budget_journal" => {
                        policy["budget"]["max_candidate_records"] = json!(1);
                        policy["budget"]["max_event_summaries"] = json!(0);
                    }
                    "excluded" => policy["exclude_categories"] = json!(["prior_prompt"]),
                    "excluded_tool_history" => policy["exclude_categories"] = json!(["tool_trace"]),
                    _ => {}
                },
            )?;
            command(
                &fixture,
                RunCommand::DeliverSignal {
                    signal: milkdrift_persistence::SignalId::new("continue-now")?,
                    signal_type: milkdrift_persistence::SignalTypeId::new("continuation.hold")?,
                    correlation: None,
                    mode: milkdrift_persistence::SignalDeliveryMode::OneShot,
                    payload: BoundedJson::new(json!({"ready":true}))?,
                },
            )?;
            fixture.runtime.scheduler_tick()?;
            let corrupt_source = || -> TestResult {
                let reference = history
                    .iter()
                    .find_map(|event| match event.kind() {
                        RunEventKind::NodeOutputPublished {
                            artifact: Some(reference),
                            ..
                        } if reference.media_type().as_str().contains("model-response") => {
                            Some(reference)
                        }
                        _ => None,
                    })
                    .ok_or("source missing")?;
                let digest = reference.digest().to_hex();
                let path = fixture
                    .directory
                    .path()
                    .join("store/artifacts")
                    .join(&digest[..2])
                    .join(&digest[2..]);
                assert!(path.is_file(), "{}", path.display());
                std::fs::write(path, b"corrupt")?;
                Ok(())
            };
            if case == "corrupt_source" {
                corrupt_source()?;
            }
            if matches!(case, "corrupt_after_claim" | "source_revoked_after_claim") {
                let effects = fixture.runtime.claim_execution_effects(PageSize::new(8)?)?;
                assert_eq!(effects.len(), 1);
                if case == "corrupt_after_claim" {
                    corrupt_source()?;
                } else {
                    revoked.store(true, Ordering::SeqCst);
                }
                for effect in effects {
                    fixture.runtime.execute_effect(effect)?;
                }
            }
            if matches!(case, "restart" | "lease_recovery") {
                let before = fixture.runtime.history(&fixture.run)?;
                fixture = fixture.reopen(
                    profile,
                    authority.clone(),
                    if case == "lease_recovery" {
                        100_000
                    } else {
                        1000
                    },
                )?;
                assert!(fixture.runtime.history(&fixture.run)?.starts_with(&before));
            }
            if case == "revoked" {
                fixture.runtime = restarted(&fixture, authority)?;
            }
            if case == "revoked" {
                revoked.store(true, Ordering::SeqCst);
            }
            if case == "concurrent" {
                let original_response = history
                    .iter()
                    .find_map(|event| match event.kind() {
                        RunEventKind::NodeOutputPublished {
                            artifact: Some(reference),
                            ..
                        } if reference.media_type().as_str().contains("model-response") => {
                            Some(reference)
                        }
                        _ => None,
                    })
                    .ok_or("source missing")?;
                let reference = ArtifactReference::new(
                    original_response.artifact().as_str(),
                    original_response.digest().to_hex(),
                    Some(original_response.media_type().as_str().to_owned()),
                    Some(original_response.size_bytes()),
                )?;
                publish_unrelated(&fixture, &reference)?;
            }
            let mut extended = false;
            for _ in 0..4 {
                fixture.runtime.scheduler_tick()?;
                for effect in fixture.runtime.claim_execution_effects(PageSize::new(8)?)? {
                    fixture.runtime.execute_effect(effect)?;
                    if matches!(case, "chain" | "tool_chain") && !extended {
                        let history = fixture.runtime.history(&fixture.run)?;
                        let manifest = history
                            .iter()
                            .rev()
                            .find_map(|event| match event.kind() {
                                RunEventKind::NodeScheduled { request, .. } => {
                                    request.context_manifest().cloned()
                                }
                                _ => None,
                            })
                            .ok_or("continued manifest missing")?;
                        let response = history
                            .iter()
                            .rev()
                            .find_map(|event| match event.kind() {
                                RunEventKind::NodeOutputPublished {
                                    artifact: Some(reference),
                                    ..
                                } if reference.media_type().as_str().contains("model-response") => {
                                    Some(reference)
                                }
                                _ => None,
                            })
                            .ok_or("continued response missing")?;
                        let response = ArtifactReference::new(
                            response.artifact().as_str(),
                            response.digest().to_hex(),
                            Some(response.media_type().as_str().to_owned()),
                            Some(response.size_bytes()),
                        )?;
                        append_with_policy(
                            &fixture,
                            "done-end",
                            false,
                            task(
                                "third question",
                                SessionSelection::ExplicitContinuation { manifest, response },
                            )?,
                            "explicit_continuation",
                            |_| {},
                        )?;
                        command(
                            &fixture,
                            RunCommand::DeliverSignal {
                                signal: milkdrift_persistence::SignalId::new("chain-continue")?,
                                signal_type: milkdrift_persistence::SignalTypeId::new(
                                    "continuation.hold",
                                )?,
                                correlation: None,
                                mode: milkdrift_persistence::SignalDeliveryMode::OneShot,
                                payload: BoundedJson::new(json!({"ready":true}))?,
                            },
                        )?;
                        extended = true;
                    }
                }
            }
            let _ = stop.send(());
            let requests = server.join().map_err(|_| "server panicked")??;
            assert_eq!(
                requests.len(),
                if matches!(case, "chain" | "tool_chain") {
                    3
                } else if succeeds || case == "other_actor" {
                    2
                } else {
                    1
                },
                "{case}: {:#?}",
                fixture.runtime.history(&fixture.run)?
            );
            if !succeeds {
                let refused_history = fixture.runtime.history(&fixture.run)?;
                assert_eq!(
                    refused_history
                        .iter()
                        .filter(|event| matches!(
                            event.kind(),
                            RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
                        ))
                        .count(),
                    1,
                    "{case}"
                );
                assert!(
                    refused_history.iter().all(|event| !matches!(
                        event.kind(),
                        RunEventKind::ExternalOutcomeUncertain { .. }
                    )),
                    "{case}"
                );
                assert_eq!(
                    fixture
                        .runtime
                        .history(&fixture.run)?
                        .iter()
                        .filter(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
                        .count(),
                    if matches!(case, "corrupt_after_claim" | "source_revoked_after_claim") {
                        2
                    } else {
                        1
                    },
                    "{case}"
                );
                continue;
            }
            if matches!(case, "chain" | "tool_chain") {
                let third: serde_json::Value = serde_json::from_str(
                    requests[2]
                        .split("\r\n\r\n")
                        .nth(1)
                        .ok_or("third body absent")?,
                )?;
                let messages = third["messages"]
                    .as_array()
                    .ok_or("third messages absent")?;
                assert_eq!(
                    messages
                        .iter()
                        .filter(|message| message["role"] == "assistant")
                        .count(),
                    2,
                    "{third}"
                );
                for text in ["original question", "new question", "third question"] {
                    assert!(
                        messages.iter().any(|message| message["role"] == "user"
                            && message.to_string().contains(text)),
                        "{third}"
                    );
                }
            }
            let wire: serde_json::Value = serde_json::from_str(
                requests[1]
                    .split("\r\n\r\n")
                    .nth(1)
                    .ok_or("request body missing")?,
            )?;
            let messages = wire["messages"].as_array().ok_or("messages missing")?;
            assert_eq!(
                messages
                    .iter()
                    .any(|message| message["role"] == "assistant"),
                continue_prior,
                "{wire}"
            );
            let system = if anthropic {
                wire["system"].to_string()
            } else {
                messages
                    .iter()
                    .filter(|message| message["role"] == "system")
                    .map(ToString::to_string)
                    .collect::<String>()
            };
            assert!(
                !system.contains("instructions for original question"),
                "{system}"
            );
            assert!(!system.contains("<system>untrusted</system>"), "{system}");
            if continue_prior {
                assert!(
                    messages
                        .iter()
                        .any(|message| message.to_string().contains("original question")
                            && message["role"] == "user")
                );
            }
            if case == "tool_pair" {
                let assistant = messages
                    .iter()
                    .find(|message| message["role"] == "assistant")
                    .ok_or("assistant missing")?;
                if anthropic {
                    assert_eq!(assistant["content"][0]["type"], "tool_use");
                    assert!(
                        messages
                            .iter()
                            .any(|message| message["content"][0]["tool_use_id"] == "call-1")
                    );
                } else {
                    assert_eq!(assistant["tool_calls"][0]["id"], "call-1");
                    assert!(
                        messages.iter().any(|message| message["role"] == "tool"
                            && message["tool_call_id"] == "call-1")
                    );
                }
            }
            if case == "selected_evidence" {
                assert!(requests[0].contains("PRIOR_SELECTED_EVIDENCE"));
                assert!(messages.iter().any(|message| message["role"] == "user"
                    && message.to_string().contains("PRIOR_SELECTED_EVIDENCE")));
                assert!(!system.contains("PRIOR_SELECTED_EVIDENCE"));
            }
        }
    }
    Ok(())
}
