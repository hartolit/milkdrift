//! Storage adoption cannot silently replace authority already frozen by older Controller work.

use super::support::*;
use milkdrift_authority::{
    ActorRef, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation, GrantId,
    GrantSetEvaluator, PolicyId,
};
use milkdrift_blueprint::{
    NewTask, TaskDefinition, TaskId, TaskInputBinding, TaskInputValue, WorkflowCreate,
    WorkflowLimits,
};
use milkdrift_capability::{CapabilityDescriptorDocument, CapabilityRequirement};
use milkdrift_control::{ActorAuthorityContext, AuthorityPreset, TaskControlService};
use milkdrift_runtime::{
    CommandAuthorityClaim, DeterministicExecutor, SequentialIdGenerator, SystemBoundaryClock,
    TaskRuntime, WorkflowRecord,
};
use serde_json::json;

fn previous_controller(config: &ActorBindingConfig) -> TestResult<AuthorityGrant> {
    let current = AuthorityPreset::Controller
        .template(
            GrantId::new(&config.grant_id)?,
            1,
            ActorRef::new(&config.actor)?,
            config.authority.resources.workflow_run.clone(),
            config.authority.resources.capability.clone(),
            config.authority.budget,
        )
        .resources(config.authority.resources.clone())
        .validity(config.authority.valid_from, config.authority.valid_until)
        .revocation_generation(config.revocation_generation)
        .build()?;
    // Construct the prior immutable grant before accepting any workflow, never patch its saved claim.
    Ok(
        AuthorityGrantBuilder::new(current.identity().clone(), 1, current.actor().clone())
            .operations(
                current
                    .operations()
                    .iter()
                    .copied()
                    .filter(|operation| *operation != AuthorityOperation::RenameDaemon)
                    .collect(),
            )
            .resources(current.resources().clone())
            .budget(current.budget())
            .validity(current.valid_from(), current.valid_until())
            .revocation_generation(current.revocation_generation())
            .build()?,
    )
}

fn save_previous_work(
    config: &DaemonConfig,
    request: &CreateWorkflowRequest,
) -> TestResult<(
    milkdrift_control_protocol::tasks::WorkflowReceipt,
    milkdrift_authority::ExecutionAuthorityBasis,
)> {
    let grant = previous_controller(config.actors.first().ok_or("controller absent")?)?;
    let authority = Arc::new(GrantSetEvaluator::new(
        PolicyId::new("daemon.authority.v1")?,
        1,
        [grant.clone()],
        BTreeMap::new(),
    )?);
    let context = ActorAuthorityContext::new(
        grant.actor().clone(),
        CommandAuthorityClaim::new(
            grant.identity().clone(),
            grant.revision(),
            grant.digest()?,
            grant.revocation_generation(),
        )?,
    );
    let mut descriptor: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../crates/capability/tests/fixtures/descriptor-v1.json"
    ))?;
    let body = descriptor
        .get_mut("descriptor")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("descriptor absent")?;
    body.insert("identity".into(), json!("golden-local-process"));
    body.insert("category".into(), json!({"type":"process"}));
    let operations = body
        .get_mut("operations")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("operations absent")?;
    let operation = operations
        .remove("model.generate")
        .ok_or("operation absent")?;
    operations.insert("process.execute".into(), operation);
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&descriptor)?)?;
    let executor = Arc::new(DeterministicExecutor::new(descriptor.body().clone()));
    let store = Arc::new(RedbStore::open(&config.data_root)?);
    let clock = Arc::new(SystemBoundaryClock);
    let runtime = Arc::new(TaskRuntime::new(
        store,
        executor.clone(),
        authority.clone(),
        clock.clone(),
        Arc::new(SequentialIdGenerator::new("prior-controller", 1)?),
    ));
    let control = TaskControlService::new(runtime.clone(), authority, clock);
    let original = request.tasks.first().ok_or("task absent")?;
    let definition = &original.definition;
    let typed = WorkflowCreate {
        title: request.title.clone(),
        description: request.description.clone(),
        tasks: vec![NewTask {
            alias: TaskId::new(&original.alias)?,
            definition: TaskDefinition {
                title: definition.title.clone(),
                description: definition.description.clone(),
                instructions: definition.instructions.clone(),
                capability: CapabilityRequirement::new(OperationId::new(
                    &definition.capability.operation,
                )?)
                .exact(CapabilityId::new(&definition.capability.id)?),
                inputs: BTreeMap::from([(
                    "brief".into(),
                    TaskInputBinding::Supplied {
                        name: "brief".into(),
                        optional: false,
                    },
                )]),
                dependencies: Default::default(),
                outputs: BTreeMap::new(),
                required: true,
                corrective_of: None,
                paths: vec![],
                tools: vec![],
                actions: None,
            },
        }],
        inputs: BTreeMap::from([(
            "brief".into(),
            TaskInputValue::Text {
                text: "Captured project brief".into(),
            },
        )]),
        limits: WorkflowLimits::default(),
        finish_when_settled: true,
    };
    let canonical = serde_json::to_vec(&serde_json::to_value(request)?)?;
    let receipt = control.create_exact(&context, &request.request_id, typed, &canonical)?;
    let record = runtime
        .workflow(&receipt.workflow)?
        .ok_or("retained workflow absent")?;
    assert_eq!(record.execution_authority.grant_digest(), &grant.digest()?);
    assert_eq!(executor.entry_count(), 0);
    let mut public_result = receipt.acceptance.result;
    public_result
        .as_object_mut()
        .ok_or("receipt result absent")?
        .insert("request_id".to_owned(), json!(request.request_id));
    Ok((
        serde_json::from_value(public_result)?,
        record.execution_authority,
    ))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_controller_grants_refuse_old_entry_without_rewriting_claims_or_receipts()
