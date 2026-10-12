//! Managed resource acceptance commits atomically with frozen task facts and retains physical fencing.
use super::*;
use milkdrift_capability::{
    CapabilityDescriptor, CapabilityDescriptorDocument, InvocationId, ResolvedCapabilitySnapshot,
    managed::*,
};
use milkdrift_persistence::managed::*;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn digest() -> String {
    format!("b3_{}", "1".repeat(64))
}
fn name(value: &str) -> TestResult<ManagedName> {
    Ok(ManagedName::new(value)?)
}
fn command(store: &RedbStore, key: &str, action: ManagedAction) -> TestResult<ManagedRequest> {
    Ok(ManagedRequest {
        schema_version: milkdrift_capability::managed::MANAGED_SCHEMA_VERSION,
        command: name(key)?,
        installation: name("installation")?,
        expected_version: store
            .managed_installation(&name("installation")?)?
            .map_or(0, |r| r.version),
        action,
    })
}
fn authority(
    request: &ManagedRequest,
    operation: AuthorityOperation,
) -> TestResult<AuthorityDecisionSnapshot> {
    let mut resources = RequestedResourceFacts::empty();
    resources.capability = Some(CapabilityId::new("managed.installation")?);
    resources.capability_operation = Some(OperationId::new(request.operation())?);
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("test-managed")?,
        1,
        AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", request.command))?,
            actor: ActorRef::new("actor-test")?,
            grant: GrantId::new("grant:managed")?,
            grant_revision: 1,
            grant_digest: GrantDigest::new(digest())?,
            revocation_generation: 0,
            operation,
            resources,
            budget: AuthorityBudget::default(),
            evaluated_at: BoundaryTimeMillis::new(10),
            provenance: Default::default(),
        },
        vec![DecisionReasonCode::Allowed],
        AuthorityBudget::default(),
        SideEffectClass::Unknown,
    )?)
}
fn setup(store: &RedbStore) -> TestResult<CapabilityDescriptor> {
    let binding = ManagedBinding {
        installation: name("installation")?,
        generation: 1,
        recipe_digest: digest(),
        resources: vec![ResourceRequirement {
            resource: name("working")?,
            mutation: true,
        }],
    };
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../crates/capability/tests/fixtures/descriptor-v1.json"
    ))?;
    *value
        .pointer_mut("/descriptor/descriptor_revision")
        .ok_or("missing descriptor revision")? = json!(1);
    value
        .pointer_mut("/descriptor/extensions")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("missing descriptor extensions")?
        .insert(
            MANAGED_BINDING_EXTENSION.to_owned(),
            serde_json::to_value(binding)?,
        );
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
        .body()
        .clone();
    *value
        .pointer_mut("/descriptor/identity")
        .ok_or("missing descriptor identity")? = json!("independent-model");
    *value
        .pointer_mut("/descriptor/extensions")
        .and_then(|extensions| extensions.get_mut(MANAGED_BINDING_EXTENSION))
        .and_then(|binding| binding.get_mut("resources"))
        .ok_or("missing binding resources")? = json!([{"resource":"attached","mutation":false}]);
    let reader = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
        .body()
        .clone();
    let recipe = RecipeReference {
        name: name("recipe")?,
        digest: digest(),
    };
    let approved = ApprovedSetup {
        protection: None,
        recipe: recipe.clone(),
        mechanism: "test-verified".to_owned(),
        configuration: BoundedJson::new(json!({}))?,
        platform_owner: "test-owner".to_owned(),
        ownership: digest(),
        resources: vec![
            ManagedResourceView {
                name: name("working")?,
                kind: ManagedResourceKind::WorkingArea,
                ownership: ResourceOwnership::Owned,
                identity: "exact-working-volume".to_owned(),
                disposition: DataDisposition::Preserve,
            },
            ManagedResourceView {
                name: name("attached")?,
                kind: ManagedResourceKind::Service,
                ownership: ResourceOwnership::Attached,
                identity: "http://external/v1".to_owned(),
                disposition: DataDisposition::Preserve,
            },
        ],
        capabilities: vec![descriptor.clone(), reader],
    };
    let request = command(store, "install", ManagedAction::Apply { recipe })?;
    store.begin_managed_change(
        &request,
        &authority(&request, AuthorityOperation::AdministerCapabilities)?,
        &ManagedChange {
            entry_authorization: None,
            identity: digest(),
            candidate: approved,
            generation: 1,
            steps: vec![ManagedStep::Verify],
            removing: false,
            running: false,
        },
    )?;
    store.advance_managed_change(
        &request.installation,
        &digest(),
        0,
        ManagedObservation {
            digest: digest(),
            summary: "test platform verified".to_owned(),
            running: false,
        },
    )?;
    Ok(descriptor)
}
#[test]
fn receipt_replay_rechecks_scope_inside_the_acceptance_transaction() -> TestResult {
    let root = TempDir::new()?;
    let store = RedbStore::open(root.path())?;
    setup(&store)?;
    let receipt = store
        .managed_receipt("actor-test", &name("install")?)?
        .ok_or("receipt missing")?;
    let change = ManagedChange {
        entry_authorization: None,
        identity: digest(),
        candidate: store
            .managed_installation(&name("installation")?)?
            .and_then(|record| record.current)
            .ok_or("setup missing")?,
        generation: 1,
        steps: vec![ManagedStep::Verify],
        removing: false,
        running: false,
    };
    assert_eq!(
        store.begin_managed_change(&receipt.request, &receipt.authorization, &change)?,
        receipt,
    );
    for changed in 0..3 {
        let mut claim = receipt.authorization.request().clone();
        match changed {
            0 => claim.resources.run = Some(RunId::new("different-run")?),
            1 => claim.grant_revision += 1,
            _ => claim.revocation_generation += 1,
        }
        let authorization = AuthorityDecisionSnapshot::from_evaluation(
            PolicyId::new("test-managed")?,
            1,
            claim,
            vec![DecisionReasonCode::Allowed],
            AuthorityBudget::default(),
            SideEffectClass::Unknown,
        )?;
        assert!(
            store
                .begin_managed_change(&receipt.request, &authorization, &change)
                .is_err()
        );
    }
    assert_eq!(
        store.managed_receipt("actor-test", &name("install")?)?,
        Some(receipt)
    );
    Ok(())
}

