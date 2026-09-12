//! Recovery and alternate-entry guards around explicit lifecycle installation.
use super::*;

fn wrapper(store: &RedbStore) -> TestResult<BlueprintRevision> {
    let body = BlueprintRevision::genesis(
        WorkflowId::new("activation-body")?,
        MutationBatch::new(vec![Mutation::AddNode {
            node: Node::new(
                NodeId::new("done")?,
                NodeKind::Terminal {
                    outcome: TerminalOutcome::Success,
                },
            )?,
        }])?,
        AuthorRef::new("human:activation")?,
        "No external work before checkpoint",
    )?;
    store.put_revision(&body)?;
    Ok(build_controller_blueprint(ControllerBlueprintSpec {
        workflow: WorkflowId::new("activation-wrapper")?,
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
            Some(1),
        )?,
        author: AuthorRef::new("human:activation")?,
    })?)
}

#[test]
fn accounted_recovery_requires_one_installation_before_admission() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path().join("activation.redb"))?);
    let run = RunId::new("activation-run")?;
    let actor = ActorRef::new("controller:activation")?;
    let grant_id = GrantId::new("grant:activation")?;
    let (runtime, service, context) =
        services(store.clone(), &actor, &run, &grant_id, "activation-first")?;
    let wrapper = wrapper(&store)?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    for _ in 0..48 {
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
    let account = runtime
        .controller_account_for_run(&run)?
        .ok_or("controller account absent")?;
    let before = runtime.projection(&run)?.sequence();
    drop(service);
    drop(runtime);
    let authority = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("test.control-service")?,
        1,
        [grant(&actor, &run, &grant_id)?],
        BTreeMap::new(),
    )?);
    let runtime = Arc::new(RuntimeService::open_closed_with_authority(
        store.clone(),
        Arc::new(DeterministicExecutor::new(admission::process_descriptor()?)),
        authority.clone(),
        Arc::new(ManualClock::new(NOW)),
        Arc::new(SequentialIdGenerator::new("activation-reopened", 1)?),
        RuntimeConfig::new(
            WorkerId::new("worker-control-service")?,
            actor,
            30_000,
            32,
            SchedulerLimits::new(8, 4, 2, 4)?,
            RetryPolicy::new(1, Vec::new(), 100, 1000, 0)?,
        )?,
    )?);
    let refused = runtime
        .recover_startup_closed()
        .err()
        .ok_or("uninstalled recovery succeeded")?;
    assert!(
        refused
            .to_string()
            .contains("requires lifecycle installation before recovery")
    );
    assert_eq!(runtime.projection(&run)?.sequence(), before);
    assert_eq!(
        runtime.controller_account_for_run(&run)?.as_ref(),
        Some(&account)
    );
    let service = ControlService::new(store, runtime.clone(), authority);
    runtime.install_controller_lifecycle(service.controller_lifecycle_owner())?;
    assert!(
        runtime
            .install_controller_lifecycle(service.controller_lifecycle_owner())
            .is_err()
    );
    runtime.initialize_startup()?;
    assert!(
        runtime
            .install_controller_lifecycle(service.controller_lifecycle_owner())
            .is_err()
    );
    assert_eq!(
        runtime.controller_account_for_run(&run)?.as_ref(),
        Some(&account)
    );
    Ok(())
}

#[test]
fn child_read_authority_does_not_disclose_the_shared_controller_account() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("account-inspection.redb"),
    )?);
    let run = RunId::new("account-inspection-root")?;
    let actor = ActorRef::new("controller:account-inspection")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &GrantId::new("grant:account-inspection")?,
        "account-inspection",
    )?;
    let wrapper = wrapper(&store)?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    for _ in 0..32 {
        runtime_tick(&runtime)?;
    }
    let child = runtime
        .history(&run)?
        .iter()
        .find_map(|event| match event.kind() {
            RunEventKind::SubworkflowCreated { child_run, .. } => Some(child_run.clone()),
            _ => None,
        })
        .ok_or("controller child absent")?;
    let inspector = ActorRef::new("human:child-inspector")?;
    let inspector_grant = GrantId::new("grant:child-inspector")?;
    let child_grant = grant(&inspector, &child, &inspector_grant)?;
    let claim = ActorAuthorityContext::new(
        inspector,
        CommandAuthorityClaim::new(inspector_grant, 1, child_grant.digest()?, 0)?,
    );
    let authority = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("test.control-service")?,
        1,
        [child_grant],
        BTreeMap::new(),
    )?);
    let read_service = ControlService::new(store, runtime, authority);
    assert!(matches!(
        read_service.execute(&command(
            "inspect-child-timeline",
            &claim,
            OptimisticGuard::default(),
            ControlCommand::InspectTimeline {
                run: child.clone(),
                after: None,
                limit: PageSize::new(8)?
            }
        )?)?,
        ControlResult::Timeline { .. }
    ));
    assert!(matches!(
        read_service.execute(&command(
            "inspect-child-account",
            &claim,
            OptimisticGuard::default(),
            ControlCommand::InspectRun { run: child }
        )?),
        Err(ControlError::AuthorizationDenied { .. })
    ));
    Ok(())
}

