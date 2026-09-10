//! Selection and reuse checks through the production discovery, journal, and artifact ports.

use super::*;
use milkdrift_blueprint::TaskContextPolicy;
use milkdrift_model::{ContextManifestDocument, ContextSource, MODEL_TASK_INPUT_NAME};
use milkdrift_persistence::{ArtifactReadAuthority, EvidenceId};

fn policy(session: &str, stopped: bool, fail_closed: bool) -> TestResult<TaskContextPolicy> {
    let mut value = serde_json::to_value(TaskContextPolicy::default())?;
    value["session"] = json!(session);
    value["fail_closed"] = json!(fail_closed);
    if stopped {
        value["truncation"] = json!("stop_at_first_overflow");
        value["budget"]["max_bytes"] = json!(100);
    }
    Ok(serde_json::from_value(value)?)
}

fn work(policy: TaskContextPolicy) -> TestResult<Node> {
    Ok(Node::new(
        NodeId::new("work")?,
        NodeKind::task(
            CapabilityRequirement::new(OperationId::new("model.generate")?),
            policy,
        )?,
    )?
    .with_control_output(PortId::new("out")?)?)
}

fn literal(node: Node, name: &str, value: serde_json::Value, required: bool) -> TestResult<Node> {
    Ok(node.with_data_input(
        PortId::new(name)?,
        DataPort::input(
            item_schema()?,
            required,
            Some(BindingSource::Literal {
                value: BoundedJson::new(value)?,
            }),
        )?,
    )?)
}

fn workflow(node: Node) -> TestResult<BlueprintRevision> {
    revision(
        "context-enforcement",
        vec![node, terminal("done", TerminalOutcome::Success)?],
        vec![control_edge("work-done", "work", "out", "done", "in")?],
    )
}

fn scheduled_request(runtime: &RuntimeService, run: &RunId) -> TestResult<InvocationRequest> {
    runtime
        .history(run)?
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { request, .. } => Some(request.clone()),
            _ => None,
        })
        .ok_or_else(|| "missing scheduled request".into())
}

fn manifest(
    store: &RedbStore,
    request: &InvocationRequest,
) -> TestResult<milkdrift_model::ContextManifest> {
    Ok(milkdrift_runtime::read_context_manifest(
        store,
        request.context_manifest().ok_or("missing manifest")?,
        ArtifactReadAuthority::Authorized {
            actor: ActorRef::new("human:context-test")?,
            evidence: EvidenceId::new("read-context-test")?,
        },
    )?)
}

#[test]
fn optional_overflow_cannot_dispatch_later_required_direct_input() -> TestResult {
    for fail_closed in [true, false] {
        let harness = Harness::new("required-context-stop")?;
        let node = literal(
            work(policy("fresh", true, fail_closed)?)?,
            "aaa-optional",
            json!("x".repeat(200)),
            false,
        )?;
        let revision = workflow(literal(node, "zzz-required", json!(1), true)?)?;
        let run = RunId::new("required-context-stop")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        harness.runtime.scheduler_tick()?;
        let actions = harness.runtime.claim_execution_effects(PageSize::new(8)?)?;
        assert_eq!(actions.is_empty(), fail_closed);
        let history = harness.runtime.history(&run)?;
        assert_eq!(
            history
                .iter()
                .any(|event| matches!(event.kind(), RunEventKind::NodePreDispatchFailed { .. })),
            fail_closed
        );
        if !fail_closed {
            let saved = manifest(&harness.store, &scheduled_request(&harness.runtime, &run)?)?;
            assert_eq!(saved.omissions().len(), 2);
            assert!(saved.omissions()[1].required);
        }
    }
    Ok(())
}

struct DenyEvidence;
impl AuthorityEvaluator for DenyEvidence {
    fn evaluate(
        &self,
        request: &milkdrift_authority::AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        if matches!(
            request.operation,
            AuthorityOperation::ReadArtifactContent | AuthorityOperation::ReadWorkspaceValue
        ) {
            AuthorityDecisionSnapshot::from_evaluation(
                PolicyId::new("test.deny-evidence")?,
                1,
                request.clone(),
                vec![DecisionReasonCode::GrantNotFound],
                AuthorityBudget::default(),
                SideEffectClass::Unknown,
            )
        } else {
            TestAuthorityEvaluator.evaluate(request)
        }
    }
}

