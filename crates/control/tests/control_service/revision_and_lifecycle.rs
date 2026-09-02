use super::*;

#[test]
fn controller_checkpoint_survives_restart_and_duplicate_approval() -> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("controller-checkpoint.redb");
    let run = RunId::new("run-controller-checkpoint")?;
    let actor = ActorRef::new("controller:checkpointed")?;
    let grant_id = GrantId::new("grant:controller-checkpointed")?;
    let controller_execution;

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context) =
            services(store.clone(), &actor, &run, &grant_id, "checkpoint-before")?;
        let body = BlueprintRevision::genesis(
            WorkflowId::new("checkpoint-body")?,
            MutationBatch::new(vec![Mutation::AddNode {
                node: Node::new(
                    NodeId::new("cycle-complete")?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Success,
                    },
                )?,
            }])?,
            AuthorRef::new("human:controller-test")?,
            "checkpoint body",
        )?;
        store.put_revision(&body)?;
        let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
            workflow: WorkflowId::new("checkpoint-wrapper")?,
            body: PinnedSubworkflow::new(
                body.semantic().workflow().clone(),
                body.id().clone(),
                WorkflowInterface::new([], [])?,
            ),
            continue_condition: Condition::Constant { value: true },
            limits: ControllerLimits::new(
                4,
                4,
                8,
                4,
                60_000,
                1_000_000,
                10_000,
                10_000,
                1_000_000,
                4,
                4,
                2,
                2,
                2,
                2,
                Some(2),
            )?,
            author: AuthorRef::new("human:controller-test")?,
        })?;
        store.put_revision(&wrapper)?;
        create_and_start(&service, &runtime, &context, &run, &wrapper)?;
        for _ in 0..48 {
            runtime.tick()?;
            let projection = runtime.projection(&run)?;
            if projection
                .repeat_continuations()
                .values()
                .any(|value| value.is_pending_approval())
            {
                break;
            }
        }
        let projection = runtime.projection(&run)?;
        controller_execution = projection
            .repeat_continuations()
            .iter()
            .find(|(_, value)| value.is_pending_approval())
            .map(|(execution, _)| execution.clone())
            .ok_or("controller did not reach the exact durable checkpoint")?;
        let status = service.execute(&command(
            "inspect-controller-checkpoint",
            &context,
            OptimisticGuard::default(),
            ControlCommand::InspectController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
            },
        )?)?;
        assert!(matches!(
            status,
            ControlResult::ControllerStatus { value }
                if value.state == milkdrift_control::ControllerLifecycleState::AwaitingHumanCheckpoint
                    && value.progress.invocations == 2
                    && value.checkpoint_id.is_some()
        ));
    }

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, revoked_context) = services_with_grant_and_revocations(
            store,
            &actor,
            &grant_id,
            "checkpoint-revoked",
            grant(&actor, &run, &grant_id)?,
            BTreeMap::from([(grant_id.clone(), 1)]),
        )?;
        let before = runtime.projection(&run)?;
        let denied = service.execute(&command(
            "continue-controller-revoked",
            &revoked_context,
            OptimisticGuard {
                expected_run_sequence: Some(before.sequence()),
                expected_revision: before.revision().cloned(),
                expected_proposal_digest: None,
            },
            ControlCommand::ContinueController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
                decision: RepeatDecisionId::new("decision-controller-revoked")?,
            },
        )?);
        assert!(matches!(
            denied,
            Err(ControlError::AuthorizationDenied { .. })
        ));
        let after = runtime.projection(&run)?;
        assert_eq!(after.sequence(), before.sequence());
        assert!(
            after
                .repeat_continuations()
                .get(&controller_execution)
                .is_some_and(|value| value.is_pending_approval())
        );
    }

    let store = Arc::new(RedbStore::open(&database)?);
    let (runtime, service, context) = services(store, &actor, &run, &grant_id, "checkpoint-after")?;
    let restarted = runtime.projection(&run)?;
    assert!(
        restarted
            .repeat_continuations()
            .get(&controller_execution)
            .is_some_and(|value| value.is_pending_approval())
    );
    let continuation = command(
        "continue-controller-checkpoint",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(restarted.sequence()),
            expected_revision: restarted.revision().cloned(),
            expected_proposal_digest: None,
        },
        ControlCommand::ContinueController {
            run: run.clone(),
            controller_execution: controller_execution.clone(),
            decision: RepeatDecisionId::new("decision-controller-checkpoint")?,
        },
    )?;
    let first = service.execute(&continuation)?;
    let second = service.execute(&continuation)?;
    assert_eq!(first, second);
    assert!(
        runtime
            .projection(&run)?
            .repeat_continuations()
            .get(&controller_execution)
            .is_some_and(|value| !value.is_pending_approval())
    );
    Ok(())
}