#[test]
fn a_task_before_the_marked_repeat_cannot_enter_without_an_account() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(directory.path().join("prelude.redb"))?);
    let run = RunId::new("prelude-run")?;
    let actor = ActorRef::new("controller:prelude")?;
    let grant_id = GrantId::new("grant:prelude")?;
    let (runtime, service, context, adapter) =
        counting_process_services(store.clone(), &actor, &run, &grant_id, "prelude")?;
    let original = wrapper(&store)?;
    store.put_revision(&original)?;
    let repeat = original
        .semantic()
        .nodes()
        .get(&NodeId::new("controller-repeat")?)
        .ok_or("repeat absent")?
        .clone()
        .with_control_input(PortId::new("in")?)?;
    let prelude = base_revision("unused-prelude")?
        .semantic()
        .nodes()
        .get(&NodeId::new("work")?)
        .ok_or("task absent")?
        .clone();
    let revised = original.revise(
        original.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode { node: repeat },
            Mutation::AddNode { node: prelude },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("prelude-repeat")?,
                    EdgeKind::Control,
                    NodeId::new("work")?,
                    PortId::new("out")?,
                    NodeId::new("controller-repeat")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:prelude")?,
        "Attempt a task before account establishment",
    )?;
    store.put_revision(&revised)?;
    create_and_start(&service, &runtime, &context, &run, &revised)?;
    let mut effects = Vec::new();
    for _ in 0..16 {
        runtime.scheduler_tick()?;
        effects.extend(runtime.claim_execution_effects(PageSize::new(4)?)?);
        if !effects.is_empty() {
            break;
        }
    }
    assert_eq!(effects.len(), 1);
    let effect = effects.pop().ok_or("effect absent")?;
    let refused = runtime
        .execute_effect(effect)
        .err()
        .ok_or("unaccounted marked task entered")?;
    assert!(
        refused
            .to_string()
            .contains("requires account establishment")
    );
    assert_eq!(adapter.entries(), 0);
    assert!(runtime.controller_account_for_run(&run)?.is_none());
    Ok(())
}

#[test]
fn a_subworkflow_before_the_marked_repeat_cannot_create_unaccounted_work() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("child-prelude.redb"),
    )?);
    let run = RunId::new("child-prelude-run")?;
    let actor = ActorRef::new("controller:child-prelude")?;
    let (runtime, service, context, adapter) = counting_process_services(
        store.clone(),
        &actor,
        &run,
        &GrantId::new("grant:child-prelude")?,
        "child-prelude",
    )?;
    let work = base_revision("child-prelude-work")?;
    store.put_revision(&work)?;
    let original = wrapper(&store)?;
    store.put_revision(&original)?;
    let repeat = original.semantic().nodes()[&NodeId::new("controller-repeat")?]
        .clone()
        .with_control_input(PortId::new("in")?)?;
    let revised = original.revise(
        original.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode { node: repeat },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("prelude")?,
                    NodeKind::Subworkflow {
                        reference: PinnedSubworkflow::new(
                            work.semantic().workflow().clone(),
                            work.id().clone(),
                            WorkflowInterface::new([], [])?,
                        ),
                    },
                )?
                .with_control_output(PortId::new("out")?)?,
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("prelude-repeat")?,
                    EdgeKind::Control,
                    NodeId::new("prelude")?,
                    PortId::new("out")?,
                    NodeId::new("controller-repeat")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:child-prelude")?,
        "An unmarked child must not bypass the marked parent's account",
    )?;
    store.put_revision(&revised)?;
    let refused = create_and_start(&service, &runtime, &context, &run, &revised)
        .err()
        .ok_or("unaccounted child creation was admitted")?;
    assert!(
        refused
            .to_string()
            .contains("account establishment before child creation")
    );
    assert_eq!(
        adapter.entries(),
        0,
        "unaccounted child entered its adapter"
    );
    assert!(
        runtime
            .history(&run)?
            .iter()
            .all(|event| !matches!(event.kind(), RunEventKind::SubworkflowCreated { .. }))
    );
    Ok(())
}

