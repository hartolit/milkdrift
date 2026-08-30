//! Production-path causal context proof over real journal/workspace/artifact ports.

use super::*;
use milkdrift_blueprint::{
    ContextBudget, ContextCategory, ContextOrdering, ContextSemanticRole, ContextSessionPolicy,
    ContextTruncation, TaskConfig, TaskContextPolicy,
};
use milkdrift_persistence::{ArtifactReadAuthority, AttemptId, EvidenceId};
use milkdrift_runtime::read_context_manifest;

struct ContextProofExecutor {
    resolver: DeterministicExecutor,
    store: Arc<RedbStore>,
    outputs: BTreeMap<String, InvocationArtifactReference>,
    verification_calls: AtomicUsize,
    verification_requests: Mutex<Vec<(AttemptId, InvocationRequest)>>,
    reviewer_request: Mutex<Option<InvocationRequest>>,
}

impl ContextProofExecutor {
    fn events(
        &self,
        dispatch: &ExecutionDispatch,
        output: Option<(&str, InvocationArtifactReference)>,
        terminal: InvocationTerminal,
    ) -> Result<ExecutionReportBatch, ExecutorError> {
        let mut events = Vec::new();
        if let Some((name, reference)) = output {
            events.push(InvocationEvent::new(
                dispatch.request().invocation().clone(),
                1,
                InvocationEventKind::Output {
                    name: name.to_owned(),
                    reference,
                },
            )?);
        }
        events.push(InvocationEvent::new(
            dispatch.request().invocation().clone(),
            u64::try_from(events.len()).unwrap_or(0).saturating_add(1),
            InvocationEventKind::Terminal { terminal },
        )?);
        ExecutionReportBatch::new(dispatch.request(), events)
    }

    fn publish_review(
        &self,
        dispatch: &ExecutionDispatch,
    ) -> Result<InvocationArtifactReference, ExecutorError> {
        let manifest = dispatch
            .request()
            .context_manifest()
            .ok_or_else(|| ExecutorError::Boundary("reviewer manifest missing".to_owned()))?;
        let manifest = durable_artifact(manifest)
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        let bytes = b"review accepted frozen causal evidence";
        let reference = milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new(format!(
                "review-output:{}",
                dispatch.request().invocation().as_str()
            ))
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("text/plain")
                .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
            u64::try_from(bytes.len())
                .map_err(|_| ExecutorError::Boundary("review byte overflow".to_owned()))?,
        );
        let metadata = ArtifactMetadata::new(
            reference.clone(),
            ArtifactSensitivity::Restricted,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::Invocation {
                    invocation: dispatch.request().invocation().clone(),
                },
                vec![CausalReference::Artifact {
                    reference: manifest,
                }],
            )
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
        )
        .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        let publication = ArtifactPublicationId::new(format!(
            "review-publication:{}",
            dispatch.request().invocation().as_str()
        ))
        .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        let usage = self
            .store
            .workspace_usage(dispatch.run())
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        self.store
            .begin_publication(
                &BeginArtifactPublication::new(
                    publication.clone(),
                    dispatch.run().clone(),
                    metadata,
                    generous_budget()
                        .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
                    usage,
                )
                .map_err(|error| ExecutorError::Boundary(error.to_string()))?,
            )
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        self.store
            .write_chunk(&publication, 0, bytes)
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        self.store
            .commit_publication(&publication)
            .map_err(|error| ExecutorError::Boundary(error.to_string()))?;
        capability_artifact(&reference).map_err(|error| ExecutorError::Boundary(error.to_string()))
    }
}

impl TaskExecutor for ContextProofExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        self.resolver.resolve(requirement, observed_at_unix_ms)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        let node = dispatch.node().as_str();
        if node == "verify" {
            self.verification_requests
                .lock()
                .map_err(|_| ExecutorError::Boundary("verification capture poisoned".to_owned()))?
                .push((dispatch.attempt().clone(), dispatch.request().clone()));
            if self.verification_calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return self.events(dispatch, None, retryable_failure()?);
            }
        }
        if node == "review" {
            let reference = self.publish_review(dispatch)?;
            *self
                .reviewer_request
                .lock()
                .map_err(|_| ExecutorError::Boundary("review capture poisoned".to_owned()))? =
                Some(dispatch.request().clone());
            return self.events(
                dispatch,
                Some(("review", reference)),
                successful_executor_terminal()?,
            );
        }
        let output = self
            .outputs
            .get(node)
            .cloned()
            .map(|reference| ("evidence", reference));
        self.events(dispatch, output, successful_executor_terminal()?)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        self.resolver.cancel(request)
    }
}

