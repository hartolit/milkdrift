//! Capacity, recursion, and cumulative caller limits are independent of a free worker slot.
use super::*;
use milkdrift_blueprint::{AdaptationScope, GoverningAgreement};
use milkdrift_persistence::ControllerAccountStore;

fn generation(fixture: &Fixture, number: u64) -> TestResult<PublishedMethod> {
    let mut method = fixture.method.clone();
    let mut descriptor = serde_json::to_value(&method.descriptor)?;
    descriptor["descriptor_revision"] = serde_json::json!(number);
    method.descriptor = serde_json::from_value(descriptor)?;
    Ok(method)
}

#[test]
fn registry_bound_refuses_before_publishing_an_unusable_generation() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "registry-bound")?;
    fixture.published.publish(
        generation(&fixture, 2)?,
        Some(1),
        &fixture.decision,
        &milkdrift_persistence::IntegrityDigest::hash(b"generation-two"),
    )?;
    assert!(
        fixture
            .published
            .publish(
                generation(&fixture, 3)?,
                Some(1),
                &fixture.decision,
                &milkdrift_persistence::IntegrityDigest::hash(b"generation-three")
            )
            .is_err()
    );
    assert!(
        fixture
            .store
            .published_method(fixture.method.descriptor.identity(), 3)?
            .is_none()
    );
    Ok(())
}

#[test]
fn self_call_is_refused_before_a_second_internal_run() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "cycle")?;
    let (base, _) = agreements::governed()?;
    let recursive = base.revise(
        base.id(),
        MutationBatch::new(vec![
            Mutation::ReplaceNode {
                node: task_node("repair.begin", "method.invoke")?,
            },
            Mutation::ReplaceNode {
                node: task_node("repair.end", "method.invoke")?,
            },
        ])?,
        AuthorRef::new("human:method-owner")?,
        "reviewed nested service contract",
    )?;
    let agreement = GoverningAgreement::seal(
        NodeId::new("agreement")?,
        &recursive,
        AdaptationScope::new(
            "repair.".to_owned(),
            8,
            vec![
                CapabilityRequirement::new(OperationId::new("method.invoke")?)
                    .maximum_side_effect(SideEffectClass::ReadOnly),
            ],
        )?
        .with_maximum_revisions(1)?,
        format!("b3_{}", "a".repeat(64)),
    )?;
    let governed = recursive.revise(
        recursive.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement.clone()),
        }])?,
        AuthorRef::new("human:method-owner")?,
        "seal nested service contract",
    )?;
    fixture.store.put_revision(&recursive)?;
    fixture.store.put_revision(&governed)?;
    let mut method = generation(&fixture, 2)?;
    method.revision = governed.id().clone();
    method.agreement = agreement.digest().to_owned();
    fixture.published.publish(
        method,
        Some(1),
        &fixture.decision,
        &milkdrift_persistence::IntegrityDigest::hash(b"recursive-generation"),
    )?;
    let run = start_outer(&fixture, "run:cycle")?;
    runtime_tick(&fixture.runtime)?;
    let plan = fixture
        .store
        .published_local_page(None, PageSize::new(8)?)?
        .0
        .pop()
        .ok_or("first association absent")?;
    for _ in 0..64 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
    }
    assert_eq!(fixture.process.entries(), 0);
    assert!(fixture.runtime.projection(&run)?.lifecycle().is_completed());
    let child = fixture.runtime.history(&plan.child_run)?;
    assert!(!child.iter().any(|event| matches!(
        event.kind(),
        RunEventKind::PublishedInvocationPlanned { .. }
    )));
    assert!(child.iter().any(
        |event| matches!(event.kind(), RunEventKind::NodeTerminal { detail: Some(detail), .. }
        if detail.as_str().contains("cycle or nesting limit"))
    ));
    Ok(())
}

#[test]
fn caller_account_is_shared_across_service_runs_and_reopen_without_double_charge() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run:published-controller")?;
    let account;
    {
        let fixture = fixture(directory.path(), "finite-parent")?;
        let base = base_revision("published-controller-body")?;
        let body = base.revise(
            base.id(),
            MutationBatch::new(vec![Mutation::ReplaceNode {
                node: task_node("work", "method.invoke")?,
            }])?,
            AuthorRef::new("human:caller")?,
            "invoke the bounded service",
        )?;
        fixture.store.put_revision(&base)?;
        fixture.store.put_revision(&body)?;
        let wrapper = build_controller_blueprint(ControllerBlueprintSpec {
            cost_currency: None,
            workflow: WorkflowId::new("published-controller")?,
            body: PinnedSubworkflow::new(
                body.semantic().workflow().clone(),
                body.id().clone(),
                WorkflowInterface::new([], [])?,
            ),
            continue_condition: Condition::Constant { value: true },
            limits: ControllerLimits::new(
                8, 8, 8, 4, 60_000, 0, 100_000, 100_000, 16_777_216, 12, 10, 1, 8, 2, 2, None,
            )?,
            author: AuthorRef::new("human:caller")?,
        })?;
        fixture.store.put_revision(&wrapper)?;
        revision_and_lifecycle::create_and_start(
            &fixture.control,
            &fixture.runtime,
            &fixture.context,
            &run,
            &wrapper,
        )?;
        account = fixture
            .store
            .controller_account_binding(&run)?
            .ok_or("parent account absent")?;
        for _ in 0..128 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture
                .store
                .controller_account(&account)?
                .ok_or("account absent")?
                .settled()
                .process_admissions()
                == 2
            {
                break;
            }
        }
        assert_eq!(
            fixture
                .store
                .controller_account(&account)?
                .ok_or("account absent")?
                .settled()
                .process_admissions(),
            2
        );
        assert_eq!(fixture.process.entries(), 2);
    }
    let fixture = fixture(directory.path(), "finite-parent-reopened")?;
    for _ in 0..256 {
        fixture.clock.advance(1)?;
        runtime_tick(&fixture.runtime)?;
        if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
            break;
        }
    }
    assert!(fixture.runtime.projection(&run)?.lifecycle().is_completed());
    assert_eq!(
        fixture.store.controller_account_binding(&run)?,
        Some(account.clone())
    );
    let state = fixture
        .store
        .controller_account(&account)?
        .ok_or("parent account absent")?;
    assert_eq!(state.settled().process_admissions(), 4);
    assert_eq!(fixture.process.entries(), 2);
    assert!(state.reservations().is_empty());
    assert!(state.blocked().is_none());
    assert_complete_integrity(fixture.store.as_ref())?;
    Ok(())
}

