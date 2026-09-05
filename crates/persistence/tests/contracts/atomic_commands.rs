use super::*;

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
            request.expected_lease_revision().cloned(),
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
            request.expected_lease_revision().cloned(),
            request.result().clone(),
            request.indexes().clone(),
        ),
        Err(PersistenceError::InvalidDocument(_))
    ));
    Ok(())
}