fn snapshot(store: &RedbStore, workflow: &WorkflowId) -> TestResult<WorkflowSnapshot> {
    Ok(store.workflow(workflow)?.unwrap_or(WorkflowSnapshot {
        workflow: workflow.clone(),
        generation: 0,
        title: "Managed task fixture".into(),
        description: String::new(),
        owner_actor: "actor-test".into(),
        updated_at: TimestampMillis::new(10),
        document: json!({"fixture":"managed acceptance"}),
        entered_invocations: Vec::new(),
        managed_executions: Vec::new(),
        artifact_capacity: 0,
        reserved_bytes: 65536,
        reserved_receipts: 0,
    }))
}
fn commit(store: &RedbStore, mut snapshot: WorkflowSnapshot) -> TestResult {
    let expected_generation = (snapshot.generation > 0).then_some(snapshot.generation);
    snapshot.generation += 1;
    store.commit_workflow(WorkflowCommit {
        expected_generation,
        snapshot,
        command: None,
    })?;
    Ok(())
}
fn schedule(
    store: &RedbStore,
    workflow: &WorkflowId,
    suffix: &str,
    descriptor: &CapabilityDescriptor,
) -> TestResult<String> {
    let invocation = InvocationId::new(format!("invocation-{suffix}"))?;
    let mut next = snapshot(store, workflow)?;
    next.managed_executions
        .push(milkdrift_persistence::WorkflowManagedExecution {
            task: TaskId::new(format!("task-{suffix}"))?,
            invocation: invocation.clone(),
            accepted_generation: next.generation + 1,
            selection: ResolvedCapabilitySnapshot::from_descriptor(
                descriptor,
                &OperationId::new("model.generate")?,
            )?,
            entered: false,
            settled: false,
        });
    commit(store, next)?;
    Ok(managed_use_id(&invocation))
}
fn enter(
    store: &RedbStore,
    workflow: &WorkflowId,
    suffix: &str,
    id: &str,
    claim: u64,
) -> TestResult {
    let mut next = snapshot(store, workflow)?;
    let invocation = InvocationId::new(format!("invocation-{suffix}"))?;
    next.managed_executions
        .iter_mut()
        .find(|fact| fact.invocation == invocation)
        .ok_or("accepted task absent")?
        .entered = true;
    next.entered_invocations.push(invocation);
    commit(store, next)?;
    store.enter_managed_use(id, claim, &format!("mdtask-{id}"))?;
    Ok(())
}
fn settle(store: &RedbStore, workflow: &WorkflowId, suffix: &str) -> TestResult {
    let mut next = snapshot(store, workflow)?;
    let invocation = InvocationId::new(format!("invocation-{suffix}"))?;
    next.managed_executions
        .iter_mut()
        .find(|fact| fact.invocation == invocation)
        .ok_or("accepted task absent")?
        .settled = true;
    commit(store, next)
}
fn stopped(id: &str) -> QuiescenceEvidence {
    QuiescenceEvidence::PhysicalStop {
        physical_identity: format!("mdtask-{id}"),
        observation_digest: digest(),
        disrupted: false,
    }
}

