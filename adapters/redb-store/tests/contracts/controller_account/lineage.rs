use super::*;

#[test]
fn controller_lineage_actions_require_exact_activation_and_child_events() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-lineage-owner")?;
    let initial_declaration = declaration(&owner, "lineage-owner")?;
    let unlinked_establishment = request(
        &owner,
        "command-controller-lineage-unlinked-establishment",
        "event-controller-lineage-unlinked-establishment",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-unlinked-establishment",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: initial_declaration.clone(),
            bind_run: owner.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&unlinked_establishment),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(
        store
            .controller_account(initial_declaration.account())?
            .is_none()
    );
    assert!(store.controller_account_binding(&owner)?.is_none());

    let unbound_assessment_run = RunId::new("run-controller-lineage-unbound-assessment")?;
    let unbound_assessment_declaration =
        declaration(&unbound_assessment_run, "lineage-unbound-assessment")?;
    let unbound_assessment = request(
        &unbound_assessment_run,
        "command-controller-lineage-unbound-assessment",
        "event-controller-lineage-unbound-assessment",
        RunSequence::ZERO,
        assessment(
            &unbound_assessment_declaration,
            "lineage-unbound-assessment",
            RunSequence::ZERO,
            ControllerAssessmentBoundary::CycleEntry,
        )?,
    )?;
    assert!(matches!(
        store.commit_command(&unbound_assessment),
        Err(PersistenceError::InvalidDocument(_))
    ));
    let unbound_activation = request(
        &owner,
        "command-controller-lineage-unbound-activation",
        "event-controller-lineage-unbound-activation",
        RunSequence::ZERO,
        activation(
            &initial_declaration,
            "lineage-unbound-activation",
            RunSequence::ZERO,
        )?,
    )?;
    assert!(matches!(
        store.commit_command(&unbound_activation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let mismatched = declaration(&owner, "lineage-mismatched-action")?;
    let mismatched_establishment = request(
        &owner,
        "command-controller-lineage-mismatched-establishment",
        "event-controller-lineage-mismatched-establishment",
        RunSequence::ZERO,
        activation(
            &initial_declaration,
            "lineage-mismatched-establishment",
            RunSequence::ZERO,
        )?,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-mismatched-establishment",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: mismatched.clone(),
            bind_run: owner.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&mismatched_establishment),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(store.controller_account(mismatched.account())?.is_none());
    assert!(store.controller_account_binding(&owner)?.is_none());

    let declaration = establish(&store, &owner, "lineage-owner")?;
    let foreign_owner = RunId::new("run-controller-lineage-foreign-owner")?;
    let foreign_declaration = establish(&store, &foreign_owner, "lineage-foreign-owner")?;
    let unbound_parent = RunId::new("run-controller-lineage-unbound-parent")?;
    let illicit_child = RunId::new("run-controller-lineage-illicit-child")?;
    let unbound_binding = request(
        &unbound_parent,
        "command-controller-lineage-unbound-binding",
        "event-controller-lineage-unbound-binding",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-unbound-binding",
        None,
        vec![ControllerAccountAction::BindRun {
            account: declaration.account().clone(),
            run: illicit_child.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&unbound_binding),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(store.controller_account_binding(&illicit_child)?.is_none());

    let established_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent")?
        .through_sequence;
    let bound_activation = request(
        &owner,
        "command-controller-lineage-bound-activation",
        "event-controller-lineage-bound-activation",
        established_head,
        activation(&declaration, "lineage-bound-activation", established_head)?,
    )?;
    let _ = store.commit_command(&bound_activation)?;

    let activated_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent after bound activation")?
        .through_sequence;
    let conflicting_activation = request(
        &owner,
        "command-controller-lineage-conflicting-activation",
        "event-controller-lineage-conflicting-activation",
        activated_head,
        activation(
            &mismatched,
            "lineage-conflicting-activation",
            activated_head,
        )?,
    )?;
    assert!(matches!(
        store.commit_command(&conflicting_activation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let bound_assessment = request(
        &owner,
        "command-controller-lineage-bound-assessment",
        "event-controller-lineage-bound-assessment",
        activated_head,
        assessment(
            &declaration,
            "lineage-bound-assessment",
            activated_head,
            ControllerAssessmentBoundary::CycleEntry,
        )?,
    )?;
    let _ = store.commit_command(&bound_assessment)?;
    let assessed_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent after bound assessment")?
        .through_sequence;
    let altered_declaration = ControllerAccountDeclaration::new(
        owner.clone(),
        declaration.controller_execution().clone(),
        declaration.policy_digest().to_owned(),
        ControllerResourceBudget::new(
            1_000_001,
            CurrencyCode::new("USD")?,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000,
            1_000,
        )?,
    )?;
    assert_eq!(altered_declaration.account(), declaration.account());
    assert_ne!(altered_declaration, declaration);
    let altered_assessment = request(
        &owner,
        "command-controller-lineage-altered-assessment",
        "event-controller-lineage-altered-assessment",
        assessed_head,
        assessment(
            &altered_declaration,
            "lineage-altered-assessment",
            assessed_head,
            ControllerAssessmentBoundary::CycleEntry,
        )?,
    )?;
    assert!(matches!(
        store.commit_command(&altered_assessment),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let multi_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent before child creation")?
        .through_sequence;
    let first_multi_child = RunId::new("run-controller-lineage-multi-child-one")?;
    let second_multi_child = RunId::new("run-controller-lineage-multi-child-two")?;
    let first_subworkflow = SubworkflowId::new("subworkflow-controller-lineage-multi-one")?;
    let second_subworkflow = SubworkflowId::new("subworkflow-controller-lineage-multi-two")?;
    let first_scope = WorkspaceScope::subworkflow(
        ScopeId::new("scope-controller-lineage-multi-one")?,
        &controller_root(&declaration)?,
        first_subworkflow.clone(),
    )?;
    let second_scope = WorkspaceScope::subworkflow(
        ScopeId::new("scope-controller-lineage-multi-two")?,
        &controller_root(&declaration)?,
        second_subworkflow.clone(),
    )?;
    let multi_child_binding = request_many_with_workspace(
        &owner,
        "command-controller-lineage-multi-child",
        "event-controller-lineage-multi-child",
        multi_head,
        vec![
            RunEventKind::SubworkflowCreated {
                subworkflow: first_subworkflow,
                parent_execution: declaration.controller_execution().clone(),
                child_run: first_multi_child.clone(),
                child_revision: revision_id()?,
                scope: first_scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs: Vec::new(),
            },
            RunEventKind::SubworkflowCreated {
                subworkflow: second_subworkflow,
                parent_execution: declaration.controller_execution().clone(),
                child_run: second_multi_child.clone(),
                child_revision: revision_id()?,
                scope: second_scope.clone(),
                ownership: SubworkflowOwnership::Attached,
                inputs: Vec::new(),
            },
        ],
        vec![
            WorkspaceMutation::CreateScope { scope: first_scope },
            WorkspaceMutation::CreateScope {
                scope: second_scope,
            },
        ],
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-multi-child",
        None,
        vec![
            ControllerAccountAction::BindRun {
                account: declaration.account().clone(),
                run: first_multi_child.clone(),
            },
            ControllerAccountAction::BindRun {
                account: declaration.account().clone(),
                run: second_multi_child.clone(),
            },
        ],
    )?)?;
    let _ = store.commit_command(&multi_child_binding)?;
    assert_eq!(
        store.controller_account_binding(&first_multi_child)?,
        Some(declaration.account().clone())
    );
    assert_eq!(
        store.controller_account_binding(&second_multi_child)?,
        Some(declaration.account().clone())
    );

    let cross_account_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent before hostile child creation")?
        .through_sequence;
    let cross_account_child = RunId::new("run-controller-lineage-cross-account-child")?;
    let cross_account_subworkflow =
        SubworkflowId::new("subworkflow-controller-lineage-cross-account")?;
    let cross_account_scope = WorkspaceScope::subworkflow(
        ScopeId::new("scope-controller-lineage-cross-account")?,
        &controller_root(&declaration)?,
        cross_account_subworkflow.clone(),
    )?;
    let cross_account_binding = request_with_workspace(
        &owner,
        "command-controller-lineage-cross-account",
        "event-controller-lineage-cross-account",
        cross_account_head,
        RunEventKind::SubworkflowCreated {
            subworkflow: cross_account_subworkflow,
            parent_execution: declaration.controller_execution().clone(),
            child_run: cross_account_child.clone(),
            child_revision: revision_id()?,
            scope: cross_account_scope.clone(),
            ownership: SubworkflowOwnership::Attached,
            inputs: Vec::new(),
        },
        vec![WorkspaceMutation::CreateScope {
            scope: cross_account_scope,
        }],
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-cross-account",
        None,
        vec![ControllerAccountAction::BindRun {
            account: foreign_declaration.account().clone(),
            run: cross_account_child.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&cross_account_binding),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(
        store
            .controller_account_binding(&cross_account_child)?
            .is_none()
    );

    let child = RunId::new("run-controller-lineage-child")?;
    let parent_head = store
        .run_summary(&owner)?
        .ok_or("controller lineage owner summary is absent")?
        .through_sequence;
    let unlinked_binding = request(
        &owner,
        "command-controller-lineage-unlinked-binding",
        "event-controller-lineage-unlinked-binding",
        parent_head,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-lineage-unlinked-binding",
        None,
        vec![ControllerAccountAction::BindRun {
            account: declaration.account().clone(),
            run: child.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&unlinked_binding),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(store.controller_account_binding(&child)?.is_none());

    let subworkflow = SubworkflowId::new("subworkflow-controller-lineage-child")?;
    let scope = WorkspaceScope::subworkflow(
        ScopeId::new("scope-controller-lineage-child")?,
        &controller_root(&declaration)?,
        subworkflow.clone(),
    )?;
    let missing_binding = request_with_workspace(
        &owner,
        "command-controller-lineage-missing-binding",
        "event-controller-lineage-missing-binding",
        parent_head,
        RunEventKind::SubworkflowCreated {
            subworkflow,
            parent_execution: declaration.controller_execution().clone(),
            child_run: child.clone(),
            child_revision: revision_id()?,
            scope: scope.clone(),
            ownership: SubworkflowOwnership::Attached,
            inputs: Vec::new(),
        },
        vec![WorkspaceMutation::CreateScope { scope }],
    )?;
    assert!(matches!(
        store.commit_command(&missing_binding),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert!(store.controller_account_binding(&child)?.is_none());

    let legacy_run = RunId::new("run-family-fixture")?;
    let seed = request_many_with_workspace(
        &legacy_run,
        "command-controller-lineage-legacy-seed",
        "event-controller-lineage-legacy-seed",
        RunSequence::ZERO,
        vec![RunEventKind::RunStarted; 16],
        Vec::new(),
    )?;
    let _ = store.commit_command(&seed)?;
    let legacy = RunEventEnvelope::from_json(include_bytes!(
        "../../../../../crates/persistence/tests/fixtures/run-event-controller-assessment-v2.json"
    ))?;
    let command = CommandId::new("command-controller-lineage-legacy-assessment")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        legacy_run.clone(),
        ActorRef::new("controller:redb-contract")?,
        RunSequence::new(16),
        legacy.occurred_at(),
        br#"{"schema_version":1,"type":"controller-redb-contract"}"#.to_vec(),
    )?;
    let result = CommandResultDocument::new(
        command,
        legacy_run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        legacy.sequence(),
        vec![legacy.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let legacy_assessment = AtomicRunCommitRequest::new(
        receipt,
        vec![legacy],
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: workspace_budget()?,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        }),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run: legacy_run,
                workflow: WorkflowId::new("workflow-controller-redb-contract")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::new(17),
                updated_at: TimestampMillis::new(17),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?;
    assert!(matches!(
        store.commit_command(&legacy_assessment),
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn controller_assessment_integrity_requires_the_exact_durable_declaration() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-assessment-integrity")?;
    let (declaration, recorded) = {
        let store = RedbStore::open(directory.path())?;
        let declaration = establish(&store, &owner, "assessment-integrity")?;
        let head = store
            .run_summary(&owner)?
            .ok_or("controller assessment owner summary is absent")?
            .through_sequence;
        let assessment = request(
            &owner,
            "command-controller-assessment-integrity",
            "event-controller-assessment-integrity",
            head,
            assessment(
                &declaration,
                "assessment-integrity-cycle",
                head,
                ControllerAssessmentBoundary::CycleEntry,
            )?,
        )?;
        let recorded = assessment
            .events()
            .first()
            .ok_or("controller assessment event is absent")?
            .clone();
        let _ = store.commit_command(&assessment)?;
        assert!(!has_integrity_failure(&store)?);
        (declaration, recorded)
    };

    let altered = ControllerAccountDeclaration::new(
        owner.clone(),
        declaration.controller_execution().clone(),
        declaration.policy_digest().to_owned(),
        ControllerResourceBudget::new(
            1_000_001,
            CurrencyCode::new("USD")?,
            1_000_000,
            1_000_000,
            1_000_000,
            1_000,
            1_000,
        )?,
    )?;
    assert_eq!(altered.account(), declaration.account());
    let replacement = RunEventEnvelope::new(
        recorded.event_id().clone(),
        owner.clone(),
        recorded.sequence(),
        recorded.occurred_at(),
        assessment(
            &altered,
            "assessment-integrity-cycle",
            RunSequence::new(recorded.sequence().get() - 1),
            ControllerAssessmentBoundary::CycleEntry,
        )?,
    )?;
    let bytes = replacement.to_canonical_json()?;
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write.open_table(RUN_EVENTS)?.insert(
        stored_event_key(&owner, recorded.sequence())?.as_slice(),
        bytes.as_slice(),
    )?;
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}