#[test]
fn controller_oversized_proposal_is_rejected_before_revision_persistence() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("controller-proposal.redb"),
    )?);
    let run = RunId::new("run-controller-proposal")?;
    let actor = ActorRef::new("controller:proposal")?;
    let grant_id = GrantId::new("grant:controller-proposal")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &grant_id,
        "controller-proposal",
    )?;
    let body = BlueprintRevision::genesis(
        WorkflowId::new("controller-proposal-body")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("cycle-complete")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:controller-test")?,
        "controller proposal body",
    )?;
    store.put_revision(&body)?;
    let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("controller-proposal-wrapper")?,
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            8, 4, 2, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 8, 8, 3, 3, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-test")?,
    })?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    runtime.tick()?;
    let observed = runtime.projection(&run)?.sequence();
    let inserted = task_node("proposal-extra", "tool.publish")?;
    let mutation = MutationBatch::new(vec![
        Mutation::RemoveEdge {
            edge: EdgeId::new("controller-finished")?,
        },
        Mutation::AddNode {
            node: inserted.clone(),
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("controller-extra")?,
                EdgeKind::Control,
                NodeId::new("controller-repeat")?,
                PortId::new("out")?,
                inserted.id().clone(),
                PortId::new("in")?,
            ),
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("extra-complete")?,
                EdgeKind::Control,
                inserted.id().clone(),
                PortId::new("out")?,
                NodeId::new("controller-complete")?,
                PortId::new("in")?,
            ),
        },
    ])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-controller-oversized")?,
        actor.clone(),
        ProposalProvenance::Direct,
        wrapper.semantic().workflow().clone(),
        Some(run.clone()),
        wrapper.id().clone(),
        wrapper.content_digest().clone(),
        Some(observed),
        mutation.clone(),
        "oversized controller proposal",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let expected = wrapper.revise(
        wrapper.id(),
        mutation,
        AuthorRef::new(format!("proposal:{}", &proposal.digest().as_str()[3..35]))?,
        format!(
            "proposal_id={};proposal_digest={};proposer={};source=direct",
            proposal.identity(),
            proposal.digest(),
            proposal.proposer()
        ),
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    let result = service.execute(&command(
        "submit-controller-oversized",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(wrapper.id().clone()),
            expected_proposal_digest: Some(document.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: document },
    )?);
    assert!(matches!(
        result,
        Err(ControlError::Bounds { location, .. })
            if location == "controller.proposal.mutations_per_proposal"
    ));
    assert!(store.revision(expected.id())?.is_none());
    assert_eq!(runtime.projection(&run)?.sequence(), observed);

    let wider = build_controller_blueprint(ControllerBlueprintSpec {
        workflow: wrapper.semantic().workflow().clone(),
        body: PinnedSubworkflow::new(
            body.semantic().workflow().clone(),
            body.id().clone(),
            WorkflowInterface::new([], [])?,
        ),
        continue_condition: Condition::Constant { value: true },
        limits: ControllerLimits::new(
            9, 4, 2, 4, 60_000, 1_000_000, 10_000, 10_000, 1_000_000, 9, 9, 3, 3, 2, 2, None,
        )?,
        author: AuthorRef::new("human:controller-test")?,
    })?;
    let mutation = MutationBatch::new(vec![
        Mutation::SetMetadata {
            metadata: wider.semantic().metadata().clone(),
        },
        Mutation::ReplaceNode {
            node: wider
                .semantic()
                .nodes()
                .get(&NodeId::new("controller-repeat")?)
                .cloned()
                .ok_or("wider controller repeat is absent")?,
        },
    ])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-controller-self-widen")?,
        actor,
        ProposalProvenance::Direct,
        wrapper.semantic().workflow().clone(),
        Some(run.clone()),
        wrapper.id().clone(),
        wrapper.content_digest().clone(),
        Some(observed),
        mutation.clone(),
        "controller attempts to widen its own policy",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let expected = wrapper.revise(
        wrapper.id(),
        mutation,
        AuthorRef::new(format!("proposal:{}", &proposal.digest().as_str()[3..35]))?,
        format!(
            "proposal_id={};proposal_digest={};proposer={};source=direct",
            proposal.identity(),
            proposal.digest(),
            proposal.proposer()
        ),
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    let result = service.execute(&command(
        "submit-controller-self-widen",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(wrapper.id().clone()),
            expected_proposal_digest: Some(document.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: document },
    )?);
    assert!(matches!(result, Err(ControlError::ForbiddenProposal)));
    assert!(store.revision(expected.id())?.is_none());
    Ok(())
}

#[test]
#[ignore = "manual release-mode controller longevity and restart proof"]
fn release_controller_longevity_stops_once_across_checkpoints_and_restart() -> TestResult {
    let directory = TempDir::new()?;
    let database = directory.path().join("controller-longevity.redb");
    let run = RunId::new("run-controller-longevity")?;
    let actor = ActorRef::new("controller:longevity")?;
    let grant_id = GrantId::new("grant:controller-longevity")?;
    let controller_execution;

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context) =
            services(store.clone(), &actor, &run, &grant_id, "longevity-before")?;
        let body = BlueprintRevision::genesis(
            WorkflowId::new("longevity-body")?,
            MutationBatch::new(vec![Mutation::AddNode {
                node: Node::new(
                    NodeId::new("cycle-complete")?,
                    NodeKind::Terminal {
                        outcome: TerminalOutcome::Success,
                    },
                )?,
            }])?,
            AuthorRef::new("human:controller-test")?,
            "longevity body",
        )?;
        store.put_revision(&body)?;
        let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
            workflow: WorkflowId::new("longevity-wrapper")?,
            body: PinnedSubworkflow::new(
                body.semantic().workflow().clone(),
                body.id().clone(),
                WorkflowInterface::new([], [])?,
            ),
            continue_condition: Condition::Constant { value: true },
            limits: ControllerLimits::new(
                9,
                4,
                8,
                4,
                60_000,
                1_000_000,
                10_000,
                10_000,
                1_000_000,
                9,
                9,
                3,
                3,
                2,
                2,
                Some(3),
            )?,
            author: AuthorRef::new("human:controller-test")?,
        })?;
        store.put_revision(&wrapper)?;
        create_and_start(&service, &runtime, &context, &run, &wrapper)?;
        for _ in 0..128 {
            runtime.tick()?;
            if runtime
                .projection(&run)?
                .repeat_continuations()
                .values()
                .any(|value| value.is_pending_approval())
            {
                break;
            }
        }
        let checkpoint = runtime.projection(&run)?;
        controller_execution = checkpoint
            .repeat_continuations()
            .iter()
            .find(|(_, value)| value.is_pending_approval())
            .map(|(execution, _)| execution.clone())
            .ok_or("controller did not reach the first exact checkpoint")?;
        service.execute(&command(
            "longevity-continue-three",
            &context,
            OptimisticGuard {
                expected_run_sequence: Some(checkpoint.sequence()),
                expected_revision: checkpoint.revision().cloned(),
                expected_proposal_digest: None,
            },
            ControlCommand::ContinueController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
                decision: RepeatDecisionId::new("longevity-decision-three")?,
            },
        )?)?;
        for _ in 0..128 {
            runtime.tick()?;
            let projection = runtime.projection(&run)?;
            if projection
                .repeat_continuations()
                .get(&controller_execution)
                .is_some_and(|value| value.is_pending_approval())
            {
                break;
            }
        }
        let checkpoint = runtime.projection(&run)?;
        let status = service.execute(&command(
            "longevity-inspect-six",
            &context,
            OptimisticGuard::default(),
            ControlCommand::InspectController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
            },
        )?)?;
        assert!(matches!(
            status,
            ControlResult::ControllerStatus { value }
                if value.progress.invocations == 6
                    && value.state == milkdrift_control::ControllerLifecycleState::AwaitingHumanCheckpoint
        ));
        assert!(
            checkpoint
                .repeat_continuations()
                .get(&controller_execution)
                .is_some_and(|value| value.is_pending_approval())
        );
    }

    {
        let store = Arc::new(RedbStore::open(&database)?);
        let (runtime, service, context) =
            services(store, &actor, &run, &grant_id, "longevity-after")?;
        let checkpoint = runtime.projection(&run)?;
        service.execute(&command(
            "longevity-continue-six",
            &context,
            OptimisticGuard {
                expected_run_sequence: Some(checkpoint.sequence()),
                expected_revision: checkpoint.revision().cloned(),
                expected_proposal_digest: None,
            },
            ControlCommand::ContinueController {
                run: run.clone(),
                controller_execution: controller_execution.clone(),
                decision: RepeatDecisionId::new("longevity-decision-six")?,
            },
        )?)?;
        for _ in 0..512 {
            runtime.tick()?;
        }
        let projection = runtime.projection(&run)?;
        assert_eq!(
            projection.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Failed)
        );
        assert_eq!(
            runtime
                .history(&run)?
                .iter()
                .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
                .count(),
            9
        );
    }

    let store = Arc::new(RedbStore::open(&database)?);
    let (runtime, _service, _context) =
        services(store, &actor, &run, &grant_id, "longevity-terminal")?;
    let before = runtime.projection(&run)?.sequence();
    for _ in 0..512 {
        runtime.tick()?;
    }
    assert_eq!(runtime.projection(&run)?.sequence(), before);
    assert_eq!(
        runtime
            .history(&run)?
            .iter()
            .filter(|event| matches!(event.kind(), RunEventKind::RepeatIterationCreated { .. }))
            .count(),
        9
    );
    Ok(())
}

