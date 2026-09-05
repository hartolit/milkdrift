use super::*;

const ACCOUNTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.accounts");
const BINDINGS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.controllers.run_bindings");
const ARTIFACT_CHARGES: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.artifact_charges");
const TRANSITIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.controllers.transitions");
const ARTIFACT_PUBLICATIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.publications");
const RUN_EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.runs.events");

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
    request_with_workspace(run, command, event, expected, kind, Vec::new())
}

fn request_with_workspace(
    run: &RunId,
    command: &str,
    event: &str,
    expected: RunSequence,
    kind: RunEventKind,
    workspace: Vec<WorkspaceMutation>,
) -> TestResult<AtomicRunCommitRequest> {
    request_many_with_workspace(run, command, event, expected, vec![kind], workspace)
}

fn request_many_with_workspace(
    run: &RunId,
    command: &str,
    event: &str,
    expected: RunSequence,
    kinds: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
) -> TestResult<AtomicRunCommitRequest> {
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("controller:redb-contract")?,
        expected,
        TimestampMillis::new(10 + expected.get()),
        br#"{"schema_version":1,"type":"controller-redb-contract"}"#.to_vec(),
    )?;
    let multiple = kinds.len() > 1;
    let mut sequence = expected;
    let events = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            sequence = sequence.next()?;
            RunEventEnvelope::new(
                EventId::new(if multiple {
                    format!("{event}-{}", index + 1)
                } else {
                    event.to_owned()
                })?,
                run.clone(),
                sequence,
                TimestampMillis::new(10 + sequence.get()),
                kind,
            )
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        sequence,
        events
            .iter()
            .map(|event| event.event_id().clone())
            .collect(),
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        events,
        workspace,
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

fn controller_root(declaration: &ControllerAccountDeclaration) -> TestResult<WorkspaceScope> {
    Ok(WorkspaceScope::run_root(
        declaration.controller_run().clone(),
        ScopeId::new(format!(
            "controller-root-{}",
            declaration.account().as_str()
        ))?,
    ))
}

fn stored_event_key(run: &RunId, sequence: RunSequence) -> TestResult<Vec<u8>> {
    let mut key = Vec::new();
    key.extend_from_slice(&u32::try_from(run.as_str().len())?.to_be_bytes());
    key.extend_from_slice(run.as_str().as_bytes());
    key.extend_from_slice(&sequence.get().to_be_bytes());
    Ok(key)
}

fn rewrite_internal_payload(
    bytes: &[u8],
    family: &str,
    from: &str,
    to: &str,
) -> TestResult<Vec<u8>> {
    let document = std::str::from_utf8(bytes)?;
    let payload_marker = "\"payload\":";
    let payload_start = document
        .find(payload_marker)
        .ok_or("internal document payload is absent")?
        + payload_marker.len();
    let payload = document
        .get(payload_start..document.len().saturating_sub(1))
        .ok_or("internal document payload bounds are invalid")?;
    if !payload.contains(from) {
        return Err("internal document payload mutation target is absent".into());
    }
    encode_internal_payload(family, &payload.replacen(from, to, 1))
}

fn encode_internal_payload(family: &str, payload: &str) -> TestResult<Vec<u8>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.redb.internal-document.v1\0");
    hasher.update(&u64::try_from(family.len())?.to_be_bytes());
    hasher.update(family.as_bytes());
    hasher.update(&u64::try_from(payload.len())?.to_be_bytes());
    hasher.update(payload.as_bytes());
    Ok(format!(
        "{{\"schema_version\":1,\"family\":{},\"checksum\":\"{}\",\"payload\":{payload}}}",
        serde_json::to_string(family)?,
        hasher.finalize().to_hex()
    )
    .into_bytes())
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

fn activation(
    declaration: &ControllerAccountDeclaration,
    suffix: &str,
    through_sequence: RunSequence,
) -> TestResult<RunEventKind> {
    assessment(
        declaration,
        suffix,
        through_sequence,
        ControllerAssessmentBoundary::Activation,
    )
}

fn assessment(
    declaration: &ControllerAccountDeclaration,
    suffix: &str,
    through_sequence: RunSequence,
    boundary: ControllerAssessmentBoundary,
) -> TestResult<RunEventKind> {
    Ok(RunEventKind::ControllerAssessmentRecorded {
        controller_id: format!("controller:{suffix}"),
        policy_digest: declaration.policy_digest().to_owned(),
        governing_revision: revision_id()?,
        controller_node: NodeId::new(format!("controller-node-{suffix}"))?,
        controller_execution: declaration.controller_execution().clone(),
        assessment_id: format!("controller-assessment:{suffix}"),
        cycle_id: None,
        boundary,
        through_sequence,
        progress: BoundedJson::new(json!({"cycle": 0}))?,
        account_declaration: Some(declaration.clone()),
        outcome: ControllerAssessmentOutcome::Continue,
    })
}

