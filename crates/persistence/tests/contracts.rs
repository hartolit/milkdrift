//! Contract and schema compatibility evidence for persistence-owned documents/ports.

use milkdrift_blueprint::{NodeId, RevisionId, WorkflowId};
use milkdrift_capability::{
    BoundedJson, CapabilityId, InvocationId, InvocationRequest, OperationId, SideEffectClass,
};
use milkdrift_persistence::{
    ActorRef, ArtifactStore, AtomicRunCommitRequest, BranchId, CommandDisposition, CommandId,
    CommandReceipt, CommandResultDocument, CurrencyCode, EventId, IndexedRunState, IntegrityDigest,
    NodeExecutionId, NodeExecutionMode, NodeOutcome, PageSize, PersistenceError, Reason,
    ReconciliationAction, ReconciliationClassification, ReconciliationId, ReconciliationItem,
    ReconciliationPlanId, RepeatContinuationCause, RevisionStore, RunEventEnvelope, RunEventKind,
    RunId, RunIndexUpdate, RunJournal, RunOutcome, RunQueryStore, RunSequence, RunSummaryIndex,
    RunnableIndexMutation, SignalId, SnapshotDocument, SnapshotId, SnapshotStore, StorageAdmin,
    SubworkflowId, SubworkflowOwnership, TimestampMillis, WorkspaceAccounting, WorkspaceMutation,
    WorkspaceStore,
};
use milkdrift_workspace::{
    ScopeId, ScopeReference, ValueKey, ValueVersion, WorkspaceBudget, WorkspaceScope,
    WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry, WorkspaceValueReference,
};
use serde_json::json;

fn sample_event(sequence: u64) -> Result<RunEventEnvelope, PersistenceError> {
    RunEventEnvelope::new(
        EventId::new(format!("event-{sequence:03}"))?,
        RunId::new("run-001").map_err(|error| {
            PersistenceError::InvalidDocument(format!("cannot build test run: {error}"))
        })?,
        RunSequence::new(sequence),
        TimestampMillis::new(1_700_000_000_123),
        RunEventKind::RunStarted,
    )
}

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

#[test]
fn golden_schema_v1_is_stable_and_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let event = sample_event(1)?;
    let encoded = event.to_canonical_json()?;
    let fixture_with_newline = include_bytes!("fixtures/run-event-started-v1.json");
    let fixture = fixture_with_newline
        .strip_suffix(b"\n")
        .unwrap_or(fixture_with_newline);
    assert_eq!(encoded, fixture);
    assert_eq!(RunEventEnvelope::from_json(fixture)?, event);
    Ok(())
}