pub(super) fn create_and_start(
    service: &ControlService,
    runtime: &RuntimeService,
    context: &ActorAuthorityContext,
    run: &RunId,
    base: &BlueprintRevision,
) -> TestResult {
    service
        .execute(&command(
            "control-create-run",
            context,
            OptimisticGuard {
                expected_run_sequence: Some(RunSequence::ZERO),
                expected_revision: Some(base.id().clone()),
                expected_proposal_digest: None,
            },
            ControlCommand::CreateRun {
                run: run.clone(),
                workflow: base.semantic().workflow().clone(),
                revision: base.id().clone(),
                root_scope: WorkspaceScope::run_root(
                    run.clone(),
                    ScopeId::new("scope-control-run")?,
                ),
                workspace_budget: WorkspaceBudget::new(
                    128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216,
                )?,
                inputs: Vec::new(),
            },
        )?)
        .map_err(|error| format!("create run through control service: {error}"))?;
    let created = runtime.projection(run)?;
    service
        .execute(&command(
            "control-start-run",
            context,
            OptimisticGuard {
                expected_run_sequence: Some(created.sequence()),
                expected_revision: Some(base.id().clone()),
                expected_proposal_digest: None,
            },
            ControlCommand::StartRun { run: run.clone() },
        )?)
        .map_err(|error| format!("start run through control service: {error}"))?;
    Ok(())
}