fn establish(
    store: &RedbStore,
    run: &RunId,
    suffix: &str,
) -> TestResult<ControllerAccountDeclaration> {
    let declaration = declaration(run, suffix)?;
    let root = controller_root(&declaration)?;
    let request = request_many_with_workspace(
        run,
        &format!("command-establish-{suffix}"),
        &format!("event-establish-{suffix}"),
        RunSequence::ZERO,
        vec![
            RunEventKind::RunCreated {
                workflow: WorkflowId::new("workflow-controller-redb-contract")?,
                revision: revision_id()?,
                revision_digest: revision_digest()?,
                root_scope: root.clone(),
                workspace_budget: workspace_budget()?,
                inputs: Vec::new(),
            },
            activation(&declaration, suffix, RunSequence::FIRST)?,
        ],
        vec![WorkspaceMutation::CreateScope { scope: root }],
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
    let request = bind_child_request(
        store,
        child,
        declaration,
        suffix,
        &format!("transition-bind-{suffix}"),
    )?;
    let _ = store.commit_command(&request)?;
    Ok(())
}

fn bind_child_request(
    store: &RedbStore,
    child: &RunId,
    declaration: &ControllerAccountDeclaration,
    suffix: &str,
    transition: &str,
) -> TestResult<AtomicRunCommitRequest> {
    let parent = declaration.controller_run();
    let expected = store
        .run_summary(parent)?
        .ok_or("controller parent run summary is absent")?
        .through_sequence;
    let subworkflow = SubworkflowId::new(format!("subworkflow-{suffix}"))?;
    let scope = WorkspaceScope::subworkflow(
        ScopeId::new(format!("subworkflow-scope-{suffix}"))?,
        &controller_root(declaration)?,
        subworkflow.clone(),
    )?;
    Ok(request_with_workspace(
        parent,
        &format!("command-bind-{suffix}"),
        &format!("event-bind-{suffix}"),
        expected,
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
    )?
    .with_controller_account_transaction(transaction(
        transition,
        None,
        vec![ControllerAccountAction::BindRun {
            account: declaration.account().clone(),
            run: child.clone(),
        }],
    )?)?)
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

fn has_integrity_failure_matching(
    store: &RedbStore,
    component: &str,
    detail: &str,
) -> TestResult<bool> {
    let mut cursor = None;
    for _ in 0..1_024 {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(64)?,
            verify_artifact_content: false,
            cursor,
        })?;
        if page.failures.iter().any(|failure| {
            failure.component.as_str() == component && failure.detail.as_str().contains(detail)
        }) {
            return Ok(true);
        }
        let Some(next) = page.next_cursor else {
            return Ok(false);
        };
        cursor = Some(next);
    }
    Err("controller integrity scan did not exhaust".into())
}

#[test]
fn account_reestablishment_and_transition_fingerprints_are_exact() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let run = RunId::new("run-controller-reestablish")?;
    let declaration = establish(&store, &run, "reestablish")?;
    let first_head = store
        .run_summary(&run)?
        .ok_or("controller run summary is absent after establishment")?
        .through_sequence;

    let redeclare = request(
        &run,
        "command-reestablish-exact",
        "event-reestablish-exact",
        first_head,
        activation(&declaration, "reestablish-exact", first_head)?,
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
    let second_head = store
        .run_summary(&run)?
        .ok_or("controller run summary is absent after reestablishment")?
        .through_sequence;

    let same_fingerprint = request(
        &run,
        "command-transition-same-fingerprint",
        "event-transition-same-fingerprint",
        second_head,
        activation(&declaration, "transition-same-fingerprint", second_head)?,
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

    let different_fingerprint = bind_child_request(
        &store,
        &RunId::new("run-transition-fingerprint-child")?,
        &declaration,
        "transition-different-fingerprint",
        "transition-reestablish-exact",
    )?;
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
fn transition_integrity_recomputes_fingerprints_and_requires_its_command_receipt() -> TestResult {
    for mutation in ["fingerprint", "command", "missing-transition"] {
        let directory = TempDir::new()?;
        let run = RunId::new(format!("run-transition-integrity-{mutation}"))?;
        {
            let store = RedbStore::open(directory.path())?;
            let _declaration = establish(&store, &run, mutation)?;
            assert!(!has_integrity_failure(&store)?);
        }

        let transition = format!("transition-establish-{mutation}");
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut transitions = write.open_table(TRANSITIONS)?;
            let stored = transitions
                .get(transition.as_str())?
                .ok_or("controller transition row is absent")?
                .value()
                .to_vec();
            if mutation == "missing-transition" {
                transitions.remove(transition.as_str())?;
            } else {
                let rewritten = if mutation == "fingerprint" {
                    rewrite_internal_payload(
                        &stored,
                        "controller transition record",
                        &format!("\"bind_run\":\"{run}\""),
                        "\"bind_run\":\"run-transition-integrity-tampered\"",
                    )?
                } else {
                    rewrite_internal_payload(
                        &stored,
                        "controller transition record",
                        &format!("\"command\":\"command-establish-{mutation}\""),
                        "\"command\":\"command-transition-integrity-missing\"",
                    )?
                };
                transitions.insert(transition.as_str(), rewritten.as_slice())?;
            }
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        assert!(has_integrity_failure(&store)?);
    }
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
    let target_head = store
        .run_summary(&target_run)?
        .ok_or("target controller run summary is absent")?
        .through_sequence;
    let request = request(
        &target_run,
        "command-conflicting-controller-declaration",
        "event-conflicting-controller-declaration",
        target_head,
        activation(&target, "conflicting-controller-declaration", target_head)?,
    )?
    .with_controller_account_transaction(transaction(
        "transition-conflicting-controller-declaration",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: target,
            bind_run: target_run,
        }],
    )?)?;
    assert_storage_corruption(store.commit_command(&request));
    Ok(())
}

