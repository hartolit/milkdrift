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
            runtime_tick(&runtime)?;
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
    runtime_tick(&runtime)?;
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
    let controller_account;

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
            runtime_tick(&runtime)?;
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
        controller_account = store
            .controller_account_binding(&run)?
            .ok_or("controller longevity run has no durable account binding")?;
        let account = store
            .controller_account(&controller_account)?
            .ok_or("controller longevity account is absent")?;
        assert_eq!(
            account.declaration().controller_execution(),
            &controller_execution
        );
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
            runtime_tick(&runtime)?;
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
            services(store.clone(), &actor, &run, &grant_id, "longevity-after")?;
        assert_eq!(
            store.controller_account_binding(&run)?.as_ref(),
            Some(&controller_account)
        );
        assert!(store.controller_account(&controller_account)?.is_some());
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
            runtime_tick(&runtime)?;
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
        services(store.clone(), &actor, &run, &grant_id, "longevity-terminal")?;
    assert_eq!(
        store.controller_account_binding(&run)?.as_ref(),
        Some(&controller_account)
    );
    let before = runtime.projection(&run)?.sequence();
    for _ in 0..512 {
        runtime_tick(&runtime)?;
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

#[path = "revision_and_lifecycle/proposals.rs"]
mod proposals;