#[test]
fn production_denied_artifact_omissions_are_redacted_when_stopped_or_excluded() -> TestResult {
    for stopped in [true, false] {
        let harness = Harness::with_descriptor_ids_and_authority(
            "denied-omission",
            RetryPolicy::new(1, Vec::new(), 1, 100, 0)?,
            test_descriptor()?,
            Arc::new(SequentialIdGenerator::new("denied-omission", 1)?),
            Arc::new(DenyEvidence),
        )?;
        let artifact = publish_artifact_with_sensitivity(
            &harness.store,
            &RunId::new("evidence-owner")?,
            "private-artifact-identity",
            b"private omitted content",
            ArtifactSensitivity::Restricted,
        )?;
        let durable = causal_context_production::durable_artifact(&artifact)?;
        let mut value = serde_json::to_value(policy("fresh", stopped, false)?)?;
        if !stopped {
            value["exclude_categories"] = json!(["artifact"]);
        }
        let policy: TaskContextPolicy = serde_json::from_value(value)?;
        let policy = policy.with_exact_sources(
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::from([serde_json::to_string(&ContextSource::Artifact {
                reference: durable,
            })?]),
        )?;
        let mut node = work(policy)?;
        if stopped {
            node = literal(node, "aaa-optional", json!("x".repeat(200)), false)?;
        }
        let revision = workflow(node)?;
        let run = RunId::new("denied-omission")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        harness.runtime.scheduler_tick()?;
        let request = scheduled_request(&harness.runtime, &run)?;
        let saved = manifest(&harness.store, &request)?;
        let encoded = ContextManifestDocument::new(saved.clone()).to_canonical_json()?;
        assert!(!std::str::from_utf8(&encoded)?.contains("private-artifact-identity"));
        let omission = saved
            .omissions()
            .iter()
            .find(|omission| omission.kind == milkdrift_model::ContextSemanticKind::Artifact)
            .ok_or("missing artifact omission")?;
        assert!(omission.source.is_none());
        assert_eq!(
            (omission.omitted_bytes, omission.omitted_artifact_bytes),
            (0, 0)
        );
        assert_eq!(
            omission.reason,
            if stopped {
                milkdrift_model::ContextOmissionReason::SelectionStopped
            } else {
                milkdrift_model::ContextOmissionReason::ExcludedCategory
            }
        );
        assert!(!request.inputs().iter().any(|input| {
            input
                .name()
                .starts_with(milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX)
        }));
        assert!(
            milkdrift_runtime::materialize_selected_context(harness.store.as_ref(), &saved)?
                .is_empty()
        );
        assert_eq!(
            harness
                .runtime
                .claim_execution_effects(PageSize::new(8)?)?
                .len(),
            1
        );
    }
    Ok(())
}

fn model_document(session: serde_json::Value) -> TestResult<serde_json::Value> {
    use milkdrift_model::{
        ContentPart, Message, MessageRole, ModelTaskRequest, ModelTaskRequestDocument,
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
        serde_json::from_value(session)?,
        None,
        32,
        false,
        BTreeMap::new(),
    )?;
    Ok(serde_json::to_value(ModelTaskRequestDocument::new(task))?)
}

#[test]
fn omitted_model_request_still_requires_independent_artifact_read_authority() -> TestResult {
    let harness = Harness::with_descriptor_ids_and_authority(
        "denied-model-request",
        RetryPolicy::new(1, Vec::new(), 1, 100, 0)?,
        test_descriptor()?,
        Arc::new(SequentialIdGenerator::new("denied-model-request", 1)?),
        Arc::new(DenyEvidence),
    )?;
    let document = model_document(json!({"type":"fresh"}))?;
    let artifact = publish_artifact_with_sensitivity(
        &harness.store,
        &RunId::new("request-owner")?,
        "restricted-model-request",
        &serde_json::to_vec(&document)?,
        ArtifactSensitivity::Restricted,
    )?;
    let mut policy = serde_json::to_value(policy("fresh", false, false)?)?;
    policy["include_categories"] = json!([]);
    policy["exclude_categories"] = json!(["direct_input"]);
    let node = work(serde_json::from_value(policy)?)?.with_data_input(
        PortId::new(MODEL_TASK_INPUT_NAME)?,
        DataPort::input(
            item_schema()?,
            true,
            Some(BindingSource::Artifact {
                reference: serde_json::to_string(&causal_context_production::durable_artifact(
                    &artifact,
                )?)?,
                contract: item_schema()?,
            }),
        )?,
    )?;
    let revision = workflow(node)?;
    let run = RunId::new("denied-model-request")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    harness.runtime.scheduler_tick()?;
    let request = scheduled_request(&harness.runtime, &run)?;
    assert!(manifest(&harness.store, &request)?.entries().is_empty());
    assert!(
        harness
            .runtime
            .claim_execution_effects(PageSize::new(8)?)?
            .is_empty()
    );
    assert!(
        !harness
            .runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
    );
    assert_eq!(request, scheduled_request(&harness.runtime, &run)?);
    Ok(())
}

