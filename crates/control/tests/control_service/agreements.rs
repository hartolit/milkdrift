//! Public proposal/adoption and recovery checks for the immutable structural boundary.
use super::*;
use milkdrift_blueprint::{
    AdaptationScope, BindingSource, DataPort, GoverningAgreement, SchemaRef,
};

fn edge(name: &str, from: &str, to: &str) -> TestResult<Edge> {
    Ok(Edge::new(
        EdgeId::new(name)?,
        EdgeKind::Control,
        NodeId::new(from)?,
        PortId::new("out")?,
        NodeId::new(to)?,
        PortId::new("in")?,
    ))
}
fn governed() -> TestResult<(BlueprintRevision, BlueprintRevision)> {
    let start = Node::new(NodeId::new("start")?, NodeKind::Wait { duration_ms: 1 })?
        .with_control_output(PortId::new("out")?)?;
    let done = Node::new(
        NodeId::new("done")?,
        NodeKind::Terminal {
            outcome: TerminalOutcome::Success,
        },
    )?
    .with_control_input(PortId::new("in")?)?;
    let begin = task_node("repair.begin", "model.generate")?;
    let NodeKind::Task { config } = begin.kind() else {
        return Err("expected task".into());
    };
    let requirement = config.requirement().clone();
    let base = BlueprintRevision::genesis(
        WorkflowId::new("governed")?,
        MutationBatch::new(vec![
            Mutation::AddNode { node: start },
            Mutation::AddNode { node: begin },
            Mutation::AddNode {
                node: task_node("repair.end", "model.generate")?,
            },
            Mutation::AddNode { node: done },
            Mutation::AddEdge {
                edge: edge("start", "start", "repair.begin")?,
            },
            Mutation::AddEdge {
                edge: edge("internal", "repair.begin", "repair.end")?,
            },
            Mutation::AddEdge {
                edge: edge("done", "repair.end", "done")?,
            },
        ])?,
        AuthorRef::new("human:agreement-owner")?,
        "bounded repair method",
    )?;
    let agreement = GoverningAgreement::seal(
        NodeId::new("agreement")?,
        &base,
        AdaptationScope::new("repair.".to_owned(), 8, vec![requirement])?
            .with_maximum_revisions(1)?,
        format!("b3_{}", "a".repeat(64)),
    )?;
    let governed = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement),
        }])?,
        AuthorRef::new("human:agreement-owner")?,
        "accept enclosing boundary",
    )?;
    Ok((base, governed))
}
fn proposal(
    context: &ActorAuthorityContext,
    run: &RunId,
    base: &BlueprintRevision,
    sequence: RunSequence,
    changes: Vec<Mutation>,
) -> TestResult<WorkflowProposalDocument> {
    Ok(WorkflowProposalDocument::new(WorkflowProposal::new(
        ProposalId::new("proposal-adaptive-repair")?,
        context.actor().clone(),
        ProposalProvenance::Direct,
        base.semantic().workflow().clone(),
        Some(run.clone()),
        base.id().clone(),
        base.content_digest().clone(),
        Some(sequence),
        MutationBatch::new(changes)?,
        "investigate selected failure before constructing a new candidate",
        None,
        vec![],
        vec![],
        vec![],
        vec![],
        ProposalApplicationPolicy::AutoApplyLowRisk,
        None,
        ClaimedStopCondition::Complete,
    )?))
}