fn reviewer_proposal(
    actor: &ActorRef,
    run: &RunId,
    base: &BlueprintRevision,
    sequence: RunSequence,
) -> TestResult<WorkflowProposalDocument> {
    let reviewer = task_node("review", "model.generate")?;
    let context_manifest = ArtifactReference::new(
        "artifact:controller-context",
        "a".repeat(64),
        Some("application/vnd.milkdrift.context-manifest.v2+json".to_owned()),
        Some(512),
    )?;
    let response_artifact = ArtifactReference::new(
        "artifact:controller-response",
        "b".repeat(64),
        Some("application/vnd.milkdrift.model-response.v1+json".to_owned()),
        Some(1_024),
    )?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-insert-reviewer")?,
        actor.clone(),
        ProposalProvenance::Model {
            capability: CapabilityId::new("model-controller")?,
            invocation: InvocationId::new("invocation-controller-review")?,
            model_profile: ProviderProfileRef::new("profile-controller-reviewed")?,
            context_manifest: context_manifest.clone(),
            response_artifact: response_artifact.clone(),
        },
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(sequence),
        MutationBatch::new(vec![
            Mutation::RemoveEdge {
                edge: EdgeId::new("work-done")?,
            },
            Mutation::AddNode { node: reviewer },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("work-review")?,
                    EdgeKind::Control,
                    NodeId::new("work")?,
                    PortId::new("out")?,
                    NodeId::new("review")?,
                    PortId::new("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("review-done")?,
                    EdgeKind::Control,
                    NodeId::new("review")?,
                    PortId::new("out")?,
                    NodeId::new("done")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        "insert a read-only reviewer before a not-yet-started successor",
        None,
        vec!["model producer calls this low risk".to_owned()],
        vec!["the successor has not started".to_owned()],
        Vec::new(),
        vec![context_manifest, response_artifact],
        ProposalApplicationPolicy::AutoApplyLowRisk,
        Some(RequestedRunAction::Pause),
        ClaimedStopCondition::Continue,
    )?;
    Ok(WorkflowProposalDocument::new(proposal))
}

#[test]
fn low_risk_live_proposal_applies_pauses_replays_and_survives_restart() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:audited-controller")?;
    let run = RunId::new("run-control-low-risk")?;
    let base = base_revision("workflow-control-low-risk")?;
    let grant_id = GrantId::new("grant:control-low-risk")?;
    store.put_revision(&base)?;
    let (runtime, service, context) =
        services(store.clone(), &actor, &run, &grant_id, "control-before")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let before = runtime.projection(&run)?;
    assert_eq!(before.lifecycle(), RunLifecycle::Running);
    let proposal = reviewer_proposal(&actor, &run, &base, before.sequence())?;
    let proposal_guard = OptimisticGuard {
        expected_run_sequence: Some(before.sequence()),
        expected_revision: Some(base.id().clone()),
        expected_proposal_digest: Some(proposal.proposal().digest().clone()),
    };
    let submit = command(
        "control-submit-reviewer",
        &context,
        proposal_guard,
        ControlCommand::SubmitProposal {
            proposal: proposal.clone(),
        },
    )?;
    let first = service.execute(&submit)?;
    let (proposed_revision, risk) = match &first {
        ControlResult::ProposalSubmitted { value } => {
            assert!(value.applied);
            (value.proposed_revision.clone(), value.classification.risk)
        }
        _ => return Err("unexpected proposal result".into()),
    };
    assert_eq!(risk, RiskClass::Low);
    let proposed = store
        .revision(&proposed_revision)?
        .ok_or("proposed revision was not stored")?;
    assert!(proposed.reason().contains(
        "model:model-controller:invocation-controller-review:profile-controller-reviewed"
    ));
    assert!(proposed.reason().contains(&"a".repeat(64)));
    assert!(proposed.reason().contains(&"b".repeat(64)));
    let applied = runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(&proposed_revision));
    assert_eq!(applied.lifecycle(), RunLifecycle::Paused);
    assert_eq!(service.execute(&submit)?, first);

    let resume = command(
        "control-resume-after-reviewer",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(applied.sequence()),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::ResumeRun { run: run.clone() },
    )?;
    service.execute(&resume)?;
    let resumed_sequence = runtime.projection(&run)?.sequence();
    drop(service);
    drop(runtime);
    drop(store);

    let reopened_store = Arc::new(RedbStore::open(directory.path())?);
    let (reopened_runtime, reopened_service, reopened_context) =
        services(reopened_store, &actor, &run, &grant_id, "control-after")?;
    let inspection = reopened_service.execute(&command(
        "control-inspect-after-restart",
        &reopened_context,
        OptimisticGuard {
            expected_run_sequence: Some(resumed_sequence),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::InspectRun { run: run.clone() },
    )?)?;
    match inspection {
        ControlResult::RunInspection { value } => {
            assert_eq!(value.revision, Some(proposed_revision));
            assert_eq!(value.lifecycle, RunLifecycle::Running);
        }
        _ => return Err("unexpected inspection result".into()),
    }
    assert_eq!(
        reopened_runtime.projection(&run)?.sequence(),
        resumed_sequence
    );
    Ok(())
}

#[test]
fn terminal_change_requires_recorded_approval_before_apply() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("human:workflow-supervisor")?;
    let run = RunId::new("run-control-approval")?;
    let base = base_revision("workflow-control-approval")?;
    let grant_id = GrantId::new("grant:control-approval")?;
    store.put_revision(&base)?;
    let (runtime, service, context) = services(store, &actor, &run, &grant_id, "control-approval")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let boundary = runtime.projection(&run)?.sequence();
    let replacement = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Failure,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let proposal = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("proposal-terminal-change")?,
        actor.clone(),
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(boundary),
        MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?,
        "change a terminal condition with explicit human approval",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::AutoApplyLowRisk,
        None,
        ClaimedStopCondition::Complete,
    )?);
    let submitted = service.execute(&command(
        "control-submit-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(boundary),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal {
            proposal: proposal.clone(),
        },
    )?)?;
    let proposed_revision = match submitted {
        ControlResult::ProposalSubmitted { value } => {
            assert_eq!(value.classification.risk, RiskClass::ApprovalRequired);
            assert!(!value.applied);
            value.proposed_revision
        }
        _ => return Err("unexpected submit result".into()),
    };
    assert_eq!(runtime.projection(&run)?.revision(), Some(base.id()));

    let decision_boundary = runtime.projection(&run)?.sequence();
    service.execute(&command(
        "control-approve-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(decision_boundary),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::ApproveProposal {
            run: run.clone(),
            proposal: proposal.proposal().identity().clone(),
            proposal_digest: proposal.proposal().digest().clone(),
            proposed_revision: proposed_revision.clone(),
            decision: ReconciliationDecisionId::new("decision-terminal-change")?,
        },
    )?)?;
    let apply_boundary = runtime.projection(&run)?.sequence();
    service.execute(&command(
        "control-apply-terminal-change",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(apply_boundary),
            expected_revision: Some(proposed_revision.clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::ApplyProposal {
            run: run.clone(),
            proposal: proposal.proposal().identity().clone(),
            proposal_digest: proposal.proposal().digest().clone(),
            proposed_revision: proposed_revision.clone(),
        },
    )?)?;
    let applied = runtime.projection(&run)?;
    assert_eq!(applied.revision(), Some(&proposed_revision));
    assert_eq!(applied.run_actor_revision_requests(), 1);
    assert_eq!(applied.run_actor_rejections(), 0);
    Ok(())
}

