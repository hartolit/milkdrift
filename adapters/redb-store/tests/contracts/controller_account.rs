use super::*;

const ACCOUNTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.accounts");
const BINDINGS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.controllers.run_bindings");
const ARTIFACT_CHARGES: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.artifact_charges");

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn controller_budget() -> TestResult<ControllerResourceBudget> {
    Ok(ControllerResourceBudget::new(
        1_000_000,
        CurrencyCode::new("USD")?,
        1_000_000,
        1_000_000,
        1_000_000,
        1_000,
        1_000,
    )?)
}

fn workspace_budget() -> TestResult<WorkspaceBudget> {
    Ok(WorkspaceBudget::new(
        128, 65_536, 1_048_576, 64, 1_048_576, 16_777_216,
    )?)
}

fn declaration(run: &RunId, suffix: &str) -> TestResult<ControllerAccountDeclaration> {
    Ok(ControllerAccountDeclaration::new(
        run.clone(),
        NodeExecutionId::new(format!("controller-execution-{suffix}"))?,
        format!("policy:controller-{suffix}"),
        controller_budget()?,
    )?)
}

fn decision(allowed: bool, suffix: &str) -> TestResult<AuthorityDecisionSnapshot> {
    let request = AuthorityRequest {
        decision: DecisionId::new(format!("decision:controller-{suffix}"))?,
        actor: ActorRef::new("controller:redb-contract")?,
        grant: GrantId::new("grant:controller-redb-contract")?,
        grant_revision: 1,
        grant_digest: GrantDigest::new(format!("b3_{}", "0".repeat(64)))?,
        revocation_generation: 0,
        operation: AuthorityOperation::InvokeCapability,
        resources: RequestedResourceFacts::empty(),
        budget: AuthorityBudget::default(),
        evaluated_at: BoundaryTimeMillis::new(10),
        provenance: AuthorityExecutionProvenance::default(),
    };
    Ok(AuthorityDecisionSnapshot::from_evaluation(
        PolicyId::new("policy:controller-redb-contract")?,
        1,
        request,
        vec![if allowed {
            DecisionReasonCode::Allowed
        } else {
            DecisionReasonCode::GrantNotFound
        }],
        AuthorityBudget::default(),
        SideEffectClass::None,
    )?)
}

fn request(
    run: &RunId,
    command: &str,
    event: &str,
    expected: RunSequence,
    kind: RunEventKind,
) -> TestResult<AtomicRunCommitRequest> {
    let sequence = expected.next()?;
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("controller:redb-contract")?,
        expected,
        TimestampMillis::new(10 + sequence.get()),
        br#"{"schema_version":1,"type":"controller-redb-contract"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(event)?,
        run.clone(),
        sequence,
        TimestampMillis::new(10 + sequence.get()),
        kind,
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        sequence,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
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
                run: run.clone(),
                workflow: WorkflowId::new("workflow-controller-redb-contract")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: sequence,
                updated_at: TimestampMillis::new(10 + sequence.get()),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn transaction(
    identity: &str,
    expected: Option<(&ControllerAccountState, &ControllerAccountDeclaration)>,
    actions: Vec<ControllerAccountAction>,
) -> TestResult<ControllerAccountTransaction> {
    Ok(ControllerAccountTransaction::new(
        ControllerTransitionId::new(identity)?,
        expected.map(|(state, declaration)| {
            (
                declaration.account().clone(),
                state.revision_digest().clone(),
            )
        }),
        actions,
    )?)
}

fn establish(
    store: &RedbStore,
    run: &RunId,
    suffix: &str,
) -> TestResult<ControllerAccountDeclaration> {
    let declaration = declaration(run, suffix)?;
    let request = request(
        run,
        &format!("command-establish-{suffix}"),
        &format!("event-establish-{suffix}"),
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        &format!("transition-establish-{suffix}"),
        None,
        vec![ControllerAccountAction::Establish {
            declaration: declaration.clone(),
            bind_run: run.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&request)?,
        AtomicRunCommitOutcome::Committed(_)
    ));
    Ok(declaration)
}

fn bind_child(
    store: &RedbStore,
    child: &RunId,
    declaration: &ControllerAccountDeclaration,
    suffix: &str,
) -> TestResult {
    let request = request(
        child,
        &format!("command-bind-{suffix}"),
        &format!("event-bind-{suffix}"),
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        &format!("transition-bind-{suffix}"),
        None,
        vec![ControllerAccountAction::BindRun {
            account: declaration.account().clone(),
            run: child.clone(),
        }],
    )?)?;
    let _ = store.commit_command(&request)?;
    Ok(())
}