#[test]
fn governed_repair_auto_applies_through_control_and_replays_after_store_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let actor = ActorRef::new("agent:repair")?;
    let run = RunId::new("run-governed")?;
    let grant_id = GrantId::new("grant:repair")?;
    let (ungoverned_base, base) = governed()?;
    let submitted;
    let revision;
    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        store.put_revision(&ungoverned_base)?;
        store.put_revision(&base)?;
        let (runtime, service, context) =
            services(store.clone(), &actor, &run, &grant_id, "governed")?;
        super::revision_and_lifecycle::create_and_start(&service, &runtime, &context, &run, &base)?;
        let sequence = runtime.projection(&run)?.sequence();
        let repaired = task_node("repair.begin", "model.generate")?.with_data_input(
            PortId::new("instruction")?,
            DataPort::input(
                SchemaRef::new(
                    milkdrift_capability::SchemaId::new("example.instruction")?,
                    1,
                )?,
                true,
                Some(BindingSource::Literal {
                    value: milkdrift_capability::BoundedJson::new(serde_json::json!(
                        "repair the candidate using retained failure evidence"
                    ))?,
                }),
            )?,
        )?;
        let proposal = proposal(
            &context,
            &run,
            &base,
            sequence,
            vec![
                Mutation::ReplaceNode { node: repaired },
                Mutation::AddNode {
                    node: task_node("repair.investigate", "model.generate")?,
                },
                Mutation::ReplaceEdge {
                    edge: edge("internal", "repair.begin", "repair.investigate")?,
                },
                Mutation::AddEdge {
                    edge: edge("investigated", "repair.investigate", "repair.end")?,
                },
            ],
        )?;
        submitted = command(
            "submit-adaptive",
            &context,
            OptimisticGuard {
                expected_run_sequence: Some(sequence),
                expected_revision: Some(base.id().clone()),
                expected_proposal_digest: Some(proposal.proposal().digest().clone()),
            },
            ControlCommand::SubmitProposal { proposal },
        )?;
        let ControlResult::ProposalSubmitted { value } = service.execute(&submitted)? else {
            return Err("proposal not submitted".into());
        };
        assert_eq!(value.classification.risk, RiskClass::Low);
        assert_eq!(
            value.classification.policy,
            "milkdrift.agreement-adaptation"
        );
        assert!(value.applied);
        revision = value.proposed_revision;
        assert_eq!(runtime.projection(&run)?.revision(), Some(&revision));
        let changed = store
            .revision(&revision)?
            .ok_or("missing changed revision")?;
        assert_eq!(base.semantic().agreement(), changed.semantic().agreement());
        let ungoverned = changed.revise(
            changed.id(),
            MutationBatch::new(vec![Mutation::SetAgreement { agreement: None }])?,
            AuthorRef::new("agent:repair")?,
            "try removing the accepted agreement",
        )?;
        store.put_revision(&ungoverned)?;
        let attempt = milkdrift_runtime::RunCommandDocument::new(
            milkdrift_persistence::CommandId::new("direct-agreement-removal")?,
            run.clone(),
            actor.clone(),
            runtime.projection(&run)?.sequence(),
            TimestampMillis::new(NOW),
            Reason::new("attempt direct adoption bypass")?,
            vec![],
            milkdrift_runtime::RunCommand::RequestRevisionAdoption {
                reconciliation: milkdrift_persistence::ReconciliationId::new("remove-agreement")?,
                revision: ungoverned.id().clone(),
                policy: milkdrift_persistence::ReconciliationPolicy::FinishCurrentThenAdopt,
            },
        )?;
        assert_eq!(
            runtime
                .handle_authorized_command(&attempt, context.authority())?
                .result()
                .disposition(),
            milkdrift_persistence::CommandDisposition::Rejected
        );
        assert_eq!(runtime.projection(&run)?.revision(), Some(&revision));
    }
    let store = Arc::new(RedbStore::open(directory.path())?);
    let (runtime, service, _) = services(store.clone(), &actor, &run, &grant_id, "reopened")?;
    let sequence = runtime.projection(&run)?.sequence();
    service.execute(&submitted)?;
    assert_eq!(runtime.projection(&run)?.sequence(), sequence);
    assert_eq!(runtime.projection(&run)?.revision(), Some(&revision));
    assert_eq!(
        store
            .revision(&revision)?
            .ok_or("missing reopened revision")?
            .semantic()
            .agreement(),
        base.semantic().agreement()
    );
    assert_eq!(runtime.projection(&run)?.agreement_adoptions(), 1);
    let current = store.revision(&revision)?.ok_or("missing revised method")?;
    let next = current.revise(
        current.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: task_node("repair.begin", "model.generate")?,
        }])?,
        AuthorRef::new("agent:repair")?,
        "another otherwise valid method change",
    )?;
    store.put_revision(&next)?;
    let exhausted = milkdrift_runtime::RunCommandDocument::new(
        milkdrift_persistence::CommandId::new("exhausted-adaptation")?,
        run.clone(),
        actor,
        runtime.projection(&run)?.sequence(),
        TimestampMillis::new(NOW),
        Reason::new("cumulative agreement limit")?,
        vec![],
        milkdrift_runtime::RunCommand::RequestRevisionAdoption {
            reconciliation: milkdrift_persistence::ReconciliationId::new("exhausted")?,
            revision: next.id().clone(),
            policy: milkdrift_persistence::ReconciliationPolicy::FinishCurrentThenAdopt,
        },
    )?;
    let (_, _, reopened_context) = services(
        store.clone(),
        &ActorRef::new("agent:repair")?,
        &run,
        &grant_id,
        "limit",
    )?;
    assert_eq!(
        runtime
            .handle_authorized_command(&exhausted, reopened_context.authority())?
            .result()
            .disposition(),
        milkdrift_persistence::CommandDisposition::Rejected
    );
    assert_eq!(runtime.projection(&run)?.agreement_adoptions(), 1);
    Ok(())
}