#[test]
fn model_session_agreement_is_checked_for_inline_artifact_and_recovered_requests() -> TestResult {
    let continuation = json!({"type":"explicit_continuation", "manifest":{"identity":"prior-manifest","digest":"0".repeat(64),"media_type":"application/json","size_bytes":1},
        "response":{"identity":"prior-response","digest":"0".repeat(64),"media_type":"application/json","size_bytes":1}});
    let sessions = [
        json!({"type":"fresh"}),
        continuation,
        json!({"type":"provider_managed","session_id":"session-1"}),
    ];
    let policies = ["fresh", "explicit_continuation", "provider_managed"];
    for artifact_backed in [false, true] {
        for recovered in [false, true] {
            for (policy_index, declared) in policies.iter().enumerate() {
                for (request_index, session) in sessions.iter().enumerate() {
                    let harness = Harness::new("session-agreement")?;
                    let document = model_document(session.clone())?;
                    milkdrift_model::ModelTaskRequestDocument::from_json(&serde_json::to_vec(
                        &document,
                    )?)?;
                    let mut node = work(policy(declared, false, true)?)?;
                    if artifact_backed {
                        let artifact = publish_artifact_in_store(
                            &harness.store,
                            &RunId::new("model-request-owner")?,
                            "model-request",
                            &serde_json::to_vec(&document)?,
                        )?;
                        let durable = causal_context_production::durable_artifact(&artifact)?;
                        node = node.with_data_input(
                            PortId::new(MODEL_TASK_INPUT_NAME)?,
                            DataPort::input(
                                item_schema()?,
                                true,
                                Some(BindingSource::Artifact {
                                    reference: serde_json::to_string(&durable)?,
                                    contract: item_schema()?,
                                }),
                            )?,
                        )?;
                    } else {
                        node = literal(node, MODEL_TASK_INPUT_NAME, document, true)?;
                    }
                    let revision = workflow(node)?;
                    let run = RunId::new("session-agreement")?;
                    harness.put_revision(&revision)?;
                    harness.create_and_start(&run, &revision)?;
                    harness.runtime.scheduler_tick()?;
                    let before = scheduled_request(&harness.runtime, &run)?;
                    if recovered {
                        harness.runtime.recover()?;
                    }
                    let actions = harness.runtime.claim_execution_effects(PageSize::new(8)?)?;
                    assert_eq!(
                        actions.len(),
                        usize::from(policy_index == request_index),
                        "{declared}, {session}, artifact={artifact_backed}, recovered={recovered}"
                    );
                    let history = harness.runtime.history(&run)?;
                    assert_eq!(
                        history
                            .iter()
                            .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. })),
                        policy_index == request_index
                    );
                    assert_eq!(before, scheduled_request(&harness.runtime, &run)?);
                }
            }
        }
    }
    Ok(())
}