fn has_integrity_failure(store: &RedbStore) -> TestResult<bool> {
    let mut cursor = None;
    for _ in 0..1_024 {
        match store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(64)?,
            verify_artifact_content: false,
            cursor,
        }) {
            Ok(page) => {
                if !page.failures.is_empty() {
                    return Ok(true);
                }
                let Some(next) = page.next_cursor else {
                    return Ok(false);
                };
                cursor = Some(next);
            }
            Err(
                PersistenceError::Storage {
                    class: StorageFailureClass::Corruption,
                    ..
                }
                | PersistenceError::Corruption(_),
            ) => return Ok(true),
            Err(error) => return Err(error.into()),
        }
    }
    Err("controller integrity scan did not exhaust".into())
}

#[test]
fn account_reestablishment_and_transition_fingerprints_are_exact() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let run = RunId::new("run-controller-reestablish")?;
    let declaration = establish(&store, &run, "reestablish")?;

    let redeclare = request(
        &run,
        "command-reestablish-exact",
        "event-reestablish-exact",
        RunSequence::FIRST,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-reestablish-exact",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: declaration.clone(),
            bind_run: run.clone(),
        }],
    )?)?;
    let _ = store.commit_command(&redeclare)?;

    let same_fingerprint = request(
        &run,
        "command-transition-same-fingerprint",
        "event-transition-same-fingerprint",
        RunSequence::new(2),
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-reestablish-exact",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: declaration.clone(),
            bind_run: run.clone(),
        }],
    )?)?;
    assert_storage_corruption(store.commit_command(&same_fingerprint));

    let different_fingerprint = request(
        &run,
        "command-transition-different-fingerprint",
        "event-transition-different-fingerprint",
        RunSequence::new(2),
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-reestablish-exact",
        None,
        vec![ControllerAccountAction::BindRun {
            account: declaration.account().clone(),
            run: RunId::new("run-transition-fingerprint-child")?,
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&different_fingerprint),
        Err(PersistenceError::ImmutableConflict {
            entity: "controller transition",
            ..
        })
    ));
    Ok(())
}

#[test]
fn conflicting_stored_declaration_is_never_treated_as_idempotent() -> TestResult {
    let directory = TempDir::new()?;
    let target_run = RunId::new("run-controller-target-declaration")?;
    let foreign_run = RunId::new("run-controller-foreign-declaration")?;
    let (target, foreign) = {
        let store = RedbStore::open(directory.path())?;
        (
            establish(&store, &target_run, "target-declaration")?,
            establish(&store, &foreign_run, "foreign-declaration")?,
        )
    };
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut accounts = write.open_table(ACCOUNTS)?;
        let foreign_bytes = accounts
            .get(foreign.account().as_str())?
            .ok_or("foreign controller account row is absent")?
            .value()
            .to_vec();
        accounts.insert(target.account().as_str(), foreign_bytes.as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let request = request(
        &target_run,
        "command-conflicting-controller-declaration",
        "event-conflicting-controller-declaration",
        RunSequence::FIRST,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-conflicting-controller-declaration",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: target,
            bind_run: target_run,
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&request),
        Err(PersistenceError::ImmutableConflict {
            entity: "controller account",
            ..
        })
    ));
    Ok(())
}