#[test]
fn reviewer_receives_frozen_causal_evidence_without_private_sibling_transcript() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let run = RunId::new("run-context-production")?;
    let architecture = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("context-proof-architecture")?,
        "architecture-decision",
        b"ADR: use durable manifests",
    )?;
    let implementation = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("context-proof-implementation")?,
        "implementation-patch",
        b"diff --git a/context.rs b/context.rs",
    )?;
    let verification = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("context-proof-verification")?,
        "verification-log",
        b"verification passed after retry",
    )?;
    let private = publish_artifact_in_store(
        store.as_ref(),
        &RunId::new("context-proof-private")?,
        "private-exploration",
        b"unrelated private branch transcript",
    )?;
    let executor = Arc::new(ContextProofExecutor {
        resolver: DeterministicExecutor::new(test_descriptor()?),
        store: store.clone(),
        outputs: BTreeMap::from([
            ("architecture".to_owned(), architecture.clone()),
            ("implementation".to_owned(), implementation.clone()),
            ("verify".to_owned(), verification),
            ("explore".to_owned(), private.clone()),
        ]),
        verification_calls: AtomicUsize::new(0),
        verification_requests: Mutex::new(Vec::new()),
        reviewer_request: Mutex::new(None),
    });
    let clock = Arc::new(ManualClock::new(NOW));
    let runtime = RuntimeService::new_with_authority(
        store.clone(),
        executor.clone(),
        test_authority(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("context-proof", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-context-proof")?,
            ActorRef::new("controller:context-proof")?,
            30_000,
            64,
            SchedulerLimits::new(64, 32, 16, 32)?,
            RetryPolicy::new(2, vec![ErrorClass::Provider], 1, 1_000, 0)?,
        )?,
    )?;
    let revision = context_production_revision()?;
    store.put_revision(&revision)?;
    let command = |command| -> TestResult {
        let document = runtime.command(
            run.clone(),
            ActorRef::new("human:context-proof")?,
            store.head(&run)?,
            Reason::new("context production proof")?,
            Vec::new(),
            command,
        )?;
        runtime.handle_authorized_command(&document, &test_authority_claim()?)?;
        Ok(())
    };
    command(RunCommand::CreateRun {
        workflow: revision.semantic().workflow().clone(),
        revision: revision.id().clone(),
        root_scope: WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-context-proof")?),
        workspace_budget: generous_budget()?,
        inputs: Vec::new(),
    })?;
    command(RunCommand::StartRun)?;
    command(RunCommand::PauseRun)?;
    for segment in 0..6_usize {
        seed_irrelevant_recovery_history(store.as_ref(), &run, segment * 700, 700)?;
        command(RunCommand::ResumeRun)?;
        command(RunCommand::PauseRun)?;
    }
    command(RunCommand::ResumeRun)?;
    for _ in 0..32 {
        runtime.tick()?;
        clock.advance(2)?;
        if runtime.projection(&run)?.is_completed() {
            break;
        }
    }
    assert_eq!(
        runtime.projection(&run)?.lifecycle(),
        RunLifecycle::Terminal(RunOutcome::Succeeded)
    );
    let verification_requests = executor
        .verification_requests
        .lock()
        .map_err(|_| "verification capture poisoned")?
        .clone();
    assert_eq!(verification_requests.len(), 2);
    let verification_manifests = verification_requests
        .iter()
        .map(|(attempt, request)| {
            read_context_manifest(
                store.as_ref(),
                request
                    .context_manifest()
                    .ok_or("verification manifest missing")?,
                ArtifactReadAuthority::Authorized {
                    actor: ActorRef::new("human:context-proof")?,
                    evidence: EvidenceId::new(format!(
                        "context-proof-retry-read:{}",
                        attempt.as_str()
                    ))?,
                },
            )
            .map_err(Into::into)
        })
        .collect::<TestResult<Vec<_>>>()?;
    assert_ne!(
        verification_manifests[0].attempt(),
        verification_manifests[1].attempt()
    );
    assert_eq!(
        verification_manifests[0].entries(),
        verification_manifests[1].entries()
    );
    assert_eq!(
        verification_manifests[0].omissions(),
        verification_manifests[1].omissions()
    );
    assert_eq!(
        verification_manifests[0].totals(),
        verification_manifests[1].totals()
    );
    assert_eq!(
        verification_manifests[0].budget(),
        verification_manifests[1].budget()
    );
    let request = executor
        .reviewer_request
        .lock()
        .map_err(|_| "review capture poisoned")?
        .clone()
        .ok_or("reviewer was not invoked")?;
    let manifest_reference = request
        .context_manifest()
        .ok_or("review request has no manifest")?;
    assert!(
        store
            .metadata(&ArtifactId::new(manifest_reference.identity())?)?
            .is_some(),
        "manifest must be durable before executor entry"
    );
    let manifest = read_context_manifest(
        store.as_ref(),
        manifest_reference,
        ArtifactReadAuthority::Authorized {
            actor: ActorRef::new("human:context-proof")?,
            evidence: EvidenceId::new("context-proof-read")?,
        },
    )?;
    let digests = manifest
        .entries()
        .iter()
        .map(|entry| entry.content_digest())
        .collect::<BTreeSet<_>>();
    assert!(digests.contains(&ContentDigest::from_hex(architecture.digest())?));
    assert!(digests.contains(&ContentDigest::from_hex(implementation.digest())?));
    assert!(manifest.entries().iter().any(|entry| {
        entry
            .semantic_roles()
            .contains(&milkdrift_blueprint::ContextSemanticRole::Decision)
    }));
    assert!(manifest.entries().iter().any(|entry| {
        entry
            .semantic_roles()
            .contains(&milkdrift_blueprint::ContextSemanticRole::Implementation)
    }));
    assert!(
        !manifest
            .entries()
            .iter()
            .any(|entry| { entry.kind() == milkdrift_model::ContextSemanticKind::Failure }),
        "a branch-local retry failure crossed the join without being a declared join output"
    );
    assert!(!digests.contains(&ContentDigest::from_hex(private.digest())?));
    assert!(manifest.omissions().iter().any(|omission| {
        omission.reason == milkdrift_model::ContextOmissionReason::BranchIsolated
            && omission.source.is_none()
            && omission.omitted_bytes == 0
            && omission.omitted_artifact_bytes == 0
    }));
    let context_inputs = request
        .inputs()
        .iter()
        .filter(|input| {
            input
                .name()
                .starts_with(milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX)
        })
        .count();
    assert_eq!(
        context_inputs,
        manifest
            .entries()
            .iter()
            .filter(|entry| !matches!(
                entry.source(),
                milkdrift_model::ContextSource::DirectInput { .. }
            ))
            .count()
    );
    let mut cursor = None;
    let review_output = loop {
        let page = runtime.history_page(&EventPageQuery::new(
            run.clone(),
            cursor,
            PageSize::new(256)?,
        )?)?;
        if let Some(reference) = page.events.iter().find_map(|event| match event.kind() {
            RunEventKind::NodeOutputPublished {
                artifact: Some(reference),
                ..
            } if reference.artifact().as_str().starts_with("review-output:") => {
                Some(reference.clone())
            }
            _ => None,
        }) {
            break reference;
        }
        let Some(next) = page.next else {
            return Err("review output was not published".into());
        };
        cursor = Some(next);
    };
    let metadata = store
        .metadata(review_output.artifact())?
        .ok_or("review output metadata missing")?;
    assert!(metadata.provenance().causes().iter().any(|cause| {
        matches!(cause, CausalReference::Artifact { reference }
            if reference.digest().to_hex() == manifest_reference.digest())
    }));
    drop(runtime);
    drop(executor);
    drop(store);
    let reopened = RedbStore::open(directory.path())?;
    let reopened_manifest = read_context_manifest(
        &reopened,
        manifest_reference,
        ArtifactReadAuthority::Authorized {
            actor: ActorRef::new("human:context-proof")?,
            evidence: EvidenceId::new("context-proof-restart-read")?,
        },
    )?;
    assert_eq!(manifest, reopened_manifest);
    Ok(())
}