#[test]
fn proposal_assessment_ignores_other_nodes_in_the_marked_revision() -> TestResult {
    let directory = TempDir::new()?;
    let store = Arc::new(RedbStore::open(
        directory.path().join("proposal-prelude.redb"),
    )?);
    let run = RunId::new("proposal-prelude")?;
    let actor = ActorRef::new("controller:proposal-prelude")?;
    let (runtime, service, context) = services(
        store.clone(),
        &actor,
        &run,
        &GrantId::new("grant:proposal-prelude")?,
        "proposal-prelude",
    )?;
    let base = wrapper(&store)?;
    store.put_revision(&base)?;
    let repeat = base
        .semantic()
        .nodes()
        .get(&NodeId::new("controller-repeat")?)
        .ok_or("repeat absent")?
        .clone()
        .with_control_input(PortId::new("in")?)?;
    let wrapper = base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode { node: repeat },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("prelude")?,
                    NodeKind::Fork {
                        config: ForkConfig::new(BTreeSet::from([
                            PortId::new("out")?,
                            PortId::new("hold")?,
                        ]))?,
                    },
                )?
                .with_control_output(PortId::new("out")?)?
                .with_control_output(PortId::new("hold")?)?,
            },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("hold")?,
                    NodeKind::SignalWait {
                        signal: OperationId::new("activation.hold")?,
                    },
                )?
                .with_control_input(PortId::new("in")?)?
                .with_control_output(PortId::new("out")?)?,
            },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("join")?,
                    NodeKind::Join {
                        config: JoinConfig::new(NodeId::new("prelude")?, JoinPolicy::All),
                    },
                )?
                .with_control_input(PortId::new("out")?)?
                .with_control_input(PortId::new("hold")?)?
                .with_control_output(PortId::new("out")?)?,
            },
            Mutation::RemoveEdge {
                edge: EdgeId::new("controller-finished")?,
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("prelude-hold")?,
                    EdgeKind::Control,
                    NodeId::new("prelude")?,
                    PortId::new("hold")?,
                    NodeId::new("hold")?,
                    PortId::new("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("hold-join")?,
                    EdgeKind::Control,
                    NodeId::new("hold")?,
                    PortId::new("out")?,
                    NodeId::new("join")?,
                    PortId::new("hold")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("repeat-join")?,
                    EdgeKind::Control,
                    NodeId::new("controller-repeat")?,
                    PortId::new("out")?,
                    NodeId::new("join")?,
                    PortId::new("out")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("join-complete")?,
                    EdgeKind::Control,
                    NodeId::new("join")?,
                    PortId::new("out")?,
                    NodeId::new("controller-complete")?,
                    PortId::new("in")?,
                ),
            },
            Mutation::AddEdge {
                edge: Edge::new(
                    EdgeId::new("prelude-repeat")?,
                    EdgeKind::Control,
                    NodeId::new("prelude")?,
                    PortId::new("out")?,
                    NodeId::new("controller-repeat")?,
                    PortId::new("in")?,
                ),
            },
        ])?,
        AuthorRef::new("human:activation")?,
        "Deterministic work before the marked repeat",
    )?;
    store.put_revision(&wrapper)?;
    create_and_start(&service, &runtime, &context, &run, &wrapper)?;
    for _ in 0..32 {
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
    let projection = runtime.projection(&run)?;
    assert!(projection.node_executions().len() > 1);
    let account = runtime
        .controller_account_for_run(&run)?
        .ok_or("account absent")?;
    let mutation = MutationBatch::new(vec![Mutation::SetMetadata {
        metadata: BlueprintMetadata::new(
            "Annotated controller",
            "Prospective description only",
            BTreeSet::new(),
            wrapper.semantic().metadata().extensions().clone(),
        )?,
    }])?;
    let proposal = WorkflowProposal::new(
        ProposalId::new("proposal-with-prelude")?,
        actor.clone(),
        ProposalProvenance::Direct,
        wrapper.semantic().workflow().clone(),
        Some(run.clone()),
        wrapper.id().clone(),
        wrapper.content_digest().clone(),
        Some(projection.sequence()),
        mutation.clone(),
        "Annotate pending work",
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        ProposalApplicationPolicy::RequireApproval,
        None,
        ClaimedStopCondition::Continue,
    )?;
    let owner = service.controller_lifecycle_owner();
    assert!(
        matches!(owner.assess_proposal(&run, &projection, &proposal, Some(&account), NOW),
        Err(ControlError::ProposalState(reason)) if reason.contains("human checkpoint"))
    );
    let proposed = wrapper.revise(
        wrapper.id(),
        mutation,
        AuthorRef::new("proposal:activation")?,
        format!("proposer={actor};source=direct"),
    )?;
    for boundary in [
        ControllerAssessmentBoundary::ProposalApproval,
        ControllerAssessmentBoundary::ProposalApplication,
    ] {
        assert!(
            matches!(owner.assess_proposal_transition(&run, &projection, &proposed, boundary, Some(&account), NOW),
            Err(ControlError::ProposalState(reason)) if reason.contains("human checkpoint"))
        );
    }
    Ok(())
}