#[test]
fn hidden_workspace_omissions_remain_redacted_through_production_discovery() -> TestResult {
    for stopped in [true, false] {
        let harness = Harness::new("hidden-omission")?;
        let hidden_run = RunId::new("hidden-owner")?;
        let root = WorkspaceScope::run_root(hidden_run.clone(), ScopeId::new("hidden-scope")?);
        let entry = WorkspaceValueEntry::initial(
            root.reference().clone(),
            ValueKey::new("hidden-value")?,
            WorkspaceValue::Json(BoundedJson::new(json!("hidden-content"))?),
        );
        let hidden_revision = revision_with_interface(
            "hidden-owner",
            WorkflowInterface::new(
                [(
                    FieldId::new("hidden-value")?,
                    InterfaceField::required(item_schema()?),
                )],
                [],
            )?,
            vec![Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?],
            Vec::new(),
        )?;
        harness.put_revision(&hidden_revision)?;
        harness.command(
            &hidden_run,
            RunCommand::CreateRun {
                workflow: hidden_revision.semantic().workflow().clone(),
                revision: hidden_revision.id().clone(),
                root_scope: root,
                workspace_budget: generous_budget()?,
                inputs: vec![entry.clone()],
            },
        )?;
        let mut value = serde_json::to_value(policy("fresh", stopped, false)?)?;
        if !stopped {
            value["exclude_categories"] = json!(["successful_output"]);
        }
        let policy: TaskContextPolicy = serde_json::from_value(value)?;
        let policy = policy.with_exact_sources(
            BTreeSet::new(),
            BTreeSet::from([serde_json::to_string(entry.reference())?]),
            BTreeSet::new(),
        )?;
        let mut node = work(policy)?;
        if stopped {
            node = literal(node, "aaa-optional", json!("x".repeat(200)), false)?;
        }
        let revision = workflow(node)?;
        let run = RunId::new("hidden-omission")?;
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
        harness.runtime.scheduler_tick()?;
        let request = scheduled_request(&harness.runtime, &run)?;
        let saved = manifest(&harness.store, &request)?;
        let bytes = ContextManifestDocument::new(saved.clone()).to_canonical_json()?;
        let text = std::str::from_utf8(&bytes)?;
        for hidden in [
            "hidden-owner",
            "hidden-scope",
            "hidden-value",
            "hidden-content",
        ] {
            assert!(!text.contains(hidden), "{text}");
        }
        let omitted = saved
            .omissions()
            .iter()
            .find(|omission| {
                omission.kind == milkdrift_model::ContextSemanticKind::SuccessfulOutput
            })
            .ok_or("missing hidden omission")?;
        assert_eq!(
            (omitted.omitted_bytes, omitted.omitted_artifact_bytes),
            (0, 0)
        );
        assert!(omitted.source.is_none());
        assert_eq!(
            omitted.reason,
            if stopped {
                milkdrift_model::ContextOmissionReason::SelectionStopped
            } else {
                milkdrift_model::ContextOmissionReason::ExcludedCategory
            }
        );
    }
    Ok(())
}

// Seed a valid historical schedule through the production commit port. All scheduling
// facts come from a real runtime; saved selection/category facts use evidence an
// older writer could have emitted. No current history is edited or storage corrupted.
#[derive(Clone, Copy)]
enum LegacyCase {
    RequiredLoss,
    DisclosureLoss,
    SessionMismatch,
}