#[test]
fn same_account_identity_with_an_altered_budget_is_not_idempotent() -> TestResult {
    let directory = TempDir::new()?;
    let owner = RunId::new("run-controller-same-account-altered-budget")?;
    let expected = declaration(&owner, "same-account-altered-budget")?;
    let altered = ControllerAccountDeclaration::new(
        owner.clone(),
        expected.controller_execution().clone(),
        expected.policy_digest().to_owned(),
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
    assert_eq!(altered.account(), expected.account());
    assert_ne!(altered, expected);
    let store = RedbStore::open(directory.path())?;
    let root = controller_root(&altered)?;
    let establish_altered = request_many_with_workspace(
        &owner,
        "command-establish-altered-controller-budget",
        "event-establish-altered-controller-budget",
        RunSequence::ZERO,
        vec![
            RunEventKind::RunCreated {
                workflow: WorkflowId::new("workflow-controller-redb-contract")?,
                revision: revision_id()?,
                revision_digest: revision_digest()?,
                root_scope: root.clone(),
                workspace_budget: workspace_budget()?,
                inputs: Vec::new(),
            },
            activation(
                &altered,
                "establish-altered-controller-budget",
                RunSequence::FIRST,
            )?,
        ],
        vec![WorkspaceMutation::CreateScope { scope: root }],
    )?
    .with_controller_account_transaction(transaction(
        "transition-establish-altered-controller-budget",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: altered,
            bind_run: owner.clone(),
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&establish_altered)?,
        AtomicRunCommitOutcome::Committed(_)
    ));
    let target_head = store
        .run_summary(&owner)?
        .ok_or("controller run summary is absent")?
        .through_sequence;
    let request = request(
        &owner,
        "command-controller-same-account-altered-budget",
        "event-controller-same-account-altered-budget",
        target_head,
        assessment(
            &expected,
            "same-account-altered-budget",
            target_head,
            ControllerAssessmentBoundary::CycleEntry,
        )?,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-same-account-altered-budget",
        None,
        vec![ControllerAccountAction::Establish {
            declaration: expected,
            bind_run: owner,
        }],
    )?)?;
    let result = store.commit_command(&request);
    assert!(
        matches!(
            &result,
            Err(PersistenceError::InvalidDocument(reason))
                if reason == "controller assessment differs from its immutable account declaration"
        ),
        "expected event-contract refusal before account application, got {result:?}"
    );
    Ok(())
}