fn seed_irrelevant_recovery_history(
    store: &RedbStore,
    run: &RunId,
    first: usize,
    count: usize,
) -> TestResult {
    let mut created = 0_usize;
    let mut batch = 0_usize;
    while created < count {
        let expected = store.head(run)?;
        let mut sequence = expected;
        let batch_size = count.saturating_sub(created).min(500);
        let mut events = Vec::with_capacity(batch_size);
        for offset in 0..batch_size {
            let number = first.saturating_add(created).saturating_add(offset);
            sequence = sequence.next()?;
            events.push(RunEventEnvelope::new(
                EventId::new(format!("context-history-recovery-{number:05}"))?,
                run.clone(),
                sequence,
                TimestampMillis::new(NOW),
                RunEventKind::RecoveryStarted {
                    controller: WorkerId::new("worker-context-history")?,
                    through_sequence: milkdrift_persistence::RunSequence::new(
                        sequence.get().saturating_sub(1),
                    ),
                },
            )?);
        }
        let command = CommandId::new(format!("seed-context-history-{first:05}-{batch:03}"))?;
        let receipt = CommandReceipt::new(
            command.clone(),
            run.clone(),
            ActorRef::new("controller:context-history")?,
            expected,
            TimestampMillis::new(NOW),
            format!(r#"{{"batch":{batch},"schema_version":1,"type":"seed_context_history"}}"#)
                .into_bytes(),
        )?;
        let result = CommandResultDocument::new(
            command,
            run.clone(),
            receipt.fingerprint().clone(),
            CommandDisposition::Accepted,
            sequence,
            events
                .iter()
                .map(|event| event.event_id().clone())
                .collect(),
            BoundedJson::new(json!({"accepted": true}))?,
        )?;
        let summary = store
            .run_summary(run)?
            .ok_or("context run summary missing")?;
        let usage = store.workspace_usage(run)?;
        store.commit_command(&AtomicRunCommitRequest::new(
            receipt,
            events,
            Vec::new(),
            Some(WorkspaceAccounting {
                budget: generous_budget()?,
                expected_usage: usage,
                resulting_usage: usage,
            }),
            Vec::new(),
            Vec::new(),
            None,
            result,
            RunIndexUpdate::new(
                Some(RunSummaryIndex {
                    run: run.clone(),
                    workflow: summary.workflow,
                    revision: summary.revision,
                    state: summary.state,
                    through_sequence: sequence,
                    updated_at: TimestampMillis::new(NOW),
                }),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        )?)?;
        created = created.saturating_add(batch_size);
        batch = batch.saturating_add(1);
    }
    Ok(())
}

fn context_production_revision() -> TestResult<BlueprintRevision> {
    let schema = item_schema()?;
    let architecture = role_task(
        "architecture",
        BTreeSet::from([
            ContextSemanticRole::Decision,
            ContextSemanticRole::Requirement,
        ]),
        false,
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("evidence")?, DataPort::output(schema.clone()))?;
    let implementation = role_task(
        "implementation",
        BTreeSet::from([ContextSemanticRole::Implementation]),
        true,
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("evidence")?, DataPort::output(schema.clone()))?;
    let fork = Node::new(
        NodeId::new("fork")?,
        NodeKind::Fork {
            config: ForkConfig::new(BTreeSet::from([
                PortId::new("verify")?,
                PortId::new("explore")?,
            ]))?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("verify")?)?
    .with_control_output(PortId::new("explore")?)?;
    let verify = role_task(
        "verify",
        BTreeSet::from([ContextSemanticRole::Verification]),
        true,
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("evidence")?, DataPort::output(schema.clone()))?;
    let explore = role_task("explore", BTreeSet::new(), true)?
        .with_control_output(PortId::new("out")?)?
        .with_data_output(PortId::new("evidence")?, DataPort::output(schema.clone()))?;
    let join = Node::new(
        NodeId::new("join")?,
        NodeKind::Join {
            config: JoinConfig::new(NodeId::new("fork")?, JoinPolicy::All),
        },
    )?
    .with_control_input(PortId::new("verify-in")?)?
    .with_control_input(PortId::new("explore-in")?)?
    .with_control_output(PortId::new("out")?)?;
    let review_policy = TaskContextPolicy::new(
        false,
        None,
        BTreeSet::new(),
        BTreeSet::from([
            ContextSemanticRole::Decision,
            ContextSemanticRole::Implementation,
            ContextSemanticRole::FailureEvidence,
        ]),
        BTreeSet::new(),
        BTreeSet::from([
            ContextCategory::RawProgress,
            ContextCategory::ToolTrace,
            ContextCategory::VerboseCommandOutput,
            ContextCategory::PriorPrompt,
        ]),
        None,
        ContextBudget::default(),
        ContextOrdering::CausalKindSource,
        ContextTruncation::OmitOversized,
        ContextSessionPolicy::Fresh,
        true,
    )?;
    let review = Node::new(
        NodeId::new("review")?,
        NodeKind::Task {
            config: TaskConfig::new(
                CapabilityRequirement::new(OperationId::new("model.generate")?),
                review_policy,
            )?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("review")?, DataPort::output(schema))?;
    revision(
        "workflow-context-production",
        vec![
            architecture,
            implementation,
            fork,
            verify,
            explore,
            join,
            review,
            terminal("done", TerminalOutcome::Success)?,
        ],
        vec![
            control_edge(
                "architecture-implementation",
                "architecture",
                "out",
                "implementation",
                "in",
            )?,
            control_edge("implementation-fork", "implementation", "out", "fork", "in")?,
            control_edge("fork-verify", "fork", "verify", "verify", "in")?,
            control_edge("fork-explore", "fork", "explore", "explore", "in")?,
            control_edge("verify-join", "verify", "out", "join", "verify-in")?,
            control_edge("explore-join", "explore", "out", "join", "explore-in")?,
            control_edge("join-review", "join", "out", "review", "in")?,
            control_edge("review-done", "review", "out", "done", "in")?,
        ],
    )
}

fn role_task(
    id: &str,
    roles: BTreeSet<ContextSemanticRole>,
    control_input: bool,
) -> TestResult<Node> {
    let config = TaskConfig::new(
        CapabilityRequirement::new(OperationId::new("model.generate")?),
        TaskContextPolicy::default(),
    )?
    .with_output_context_roles(roles)?;
    let node = Node::new(NodeId::new(id)?, NodeKind::Task { config })?;
    if control_input {
        Ok(node.with_control_input(PortId::new("in")?)?)
    } else {
        Ok(node)
    }
}

fn retryable_failure() -> Result<InvocationTerminal, ExecutorError> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Failure,
        Vec::new(),
        Some(InvocationFailure::new(
            ErrorClass::Provider,
            true,
            "verification_failed",
            "verification failed against the implementation",
            None,
        )?),
        None,
        SideEffectClass::None,
    )?)
}

fn successful_executor_terminal() -> Result<InvocationTerminal, ExecutorError> {
    Ok(InvocationTerminal::new(
        TerminalStatus::Success,
        Vec::new(),
        None,
        None,
        SideEffectClass::None,
    )?)
}

fn durable_artifact(
    reference: &InvocationArtifactReference,
) -> Result<milkdrift_workspace::ArtifactReference, Box<dyn std::error::Error>> {
    Ok(milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(reference.identity())?,
        ContentDigest::from_hex(reference.digest())?,
        MediaType::new(
            reference
                .media_type()
                .ok_or("artifact media type missing")?,
        )?,
        reference.size_bytes().ok_or("artifact size missing")?,
    ))
}

fn capability_artifact(
    reference: &milkdrift_workspace::ArtifactReference,
) -> Result<InvocationArtifactReference, Box<dyn std::error::Error>> {
    Ok(InvocationArtifactReference::new(
        reference.artifact().as_str(),
        reference.digest().to_hex(),
        Some(reference.media_type().as_str().to_owned()),
        Some(reference.size_bytes()),
    )?)
}