fn legacy_schedule(case: LegacyCase) -> TestResult<(Harness, RunId, InvocationRequest)> {
    use milkdrift_model::{
        ContextManifest, ContextOmission, ContextOmissionReason, ContextSemanticKind, ContextTotals,
    };
    use milkdrift_persistence::{LeaseIndexMutation, RunnableIndexMutation};
    let source = Harness::new("legacy-context")?;
    let mut target = Harness::new("legacy-context")?;
    let legacy_session = matches!(case, LegacyCase::SessionMismatch);
    let required_loss = matches!(case, LegacyCase::RequiredLoss);
    let node = if legacy_session {
        literal(
            work(policy("provider_managed", false, true)?)?,
            MODEL_TASK_INPUT_NAME,
            model_document(json!({"type":"fresh"}))?,
            true,
        )?
    } else {
        work(policy("fresh", false, true)?)?
    };
    let revision = workflow(node)?;
    let run = RunId::new("legacy-context")?;
    for harness in [&source, &target] {
        harness.put_revision(&revision)?;
        harness.create_and_start(&run, &revision)?;
    }
    let expected = target.store.head(&run)?;
    source.runtime.scheduler_tick()?;
    let request = scheduled_request(&source.runtime, &run)?;
    let current = manifest(&source.store, &request)?;
    let omitted = ContextOmission {
        source: if required_loss {
            None
        } else {
            Some(ContextSource::NodeExecution {
                node: NodeId::new("hidden")?,
                execution: NodeExecutionId::new("hidden-execution")?,
                attempt: None,
                event_sequence: None,
            })
        },
        kind: ContextSemanticKind::SuccessfulOutput,
        reason: ContextOmissionReason::SelectionStopped,
        required: required_loss,
        omitted_bytes: 0,
        omitted_artifact_bytes: 0,
    };
    let legacy = ContextManifest::new(
        current.run().clone(),
        current.revision().clone(),
        current.node().clone(),
        current.execution().clone(),
        current.attempt().clone(),
        1,
        current.policy_digest().clone(),
        current.entries().to_vec(),
        if legacy_session {
            Vec::new()
        } else {
            vec![omitted]
        },
        if legacy_session {
            current.totals()
        } else {
            ContextTotals::default()
        },
        current.budget(),
    )?;
    let reference = milkdrift_runtime::persist_context_manifest(
        target.store.as_ref(),
        &legacy,
        generous_budget()?,
        target.store.workspace_usage(&run)?,
    )?;
    let metadata = target
        .store
        .metadata(&ArtifactId::new(reference.identity())?)?
        .ok_or("missing legacy metadata")?;
    let request = InvocationRequest::new(
        request.invocation().clone(),
        request.capability().clone(),
        request.operation().clone(),
        request.provider_profile().cloned(),
        request.idempotency_key().cloned(),
        request.inputs().to_vec(),
        request.extensions().clone(),
    )?
    .with_context_materialization(reference, Vec::new())?;
    let history = source.runtime.history(&run)?;
    let legacy_snapshot = if legacy_session {
        let snapshot = history
            .iter()
            .find_map(|event| match event.kind() {
                RunEventKind::CapabilityResolved { snapshot, .. } => Some(snapshot),
                _ => None,
            })
            .ok_or("missing capability snapshot")?;
        let mut value = serde_json::to_value(snapshot)?;
        let object = value.as_object_mut().ok_or("snapshot is not an object")?;
        object.remove("category");
        object.remove("digest");
        object.insert("schema_version".to_owned(), json!(1));
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.resolved-capability-snapshot.v1\0");
        hasher.update(&serde_json::to_vec(&value)?);
        let object = value.as_object_mut().ok_or("snapshot is not an object")?;
        object.remove("schema_version");
        object.insert(
            "digest".to_owned(),
            json!(hasher.finalize().to_hex().to_string()),
        );
        // The production reader validates the old digest, not just this fixture's flags.
        Some(serde_json::from_value::<ResolvedCapabilitySnapshot>(value)?)
    } else {
        None
    };
    let events = history
        .into_iter()
        .filter(|event| event.sequence() > expected)
        .map(|event| {
            let mut kind = event.kind().clone();
            match &mut kind {
                RunEventKind::ArtifactPublished { metadata: saved } => *saved = metadata.clone(),
                RunEventKind::NodeScheduled { request: saved, .. } => *saved = request.clone(),
                RunEventKind::CapabilityResolved { snapshot, .. }
                | RunEventKind::CapabilityResolutionDecisionRecorded { snapshot, .. } => {
                    if let Some(legacy) = &legacy_snapshot {
                        *snapshot = legacy.clone();
                    }
                }
                _ => {}
            }
            if legacy_session
                && matches!(
                    kind,
                    RunEventKind::CapabilityResolved { .. }
                        | RunEventKind::CapabilityResolutionDecisionRecorded { .. }
                )
            {
                let mut value = serde_json::to_value(&event)?;
                value["kind"] = serde_json::to_value(kind)?;
                value["schema_version"] = json!(1);
                let object = value.as_object_mut().ok_or_else(|| {
                    milkdrift_persistence::PersistenceError::InvalidDocument(
                        "event is not an object".to_owned(),
                    )
                })?;
                object.remove("checksum");
                object.insert(
                    "domain".to_owned(),
                    json!("milkdrift.run-event-envelope.v1"),
                );
                let checksum =
                    milkdrift_persistence::IntegrityDigest::hash(&serde_json::to_vec(&value)?);
                let object = value.as_object_mut().ok_or_else(|| {
                    milkdrift_persistence::PersistenceError::InvalidDocument(
                        "event is not an object".to_owned(),
                    )
                })?;
                object.remove("domain");
                object.insert("checksum".to_owned(), serde_json::to_value(checksum)?);
                return RunEventEnvelope::from_json(&serde_json::to_vec(&value)?);
            }
            RunEventEnvelope::new(
                event.event_id().clone(),
                run.clone(),
                event.sequence(),
                event.occurred_at(),
                kind,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let summary = source.store.run_summary(&run)?.ok_or("missing summary")?;
    let leases = source
        .store
        .active_leases(PageSize::new(8)?)?
        .entries
        .into_iter()
        .map(|entry| LeaseIndexMutation::Upsert { entry })
        .collect();
    let command = CommandId::new("seed-legacy-context")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("controller:legacy")?,
        expected,
        TimestampMillis::new(NOW),
        br#"{"type":"seed_legacy_context"}"#.to_vec(),
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        summary.through_sequence,
        events
            .iter()
            .map(|event| event.event_id().clone())
            .collect(),
        BoundedJson::new(json!({"accepted":true}))?,
    )?;
    let usage = target.store.workspace_usage(&run)?;
    target.runtime = RuntimeService::new_with_authority(
        target.store.clone(),
        target.executor.clone(),
        test_authority(),
        target.clock.clone(),
        Arc::new(SequentialIdGenerator::new("legacy-continuation", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-legacy-context")?,
            ActorRef::new("controller:legacy-context")?,
            30_000,
            64,
            SchedulerLimits::new(64, 32, 16, 32)?,
            RetryPolicy::new(2, vec![ErrorClass::Provider], 1, 1000, 0)?,
        )?,
    )?;
    target.store.commit_command(&AtomicRunCommitRequest::new(
        receipt,
        events,
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: generous_budget()?,
            expected_usage: usage,
            resulting_usage: usage,
        }),
        vec![metadata.reference().clone()],
        Vec::new(),
        Some(target.store.active_leases(PageSize::new(8)?)?.revision),
        result,
        RunIndexUpdate::new(
            Some(summary),
            vec![RunnableIndexMutation::Remove {
                run: run.clone(),
                execution: current.execution().clone(),
            }],
            Vec::new(),
            leases,
        ),
    )?)?;
    Ok((target, run, request))
}

#[test]
fn restart_refuses_unsafe_legacy_selection_without_changing_saved_evidence() -> TestResult {
    for case in [LegacyCase::RequiredLoss, LegacyCase::DisclosureLoss] {
        let (harness, run, request) = legacy_schedule(case)?;
        let saved = ContextManifestDocument::new(manifest(&harness.store, &request)?)
            .to_canonical_json()?;
        let history = harness.runtime.history(&run)?;
        let directory = harness.close();
        let (store, _, executor, runtime) =
            open_closed_runtime_at(directory.path(), "legacy-context-reopen", NOW, 64)?;
        assert!(runtime.initialize_startup().is_err());
        assert!(!runtime.is_accepting_admission());
        assert_eq!(executor.entry_count(), 0);
        assert_eq!(history, runtime.history(&run)?);
        assert_eq!(
            saved,
            ContextManifestDocument::new(manifest(&store, &request)?).to_canonical_json()?
        );
    }
    Ok(())
}

#[test]
fn retry_refuses_unsafe_legacy_selection_without_rescanning_or_rewriting_it() -> TestResult {
    for case in [LegacyCase::RequiredLoss, LegacyCase::DisclosureLoss] {
        let (harness, run, request) = legacy_schedule(case)?;
        let saved = ContextManifestDocument::new(manifest(&harness.store, &request)?)
            .to_canonical_json()?;
        harness.executor.set_script(
            OperationId::new("model.generate")?,
            vec![InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    TerminalStatus::Failure,
                    Vec::new(),
                    Some(InvocationFailure::new(
                        ErrorClass::Provider,
                        true,
                        "retry",
                        "retryable fixture",
                        None,
                    )?),
                    None,
                    SideEffectClass::None,
                )?,
            }],
        )?;
        for action in harness.runtime.claim_execution_effects(PageSize::new(8)?)? {
            harness.runtime.execute_effect(action)?;
        }
        assert_eq!(harness.executor.entry_count(), 1);
        harness.clock.advance(2)?;
        assert!(harness.runtime.scheduler_tick().is_err());
        assert!(
            harness
                .runtime
                .claim_execution_effects(PageSize::new(8)?)?
                .is_empty()
        );
        assert_eq!(harness.executor.entry_count(), 1);
        assert_eq!(scheduled_request(&harness.runtime, &run)?, request);
        assert_eq!(
            saved,
            ContextManifestDocument::new(manifest(&harness.store, &request)?)
                .to_canonical_json()?
        );
    }
    Ok(())
}