#[test]
fn stale_or_invalid_proposal_changes_no_run_state() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:stale-proposer")?;
    let run = RunId::new("run-control-stale")?;
    let base = base_revision("workflow-control-stale")?;
    let grant_id = GrantId::new("grant:control-stale")?;
    store.put_revision(&base)?;
    let (runtime, service, context) = services(store, &actor, &run, &grant_id, "control-stale")?;
    create_and_start(&service, &runtime, &context, &run, &base)?;
    let observed = runtime.projection(&run)?.sequence();
    let proposal = reviewer_proposal(&actor, &run, &base, observed)?;
    service.execute(&command(
        "control-pause-before-stale-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: None,
        },
        ControlCommand::PauseRun { run: run.clone() },
    )?)?;
    let after_pause = runtime.projection(&run)?.sequence();
    let stale = service.execute(&command(
        "control-stale-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(observed),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(proposal.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal },
    )?);
    assert!(matches!(stale, Err(ControlError::StaleRunSequence { .. })));
    let current = runtime.projection(&run)?;
    assert_eq!(current.sequence(), after_pause);
    assert_eq!(current.revision(), Some(base.id()));

    let invalid = WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("proposal-invalid-mutation")?,
        actor,
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(after_pause),
        MutationBatch::new(vec![Mutation::RemoveNode {
            node: NodeId::new("absent-node")?,
        }])?,
        "invalid closed mutation",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?);
    let rejected = service.execute(&command(
        "control-invalid-submit",
        &context,
        OptimisticGuard {
            expected_run_sequence: Some(after_pause),
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(invalid.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: invalid },
    )?);
    assert!(matches!(rejected, Err(ControlError::Blueprint(_))));
    assert_eq!(runtime.projection(&run)?.sequence(), after_pause);
    Ok(())
}

