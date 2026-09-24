//! Resource acceptance is part of the real journal transaction, including guarded child transfer.
use super::*;
use milkdrift_authority::ExecutionAuthorityBasis;
use milkdrift_capability::{
    CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityRequirement, InvocationId,
    ResolvedCapabilitySnapshot, managed::*,
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
    value["descriptor"]["descriptor_revision"] = json!(1);
    value["descriptor"]["extensions"][MANAGED_BINDING_EXTENSION] = serde_json::to_value(binding)?;
    let descriptor = CapabilityDescriptorDocument::from_json(&serde_json::to_vec(&value)?)?
        .body()
        .clone();
    value["descriptor"]["identity"] = json!("independent-model");
    value["descriptor"]["extensions"][MANAGED_BINDING_EXTENSION]["resources"] =
        json!([{"resource":"attached","mutation":false}]);
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

fn append(
    store: &RedbStore,
    run: &RunId,
    key: &str,
    events: Vec<RunEventKind>,
    scopes: Vec<WorkspaceMutation>,
) -> TestResult<RunSequence> {
    let head = store.head(run)?;
    let request = accepted_workspace_followup_request(
        run.clone(),
        head,
        key,
        key,
        events,
        scopes,
        WorkspaceAccounting {
            budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        },
    )?;
    store.commit_command(&request)?;
    Ok(store.head(run)?)
}
fn create_run(
    store: &RedbStore,
    run: &RunId,
    basis: ExecutionAuthorityBasis,
) -> TestResult<WorkspaceScope> {
    let scope = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    append(
        store,
        run,
        &format!("create-{run}"),
        vec![
            run_created_kind(
                scope.clone(),
                WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
                Vec::new(),
            )?,
            RunEventKind::ExecutionAuthorityEstablished { basis },
        ],
        vec![WorkspaceMutation::CreateScope {
            scope: scope.clone(),
        }],
    )?;
    Ok(scope)
}
fn schedule(
    store: &RedbStore,
    run: &RunId,
    suffix: &str,
    descriptor: &CapabilityDescriptor,
) -> TestResult<String> {
    let invocation = InvocationId::new(format!("invocation-{suffix}"))?;
    let operation = OperationId::new("model.generate")?;
    let execution = NodeExecutionId::new(format!("execution-{suffix}"))?;
    let attempt = AttemptId::new(format!("attempt-{suffix}"))?;
    let request = InvocationRequest::new(
        invocation.clone(),
        descriptor.identity().clone(),
        operation.clone(),
        descriptor.provider_profile().cloned(),
        None,
        Vec::new(),
        Default::default(),
    )?;
    append(
        store,
        run,
        &format!("schedule-{suffix}"),
        vec![
            RunEventKind::CapabilityResolved {
                execution: execution.clone(),
                attempt: attempt.clone(),
                requirement: CapabilityRequirement::new(operation.clone())
                    .exact(descriptor.identity().clone()),
                snapshot: ResolvedCapabilitySnapshot::from_descriptor(descriptor, &operation)?,
            },
            RunEventKind::NodeScheduled {
                node: NodeId::new("worker")?,
                execution,
                attempt,
                invocation: invocation.clone(),
                idempotency_key: None,
                request,
            },
        ],
        Vec::new(),
    )?;
    Ok(managed_use_id(&invocation))
}
fn enter(store: &RedbStore, run: &RunId, suffix: &str, id: &str, claim: u64) -> TestResult {
    let auth = command(store, &format!("entry-{suffix}"), ManagedAction::Inspect {})?;
    append(
        store,
        run,
        &format!("entry-{suffix}"),
        vec![RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: AttemptId::new(format!("attempt-{suffix}"))?,
            authorization: authority(&auth, AuthorityOperation::InvokeCapability)?,
            controller_admission: ControllerAdmissionOutcome::NotControlled,
        }],
        Vec::new(),
    )?;
    store.enter_managed_use(id, claim, &format!("mdtask-{id}"))?;
    Ok(())
}
fn stopped(id: &str) -> QuiescenceEvidence {
    QuiescenceEvidence::PhysicalStop {
        physical_identity: format!("mdtask-{id}"),
        observation_digest: digest(),
        disrupted: false,
    }
}

