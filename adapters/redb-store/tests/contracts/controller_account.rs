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

#[test]
fn final_entry_rejects_unlinked_and_duplicate_admission_actions() -> TestResult {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let owner = RunId::new("run-controller-admission-link-owner")?;
    let child = RunId::new("run-controller-admission-link-child")?;
    let declaration = establish(&store, &owner, "admission-link-owner")?;
    bind_child(&store, &child, &declaration, "admission-link-child")?;
    let state = store
        .controller_account(declaration.account())?
        .ok_or("controller account is absent")?;

    let attempt = AttemptId::new("attempt-controller-admission-link")?;
    let reservation = ControllerReservationId::for_attempt(declaration.account(), &attempt)?;
    let envelope = InvocationAdmissionEnvelope::not_applicable();
    let mut candidate = state.clone();
    let outcome = candidate.admit(
        reservation.clone(),
        attempt.clone(),
        CapabilityCategory::Tool,
        &envelope,
    )?;
    let action = ControllerAccountAction::AdmitEntry {
        account: declaration.account().clone(),
        reservation: reservation.clone(),
        attempt: attempt.clone(),
        category: CapabilityCategory::Tool,
        envelope: envelope.clone(),
        expected_outcome: outcome.clone(),
    };

    let unlinked = request(
        &child,
        "command-controller-admission-unlinked",
        "event-controller-admission-unlinked",
        RunSequence::ZERO,
        RunEventKind::RunStarted,
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-unlinked",
        Some((&state, &declaration)),
        vec![action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&unlinked),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(
        store
            .controller_account(declaration.account())?
            .ok_or("controller account disappeared")?,
        state
    );

    let duplicate = request(
        &child,
        "command-controller-admission-duplicate",
        "event-controller-admission-duplicate",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "admission-duplicate")?,
            controller_admission: outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-duplicate",
        Some((&state, &declaration)),
        vec![action.clone(), action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&duplicate),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let unterminated_reservation = request_many_with_workspace(
        &child,
        "command-controller-admission-unterminated-reservation",
        "event-controller-admission-unterminated-reservation",
        RunSequence::ZERO,
        vec![
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: attempt.clone(),
                authorization: decision(true, "admission-unterminated-reservation")?,
                controller_admission: outcome.clone(),
            },
            RunEventKind::NodeTerminal {
                execution: NodeExecutionId::new("execution-admission-unterminated-reservation")?,
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        ],
        Vec::new(),
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-unterminated-reservation",
        Some((&state, &declaration)),
        vec![action.clone()],
    )?)?;
    assert!(matches!(
        store.commit_command(&unterminated_reservation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let late_without_reservation = request(
        &child,
        "command-controller-admission-late-without-reservation",
        "event-controller-admission-late-without-reservation",
        RunSequence::ZERO,
        RunEventKind::LateTerminalEvidenceRecorded {
            attempt: AttemptId::new("attempt-controller-admission-late-without-reservation")?,
            worker: WorkerId::new("worker-controller-admission-late-without-reservation")?,
            report_sequence: 1,
            terminal: InvocationTerminal::new(
                TerminalStatus::Success,
                Vec::new(),
                None,
                None,
                SideEffectClass::None,
            )?,
        },
    )?;
    assert!(matches!(
        store.commit_command(&late_without_reservation),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let forged_event = request(
        &child,
        "command-controller-admission-forged-event",
        "event-controller-admission-forged-event",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt: attempt.clone(),
            authorization: decision(true, "admission-forged-event")?,
            controller_admission: ControllerAdmissionOutcome::Denied {
                account: declaration.account().clone(),
                reason: milkdrift_persistence::ControllerAdmissionDenial::Limit {
                    dimension: "process_admissions".to_owned(),
                },
            },
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-forged-event",
        Some((&state, &declaration)),
        vec![action],
    )?)?;
    assert!(matches!(
        store.commit_command(&forged_event),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let action_attempt = AttemptId::new("attempt-controller-admission-other-action")?;
    let action_reservation =
        ControllerReservationId::for_attempt(declaration.account(), &action_attempt)?;
    let mut action_candidate = state.clone();
    let action_outcome = action_candidate.admit(
        action_reservation.clone(),
        action_attempt.clone(),
        CapabilityCategory::Tool,
        &envelope,
    )?;
    let mismatched_attempt = request(
        &child,
        "command-controller-admission-mismatched-attempt",
        "event-controller-admission-mismatched-attempt",
        RunSequence::ZERO,
        RunEventKind::CapabilityAdapterEntryDecisionRecorded {
            attempt,
            authorization: decision(true, "admission-mismatched-attempt")?,
            controller_admission: action_outcome.clone(),
        },
    )?
    .with_controller_account_transaction(transaction(
        "transition-controller-admission-mismatched-attempt",
        Some((&state, &declaration)),
        vec![ControllerAccountAction::AdmitEntry {
            account: declaration.account().clone(),
            reservation: action_reservation,
            attempt: action_attempt,
            category: CapabilityCategory::Tool,
            envelope,
            expected_outcome: action_outcome,
        }],
    )?)?;
    assert!(matches!(
        store.commit_command(&mismatched_attempt),
        Err(PersistenceError::InvalidDocument(_))
    ));

    assert_eq!(
        store
            .controller_account(declaration.account())?
            .ok_or("controller account disappeared")?,
        state
    );
    Ok(())
}

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
        "../../../../crates/persistence/tests/fixtures/run-event-controller-assessment-v2.json"
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
        bind_child(&store, &denied_child, &origin, "denied-child")?;
        let denied = request(
            &denied_child,
            "command-denied-uncontrolled",
            "event-denied-uncontrolled",
            RunSequence::ZERO,
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
        let reservation_envelope = InvocationAdmissionEnvelope::new(
            AdmissionBound::Bounded(8),
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
            AdmissionBound::NotApplicable,
        );
        let mut candidate = state.clone();
        let outcome = candidate.admit(
            reservation.clone(),
            attempt.clone(),
            CapabilityCategory::Process,
            &reservation_envelope,
        )?;
        let reserved = request(
            &reserved_child,
            "command-reserved-link",
            "event-reserved-link",
            RunSequence::ZERO,
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
                reservation: reservation.clone(),
                attempt: attempt.clone(),
                category: CapabilityCategory::Process,
                envelope: reservation_envelope.clone(),
                expected_outcome: outcome,
            }],
        )?)?;
        let _ = store.commit_command(&reserved)?;
        let missing_settlement = request(
            &reserved_child,
            "command-reserved-link-missing-settlement",
            "event-reserved-link-missing-settlement",
            RunSequence::FIRST,
            RunEventKind::NodeTerminal {
                execution: NodeExecutionId::new("execution-reserved-link")?,
                attempt: attempt.clone(),
                report_sequence: 1,
                outcome: NodeOutcome::Succeeded,
                error_class: None,
                detail: None,
            },
        )?;
        assert!(matches!(
            store.commit_command(&missing_settlement),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let missing_late_settlement = request(
            &reserved_child,
            "command-reserved-link-missing-late-settlement",
            "event-reserved-link-missing-late-settlement",
            RunSequence::FIRST,
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt: attempt.clone(),
                worker: WorkerId::new("worker-reserved-link-late")?,
                report_sequence: 2,
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        )?;
        assert!(matches!(
            store.commit_command(&missing_late_settlement),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let admitted = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared before settlement")?;
        let usage = AttemptUsage {
            input_units: Some(3),
            output_units: None,
            duration_ms: Some(4),
            cost: None,
        };
        let cross_attempt_usage = request_many_with_workspace(
            &reserved_child,
            "command-reserved-link-cross-attempt-usage",
            "event-reserved-link-cross-attempt-usage",
            RunSequence::FIRST,
            vec![
                RunEventKind::AttemptUsageRecorded {
                    attempt: AttemptId::new("attempt-reserved-link-other-usage")?,
                    usage: usage.clone(),
                },
                RunEventKind::NodeTerminal {
                    execution: NodeExecutionId::new("execution-reserved-link-cross-usage")?,
                    attempt: attempt.clone(),
                    report_sequence: 3,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            ],
            Vec::new(),
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-cross-attempt-usage",
            Some((&admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: reservation.clone(),
                usage: Some(usage.clone()),
            }],
        )?)?;
        assert!(matches!(
            store.commit_command(&cross_attempt_usage),
            Err(PersistenceError::InvalidDocument(_))
        ));
        let terminal = request_many_with_workspace(
            &reserved_child,
            "command-reserved-link-terminal",
            "event-reserved-link-terminal",
            RunSequence::FIRST,
            vec![
                RunEventKind::AttemptUsageRecorded {
                    attempt: attempt.clone(),
                    usage: usage.clone(),
                },
                RunEventKind::NodeTerminal {
                    execution: NodeExecutionId::new("execution-reserved-link")?,
                    attempt: attempt.clone(),
                    report_sequence: 3,
                    outcome: NodeOutcome::Succeeded,
                    error_class: None,
                    detail: None,
                },
            ],
            Vec::new(),
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-terminal",
            Some((&admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: reservation.clone(),
                usage: Some(usage),
            }],
        )?)?;
        let _ = store.commit_command(&terminal)?;
        let settled = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared after settlement")?;
        assert_eq!(settled.settled().input_units(), 3);
        assert!(!settled.reservations().contains_key(&reservation));

        let late_attempt = AttemptId::new("attempt-reserved-link-late-positive")?;
        let late_reservation =
            ControllerReservationId::for_attempt(origin.account(), &late_attempt)?;
        let mut late_candidate = settled.clone();
        let late_outcome = late_candidate.admit(
            late_reservation.clone(),
            late_attempt.clone(),
            CapabilityCategory::Tool,
            &reservation_envelope,
        )?;
        let late_entry = request(
            &reserved_child,
            "command-reserved-link-late-entry",
            "event-reserved-link-late-entry",
            RunSequence::new(3),
            RunEventKind::CapabilityAdapterEntryDecisionRecorded {
                attempt: late_attempt.clone(),
                authorization: decision(true, "reserved-link-late-entry")?,
                controller_admission: late_outcome.clone(),
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-late-entry",
            Some((&settled, &origin)),
            vec![ControllerAccountAction::AdmitEntry {
                account: origin.account().clone(),
                reservation: late_reservation.clone(),
                attempt: late_attempt.clone(),
                category: CapabilityCategory::Tool,
                envelope: reservation_envelope,
                expected_outcome: late_outcome,
            }],
        )?)?;
        let _ = store.commit_command(&late_entry)?;
        let late_admitted = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared before late settlement")?;
        let late_usage = AttemptUsage {
            input_units: Some(2),
            output_units: None,
            duration_ms: Some(5),
            cost: None,
        };
        let late_terminal = request(
            &reserved_child,
            "command-reserved-link-late-terminal",
            "event-reserved-link-late-terminal",
            RunSequence::new(4),
            RunEventKind::LateTerminalEvidenceRecorded {
                attempt: late_attempt,
                worker: WorkerId::new("worker-reserved-link-late-positive")?,
                report_sequence: 4,
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    Some(UsageObservation::new(
                        Some(2),
                        None,
                        Some(5),
                        None,
                        None,
                        std::collections::BTreeMap::new(),
                    )?),
                    SideEffectClass::None,
                )?,
            },
        )?
        .with_controller_account_transaction(transaction(
            "transition-reserved-link-late-terminal",
            Some((&late_admitted, &origin)),
            vec![ControllerAccountAction::SettleTerminal {
                account: origin.account().clone(),
                reservation: late_reservation.clone(),
                usage: Some(late_usage),
            }],
        )?)?;
        let _ = store.commit_command(&late_terminal)?;
        let late_settled = store
            .controller_account(origin.account())?
            .ok_or("originating controller account disappeared after late settlement")?;
        assert_eq!(late_settled.settled().input_units(), 5);
        assert!(!late_settled.reservations().contains_key(&late_reservation));
        bind_child(&store, &allowed_child, &origin, "late-bind-allowed-child")?;
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

#[path = "controller_account/final_entry.rs"]
mod final_entry;
#[path = "controller_account/integrity.rs"]
mod integrity;