#[path = "controller_account/artifact_charge.rs"]
mod artifact_charge;
#[test]
fn unbound_publication_integrity_rejects_an_invocation_reservation_owner() -> TestResult {
    let directory = TempDir::new()?;
    let run = RunId::new("run-unbound-invocation-publication")?;
    let publication = ArtifactPublicationId::new("publication-unbound-invocation")?;
    {
        let store = RedbStore::open(directory.path())?;
        let bytes = b"x";
        let metadata = ArtifactMetadata::new(
            milkdrift_workspace::ArtifactReference::new(
                ArtifactId::new("artifact-unbound-invocation")?,
                ContentDigest::for_bytes(bytes),
                MediaType::new("application/octet-stream")?,
                1,
            ),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::External {
                    source: CausalId::new("unbound-invocation-publication-test")?,
                },
                Vec::new(),
            )?,
        )?;
        let request = BeginArtifactPublication::new(
            publication.clone(),
            run,
            metadata,
            workspace_budget()?,
            WorkspaceUsage::EMPTY,
        )?;
        let _ = store.begin_publication(&request)?;
        let _ = store.write_chunk(&publication, 0, bytes)?;
        let _ = store.commit_publication(&publication)?;
        assert!(!has_integrity_failure(&store)?);
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut publications = write.open_table(ARTIFACT_PUBLICATIONS)?;
        let bytes = publications
            .get(publication.as_str())?
            .ok_or("unbound artifact publication is absent")?
            .value()
            .to_vec();
        let altered = rewrite_internal_payload(
            &bytes,
            "artifact publication",
            r#""controller_owner":{"type":"run_binding"}"#,
            r#""controller_owner":{"type":"invocation_reservation","reservation":"controller-reservation:unbound-invocation"}"#,
        )?;
        publications.insert(publication.as_str(), altered.as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(has_integrity_failure(&store)?);
    Ok(())
}

#[test]
fn invocation_artifact_above_reservation_blocks_account_without_charging_metadata() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-artifact-envelope-owner")?;
    let child = RunId::new("run-controller-artifact-envelope-child")?;
    let declaration = establish(&store, &owner, "artifact-envelope-owner")?;
    bind_child(&store, &child, &declaration, "artifact-envelope-child")?;

    let state = store
        .controller_account(declaration.account())?
        .ok_or("controller account is absent")?;
    let attempt = AttemptId::new("attempt-controller-artifact-envelope")?;
    let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
    let envelope = InvocationAdmissionEnvelope::new(
        AdmissionBound::NotApplicable,
        AdmissionBound::NotApplicable,
        AdmissionBound::Bounded(1),
        AdmissionBound::NotApplicable,
    );
    let mut candidate = state.clone();
    let outcome = candidate.admit(
        reservation.clone(),
        attempt.clone(),
        CapabilityCategory::Tool,
        &envelope,
    )?;
    let entry = request(
        &child,
        "command-controller-artifact-envelope-entry",
        "event-controller-artifact-envelope-entry",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "artifact-envelope-entry")?,
            controller_admission: outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-artifact-envelope-entry",
        Some((&state, &declaration)),
        vec![ControllerAccountAction::AdmitEntry {
            account: declaration.account().clone(),
            reservation: reservation.clone(),
            attempt,
            category: CapabilityCategory::Tool,
            envelope,
            expected_outcome: outcome,
        }],
    )?)?;
    let _ = store.commit_command(&entry)?;

    let bytes = b"xx";
    let artifact = ArtifactId::new("artifact-controller-envelope-excess")?;
    let metadata = ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            artifact.clone(),
            ContentDigest::for_bytes(bytes),
            MediaType::new("application/octet-stream")?,
            2,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("controller-envelope-excess-test")?,
            },
            Vec::new(),
        )?,
    )?;
    let publication = BeginArtifactPublication::for_invocation(
        ArtifactPublicationId::new("publication-controller-envelope-excess")?,
        child,
        metadata,
        workspace_budget()?,
        WorkspaceUsage::EMPTY,
        reservation,
    )?;
    let _ = store.begin_publication(&publication)?;
    let _ = store.write_chunk(publication.publication(), 0, bytes)?;
    assert!(matches!(
        store.commit_publication(publication.publication()),
        Err(PersistenceError::Bounds {
            location: "controller.artifact_reservation",
            ..
        })
    ));
    assert!(store.metadata(&artifact)?.is_none());
    let blocked = store
        .controller_account(declaration.account())?
        .ok_or("controller account disappeared")?;
    assert_eq!(blocked.settled().artifact_bytes(), 0);
    assert_eq!(blocked.outstanding().artifact_bytes(), 1);
    assert!(matches!(
        blocked.blocked(),
        Some(milkdrift_persistence::ControllerAccountBlock::ContractViolation {
            dimension,
            observed: 2,
            reserved: 1,
            ..
        }) if dimension == "artifact_bytes"
    ));
    assert!(!has_integrity_failure(&store)?);
    Ok(())
}

#[path = "controller_account/final_entry.rs"]
mod final_entry;
#[path = "controller_account/integrity.rs"]
mod integrity;

#[path = "controller_account/lineage.rs"]
mod lineage;

#[path = "controller_account/entry_links.rs"]
mod entry_links;