-> TestResult {
    for revision in [1, 2] {
        let directory = TempDir::new()?;
        let profile = configured_process_profile(&directory)?;
        let mut profile_document: serde_json::Value = serde_json::from_slice(&fs::read(&profile)?)?;
        *profile_document
            .pointer_mut("/profile/arguments")
            .ok_or("arguments absent")? =
            json!(["placement", directory.path(), "controller-upgrade"]);
        *profile_document
            .pointer_mut("/profile/side_effect")
            .ok_or("effect absent")? = json!("non_idempotent_write");
        fs::write(&profile, serde_json::to_vec(&profile_document)?)?;
        fs::write(directory.path().join("repository.txt"), "preserved-work")?;
        let mut config =
            configuration_document_with_process_profiles(&directory, 32, vec![profile])?;
        config
            .actors
            .first_mut()
            .ok_or("controller absent")?
            .grant_revision = revision;
        let request = create_request("prior-controller-work", true);
        let (accepted, original_basis) = save_previous_work(&config, &request)?;
        let daemon = start(config.validate(directory.path())?, CONTROLLER_TOKEN).await?;
        let task = accepted.aliases.get("inspect").ok_or("task alias absent")?;
        let refused = wait_task(
            &daemon.client,
            &accepted.workflow_id,
            task,
            TaskStateDto::Failed,
        )
        .await?;
        assert!(
            refused
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains(if revision == 1 {
                    "GrantDigestMismatch"
                } else {
                    "GrantRevisionMismatch"
                })),
            "unexpected refusal: {:?}",
            refused.reason
        );
        assert!(
            refused.started_at.is_none() && refused.frozen.is_none() && refused.result.is_none()
        );
        assert!(!directory.path().join("entries").exists());
        assert_eq!(
            daemon.client.workflow(&accepted.workflow_id).await?.title,
            request.title
        );
        assert_eq!(
            daemon.client.workflow_receipt(&request.request_id).await?,
            accepted
        );
        assert_eq!(daemon.client.create_workflow(&request).await?, accepted);
        // The configured process can enter under a new, explicitly current grant claim.
        let current = daemon
            .client
            .create_workflow(&create_request("current-controller-work", true))
            .await?;
        wait_task(
            &daemon.client,
            &current.workflow_id,
            current
                .aliases
                .get("inspect")
                .ok_or("current alias absent")?,
            TaskStateDto::Completed,
        )
        .await?;
        assert_eq!(
            fs::read_to_string(directory.path().join("entries"))?,
            "host=controller-upgrade repository=preserved-work\n"
        );
        daemon.stop().await?;
        let store = RedbStore::open(directory.path().join("data"))?;
        let record: WorkflowRecord = serde_json::from_value(
            store
                .workflow(&WorkflowId::new(&accepted.workflow_id)?)?
                .ok_or("workflow absent after restart")?
                .document,
        )?;
        assert_eq!(record.execution_authority, original_basis);
        assert!(record.tasks.values().all(|task| task.frozen.is_none()));
        assert_eq!(
            store
                .workflow_receipt("human:integration-controller", &request.request_id)?
                .ok_or("receipt absent")?
                .acceptance
                .result,
            json!({"workflow_id":accepted.workflow_id,"generation":accepted.generation,"aliases":accepted.aliases})
        );
    }
    Ok(())
}