#[test]
fn preexisting_artifact_charge_is_corruption_not_a_conflict() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run-controller-artifact-charge-row")?;
    let first_publication = ArtifactPublicationId::new("publication-controller-charge-first")?;
    let second_publication = ArtifactPublicationId::new("publication-controller-charge-second")?;
    {
        let store = RedbStore::open(directory.path())?;
        let _declaration = establish(&store, &run, "artifact-charge-row")?;
        let bytes = b"x";
        let metadata = ArtifactMetadata::new(
            milkdrift_workspace::ArtifactReference::new(
                ArtifactId::new("artifact-controller-charge-first")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/octet-stream")?,
                1,
            ),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("controller-charge-test")?,
                },
                Vec::new(),
            )?,
        )?;
        let publication = BeginArtifactPublication::new(
            first_publication.clone(),
            run.clone(),
            metadata,
            workspace_budget()?,
            WorkspaceUsage::EMPTY,
        )?;
        let _ = store.begin_publication(&publication)?;
        let _ = store.write_chunk(&first_publication, 0, bytes)?;
        let _ = store.commit_publication(&first_publication)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut charges = write.open_table(ARTIFACT_CHARGES)?;
        let bytes = charges
            .get(first_publication.as_str())?
            .ok_or("first controller artifact charge is absent")?
            .value()
            .to_vec();
        charges.insert(second_publication.as_str(), bytes.as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let bytes = b"x";
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new("artifact-controller-charge-second")?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("application/octet-stream")?,
            1,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("controller-charge-test")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = BeginArtifactPublication::new(
        second_publication.clone(),
        run.clone(),
        metadata,
        workspace_budget()?,
        store.workspace_usage(&run)?,
    )?;
    let _ = store.begin_publication(&publication)?;
    let _ = store.write_chunk(&second_publication, 0, bytes)?;
    assert_storage_corruption(store.commit_publication(&second_publication));
    Ok(())
}