#[test]
fn descendants_cannot_reset_an_ancestors_stricter_depth_ceiling() -> TestResult {
    for maximum_depth in [1, 2] {
        let directory = TempDir::new()?;
        let fixture = fixture(directory.path(), "inherited-depth")?;
        let inner_id = CapabilityId::new("method:inner")?;
        let mut inner = fixture.method.clone();
        let mut descriptor = serde_json::to_value(&inner.descriptor)?;
        descriptor["identity"] = serde_json::json!(inner_id);
        inner.descriptor = serde_json::from_value(descriptor)?;
        inner.maximum_depth = 32;
        let caller = publication_grant("human:caller", "grant:caller")?;
        let authority = GrantSetEvaluator::new(
            PolicyId::new("test.publication")?,
            1,
            [caller],
            BTreeMap::new(),
        )?;
        let mut request = fixture.decision.request().clone();
        request.resources.capability = Some(inner_id.clone());
        fixture.published.publish(
            inner,
            None,
            &authority.evaluate(&request)?,
            &milkdrift_persistence::IntegrityDigest::hash(b"inner-publication"),
        )?;
        let (base, _) = agreements::governed()?;
        let requirement = CapabilityRequirement::new(OperationId::new("method.invoke")?)
            .exact(inner_id)
            .maximum_side_effect(SideEffectClass::ReadOnly);
        let work = Node::new(
            NodeId::new("repair.begin")?,
            NodeKind::task_direct_inputs(requirement.clone())?,
        )?
        .with_control_input(PortId::new("in")?)?
        .with_control_output(PortId::new("out")?)?;
        let revised = base.revise(
            base.id(),
            MutationBatch::new(vec![Mutation::ReplaceNode { node: work }])?,
            AuthorRef::new("human:method-owner")?,
            "call a distinct publication",
        )?;
        let agreement = GoverningAgreement::seal(
            NodeId::new("agreement")?,
            &revised,
            AdaptationScope::new(
                "repair.".to_owned(),
                2,
                vec![
                    requirement,
                    CapabilityRequirement::new(OperationId::new("model.generate")?)
                        .maximum_side_effect(SideEffectClass::ReadOnly),
                ],
            )?
            .with_maximum_revisions(1)?,
            format!("b3_{}", "a".repeat(64)),
        )?;
        let governed = revised.revise(
            revised.id(),
            MutationBatch::new(vec![Mutation::SetAgreement {
                agreement: Some(agreement.clone()),
            }])?,
            AuthorRef::new("human:method-owner")?,
            "seal depth parent",
        )?;
        for revision in [&base, &revised, &governed] {
            fixture.store.put_revision(revision)?;
        }
        let mut parent = generation(&fixture, 2)?;
        parent.revision = governed.id().clone();
        parent.agreement = agreement.digest().to_owned();
        parent.maximum_depth = maximum_depth;
        fixture.published.publish(
            parent,
            Some(1),
            &fixture.decision,
            &milkdrift_persistence::IntegrityDigest::hash(b"depth-parent"),
        )?;
        let run = start_outer(&fixture, "run:depth-parent")?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop()
            .ok_or("parent association absent")?;
        assert_eq!(plan.capability, *fixture.method.descriptor.identity());
        assert_eq!(plan.generation, 2);
        for _ in 0..128 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        let nested: Vec<_> = fixture
            .runtime
            .history(&plan.child_run)?
            .into_iter()
            .filter_map(|event| match event.kind() {
                RunEventKind::PublishedInvocationPlanned { plan, .. } => Some(plan.clone()),
                _ => None,
            })
            .collect();
        if maximum_depth == 1 {
            assert!(
                nested.is_empty(),
                "ancestor ceiling must refuse before nested creation"
            );
            assert_eq!(fixture.process.entries(), 0);
            assert_eq!(
                fixture.runtime.projection(&run)?.lifecycle(),
                RunLifecycle::Terminal(RunOutcome::Failed)
            );
        } else {
            assert_eq!(nested.len(), 1);
            assert_eq!(nested[0].ancestry.len(), 1);
            assert_eq!(nested[0].ancestry[0].maximum_depth(), maximum_depth);
            assert_eq!(fixture.process.entries(), 3);
            assert_eq!(
                fixture.runtime.projection(&run)?.lifecycle(),
                RunLifecycle::Terminal(RunOutcome::Succeeded)
            );
        }
        assert_complete_integrity(fixture.store.as_ref())?;
    }
    Ok(())
}