#[test]
fn unauthorized_provider_expansion_stores_no_revision() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:provider-confined")?;
    let workflow = WorkflowId::new("workflow-control-provider-confined")?;
    let base = base_revision(workflow.as_str())?;
    let grant_id = GrantId::new("grant:control-provider-confined")?;
    store.put_revision(&base)?;
    let allowed_profile = ProviderProfileRef::new("profile-allowed")?;
    let grant = AuthorityPreset::Controller
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Workflow {
                workflow: workflow.clone(),
            },
            CapabilityAuthorityScopeBuilder::new(SideEffectClass::ReadOnly)
                .only_operations(BTreeSet::from([OperationId::new("model.generate")?]))?
                .only_provider_profiles(BTreeSet::from([allowed_profile]))?
                .build(),
            AuthorityBudget {
                invocations: Some(16),
                ..AuthorityBudget::default()
            },
        )
        .build()?;
    let (_runtime, service, context) = services_with_grant(
        store.clone(),
        &actor,
        &grant_id,
        "control-provider-confined",
        grant,
    )?;
    let replacement = Node::new(
        NodeId::new("work")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("model.generate")?)
                .provider_profile(ProviderProfileRef::new("profile-forbidden")?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_output(PortId::new("out")?)?;
    let mutation = MutationBatch::new(vec![Mutation::ReplaceNode { node: replacement }])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-provider-expansion")?,
        actor.clone(),
        ProposalProvenance::Direct,
        workflow.clone(),
        None,
        base.id().clone(),
        base.content_digest().clone(),
        None,
        mutation.clone(),
        "attempt to select an unauthorized provider profile",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let expected = base.revise(
        base.id(),
        mutation,
        AuthorRef::new(format!("proposal:{}", &proposal.digest().as_str()[3..35]))?,
        format!(
            "proposal_id={};proposal_digest={};proposer={};source=direct",
            proposal.identity(),
            proposal.digest(),
            proposal.proposer()
        ),
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    let result = service.execute(&command(
        "control-provider-expansion",
        &context,
        OptimisticGuard {
            expected_run_sequence: None,
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(document.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: document },
    )?);
    assert!(matches!(
        result,
        Err(ControlError::AuthorizationDenied { .. })
    ));
    assert!(store.revision(expected.id())?.is_none());

    let item_schema = SchemaRef::new(SchemaId::new("control.reducer.item")?, 1)?;
    let producer = Node::new(
        NodeId::new("work")?,
        NodeKind::task_direct_inputs(
            CapabilityRequirement::new(OperationId::new("model.generate")?)
                .maximum_side_effect(SideEffectClass::ReadOnly),
        )?,
    )?
    .with_control_output(PortId::new("out")?)?
    .with_data_output(PortId::new("item")?, DataPort::output(item_schema.clone()))?;
    let reducer = Node::new(
        NodeId::new("reducer")?,
        NodeKind::Reducer {
            config: ReducerConfig::new(
                PortId::new("items")?,
                item_schema.clone(),
                1,
                ReducerStrategy::Capability(OperationId::new("filesystem.write")?),
            )?,
        },
    )?
    .with_control_input(PortId::new("in")?)?
    .with_control_output(PortId::new("out")?)?
    .with_data_input(
        PortId::new("items")?,
        DataPort::input(item_schema, false, None)?,
    )?;
    let mutation = MutationBatch::new(vec![
        Mutation::ReplaceNode { node: producer },
        Mutation::AddNode { node: reducer },
        Mutation::RemoveEdge {
            edge: EdgeId::new("work-done")?,
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("work-reducer")?,
                EdgeKind::Control,
                NodeId::new("work")?,
                PortId::new("out")?,
                NodeId::new("reducer")?,
                PortId::new("in")?,
            ),
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("reducer-done")?,
                EdgeKind::Control,
                NodeId::new("reducer")?,
                PortId::new("out")?,
                NodeId::new("done")?,
                PortId::new("in")?,
            ),
        },
        Mutation::AddEdge {
            edge: Edge::new(
                EdgeId::new("work-reducer-item")?,
                EdgeKind::Data,
                NodeId::new("work")?,
                PortId::new("item")?,
                NodeId::new("reducer")?,
                PortId::new("items")?,
            ),
        },
    ])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-reducer-expansion")?,
        actor,
        ProposalProvenance::Direct,
        workflow,
        None,
        base.id().clone(),
        base.content_digest().clone(),
        None,
        mutation.clone(),
        "attempt to select an unauthorized reducer operation",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::ProposeOnly,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let expected = base.revise(
        base.id(),
        mutation,
        AuthorRef::new(format!("proposal:{}", &proposal.digest().as_str()[3..35]))?,
        format!(
            "proposal_id={};proposal_digest={};proposer={};source=direct",
            proposal.identity(),
            proposal.digest(),
            proposal.proposer()
        ),
    )?;
    let document = WorkflowProposalDocument::new(proposal);
    let result = service.execute(&command(
        "control-reducer-expansion",
        &context,
        OptimisticGuard {
            expected_run_sequence: None,
            expected_revision: Some(base.id().clone()),
            expected_proposal_digest: Some(document.proposal().digest().clone()),
        },
        ControlCommand::SubmitProposal { proposal: document },
    )?);
    assert!(matches!(
        result,
        Err(ControlError::AuthorizationDenied { .. })
    ));
    assert!(store.revision(expected.id())?.is_none());
    Ok(())
}

