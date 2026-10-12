//! Ordinary commands preserve the same exact actor, operation and workflow grant boundary.
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityGrantBuilder, AuthorityOperation, GrantId,
    GrantSetEvaluator, PolicyId, WorkflowRunScope,
};
use milkdrift_blueprint::WorkflowCreate;
use milkdrift_capability::CapabilityDescriptorDocument;
use milkdrift_control::{ActorAuthorityContext, TaskControlService};
use milkdrift_persistence::WorkflowStore;
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, SequentialIdGenerator, SystemBoundaryClock,
    TaskRuntime,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn creating_future_work_does_not_supply_start_permission_or_widen_workflow_scope() -> TestResult {
    let root = tempfile::tempdir()?;
    let actor = ActorRef::new("human:scoped-editor")?;
    let workflow = TaskRuntime::workflow_identity(actor.as_str(), "allowed")?;
    let template = AuthorityGrantBuilder::new(GrantId::new("grant:editor")?, 1, actor.clone())
        .operations(BTreeSet::from([AuthorityOperation::CreateRun]))
        .build()?;
    let mut resources = template.resources().clone();
    resources.workflow_run = WorkflowRunScope::Workflow {
        workflow: workflow.clone(),
    };
    let grant = AuthorityGrantBuilder::new(template.identity().clone(), 1, actor.clone())
        .operations(BTreeSet::from([
            AuthorityOperation::CreateRun,
            AuthorityOperation::InspectRun,
        ]))
        .resources(resources)
        .budget(AuthorityBudget::default())
        .build()?;
    let context = ActorAuthorityContext::new(
        actor,
        CommandAuthorityClaim::new(grant.identity().clone(), 1, grant.digest()?, 0)?,
    );
    let evaluator = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("policy:task-control")?,
        1,
        [grant],
        BTreeMap::new(),
    )?);
    let descriptor = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let store = Arc::new(milkdrift_redb_store::RedbStore::open(root.path())?);
    let clock = Arc::new(SystemBoundaryClock);
    let runtime = Arc::new(TaskRuntime::new(
        store.clone(),
        Arc::new(DeterministicExecutor::new(descriptor.body().clone())),
        evaluator.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("control-test", 1)?),
    ));
    let control = TaskControlService::new(runtime, evaluator, clock);
    let capability = milkdrift_capability::CapabilityRequirement::new(
        milkdrift_capability::OperationId::new("model.generate")?,
    );
    let request: WorkflowCreate = serde_json::from_value(
        serde_json::json!({"title":"Exact scoped task","tasks":[{"alias":"inspect","definition":{"title":"Inspect","instructions":"Inspect the supplied evidence","capability":capability}}]}),
    )?;
    assert!(
        control
            .create(&context, "allowed", request.clone())
            .is_err()
    );
    assert!(
        control
            .create(&context, "another-workflow", request)
            .is_err()
    );
    assert!(store.workflows(None, 10)?.items.is_empty());
    assert!(control.workflow(&context, &workflow)?.is_none());
    let other = TaskRuntime::workflow_identity(context.actor().as_str(), "another-workflow")?;
    assert!(control.workflow(&context, &other).is_err());
    Ok(())
}