#[test]
fn future_and_malformed_versions_fail_before_interpretation()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = sample_event(1)?.to_canonical_json()?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
    value["schema_version"] = json!(2);
    let future = serde_json::to_vec(&value)?;
    assert!(matches!(
        RunEventEnvelope::from_json(&future),
        Err(PersistenceError::UnsupportedVersion {
            document: "run_event",
            found: 2,
            supported: 1
        })
    ));

    value["schema_version"] = json!("one");
    let malformed = serde_json::to_vec(&value)?;
    assert!(matches!(
        RunEventEnvelope::from_json(&malformed),
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn checksum_tampering_and_duplicate_keys_are_corruption() -> Result<(), Box<dyn std::error::Error>>
{
    let encoded = String::from_utf8(sample_event(1)?.to_canonical_json()?)?;
    let tampered = encoded.replace("1700000000123", "1700000000124");
    assert!(matches!(
        RunEventEnvelope::from_json(tampered.as_bytes()),
        Err(PersistenceError::Corruption(_))
    ));

    let duplicate = encoded.replacen("{", "{\"schema_version\":1,", 1);
    assert!(matches!(
        RunEventEnvelope::from_json(duplicate.as_bytes()),
        Err(PersistenceError::Json(_))
    ));
    Ok(())
}

#[test]
fn command_results_reject_duplicate_json_keys_before_interpretation()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-command-result-duplicate")?;
    let command = CommandId::new("command-result-duplicate")?;
    let result = CommandResultDocument::new(
        command.clone(),
        run,
        IntegrityDigest::hash(b"command-result-duplicate"),
        CommandDisposition::Rejected,
        RunSequence::ZERO,
        Vec::new(),
        BoundedJson::new(json!({"accepted": false}))?,
    )?;
    let encoded = String::from_utf8(result.to_canonical_json()?)?;
    let duplicate = encoded.replacen("{", "{\"schema_version\":1,", 1);
    assert!(matches!(
        CommandResultDocument::from_json(duplicate.as_bytes()),
        Err(PersistenceError::Json(_))
    ));
    Ok(())
}

#[test]
fn identity_text_and_page_bounds_are_enforced() {
    assert!(CommandId::new("").is_err());
    assert!(CommandId::new("unsafe identity").is_err());
    assert!(CommandId::new("x".repeat(193)).is_err());
    assert!(Reason::new("contains\ncontrol").is_err());
    assert!(Reason::new("é".repeat(1_024)).is_ok());
    assert!(Reason::new("é".repeat(1_025)).is_err());
    assert!(PageSize::new(0).is_err());
    assert!(PageSize::new(1_001).is_err());
}

#[test]
fn atomic_commit_rejects_noncontiguous_and_mismatched_result()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-001")?;
    let command = CommandId::new("command-001")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-001")?,
        RunSequence::ZERO,
        TimestampMillis::new(1),
        br#"{"schema_version":1,"type":"start"}"#.to_vec(),
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::new(2),
        vec![EventId::new("event-002")?],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let revision = revision_id()?;
    let indexes = RunIndexUpdate::new(
        Some(RunSummaryIndex {
            run,
            workflow: WorkflowId::new("workflow-001")?,
            revision,
            state: IndexedRunState::Active,
            through_sequence: RunSequence::new(2),
            updated_at: TimestampMillis::new(1),
        }),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let budget = WorkspaceBudget::new(10, 1024, 4096, 10, 1024, 4096)?;
    let accounting = WorkspaceAccounting {
        budget,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: WorkspaceUsage::EMPTY,
    };
    let result = AtomicRunCommitRequest::new(
        receipt,
        vec![sample_event(2)?],
        Vec::new(),
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        indexes,
    );
    assert!(matches!(result, Err(PersistenceError::InvalidDocument(_))));
    Ok(())
}

#[test]
fn valid_acceptance_and_rejection_documents_preserve_one_sequence_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-valid")?;
    let command = CommandId::new("command-valid")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-valid")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"start"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new("event-valid")?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::RunStarted,
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let accounting = WorkspaceAccounting {
        budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: WorkspaceUsage::EMPTY,
    };
    let request = AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-valid")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?;
    let execution = NodeExecutionId::new("execution-duplicate-index")?;
    let duplicate = RunnableIndexMutation::Remove {
        run: request.receipt().run().clone(),
        execution,
    };
    let duplicate_indexes = RunIndexUpdate::new(
        request.indexes().summary().cloned(),
        vec![duplicate.clone(), duplicate],
        request.indexes().timers().to_vec(),
        request.indexes().leases().to_vec(),
    );
    assert!(matches!(
        AtomicRunCommitRequest::new(
            request.receipt().clone(),
            request.events().to_vec(),
            request.workspace().to_vec(),
            request.workspace_accounting().cloned(),
            request.required_artifacts().to_vec(),
            request.newly_referenced_artifacts().to_vec(),
            request.expected_lease_catalog().cloned(),
            request.result().clone(),
            duplicate_indexes,
        ),
        Err(PersistenceError::InvalidDocument(_))
    ));

    let rejected_run = RunId::new("run-rejected")?;
    let rejected_command = CommandId::new("command-rejected")?;
    let rejected_receipt = CommandReceipt::new(
        rejected_command.clone(),
        rejected_run.clone(),
        ActorRef::new("actor-valid")?,
        RunSequence::ZERO,
        TimestampMillis::new(11),
        br#"{"schema_version":1,"type":"invalid"}"#.to_vec(),
    )?;
    let rejected_result = CommandResultDocument::new(
        rejected_command,
        rejected_run,
        rejected_receipt.fingerprint().clone(),
        CommandDisposition::Rejected,
        RunSequence::ZERO,
        Vec::new(),
        BoundedJson::new(json!({"rejected": true}))?,
    )?;
    assert!(
        AtomicRunCommitRequest::new(
            rejected_receipt,
            Vec::new(),
            Vec::new(),
            None,
            Vec::new(),
            Vec::new(),
            None,
            rejected_result,
            RunIndexUpdate::default(),
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn signal_deduplication_fact_is_bound_to_its_atomic_command()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-signal-dedup-binding")?;
    let command = CommandId::new("command-current-delivery")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-signal")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"deliver_signal"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new("event-signal-dedup")?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::SignalDeduplicated {
            signal: SignalId::new("signal-existing")?,
            duplicate_command: CommandId::new("command-unrelated")?,
        },
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let accounting = WorkspaceAccounting {
        budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: WorkspaceUsage::EMPTY,
    };
    let request = AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-signal")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    );
    assert!(matches!(request, Err(PersistenceError::InvalidDocument(_))));
    Ok(())
}

#[test]
fn atomic_workspace_mutations_exactly_materialize_subworkflow_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-subworkflow-materialization")?;
    let command = CommandId::new("command-subworkflow-materialization")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-subworkflow")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"materialize_subworkflow"}"#.to_vec(),
    )?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let subworkflow = SubworkflowId::new("subworkflow-materialized")?;
    let scope = WorkspaceScope::subworkflow(
        ScopeId::new("subworkflow-scope")?,
        &root,
        subworkflow.clone(),
    )?;
    let input = WorkspaceValueEntry::initial(
        scope.reference().clone(),
        ValueKey::new("input")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 1}))?),
    );
    let event = RunEventEnvelope::new(
        EventId::new("event-subworkflow-materialized")?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::SubworkflowCreated {
            subworkflow,
            parent_execution: NodeExecutionId::new("execution-parent")?,
            child_run: RunId::new("run-subworkflow-child")?,
            child_revision: revision_id()?,
            scope: scope.clone(),
            ownership: SubworkflowOwnership::Attached,
            inputs: vec![input.reference().clone()],
        },
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let budget = WorkspaceBudget::new(10, 10_000, 10_000, 10, 10_000, 10_000)?;
    let accounting = WorkspaceAccounting {
        budget: budget.clone(),
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: budget.admit_value(&WorkspaceUsage::EMPTY, input.value())?,
    };
    let request = AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        vec![
            WorkspaceMutation::CreateScope {
                scope: scope.clone(),
            },
            WorkspaceMutation::PutValue {
                entry: input.clone(),
            },
        ],
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-subworkflow")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?;

    let hidden = WorkspaceValueEntry::initial(
        scope.reference().clone(),
        ValueKey::new("hidden")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"hidden": true}))?),
    );
    let mut hidden_workspace = request.workspace().to_vec();
    hidden_workspace.push(WorkspaceMutation::PutValue { entry: hidden });
    assert!(matches!(
        AtomicRunCommitRequest::new(
            request.receipt().clone(),
            request.events().to_vec(),
            hidden_workspace,
            request.workspace_accounting().cloned(),
            request.required_artifacts().to_vec(),
            request.newly_referenced_artifacts().to_vec(),
            request.expected_lease_catalog().cloned(),
            request.result().clone(),
            request.indexes().clone(),
        ),
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn command_fingerprint_binds_identity_actor_and_semantic_document()
-> Result<(), Box<dyn std::error::Error>> {
    let make = |actor: &str, document: &[u8]| {
        Ok::<_, Box<dyn std::error::Error>>(CommandReceipt::new(
            CommandId::new("command-fingerprint")?,
            RunId::new("run-fingerprint")?,
            ActorRef::new(actor)?,
            RunSequence::new(9),
            TimestampMillis::new(10),
            document.to_vec(),
        )?)
    };
    let first = make("actor-one", br#"{"schema_version":1,"type":"pause"}"#)?;
    let same = make("actor-one", br#"{"schema_version":1,"type":"pause"}"#)?;
    let other_actor = make("actor-two", br#"{"schema_version":1,"type":"pause"}"#)?;
    let other_body = make("actor-one", br#"{"schema_version":1,"type":"resume"}"#)?;
    assert_eq!(first.fingerprint(), same.fingerprint());
    assert_ne!(first.fingerprint(), other_actor.fingerprint());
    assert_ne!(first.fingerprint(), other_body.fingerprint());
    assert!(
        CommandReceipt::new(
            CommandId::new("noncanonical")?,
            RunId::new("run-fingerprint")?,
            ActorRef::new("actor-one")?,
            RunSequence::ZERO,
            TimestampMillis::new(1),
            b"{ \"type\": \"pause\", \"schema_version\": 1 }".to_vec(),
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn event_semantics_reject_cross_run_workspace_and_incomplete_failure_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let cross_run = RunEventEnvelope::new(
        EventId::new("event-cross-run")?,
        RunId::new("run-one")?,
        RunSequence::FIRST,
        TimestampMillis::new(1),
        RunEventKind::NodeBecameEligible {
            node: NodeId::new("node-one")?,
            execution: NodeExecutionId::new("execution-one")?,
            scope: ScopeReference::new(RunId::new("run-two")?, ScopeId::new("root")?),
            mode: NodeExecutionMode::Executor,
        },
    );
    assert!(matches!(
        cross_run,
        Err(PersistenceError::InvalidDocument(_))
    ));

    let incomplete_failure = RunEventEnvelope::new(
        EventId::new("event-failure")?,
        RunId::new("run-one")?,
        RunSequence::FIRST,
        TimestampMillis::new(1),
        RunEventKind::NodeTerminal {
            execution: NodeExecutionId::new("execution-one")?,
            attempt: milkdrift_persistence::AttemptId::new("attempt-one")?,
            report_sequence: 1,
            outcome: NodeOutcome::Failed,
            error_class: None,
            detail: None,
        },
    );
    assert!(matches!(
        incomplete_failure,
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn executor_report_facts_require_nonzero_report_sequences() -> Result<(), Box<dyn std::error::Error>>
{
    let run = RunId::new("run-report-sequence")?;
    let execution = NodeExecutionId::new("execution-report-sequence")?;
    let attempt = milkdrift_persistence::AttemptId::new("attempt-report-sequence")?;
    let value = WorkspaceValueReference::new(
        ScopeReference::new(run.clone(), ScopeId::new("root")?),
        ValueKey::new("output")?,
        ValueVersion::FIRST,
    );
    let kinds = [
        RunEventKind::NodeOutputPublished {
            execution: execution.clone(),
            attempt: attempt.clone(),
            report_sequence: 0,
            value,
            artifact: None,
        },
        RunEventKind::NodeTerminal {
            execution,
            attempt: attempt.clone(),
            report_sequence: 0,
            outcome: NodeOutcome::Succeeded,
            error_class: None,
            detail: None,
        },
        RunEventKind::ExternalOutcomeUncertain {
            attempt,
            report_sequence: 0,
            side_effect: SideEffectClass::Unknown,
            reason: Reason::new("outcome unavailable")?,
            evidence: Vec::new(),
        },
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        assert!(matches!(
            RunEventEnvelope::new(
                EventId::new(format!("event-zero-report-{index}"))?,
                run.clone(),
                RunSequence::FIRST,
                TimestampMillis::new(1),
                kind,
            ),
            Err(PersistenceError::InvalidDocument(_))
        ));
    }
    Ok(())
}

#[test]
fn scheduled_request_must_match_its_invocation_and_idempotency_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-scheduled-request")?;
    let request = InvocationRequest::new(
        InvocationId::new("invocation-request")?,
        CapabilityId::new("capability-request")?,
        OperationId::new("tool.execute")?,
        None,
        None,
        Vec::new(),
        std::collections::BTreeMap::new(),
    )?;
    let mismatched = RunEventEnvelope::new(
        EventId::new("event-scheduled-request")?,
        run,
        RunSequence::FIRST,
        TimestampMillis::new(1),
        RunEventKind::NodeScheduled {
            node: NodeId::new("node-request")?,
            execution: NodeExecutionId::new("execution-request")?,
            attempt: milkdrift_persistence::AttemptId::new("attempt-request")?,
            invocation: InvocationId::new("invocation-event")?,
            idempotency_key: None,
            request,
        },
    );
    assert!(matches!(
        mismatched,
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn repeat_continuation_requests_require_bounded_limits_and_exhausted_typed_causes()
-> Result<(), Box<dyn std::error::Error>> {
    let run = RunId::new("run-repeat-request-validation")?;
    let execution = NodeExecutionId::new("execution-repeat")?;
    let invalid_kinds = [
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: execution.clone(),
            frontier_iteration: milkdrift_persistence::IterationId::new("iteration-zero")?,
            initial_iteration_limit: 0,
            effective_iteration_limit: 1,
            cause: RepeatContinuationCause::IterationLimit,
        },
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: execution.clone(),
            frontier_iteration: milkdrift_persistence::IterationId::new("iteration-duration")?,
            initial_iteration_limit: 1,
            effective_iteration_limit: 1,
            cause: RepeatContinuationCause::DurationBudget {
                maximum_ms: 100,
                observed_ms: 99,
            },
        },
        RunEventKind::RepeatContinuationRequested {
            repeat_execution: execution,
            frontier_iteration: milkdrift_persistence::IterationId::new("iteration-cost")?,
            initial_iteration_limit: 1,
            effective_iteration_limit: 1,
            cause: RepeatContinuationCause::CostBudget {
                maximum_micros: 100,
                observed_micros: 99,
                currency: CurrencyCode::new("USD")?,
            },
        },
    ];
    for (index, kind) in invalid_kinds.into_iter().enumerate() {
        assert!(matches!(
            RunEventEnvelope::new(
                EventId::new(format!("event-invalid-repeat-request-{index}"))?,
                run.clone(),
                RunSequence::FIRST,
                TimestampMillis::new(1),
                kind,
            ),
            Err(PersistenceError::InvalidDocument(_))
        ));
    }
    Ok(())
}

#[test]
fn reconciliation_plan_bound_fits_atomic_action_application()
-> Result<(), Box<dyn std::error::Error>> {
    let make_items = |count: usize| -> Result<Vec<ReconciliationItem>, Box<dyn std::error::Error>> {
        (0..count)
            .map(|index| {
                Ok(ReconciliationItem {
                    node: Some(NodeId::new(format!("node-{index}"))?),
                    execution: None,
                    classification: ReconciliationClassification::Added,
                    action: ReconciliationAction::UseNewOnNextInvocation,
                    reason: Reason::new("prospective node")?,
                })
            })
            .collect()
    };
    let make_event = |items| {
        RunEventEnvelope::new(
            EventId::new("event-reconciliation-bound")?,
            RunId::new("run-reconciliation-bound").map_err(|error| {
                PersistenceError::InvalidDocument(format!("cannot build test run: {error}"))
            })?,
            RunSequence::FIRST,
            TimestampMillis::new(1),
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation: ReconciliationId::new("reconciliation-bound")?,
                plan: ReconciliationPlanId::new("plan-bound")?,
                from_revision: revision_id()?,
                to_revision: revision_id()?,
                based_on_sequence: RunSequence::ZERO,
                items,
            },
        )
    };
    assert!(make_event(make_items(510)?).is_ok());
    assert!(matches!(
        make_event(make_items(511)?),
        Err(PersistenceError::Bounds {
            location: "event.reconciliation.items",
            ..
        })
    ));
    Ok(())
}

#[test]
fn deterministic_branch_and_cross_run_subworkflow_output_facts_are_explicit()
-> Result<(), Box<dyn std::error::Error>> {
    let parent_run = RunId::new("parent-output-run")?;
    let child_run = RunId::new("child-output-run")?;
    let parent_value = WorkspaceValueReference::new(
        ScopeReference::new(parent_run.clone(), ScopeId::new("parent-root")?),
        ValueKey::new("imported")?,
        ValueVersion::FIRST,
    );
    let child_value = WorkspaceValueReference::new(
        ScopeReference::new(child_run.clone(), ScopeId::new("child-root")?),
        ValueKey::new("result")?,
        ValueVersion::FIRST,
    );
    let event = |id: &str, kind| {
        RunEventEnvelope::new(
            EventId::new(id)?,
            parent_run.clone(),
            RunSequence::FIRST,
            TimestampMillis::new(1),
            kind,
        )
    };

    assert!(
        event(
            "event-deterministic-output",
            RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-reducer")?,
                value: parent_value.clone(),
                artifact: None,
            },
        )
        .is_ok()
    );
    assert!(
        event(
            "event-branch-terminal",
            RunEventKind::BranchTerminal {
                branch: BranchId::new("branch-one")?,
                outcome: milkdrift_persistence::RunOutcome::Succeeded,
                outputs: vec![parent_value.clone()],
            },
        )
        .is_ok()
    );
    assert!(
        event(
            "event-subworkflow-terminal",
            RunEventKind::SubworkflowTerminal {
                subworkflow: SubworkflowId::new("subworkflow-one")?,
                child_run: child_run.clone(),
                outcome: milkdrift_persistence::RunOutcome::Succeeded,
                outputs: vec![child_value.clone()],
            },
        )
        .is_ok()
    );
    assert!(
        event(
            "event-subworkflow-import",
            RunEventKind::SubworkflowOutputImported {
                subworkflow: SubworkflowId::new("subworkflow-one")?,
                child_value: child_value.clone(),
                parent_value: parent_value.clone(),
            },
        )
        .is_ok()
    );
    assert!(matches!(
        event(
            "event-invalid-subworkflow-import",
            RunEventKind::SubworkflowOutputImported {
                subworkflow: SubworkflowId::new("subworkflow-one")?,
                child_value: parent_value,
                parent_value: child_value,
            },
        ),
        Err(PersistenceError::InvalidDocument(_))
    ));
    let plan = ReconciliationPlanId::new("plan-apply-actions")?;
    assert!(
        event(
            "event-reconciliation-removed",
            RunEventKind::ReconciliationExecutionRemoved {
                plan: plan.clone(),
                execution: NodeExecutionId::new("execution-removed")?,
            },
        )
        .is_ok()
    );
    assert!(
        event(
            "event-reconciliation-cancel",
            RunEventKind::ReconciliationCancellationRequested {
                plan: plan.clone(),
                execution: NodeExecutionId::new("execution-cancelled")?,
                attempt: milkdrift_persistence::AttemptId::new("attempt-cancelled")?,
                reason: Reason::new("safe cancellation boundary")?,
            },
        )
        .is_ok()
    );
    assert!(
        event(
            "event-reconciliation-remediation",
            RunEventKind::ReconciliationRemediationCreated {
                plan,
                source_execution: NodeExecutionId::new("execution-source")?,
                source_attempt: None,
                execution: NodeExecutionId::new("execution-remediation")?,
                node: NodeId::new("node-remediation")?,
                scope: ScopeReference::new(parent_run.clone(), ScopeId::new("parent-root")?),
                mode: NodeExecutionMode::Executor,
                reason: Reason::new("prospective remediation")?,
            },
        )
        .is_ok()
    );
    Ok(())
}

#[test]
fn workflow_level_reconciliation_incompatibility_may_omit_node_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let make = |classification| {
        RunEventEnvelope::new(
            EventId::new(format!("event-reconciliation-{classification:?}"))?,
            RunId::new("run-reconciliation").map_err(|error| {
                PersistenceError::InvalidDocument(format!("cannot build test run: {error}"))
            })?,
            RunSequence::FIRST,
            TimestampMillis::new(1),
            RunEventKind::ReconciliationPlanRecorded {
                reconciliation: ReconciliationId::new("reconciliation-one")?,
                plan: ReconciliationPlanId::new("plan-one")?,
                from_revision: revision_id()?,
                to_revision: serde_json::from_value(json!(format!("rev_{}", "1".repeat(64))))?,
                based_on_sequence: RunSequence::FIRST,
                items: vec![ReconciliationItem {
                    node: None,
                    execution: None,
                    classification,
                    action: ReconciliationAction::RequireAuthority,
                    reason: Reason::new("workflow interface changed")?,
                }],
            },
        )
    };

    assert!(make(ReconciliationClassification::IncompatibleInterfaceOrSubworkflow).is_ok());
    assert!(matches!(
        make(ReconciliationClassification::RequiresAuthority),
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}

#[test]
fn snapshot_checksum_and_history_prefix_are_verified() -> Result<(), Box<dyn std::error::Error>> {
    let event = sample_event(1)?;
    let history = milkdrift_persistence::history_digest(std::slice::from_ref(&event))?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-001")?,
        event.run_id().clone(),
        RunSequence::FIRST,
        history,
        1,
        br#"{"projection":"created"}"#.to_vec(),
    )?;
    let encoded = snapshot.to_canonical_json()?;
    assert_eq!(SnapshotDocument::from_json(&encoded)?, snapshot);
    let mut tampered_value: serde_json::Value = serde_json::from_slice(&encoded)?;
    tampered_value["payload"][0] = json!(0);
    let tampered = serde_json::to_vec(&tampered_value)?;
    assert!(matches!(
        SnapshotDocument::from_json(&tampered),
        Err(PersistenceError::Corruption(_))
    ));
    let duplicate = String::from_utf8(encoded)?.replacen("{", "{\"schema_version\":1,", 1);
    assert!(matches!(
        SnapshotDocument::from_json(duplicate.as_bytes()),
        Err(PersistenceError::Json(_))
    ));
    assert!(milkdrift_persistence::history_digest(&[sample_event(2)?]).is_err());
    Ok(())
}

#[test]
fn every_port_is_object_safe() {
    fn accepts_object<T: ?Sized>() {}
    accepts_object::<dyn RunJournal>();
    accepts_object::<dyn RunQueryStore>();
    accepts_object::<dyn RevisionStore>();
    accepts_object::<dyn SnapshotStore>();
    accepts_object::<dyn WorkspaceStore>();
    accepts_object::<dyn ArtifactStore>();
    accepts_object::<dyn StorageAdmin>();
}

#[test]
fn integrity_digest_has_one_canonical_spelling() {
    assert!(IntegrityDigest::new(format!("b3_{}", "a".repeat(64))).is_ok());
    assert!(IntegrityDigest::new("a".repeat(64)).is_err());
    assert!(IntegrityDigest::new(format!("b3_{}", "A".repeat(64))).is_err());
}

#[test]
fn explicit_failure_drain_is_additive_schema_v1_history() -> Result<(), Box<dyn std::error::Error>>
{
    let run = RunId::new("run-explicit-failure-drain")?;
    let event = RunEventEnvelope::new(
        EventId::new("event-explicit-failure-drain")?,
        run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(10),
        RunEventKind::RunTerminationRequested {
            outcome: RunOutcome::Failed,
            reason: Reason::new("explicit terminal selected failure")?,
        },
    )?;
    assert_eq!(
        RunEventEnvelope::from_json(&event.to_canonical_json()?)?,
        event
    );

    for (suffix, outcome) in [
        ("success", RunOutcome::Succeeded),
        ("cancelled", RunOutcome::Cancelled),
    ] {
        assert!(matches!(
            RunEventEnvelope::new(
                EventId::new(format!("event-invalid-explicit-{suffix}"))?,
                run.clone(),
                RunSequence::FIRST,
                TimestampMillis::new(10),
                RunEventKind::RunTerminationRequested {
                    outcome,
                    reason: Reason::new("unsupported internal drain")?,
                },
            ),
            Err(PersistenceError::InvalidDocument(_))
        ));
    }
    Ok(())
}

#[test]
fn semantic_command_fingerprint_excludes_retry_delivery_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let command = CommandId::new("semantic-redelivery")?;
    let run = RunId::new("run-semantic-redelivery")?;
    let actor = ActorRef::new("actor-semantic-redelivery")?;
    let intent = br#"{"command":"pause","schema_version":1}"#.to_vec();
    let first = CommandReceipt::new_idempotent(
        command.clone(),
        run.clone(),
        actor.clone(),
        RunSequence::new(1),
        TimestampMillis::new(10),
        br#"{"expected_sequence":1,"issued_at":10}"#.to_vec(),
        intent.clone(),
    )?;
    let retry = CommandReceipt::new_idempotent(
        command,
        run,
        actor,
        RunSequence::new(9),
        TimestampMillis::new(20),
        br#"{"expected_sequence":9,"issued_at":20}"#.to_vec(),
        intent,
    )?;
    assert_eq!(first.fingerprint(), retry.fingerprint());
    assert_eq!(first.canonical_intent(), retry.canonical_intent());
    assert_ne!(first.canonical_document(), retry.canonical_document());
    assert_ne!(first.expected_sequence(), retry.expected_sequence());
    assert_ne!(first.submitted_at(), retry.submitted_at());
    Ok(())
}