#[test]
fn recovered_snapshot_without_category_cannot_bypass_model_session_agreement() -> TestResult {
    let (harness, run, request) = legacy_schedule(LegacyCase::SessionMismatch)?;
    let history = harness.runtime.history(&run)?;
    let directory = harness.close();
    let executor = Arc::new(DeterministicExecutor::new(test_descriptor()?));
    let (_store, _, runtime) = runtime_with_executor_at(
        directory.path(),
        "legacy-session-reopen",
        "legacy-context",
        NOW,
        64,
        executor.clone(),
    )?;
    assert!(
        runtime
            .claim_execution_effects(PageSize::new(8)?)?
            .is_empty()
    );
    assert_eq!(executor.entry_count(), 0);
    assert_eq!(scheduled_request(&runtime, &run)?, request);
    assert!(runtime.history(&run)?.starts_with(&history));
    assert!(runtime.history(&run)?.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Rejected,
            error_class: Some(ErrorClass::InvalidRequest),
            ..
        }
    )));
    assert!(
        !runtime
            .history(&run)?
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::NodeStarted { .. }))
    );
    Ok(())
}

#[test]
fn corrected_selection_and_model_session_survive_retry_after_store_reopen() -> TestResult {
    let harness = Harness::with_retry_policy(
        "context-retry-reopen",
        RetryPolicy::new(2, vec![ErrorClass::Provider], 1, 1000, 0)?,
    )?;
    let node = literal(
        work(policy("fresh", false, true)?)?,
        MODEL_TASK_INPUT_NAME,
        model_document(json!({"type":"fresh"}))?,
        true,
    )?;
    let revision = workflow(node)?;
    let run = RunId::new("context-retry-reopen")?;
    harness.put_revision(&revision)?;
    harness.create_and_start(&run, &revision)?;
    harness.executor.set_script(
        OperationId::new("model.generate")?,
        vec![InvocationEventKind::Terminal {
            terminal: InvocationTerminal::new(
                TerminalStatus::Failure,
                Vec::new(),
                Some(InvocationFailure::new(
                    ErrorClass::Provider,
                    true,
                    "retry",
                    "retryable fixture",
                    None,
                )?),
                None,
                SideEffectClass::None,
            )?,
        }],
    )?;
    runtime_tick(&harness.runtime)?;
    let first = scheduled_request(&harness.runtime, &run)?;
    let prior_manifest = manifest(&harness.store, &first)?;
    let history = harness.runtime.history(&run)?;
    let directory = harness.close();
    let executor = Arc::new(DeterministicExecutor::new(test_descriptor()?));
    let (store, _, runtime) = runtime_with_executor_at(
        directory.path(),
        "context-reopened",
        "context-retry-reopen",
        NOW + 2,
        64,
        executor.clone(),
    )?;
    runtime_tick(&runtime)?;
    assert_eq!(executor.entry_count(), 1);
    let requests = runtime
        .history(&run)?
        .into_iter()
        .filter_map(|event| match event.kind() {
            RunEventKind::NodeScheduled { request, .. } => Some(request.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    let retry_manifest = manifest(&store, &requests[1])?;
    assert_ne!(prior_manifest.attempt(), retry_manifest.attempt());
    assert_eq!(prior_manifest.entries(), retry_manifest.entries());
    assert_eq!(prior_manifest.omissions(), retry_manifest.omissions());
    assert_eq!(prior_manifest.policy_version(), 2);
    assert_eq!(prior_manifest, manifest(&store, &first)?);
    assert!(runtime.history(&run)?.starts_with(&history));
    Ok(())
}