#[test]
fn final_entry_integrity_distinguishes_denied_uncontrolled_and_reserved_links() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-event-link-owner")?;
    let allowed_child = RunId::new("run-controller-event-link-allowed-child")?;
    let denied_child = RunId::new("run-controller-event-link-denied-child")?;
    let reserved_child = RunId::new("run-controller-event-link-reserved-child")?;
    let foreign_owner = RunId::new("run-controller-event-link-foreign-owner")?;
    let (origin, foreign) = {
        let store = RedbStore::open(directory.path())?;
        let origin = establish(&store, &owner, "event-link-owner")?;
        let foreign = establish(&store, &foreign_owner, "event-link-foreign")?;

        let allowed_start = request(
            &allowed_child,
            "command-allowed-child-start",
            "event-allowed-child-start",
            RunSequence::ZERO,
            RunEventKind::RunStarted,
        )?;
        let _ = store.commit_command(&allowed_start)?;
        let allowed = request(
            &allowed_child,
            "command-allowed-uncontrolled",
            "event-allowed-uncontrolled",
            RunSequence::FIRST,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: AttemptId::new("attempt-allowed-uncontrolled")?,
                authorization: decision(true, "allowed-uncontrolled")?,
                controller_admission: ControllerAdmissionOutcome::NotControlled,
            },
        )?;
        let _ = store.commit_command(&allowed)?;
        let late_bind = request(
            &allowed_child,
            "command-late-bind-allowed-child",
            "event-late-bind-allowed-child",
            RunSequence::new(2),
            RunEventKind::RunStarted,
        )?
        .with_controller_account_transaction(transaction(
            "transition-late-bind-allowed-child",
            None,
            vec![ControllerAccountAction::BindRun {
                account: origin.account().clone(),
                run: allowed_child.clone(),
            }],
        )?)?;
        let _ = store.commit_command(&late_bind)?;

        bind_child(&store, &denied_child, &origin, "denied-child")?;
        let denied = request(
            &denied_child,
            "command-denied-uncontrolled",
            "event-denied-uncontrolled",
            RunSequence::FIRST,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: AttemptId::new("attempt-denied-uncontrolled")?,
                authorization: decision(false, "denied-uncontrolled")?,
                controller_admission: ControllerAdmissionOutcome::NotControlled,
            },
        )?;
        let _ = store.commit_command(&denied)?;

        bind_child(&store, &reserved_child, &origin, "reserved-child")?;
        let state = store
            .controller_account(origin.account())?
            .ok_or("originating controller account is absent")?;
        let attempt = AttemptId::new("attempt-reserved-link")?;
        let reservation = ControllerReservationId::for_attempt(origin.account(), &attempt)?;
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?;
        let reserved = request(
            &reserved_child,
            "command-reserved-link",
            "event-reserved-link",
            RunSequence::FIRST,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "reserved-link")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link",
            Some((&state, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation,
                attempt,
                category: CapabilityCategory::Process,
                envelope: InvocationAdmissionEnvelope::not_applicable(),
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&reserved)?;
        assert!(has_integrity_failure(&store)?);
        (origin, foreign)
    };

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut bindings = write.open_table(BINDINGS)?;
        bindings.insert(reserved_child.as_str(), foreign.account().as_str())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_eq!(
        store.controller_account_binding(&denied_child)?.as_ref(),
        Some(origin.account())
    );
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn denied_uncontrolled_final_entry_is_valid_only_for_a_denied_decision() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-denied-link-owner")?;
    let child = RunId::new("run-controller-denied-link-child")?;
    let origin = establish(&store, &owner, "denied-link-owner")?;
    bind_child(&store, &child, &origin, "denied-link-child")?;
    let denied = request(
        &child,
        "command-denied-link-only",
        "event-denied-link-only",
        RunSequence::FIRST,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: AttemptId::new("attempt-denied-link-only")?,
            authorization: decision(false, "denied-link-only")?,
            controller_admission: ControllerAdmissionOutcome::NotControlled,
        },
    )?;
    let _ = store.commit_command(&denied)?;
    assert!(!has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn allowed_uncontrolled_final_entry_is_corrupt_after_a_late_binding() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-allowed-link-owner")?;
    let child = RunId::new("run-controller-allowed-link-child")?;
    let origin = establish(&store, &owner, "allowed-link-owner")?;
    let start = request(
        &child,
        "command-allowed-link-child-start",
        "event-allowed-link-child-start",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?;
    let _ = store.commit_command(&start)?;
    let allowed = request(
        &child,
        "command-allowed-link-only",
        "event-allowed-link-only",
        RunSequence::FIRST,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: AttemptId::new("attempt-allowed-link-only")?,
            authorization: decision(true, "allowed-link-only")?,
            controller_admission: ControllerAdmissionOutcome::NotControlled,
        },
    )?;
    let _ = store.commit_command(&allowed)?;
    let bind = request(
        &child,
        "command-allowed-link-late-bind",
        "event-allowed-link-late-bind",
        RunSequence::new(2),
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-allowed-link-late-bind",
        None,
        vec![ControllerAccountAction::BindRun {
            account: origin.account().clone(),
            run: child,
        }],
    )?)?;
    let _ = store.commit_command(&bind)?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn reserved_final_entry_must_match_both_binding_and_attempt_identity() -> TestResult {
    let directory = TempDir::new()?;
    let child = RunId::new("run-controller-reserved-link-child-only")?;
    let foreign;
    {
        let store = RedbStore::open(directory.path())?;
        let owner = RunId::new("run-controller-reserved-link-owner-only")?;
        let foreign_owner = RunId::new("run-controller-reserved-link-foreign-only")?;
        let origin = establish(&store, &owner, "reserved-link-owner-only")?;
        foreign = establish(&store, &foreign_owner, "reserved-link-foreign-only")?;
        bind_child(&store, &child, &origin, "reserved-link-child-only")?;
        let state = store
            .controller_account(origin.account())?
            .ok_or("reserved-link origin account is absent")?;
        let attempt = AttemptId::new("attempt-reserved-link-only")?;
        let reservation = ControllerReservationId::for_attempt(origin.account(), &attempt)?;
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &InvocationAdmissionEnvelope::not_applicable(),
        )?;
        let reserved = request(
            &child,
            "command-reserved-link-only",
            "event-reserved-link-only",
            RunSequence::FIRST,
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "reserved-link-only")?,
                controller_admission: outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-only",
            Some((&state, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation,
                attempt,
                category: CapabilityCategory::Process,
                envelope: InvocationAdmissionEnvelope::not_applicable(),
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&reserved)?;
        assert!(!has_integrity_failure(&store)?);
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut bindings = write.open_table(BINDINGS)?;
        bindings.insert(child.as_str(), foreign.account().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}