#[test]
fn journal_acceptance_and_resource_hold_commit_or_refuse_together() -> TestResult {
    let root = TempDir::new()?;
    let store = RedbStore::open(root.path())?;
    let descriptor = setup(&store)?;
    let run = RunId::new("parent")?;
    let start = authority(
        &command(&store, "start", ManagedAction::Inspect {})?,
        AuthorityOperation::StartRun,
    )?;
    let basis = ExecutionAuthorityBasis::from_start_decision(
        &start,
        WorkflowId::new("workflow-test")?,
        run.clone(),
        revision_id()?,
    )?;
    create_run(&store, &run, basis)?;
    let id = schedule(&store, &run, "first", &descriptor)?;
    let head = store.head(&run)?;
    assert!(schedule(&store, &run, "conflict", &descriptor).is_err());
    assert_eq!(store.head(&run)?, head);
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
    append(
        &store,
        &run,
        "uncertain",
        vec![RunEventKind::NodeTerminal {
            execution: NodeExecutionId::new("execution-first")?,
            attempt: AttemptId::new("attempt-first")?,
            report_sequence: 1,
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        }],
        Vec::new(),
    )?;
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
fn exact_child_handoff_restart_fencing_and_authorized_return_never_have_two_editors() -> TestResult
{
    handoff_case(false)?;
    handoff_case(true)?;
    Ok(())
}

fn handoff_case(cancelled: bool) -> TestResult {
    let root = TempDir::new()?;
    let store = RedbStore::open(root.path())?;
    let descriptor = setup(&store)?;
    let parent = RunId::new("parent")?;
    let child = RunId::new("child")?;
    let start = authority(
        &command(&store, "start", ManagedAction::Inspect {})?,
        AuthorityOperation::StartRun,
    )?;
    let basis = ExecutionAuthorityBasis::from_start_decision(
        &start,
        WorkflowId::new("workflow-test")?,
        parent.clone(),
        revision_id()?,
    )?;
    let parent_root = create_run(&store, &parent, basis.clone())?;
    create_run(&store, &child, basis)?;
    let parent_use = schedule(&store, &parent, "parent", &descriptor)?;
    enter(&store, &parent, "parent", &parent_use, 1)?;
    assert!(schedule(&store, &child, "premature-child", &descriptor).is_err());
    store.quiesce_managed_use(&parent_use, 1, &stopped(&parent_use))?;
    assert!(schedule(&store, &child, "unlinked-child", &descriptor).is_err());
    let scope = WorkspaceScope::subworkflow(
        ScopeId::new("child-scope")?,
        &parent_root,
        SubworkflowId::new("child-link")?,
    )?;
    let link = append(
        &store,
        &parent,
        "associate",
        vec![RunEventKind::SubworkflowCreated {
            subworkflow: SubworkflowId::new("child-link")?,
            parent_execution: NodeExecutionId::new("execution-parent")?,
            child_run: child.clone(),
            child_revision: revision_id()?,
            scope: scope.clone(),
            ownership: SubworkflowOwnership::Attached,
            inputs: Vec::new(),
        }],
        vec![WorkspaceMutation::CreateScope { scope }],
    )?;
    let child_use = schedule(&store, &child, "child", &descriptor)?;
    assert!(enter(&store, &child, "child", &child_use, 1).is_err());
    let transfer = EditingHandoff {
        installation: name("installation")?,
        generation: 1,
        parent: parent_use.clone(),
        child: child_use.clone(),
        parent_claim: 1,
        child_claim: 1,
        association: format!("{parent}:{}", link.get()),
    };
    let request = command(
        &store,
        "handoff",
        ManagedAction::Handoff {
            transfer: transfer.clone(),
        },
    )?;
    let mut wrong = request.clone();
    if let ManagedAction::Handoff { transfer } = &mut wrong.action {
        transfer.association = "parent:99999".to_owned();
    }
    assert!(
        store
            .transfer_managed_editing(
                &wrong,
                &authority(&wrong, AuthorityOperation::AdministerCapabilities)?
            )
            .is_err()
    );
    let receipt = store.transfer_managed_editing(
        &request,
        &authority(&request, AuthorityOperation::AdministerCapabilities)?,
    )?;
    assert!(
        store
            .enter_managed_use(&parent_use, 1, "old-writer")
            .is_err()
    );
    assert!(
        store
            .enter_managed_use(&parent_use, 2, "new-writer")
            .is_err()
    );
    drop(store);
    let store = RedbStore::open(root.path())?;
    store.verify_managed_integrity()?;
    assert_eq!(
        store.transfer_managed_editing(
            &request,
            &authority(&request, AuthorityOperation::AdministerCapabilities)?
        )?,
        receipt
    );
    enter(&store, &child, "child", &child_use, 2)?;
    assert!(schedule(&store, &parent, "unrelated", &descriptor).is_err());
    let model = store
        .managed_installation(&name("installation")?)?
        .ok_or("inventory absent")?
        .current
        .ok_or("setup absent")?
        .capabilities
        .into_iter()
        .find(|d| d.identity().as_str() == "independent-model")
        .ok_or("independent reader absent")?;
    let model_use = schedule(&store, &parent, "model-reader", &model)?;
    append(
        &store,
        &parent,
        "cancel-reader-before-entry",
        vec![RunEventKind::NodeTerminal {
            execution: NodeExecutionId::new("execution-model-reader")?,
            attempt: AttemptId::new("attempt-model-reader")?,
            report_sequence: 1,
            outcome: NodeOutcome::Cancelled,
            error_class: None,
            detail: None,
        }],
        Vec::new(),
    )?;
    assert!(store.managed_use(&model_use)?.is_none());

    let resolve = command(
        &store,
        "resolve-child",
        ManagedAction::Resolve {
            use_id: child_use.clone(),
            expected_claim: 2,
        },
    )?;
    store.begin_managed_resolution(
        &resolve,
        &authority(&resolve, AuthorityOperation::AdministerCapabilities)?,
    )?;
    store.quiesce_managed_use(&child_use, 3, &stopped(&child_use))?;
    assert!(
        store.release_managed_use(&child_use, 3).is_err(),
        "parent lifetime relationship is still retained"
    );
    if cancelled {
        append(
            &store,
            &parent,
            "parent-cancelled",
            vec![RunEventKind::NodeTerminal {
                execution: NodeExecutionId::new("execution-parent")?,
                attempt: AttemptId::new("attempt-parent")?,
                report_sequence: 1,
                outcome: NodeOutcome::Cancelled,
                error_class: None,
                detail: None,
            }],
            Vec::new(),
        )?;
    }
    let returned = command(
        &store,
        "return",
        ManagedAction::Return {
            transfer: EditingHandoff {
                parent_claim: 2,
                child_claim: 3,
                ..transfer
            },
            resume_parent: true,
        },
    )?;
    let mut returned = returned;
    if cancelled {
        assert!(
            store
                .transfer_managed_editing(
                    &returned,
                    &authority(&returned, AuthorityOperation::AdministerCapabilities)?
                )
                .is_err()
        );
        if let ManagedAction::Return { resume_parent, .. } = &mut returned.action {
            *resume_parent = false;
        }
    }
    store.transfer_managed_editing(
        &returned,
        &authority(&returned, AuthorityOperation::AdministerCapabilities)?,
    )?;
    assert!(store.managed_use(&child_use)?.is_none());
    assert!(
        store
            .enter_managed_use(&parent_use, 1, "old-writer")
            .is_err()
    );
    if cancelled {
        assert!(
            store
                .enter_managed_use(&parent_use, 3, &format!("mdtask-{parent_use}"))
                .is_err()
        );
    } else {
        store.enter_managed_use(&parent_use, 3, &format!("mdtask-{parent_use}"))?;
        store.quiesce_managed_use(&parent_use, 3, &stopped(&parent_use))?;
    }
    store.release_managed_use(&parent_use, 3)?;
    store.verify_managed_integrity()?;
    assert!(
        store
            .managed_installation(&name("installation")?)?
            .ok_or("inventory absent")?
            .uses
            .is_empty()
    );
    Ok(())
}