#[test]
fn malformed_control_capability_input_is_a_normal_rejected_terminal() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let actor = ActorRef::new("ai:hostile-output")?;
    let run = RunId::new("run-control-hostile-output")?;
    let grant_id = GrantId::new("grant:control-hostile-output")?;
    let (_runtime, service, _context) =
        services(store, &actor, &run, &grant_id, "control-hostile-output")?;
    let descriptor = workflow_control_descriptor()?;
    let operation = OperationId::new(WORKFLOW_PROPOSE_OPERATION)?;
    let resolution = ResolvedCapabilitySnapshot::from_descriptor(&descriptor, &operation)?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-hostile-control-output")?,
        descriptor.identity().clone(),
        operation,
        descriptor.provider_profile().cloned(),
        None,
        vec![InputReference::new(
            "milkdrift.control_request",
            InvocationValueReference::Inline {
                value: BoundedJson::new(serde_json::json!({
                    "schema_version": 1,
                    "hostile_untrusted_output": ["not", "a", "command"]
                }))?,
            },
        )?],
        BTreeMap::new(),
    )?;
    let adapter = WorkflowControlAdapter::new(service, Arc::new(UnusedResultSink));
    let reporter = RecordingReporter::default();
    adapter.execute(&AdapterInvocation::new(&resolution, &request), &reporter)?;

    let events = reporter.0.lock().map_err(|_| "reporter lock poisoned")?;
    assert_eq!(events.len(), 1);
    let terminal = events[0]
        .kind()
        .terminal()
        .ok_or("malformed control output did not produce a terminal event")?;
    assert_eq!(terminal.status(), TerminalStatus::Rejected);
    assert_eq!(terminal.side_effect(), SideEffectClass::None);
    assert!(terminal.failure().is_some());
    Ok(())
}