#[test]
fn inherited_agreement_blocks_indirect_child_adoption_after_reopen() -> TestResult {
    let directory = TempDir::new()?;
    let actor = ActorRef::new("agent:child-control")?;
    let root = RunId::new("run-parent-agreement")?;
    let grant_id = GrantId::new("grant:child-control")?;
    let child = BlueprintRevision::genesis(
        WorkflowId::new("protected-child")?,
        MutationBatch::new(vec![
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("wait")?,
                    NodeKind::Wait {
                        duration_ms: 60_000,
                    },
                )?
                .with_control_output(PortId::new("out")?)?,
            },
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
                edge: edge("done", "wait", "done")?,
            },
        ])?,
        AuthorRef::new("human:agreement-owner")?,
        "protected child wait",
    )?;
    let replacement = child.revise(
        child.id(),
        MutationBatch::new(vec![Mutation::ReplaceNode {
            node: Node::new(NodeId::new("wait")?, NodeKind::Wait { duration_ms: 1 })?
                .with_control_output(PortId::new("out")?)?,
        }])?,
        AuthorRef::new("agent:child-control")?,
        "try changing protected child semantics",
    )?;
    let (base, _) = governed()?;
    let method = base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode {
                node: Node::new(
                    NodeId::new("start")?,
                    NodeKind::task_direct_inputs(
                        CapabilityRequirement::new(OperationId::new("model.generate")?)
                            .maximum_side_effect(SideEffectClass::ReadOnly),
                    )?,
                )?
                .with_control_output(PortId::new("out")?)?,
            },
            Mutation::AddNode {
                node: Node::new(
                    NodeId::new("child")?,
                    NodeKind::Subworkflow {
                        reference: PinnedSubworkflow::new(
                            child.semantic().workflow().clone(),
                            child.id().clone(),
                            child.semantic().interface().clone(),
                        ),
                    },
                )?
                .with_control_input(PortId::new("in")?)?
                .with_control_output(PortId::new("out")?)?,
            },
            Mutation::ReplaceEdge {
                edge: edge("done", "repair.end", "child")?,
            },
            Mutation::AddEdge {
                edge: edge("child-done", "child", "done")?,
            },
        ])?,
        AuthorRef::new("human:agreement-owner")?,
        "compose protected child",
    )?;
    let NodeKind::Task { config } = method
        .semantic()
        .nodes()
        .get(&NodeId::new("repair.begin")?)
        .ok_or("missing repair")?
        .kind()
    else {
        return Err("expected task".into());
    };
    let agreement = GoverningAgreement::seal(
        NodeId::new("parent-agreement")?,
        &method,
        AdaptationScope::new("repair.".to_owned(), 8, vec![config.requirement().clone()])?,
        format!("b3_{}", "b".repeat(64)),
    )?;
    let governed = method.revise(
        method.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement),
        }])?,
        AuthorRef::new("human:agreement-owner")?,
        "seal protected child",
    )?;
    let grant = AuthorityPreset::Autonomous
        .template(
            grant_id.clone(),
            1,
            actor.clone(),
            WorkflowRunScope::Any,
            CapabilityAuthorityScope::allow_any(SideEffectClass::ReadOnly),
            AuthorityBudget {
                cost_minor: Some(1_000_000),
                duration_ms: Some(3_600_000),
                invocations: Some(1000),
                artifact_bytes: Some(16_777_216),
                units: Some(1_000_000),
                concurrency: Some(32),
            },
        )
        .build()?;
    let child_run;
    {
        let store = Arc::new(RedbStore::open(directory.path())?);
        for revision in [&child, &replacement, &base, &method, &governed] {
            store.put_revision(revision)?;
        }
        let (runtime, service, context) = services_with_grant_and_revocations(
            store,
            &actor,
            &grant_id,
            "parent",
            grant.clone(),
            BTreeMap::new(),
        )?;
        create_and_start(&service, &runtime, &context, &root, &governed)?;
        let mut found = None;
        for _ in 0..32 {
            runtime_tick(&runtime)?;
            let projection = runtime.projection(&root)?;
            if let Some(child) = projection.subworkflows().values().next() {
                let projected = runtime.projection(child.child_run())?;
                if projected.lifecycle().is_active() {
                    found = Some(child.child_run().clone());
                    break;
                }
            }
        }
        child_run = found.ok_or("protected child did not start")?;
        let parent = runtime.projection(&root)?;
        let child = runtime.projection(&child_run)?;
        assert!(parent.accepted_agreement().is_some());
        assert_eq!(child.accepted_agreement(), parent.accepted_agreement());
    }
    let store = Arc::new(RedbStore::open(directory.path())?);
    let (runtime, _, context) = services_with_grant_and_revocations(
        store,
        &actor,
        &grant_id,
        "child-reopen",
        grant,
        BTreeMap::new(),
    )?;
    let projection = runtime.projection(&child_run)?;
    assert_eq!(
        projection.accepted_agreement().map(|a| a.origin_run()),
        Some(&root)
    );
    let command = milkdrift_runtime::RunCommandDocument::new(
        milkdrift_persistence::CommandId::new("alter-child")?,
        child_run.clone(),
        actor,
        projection.sequence(),
        TimestampMillis::new(NOW),
        Reason::new("attempt indirect child substitution")?,
        vec![],
        milkdrift_runtime::RunCommand::RequestRevisionAdoption {
            reconciliation: milkdrift_persistence::ReconciliationId::new("substitute-child")?,
            revision: replacement.id().clone(),
            policy: milkdrift_persistence::ReconciliationPolicy::FinishCurrentThenAdopt,
        },
    )?;
    let refused = runtime.handle_authorized_command(&command, context.authority())?;
    assert_eq!(
        refused.result().disposition(),
        milkdrift_persistence::CommandDisposition::Rejected
    );
    assert!(
        String::from_utf8(refused.result().to_canonical_json()?)?
            .contains("governed enclosing method pins child work")
    );
    assert_eq!(runtime.projection(&child_run)?.revision(), Some(child.id()));
    Ok(())
}