#[test]
fn task_acceptance_and_resource_hold_commit_or_refuse_together() -> TestResult {
    let root = TempDir::new()?;
    let store = RedbStore::open(root.path())?;
    let descriptor = setup(&store)?;
    let run = WorkflowId::new("parent")?;
    let id = schedule(&store, &run, "first", &descriptor)?;
    let head = store.workflow(&run)?;
    assert!(schedule(&store, &run, "conflict", &descriptor).is_err());
    assert_eq!(store.workflow(&run)?, head);
    let record = store.managed_use(&id)?.ok_or("accepted use missing")?;
    assert!(!record.entry_committed);
    assert!(store.enter_managed_use(&id, 1, "arbitrary").is_err());
    let maintenance = command(&store, "stop", ManagedAction::Stop {})?;
    let current = store
        .managed_installation(&name("installation")?)?
        .ok_or("inventory absent")?;
    assert!(
        store
            .begin_managed_change(
                &maintenance,
                &authority(&maintenance, AuthorityOperation::AdministerCapabilities)?,
                &ManagedChange {
                    entry_authorization: None,
                    identity: format!("b3_{}", "2".repeat(64)),
                    candidate: current.current.ok_or("setup absent")?,
                    generation: 1,
                    steps: vec![ManagedStep::StopService],
                    removing: false,
                    running: false
                }
            )
            .is_err()
    );
    enter(&store, &run, "first", &id, 1)?;
    settle(&store, &run, "first")?;
    assert!(
        store.managed_use(&id)?.is_some(),
        "terminal uncertainty is not stop evidence"
    );
    let resolve = command(
        &store,
        "resolve",
        ManagedAction::Resolve {
            use_id: id.clone(),
            expected_claim: 1,
        },
    )?;
    let receipt = store.begin_managed_resolution(
        &resolve,
        &authority(&resolve, AuthorityOperation::AdministerCapabilities)?,
    )?;
    assert!(
        store
            .enter_managed_use(&id, 1, &format!("mdtask-{id}"))
            .is_err()
    );
    assert!(schedule(&store, &run, "fencing-conflict", &descriptor).is_err());
    assert!(store.quiesce_managed_use(&id, 1, &stopped(&id)).is_err());
    store.quiesce_managed_use(&id, 2, &stopped(&id))?;
    store.release_managed_use(&id, 2)?;
    assert_eq!(
        store.begin_managed_resolution(
            &resolve,
            &authority(&resolve, AuthorityOperation::AdministerCapabilities)?
        )?,
        receipt
    );
    store.verify_managed_integrity()?;
    schedule(&store, &run, "after-fence", &descriptor)?;
    store.verify_managed_integrity()?;
    Ok(())
}

#[test]
fn unentered_settlement_releases_hold_and_frozen_descriptor_survives_reopen() -> TestResult {
    let root = TempDir::new()?;
    let store = RedbStore::open(root.path())?;
    let descriptor = setup(&store)?;
    let workflow = WorkflowId::new("managed-reopen")?;
    let id = schedule(&store, &workflow, "reserved", &descriptor)?;
    let accepted = store.workflow(&workflow)?.ok_or("workflow absent")?;
    drop(store);
    let store = RedbStore::open(root.path())?;
    assert_eq!(store.workflow(&workflow)?, Some(accepted.clone()));
    store.verify_managed_integrity()?;
    assert!(store.managed_use(&id)?.is_some());
    let mut altered = accepted;
    altered
        .managed_executions
        .first_mut()
        .ok_or("managed fact absent")?
        .accepted_generation += 1;
    assert!(commit(&store, altered).is_err());
    settle(&store, &workflow, "reserved")?;
    assert!(
        store.managed_use(&id)?.is_none(),
        "durable never-entered settlement must release reserved editing"
    );
    schedule(&store, &workflow, "replacement", &descriptor)?;
    store.verify_managed_integrity()?;
    Ok(())
}
