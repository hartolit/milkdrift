//! Declared choices become isolated run inputs; an unapproved target cannot create a child.
use super::*;
use milkdrift_blueprint::{
    AdaptationScope, BindingSource, FieldId, GoverningAgreement, InterfaceField,
};
use milkdrift_persistence::published::PublishedInput;

#[test]
fn two_callers_receive_isolated_inputs_and_unapproved_values_create_no_child() -> TestResult {
    let directory = TempDir::new()?;
    let fixture = fixture(directory.path(), "public-inputs")?;
    let (base, _) = agreements::governed()?;
    let schema = SchemaRef::new(SchemaId::new("example.approved-target")?, 1)?;
    let input_base = base.revise(
        base.id(),
        MutationBatch::new(vec![Mutation::SetInterface {
            interface: WorkflowInterface::new(
                [(
                    FieldId::new("target")?,
                    InterfaceField::required(schema.clone()),
                )],
                [],
            )?,
        }])?,
        AuthorRef::new("human:owner")?,
        "declare a finite service input",
    )?;
    let NodeKind::Task { config } = base
        .semantic()
        .nodes()
        .get(&NodeId::new("repair.begin")?)
        .ok_or("repair node absent")?
        .kind()
    else {
        return Err("repair node is not a task".into());
    };
    let agreement = GoverningAgreement::seal(
        NodeId::new("agreement")?,
        &input_base,
        AdaptationScope::new("repair.".to_owned(), 8, vec![config.requirement().clone()])?
            .with_maximum_revisions(1)?,
        format!("b3_{}", "a".repeat(64)),
    )?;
    let governed = input_base.revise(
        input_base.id(),
        MutationBatch::new(vec![Mutation::SetAgreement {
            agreement: Some(agreement.clone()),
        }])?,
        AuthorRef::new("human:owner")?,
        "seal the reviewed input boundary",
    )?;
    fixture.store.put_revision(&input_base)?;
    fixture.store.put_revision(&governed)?;
    let mut method = fixture.method.clone();
    let mut descriptor = serde_json::to_value(&method.descriptor)?;
    descriptor["descriptor_revision"] = serde_json::json!(2);
    method.descriptor = serde_json::from_value(descriptor)?;
    method.revision = governed.id().clone();
    method.agreement = agreement.digest().to_owned();
    method.inputs.insert(
        "target".to_owned(),
        PublishedInput::Choice {
            values: ["test-a", "test-b"]
                .into_iter()
                .map(|value| BoundedJson::new(serde_json::json!(value)))
                .collect::<Result<_, _>>()?,
        },
    );
    fixture.published.publish(
        method,
        Some(1),
        &fixture.decision,
        &milkdrift_persistence::IntegrityDigest::hash(b"public-input-method"),
    )?;
    let other = publication_grant("human:other", "grant:other")?;
    let other_context = ActorAuthorityContext::new(
        other.actor().clone(),
        CommandAuthorityClaim::new(other.identity().clone(), 1, other.digest()?, 0)?,
    );
    let mut children = Vec::new();
    for (index, (target, context)) in [
        ("test-a", &fixture.context),
        ("test-b", &other_context),
        ("production", &fixture.context),
    ]
    .into_iter()
    .enumerate()
    {
        let outer_base = base_revision(&format!("outer-input-{index}"))?;
        let work = task_node("work", "method.invoke")?.with_data_input(
            PortId::new("target")?,
            DataPort::input(
                schema.clone(),
                true,
                Some(BindingSource::Literal {
                    value: BoundedJson::new(serde_json::json!(target))?,
                }),
            )?,
        )?;
        let outer = outer_base.revise(
            outer_base.id(),
            MutationBatch::new(vec![Mutation::ReplaceNode { node: work }])?,
            AuthorRef::new("human:caller")?,
            "supply a public target",
        )?;
        fixture.store.put_revision(&outer_base)?;
        fixture.store.put_revision(&outer)?;
        let run = RunId::new(format!("input-parent-{index}"))?;
        revision_and_lifecycle::create_and_start(
            &fixture.control,
            &fixture.runtime,
            context,
            &run,
            &outer,
        )?;
        runtime_tick(&fixture.runtime)?;
        let plan = fixture
            .store
            .published_local_page(None, PageSize::new(8)?)?
            .0
            .pop();
        if target == "production" {
            assert!(
                plan.is_none(),
                "unapproved input must fail before child association"
            );
            continue;
        }
        let plan = plan.ok_or("approved input was not accepted")?;
        assert_eq!(plan.caller.request().actor, *context.actor());
        for _ in 0..48 {
            fixture.clock.advance(1)?;
            runtime_tick(&fixture.runtime)?;
            if fixture.runtime.projection(&run)?.lifecycle().is_completed() {
                break;
            }
        }
        assert_eq!(
            fixture.runtime.projection(&run)?.lifecycle(),
            RunLifecycle::Terminal(RunOutcome::Succeeded)
        );
        let child = fixture.runtime.projection(&plan.child_run)?;
        let reference = child
            .workspace_values()
            .iter()
            .find(|reference| reference.key().as_str() == "target")
            .ok_or("child input absent")?;
        let entry = fixture
            .store
            .value(reference)?
            .ok_or("input bytes absent")?;
        assert_eq!(
            entry.value(),
            &milkdrift_workspace::WorkspaceValue::Json(BoundedJson::new(serde_json::json!(
                target
            ))?)
        );
        children.push(plan.child_run);
    }
    assert_ne!(children[0], children[1]);
    assert_eq!(
        fixture.process.entries(),
        4,
        "only the two approved method runs may enter"
    );
    Ok(())
}
