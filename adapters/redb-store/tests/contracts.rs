//! End-to-end durability, conflict, corruption, and artifact contract tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use milkdrift_blueprint::{BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    AtomicRunCommitOutcome, AtomicRunCommitRequest, AttemptId, BeginArtifactOutcome,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, EventPageQuery, ImmutableRevisionPut, IndexedRunState, IntegrityScanRequest, LeaseId,
    LeaseIndexEntry, LeaseIndexMutation, NodeExecutionId, OrphanCleanupRequest, PageSize,
    PersistenceError, RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal,
    RunQueryStore, RunSequence, RunSummaryIndex, RunnableIndexEntry, RunnableIndexMutation,
    SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore, StorageAdmin, StorageFailureClass,
    TimestampMillis, WorkerId, WorkspaceAccounting, WorkspaceMutation, WorkspaceStore,
    history_digest,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    BranchId, CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ValueKey,
    WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry,
};
use redb::{Database, TableDefinition};
use serde_json::json;
use tempfile::TempDir;

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn accepted_request(
    run: &str,
    command: &str,
    event: &str,
    command_type: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let run = RunId::new(run)?;
    let command = CommandId::new(command)?;
    let document = format!(r#"{{"schema_version":1,"type":"{command_type}"}}"#).into_bytes();
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        document,
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(event)?,
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
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(accounting),
        Vec::new(),
        Vec::new(),
        result,
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            ..RunIndexUpdate::default()
        },
    )?)
}

fn accepted_request_with_runnable(
    run: &str,
    command: &str,
    event: &str,
    entries: Vec<RunnableIndexEntry>,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let request = accepted_request(run, command, event, "start")?;
    let mut indexes = request.indexes.clone();
    indexes.runnable = entries
        .into_iter()
        .map(|entry| RunnableIndexMutation::Upsert { entry })
        .collect();
    Ok(AtomicRunCommitRequest::new(
        request.receipt,
        request.events,
        request.workspace,
        request.workspace_accounting,
        request.required_artifacts,
        request.newly_referenced_artifacts,
        request.result,
        indexes,
    )?)
}

fn accepted_request_with_lease(
    run: &str,
    command: &str,
    event: &str,
    entry: LeaseIndexEntry,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let request = accepted_request(run, command, event, "start")?;
    let mut indexes = request.indexes.clone();
    indexes.leases = vec![LeaseIndexMutation::Upsert { entry }];
    Ok(AtomicRunCommitRequest::new(
        request.receipt,
        request.events,
        request.workspace,
        request.workspace_accounting,
        request.required_artifacts,
        request.newly_referenced_artifacts,
        request.result,
        indexes,
    )?)
}

#[test]
fn reopen_and_single_owner_are_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.schema_info()?.stored_version, 1);
    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            ..
        })
    ));
    drop(store);
    let reopened = RedbStore::open(directory.path())?;
    assert_eq!(reopened.schema_info()?.stored_version, 1);
    Ok(())
}

#[test]
fn future_storage_schema_is_refused() -> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    let directory = TempDir::new()?;
    drop(RedbStore::open(directory.path())?);
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(METADATA)?;
        metadata.insert("storage_schema_version", 2)?;
    }
    write.commit()?;
    drop(database);
    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::UnsupportedVersion {
            document: "storage",
            found: 2,
            supported: 1
        })
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn owned_storage_paths_refuse_symlinked_database_and_artifact_directories()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let database_root = TempDir::new()?;
    let external_database = tempfile::NamedTempFile::new()?;
    symlink(
        external_database.path(),
        database_root.path().join("milkdrift.redb"),
    )?;
    assert!(matches!(
        RedbStore::open(database_root.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));

    let artifact_root = TempDir::new()?;
    let external_artifacts = TempDir::new()?;
    symlink(
        external_artifacts.path(),
        artifact_root.path().join("artifacts"),
    )?;
    assert!(matches!(
        RedbStore::open(artifact_root.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

#[test]
fn revision_lookup_and_integrity_scan_detect_physical_key_mismatches()
-> Result<(), Box<dyn std::error::Error>> {
    const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.revisions.by_id");
    const EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.runs.events");

    let directory = TempDir::new()?;
    let revision_bytes =
        include_bytes!("../../../crates/blueprint/tests/fixtures/revision-v1.json");
    let (_document, revision) = BlueprintRevisionDocument::from_json(revision_bytes)?;
    let request = accepted_request(
        "run-key-audit",
        "command-key-audit",
        "event-key-audit",
        "start",
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        store.put_revision(&revision)?;
        store.commit_command(&request)?;
    }

    let wrong_revision = format!("rev_{}", "f".repeat(64));
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut revisions = write.open_table(REVISIONS)?;
        revisions.insert(wrong_revision.as_str(), revision_bytes.as_slice())?;
        let mut events = write.open_table(EVENTS)?;
        let event_bytes = request.events[0].to_canonical_json()?;
        let mut wrong_event_key = Vec::new();
        wrong_event_key
            .extend_from_slice(&(request.receipt.run().as_str().len() as u32).to_be_bytes());
        wrong_event_key.extend_from_slice(request.receipt.run().as_str().as_bytes());
        wrong_event_key.extend_from_slice(&2_u64.to_be_bytes());
        events.insert(wrong_event_key.as_slice(), event_bytes.as_slice())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let wrong_revision = serde_json::from_value(json!(wrong_revision))?;
    assert!(matches!(
        store.revision(&wrong_revision),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    let scan = store.scan_integrity(IntegrityScanRequest {
        limit: PageSize::new(100)?,
        verify_artifact_content: false,
    })?;
    assert!(scan.failures.len() >= 2);
    Ok(())
}

#[test]
fn journal_reopens_and_idempotency_conflicts_without_duplicate_events()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = accepted_request("run-idempotent", "command-one", "event-one", "start")?;
    {
        let store = RedbStore::open(directory.path())?;
        assert!(matches!(
            store.commit_command(&request)?,
            AtomicRunCommitOutcome::Committed(_)
        ));
        assert!(matches!(
            store.commit_command(&request)?,
            AtomicRunCommitOutcome::Replayed(_)
        ));
        let conflict =
            accepted_request("run-idempotent", "command-one", "event-other", "different")?;
        assert!(matches!(
            store.commit_command(&conflict),
            Err(PersistenceError::IdempotencyConflict { .. })
        ));
    }
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.head(request.receipt.run())?, RunSequence::FIRST);
    let page = store.events(&EventPageQuery::new(
        request.receipt.run().clone(),
        None,
        PageSize::new(10)?,
    )?)?;
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id(), request.events[0].event_id());
    Ok(())
}

#[test]
fn runnable_page_is_bounded_by_distinct_runs_not_noisy_run_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let noisy_run = RunId::new("run-noisy")?;
    let quiet_run = RunId::new("run-quiet")?;
    let noisy_entries = (0_u16..64)
        .map(|priority| {
            Ok(RunnableIndexEntry {
                run: noisy_run.clone(),
                execution: NodeExecutionId::new(format!("noisy-execution-{priority:02}"))?,
                eligible_at: TimestampMillis::new(1),
                priority,
                through_sequence: RunSequence::FIRST,
            })
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    store.commit_command(&accepted_request_with_runnable(
        noisy_run.as_str(),
        "command-noisy",
        "event-noisy",
        noisy_entries,
    )?)?;
    store.commit_command(&accepted_request_with_runnable(
        quiet_run.as_str(),
        "command-quiet",
        "event-quiet",
        vec![RunnableIndexEntry {
            run: quiet_run.clone(),
            execution: NodeExecutionId::new("quiet-execution")?,
            eligible_at: TimestampMillis::new(2),
            priority: 1,
            through_sequence: RunSequence::FIRST,
        }],
    )?)?;

    // A raw timestamp-ordered page of this size contains only noisy-run rows.
    // The query contract instead walks the grouped identity index and returns one
    // best candidate for each distinct run represented by the page bound.
    let runnable = store.runnable(TimestampMillis::new(10), PageSize::new(2)?)?;
    assert_eq!(runnable.len(), 2);
    assert!(runnable.iter().any(|entry| entry.run == quiet_run));
    let selected_noisy = runnable
        .iter()
        .find(|entry| entry.run == noisy_run)
        .ok_or("no noisy-run candidate was returned")?;
    assert_eq!(selected_noisy.priority, 63);
    assert_eq!(selected_noisy.execution.as_str(), "noisy-execution-63");

    // Even a one-item scheduler budget progresses to another run on the next
    // bounded discovery call instead of restarting at the noisy run forever.
    let first = store.runnable_page(TimestampMillis::new(10), None, PageSize::new(1)?)?;
    assert_eq!(first.entries.len(), 1);
    assert!(first.next.is_some());
    let second = store.runnable_page(
        TimestampMillis::new(10),
        first.next.as_ref(),
        PageSize::new(1)?,
    )?;
    assert_eq!(second.entries.len(), 1);
    assert!(second.next.is_none());
    assert_ne!(first.entries[0].run, second.entries[0].run);
    assert!(
        first
            .entries
            .iter()
            .chain(&second.entries)
            .any(|entry| entry.run == quiet_run)
    );
    Ok(())
}

#[test]
fn nonterminal_run_pages_resume_past_early_runs() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..5 {
        store.commit_command(&accepted_request(
            &format!("run-page-{ordinal}"),
            &format!("command-page-{ordinal}"),
            &format!("event-page-{ordinal}"),
            "start",
        )?)?;
    }

    let first = store.nonterminal_run_page(None, PageSize::new(2)?)?;
    assert_eq!(first.runs.len(), 2);
    let second = store.nonterminal_run_page(first.next.as_ref(), PageSize::new(2)?)?;
    assert_eq!(second.runs.len(), 2);
    let third = store.nonterminal_run_page(second.next.as_ref(), PageSize::new(2)?)?;
    assert_eq!(third.runs.len(), 1);
    assert!(third.next.is_none());

    let discovered = first
        .runs
        .iter()
        .chain(&second.runs)
        .chain(&third.runs)
        .map(|summary| summary.run.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(discovered.len(), 5);
    for ordinal in 0..5 {
        assert!(discovered.contains(format!("run-page-{ordinal}").as_str()));
    }
    Ok(())
}

#[test]
fn active_lease_query_is_a_bounded_complete_prefix() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..3 {
        let run = RunId::new(format!("run-lease-{ordinal}"))?;
        store.commit_command(&accepted_request_with_lease(
            run.as_str(),
            &format!("command-lease-{ordinal}"),
            &format!("event-lease-{ordinal}"),
            LeaseIndexEntry {
                run: run.clone(),
                lease: LeaseId::new(format!("lease-{ordinal}"))?,
                attempt: AttemptId::new(format!("attempt-{ordinal}"))?,
                worker: WorkerId::new("worker-test")?,
                expires_at: TimestampMillis::new(100 + ordinal),
                through_sequence: RunSequence::FIRST,
            },
        )?)?;
    }

    assert_eq!(store.active_leases(PageSize::new(2)?)?.len(), 2);
    assert_eq!(store.active_leases(PageSize::new(4)?)?.len(), 3);
    Ok(())
}

#[test]
fn durable_workspace_imports_require_an_exact_cross_run_source_without_ancestry()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;

    let child_root =
        WorkspaceScope::run_root(RunId::new("child-import-run")?, ScopeId::new("child-root")?);
    let child_value = WorkspaceValueEntry::initial(
        child_root.reference().clone(),
        ValueKey::new("result")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 42}))?),
    );
    let child_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let child_usage = child_budget.admit_value(&WorkspaceUsage::EMPTY, child_value.value())?;
    let mut child_request = accepted_request(
        "child-import-run",
        "command-child-import",
        "event-child-import",
        "start",
    )?;
    child_request.workspace = vec![
        WorkspaceMutation::CreateScope {
            scope: child_root.clone(),
        },
        WorkspaceMutation::PutValue {
            entry: child_value.clone(),
        },
    ];
    child_request.workspace_accounting = Some(WorkspaceAccounting {
        budget: child_budget,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: child_usage,
    });
    store.commit_command(&child_request)?;

    let parent_root = WorkspaceScope::run_root(
        RunId::new("parent-import-run")?,
        ScopeId::new("parent-root")?,
    );
    let imported = WorkspaceValueEntry::imported(
        parent_root.reference().clone(),
        ValueKey::new("child-result")?,
        child_value.reference().clone(),
        child_value.value().clone(),
    )?;
    let parent_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let parent_usage = parent_budget.admit_value(&WorkspaceUsage::EMPTY, imported.value())?;
    let mut parent_request = accepted_request(
        "parent-import-run",
        "command-parent-import",
        "event-parent-import",
        "start",
    )?;
    parent_request.workspace = vec![
        WorkspaceMutation::CreateScope { scope: parent_root },
        WorkspaceMutation::PutValue {
            entry: imported.clone(),
        },
    ];
    parent_request.workspace_accounting = Some(WorkspaceAccounting {
        budget: parent_budget,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: parent_usage,
    });
    store.commit_command(&parent_request)?;
    assert_eq!(store.value(imported.reference())?, Some(imported));

    let altered_root = WorkspaceScope::run_root(
        RunId::new("parent-altered-import")?,
        ScopeId::new("parent-root")?,
    );
    let altered_import = WorkspaceValueEntry::imported(
        altered_root.reference().clone(),
        ValueKey::new("altered")?,
        child_value.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 43}))?),
    )?;
    let altered_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let altered_usage =
        altered_budget.admit_value(&WorkspaceUsage::EMPTY, altered_import.value())?;
    let mut altered_request = accepted_request(
        "parent-altered-import",
        "command-altered-import",
        "event-altered-import",
        "start",
    )?;
    altered_request.workspace = vec![
        WorkspaceMutation::CreateScope {
            scope: altered_root,
        },
        WorkspaceMutation::PutValue {
            entry: altered_import,
        },
    ];
    altered_request.workspace_accounting = Some(WorkspaceAccounting {
        budget: altered_budget,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: altered_usage,
    });
    assert!(matches!(
        store.commit_command(&altered_request),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(
        store.head(altered_request.receipt.run())?,
        RunSequence::ZERO
    );

    let missing_root = WorkspaceScope::run_root(
        RunId::new("parent-missing-import")?,
        ScopeId::new("parent-root")?,
    );
    let missing_source = WorkspaceValueEntry::initial(
        WorkspaceScope::run_root(RunId::new("absent-child-run")?, ScopeId::new("child-root")?)
            .reference()
            .clone(),
        ValueKey::new("absent")?,
        WorkspaceValue::Json(BoundedJson::new(json!(null))?),
    );
    let missing_import = WorkspaceValueEntry::imported(
        missing_root.reference().clone(),
        ValueKey::new("missing")?,
        missing_source.reference().clone(),
        missing_source.value().clone(),
    )?;
    let missing_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let missing_usage =
        missing_budget.admit_value(&WorkspaceUsage::EMPTY, missing_import.value())?;
    let mut missing_request = accepted_request(
        "parent-missing-import",
        "command-missing-import",
        "event-missing-import",
        "start",
    )?;
    missing_request.workspace = vec![
        WorkspaceMutation::CreateScope {
            scope: missing_root,
        },
        WorkspaceMutation::PutValue {
            entry: missing_import,
        },
    ];
    missing_request.workspace_accounting = Some(WorkspaceAccounting {
        budget: missing_budget,
        expected_usage: WorkspaceUsage::EMPTY,
        resulting_usage: missing_usage,
    });
    assert!(matches!(
        store.commit_command(&missing_request),
        Err(PersistenceError::NotFound {
            entity: "imported_workspace_value",
            ..
        })
    ));
    assert_eq!(
        store.head(missing_request.receipt.run())?,
        RunSequence::ZERO
    );
    Ok(())
}

#[test]
fn durable_workspace_rejects_scope_lineages_beyond_the_contract_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let mut request = accepted_request(
        "run-scope-depth",
        "command-scope-depth",
        "event-scope-depth",
        "start",
    )?;
    let root = WorkspaceScope::run_root(
        request.receipt.run().clone(),
        ScopeId::new("scope-depth-root")?,
    );
    request.workspace.push(WorkspaceMutation::CreateScope {
        scope: root.clone(),
    });
    let mut parent = root;
    for depth in 1..milkdrift_workspace::MAX_SCOPE_DEPTH {
        let child = WorkspaceScope::branch(
            ScopeId::new(format!("scope-depth-{depth}"))?,
            &parent,
            BranchId::new(format!("branch-depth-{depth}"))?,
        )?;
        request.workspace.push(WorkspaceMutation::CreateScope {
            scope: child.clone(),
        });
        parent = child;
    }
    let too_deep = WorkspaceScope::branch(
        ScopeId::new("scope-depth-overflow")?,
        &parent,
        BranchId::new("branch-depth-overflow")?,
    )?;
    request
        .workspace
        .push(WorkspaceMutation::CreateScope { scope: too_deep });
    assert!(matches!(
        store.commit_command(&request),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(store.head(request.receipt.run())?, RunSequence::ZERO);
    let root_scope_id = ScopeId::new("scope-depth-root")?;
    assert!(
        store
            .scope(request.receipt.run(), &root_scope_id)?
            .is_none()
    );
    Ok(())
}

struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

#[test]
fn command_fault_boundaries_are_atomic_and_replayable() -> Result<(), Box<dyn std::error::Error>> {
    let before_directory = TempDir::new()?;
    let before = RedbStore::open_with_config(
        RedbStoreConfig::new(before_directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::BeforeCommandCommit))),
    )?;
    let request = accepted_request("run-before", "command-before", "event-before", "start")?;
    assert!(before.commit_command(&request).is_err());
    assert_eq!(before.head(request.receipt.run())?, RunSequence::ZERO);
    assert!(
        before
            .command_result(request.receipt.run(), request.receipt.command())?
            .is_none()
    );

    let after_directory = TempDir::new()?;
    let after = RedbStore::open_with_config(
        RedbStoreConfig::new(after_directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterCommandCommit))),
    )?;
    let request = accepted_request("run-after", "command-after", "event-after", "start")?;
    assert!(after.commit_command(&request).is_err());
    assert_eq!(after.head(request.receipt.run())?, RunSequence::FIRST);
    assert!(matches!(
        after.commit_command(&request)?,
        AtomicRunCommitOutcome::Replayed(_)
    ));
    Ok(())
}

#[test]
fn revision_fault_boundaries_are_atomic_and_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let revision_bytes =
        include_bytes!("../../../crates/blueprint/tests/fixtures/revision-v1.json");
    let (_document, revision) = BlueprintRevisionDocument::from_json(revision_bytes)?;
    for point in [
        FaultPoint::BeforeRevisionCommit,
        FaultPoint::AfterRevisionCommit,
    ] {
        let directory = TempDir::new()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        assert!(store.put_revision(&revision).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        let was_committed = point == FaultPoint::AfterRevisionCommit;
        assert_eq!(reopened.revision(revision.id())?.is_some(), was_committed);
        assert_eq!(
            reopened.put_revision(&revision)?,
            if was_committed {
                ImmutableRevisionPut::AlreadyPresent
            } else {
                ImmutableRevisionPut::Inserted
            }
        );
    }
    Ok(())
}

#[test]
fn snapshot_put_and_discard_fault_boundaries_reopen_safely()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeSnapshotCommit,
        FaultPoint::AfterSnapshotCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let request = accepted_request(
            &format!("run-snapshot-put-{index}"),
            &format!("command-snapshot-put-{index}"),
            &format!("event-snapshot-put-{index}"),
            "start",
        )?;
        let snapshot = SnapshotDocument::new(
            SnapshotId::new(format!("snapshot-put-{index}"))?,
            request.receipt.run().clone(),
            RunSequence::FIRST,
            history_digest(&request.events)?,
            1,
            b"projection".to_vec(),
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.commit_command(&request)?;
        assert!(store.put_snapshot(&snapshot).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeSnapshotCommit {
            assert_eq!(
                reopened.latest_snapshot(request.receipt.run())?,
                SnapshotLoad::Absent
            );
        } else {
            assert_eq!(
                reopened.latest_snapshot(request.receipt.run())?,
                SnapshotLoad::Verified(snapshot.clone())
            );
        }
        reopened.put_snapshot(&snapshot)?;
        assert_eq!(
            reopened.latest_snapshot(request.receipt.run())?,
            SnapshotLoad::Verified(snapshot)
        );
    }

    for (index, point) in [
        FaultPoint::BeforeSnapshotDiscardCommit,
        FaultPoint::AfterSnapshotDiscardCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let request = accepted_request(
            &format!("run-snapshot-discard-{index}"),
            &format!("command-snapshot-discard-{index}"),
            &format!("event-snapshot-discard-{index}"),
            "start",
        )?;
        let snapshot = SnapshotDocument::new(
            SnapshotId::new(format!("snapshot-discard-{index}"))?,
            request.receipt.run().clone(),
            RunSequence::FIRST,
            history_digest(&request.events)?,
            1,
            b"projection".to_vec(),
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.commit_command(&request)?;
        store.put_snapshot(&snapshot)?;
        assert!(
            store
                .discard_snapshot(request.receipt.run(), snapshot.snapshot())
                .is_err()
        );
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeSnapshotDiscardCommit {
            assert_eq!(
                reopened.latest_snapshot(request.receipt.run())?,
                SnapshotLoad::Verified(snapshot.clone())
            );
        } else {
            assert_eq!(
                reopened.latest_snapshot(request.receipt.run())?,
                SnapshotLoad::Absent
            );
        }
        reopened.discard_snapshot(request.receipt.run(), snapshot.snapshot())?;
        assert_eq!(
            reopened.latest_snapshot(request.receipt.run())?,
            SnapshotLoad::Absent
        );
    }
    Ok(())
}

#[test]
fn malformed_stored_event_is_classified_as_corruption() -> Result<(), Box<dyn std::error::Error>> {
    const EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.runs.events");
    let directory = TempDir::new()?;
    let request = accepted_request("run-corrupt", "command-corrupt", "event-corrupt", "start")?;
    let run = request.receipt.run().clone();
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&request)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut events = write.open_table(EVENTS)?;
        let mut key = Vec::new();
        key.extend_from_slice(&(run.as_str().len() as u32).to_be_bytes());
        key.extend_from_slice(run.as_str().as_bytes());
        key.extend_from_slice(&1_u64.to_be_bytes());
        events.insert(key.as_slice(), b"not-json".as_slice())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(directory.path())?;
    let error = store.events(&EventPageQuery::new(run, None, PageSize::new(1)?)?);
    assert!(matches!(error, Err(PersistenceError::Corruption(_))));
    Ok(())
}

#[test]
fn verified_snapshot_survives_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = accepted_request(
        "run-snapshot",
        "command-snapshot",
        "event-snapshot",
        "start",
    )?;
    let snapshot = SnapshotDocument::new(
        SnapshotId::new("snapshot-one")?,
        request.receipt.run().clone(),
        RunSequence::FIRST,
        history_digest(&request.events)?,
        1,
        br#"{"projection":"stable"}"#.to_vec(),
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&request)?;
        store.put_snapshot(&snapshot)?;
        assert_eq!(
            store.latest_snapshot(request.receipt.run())?,
            SnapshotLoad::Verified(snapshot.clone())
        );
    }
    let store = RedbStore::open(directory.path())?;
    assert_eq!(
        store.latest_snapshot(request.receipt.run())?,
        SnapshotLoad::Verified(snapshot)
    );
    Ok(())
}

fn artifact_metadata(
    id: &str,
    bytes: &[u8],
    sensitivity: ArtifactSensitivity,
) -> Result<ArtifactMetadata, Box<dyn std::error::Error>> {
    let reference = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new(id)?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("application/octet-stream")?,
        bytes.len() as u64,
    );
    Ok(ArtifactMetadata::new(
        reference,
        sensitivity,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("test-source")?,
            },
            Vec::new(),
        )?,
    )?)
}

#[test]
fn artifact_begin_and_chunk_fault_boundaries_resume_exact_durable_offsets()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactBeginCommit,
        FaultPoint::AfterArtifactBeginCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-begin-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-begin-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-begin-{index}"))?,
            RunId::new(format!("run-begin-{index}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        assert!(store.begin_publication(&request).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactBeginCommit {
            assert_eq!(
                reopened.begin_publication(&request)?,
                BeginArtifactOutcome::Writable
            );
        } else {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(0));
        }
        reopened.abort_publication(&request.publication)?;
    }

    for (index, point) in [
        FaultPoint::BeforeArtifactChunkWrite,
        FaultPoint::AfterArtifactChunkSync,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-chunk-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-chunk-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-chunk-{index}"))?,
            RunId::new(format!("run-chunk-{index}"))?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        assert!(store.write_chunk(&request.publication, 0, &bytes).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        let durable_offset = if point == FaultPoint::AfterArtifactChunkSync {
            bytes.len() as u64
        } else {
            0
        };
        assert_eq!(
            reopened.begin_publication(&request)?.next_offset(),
            Some(durable_offset)
        );
        if durable_offset == 0 {
            reopened.write_chunk(&request.publication, 0, &bytes)?;
        }
        reopened.commit_publication(&request.publication)?;
        assert!(reopened.is_committed(metadata.reference())?);
    }
    Ok(())
}

#[test]
fn artifact_abort_fault_boundaries_are_retryable_and_release_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactAbortCommit,
        FaultPoint::AfterArtifactAbortCommit,
        FaultPoint::BeforeArtifactAbortDelete,
        FaultPoint::AfterArtifactAbortDelete,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-abort-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-abort-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-abort-{index}"))?,
            RunId::new(format!("run-abort-{index}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..3])?;
        assert!(store.abort_publication(&request.publication).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactAbortCommit {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(3));
        }
        reopened.abort_publication(&request.publication)?;
        assert_eq!(
            reopened.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        reopened.abort_publication(&request.publication)?;
    }
    Ok(())
}

#[test]
fn cleanup_fault_boundaries_expire_writable_sessions_and_release_reservations()
-> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeArtifactCleanupCommit,
        FaultPoint::AfterArtifactCleanupCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let bytes = format!("artifact-cleanup-{index}").into_bytes();
        let run = RunId::new(format!("run-cleanup-{index}"))?;
        let metadata = artifact_metadata(
            &format!("artifact-cleanup-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-cleanup-{index}"))?,
            run.clone(),
            metadata,
            WorkspaceBudget::new(0, 0, 0, 2, 2048, 2048)?,
            WorkspaceUsage::EMPTY,
        )?;
        let cleanup_request = OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX - 1),
            limit: PageSize::new(100)?,
        };
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..3])?;
        assert!(store.cleanup_orphans(cleanup_request).is_err());
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeArtifactCleanupCommit {
            assert_eq!(reopened.begin_publication(&request)?.next_offset(), Some(3));
        }
        let cleanup = reopened.cleanup_orphans(cleanup_request)?;
        assert_eq!(cleanup.temporary_publications_removed, 1);

        let replacement_bytes = format!("replacement-cleanup-{index}").into_bytes();
        let replacement = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-replacement-{index}"))?,
            run,
            artifact_metadata(
                &format!("artifact-replacement-{index}"),
                &replacement_bytes,
                ArtifactSensitivity::Public,
            )?,
            WorkspaceBudget::new(0, 0, 0, 2, 2048, 2048)?,
            WorkspaceUsage::EMPTY,
        )?;
        assert_eq!(
            reopened.begin_publication(&replacement)?,
            BeginArtifactOutcome::Writable
        );
        reopened.abort_publication(&replacement.publication)?;
    }
    Ok(())
}

#[test]
fn cleanup_expires_a_session_crashed_after_content_rename() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let bytes = b"renamed-before-metadata";
    let metadata = artifact_metadata(
        "artifact-renamed-orphan",
        bytes,
        ArtifactSensitivity::Public,
    )?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-renamed-orphan")?,
        RunId::new("run-renamed-orphan")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterArtifactRename))),
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(&request.publication, 0, bytes)?;
    assert!(store.commit_publication(&request.publication).is_err());
    assert!(!store.is_committed(metadata.reference())?);
    drop(store);

    let reopened = RedbStore::open(directory.path())?;
    let cleanup = reopened.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(u64::MAX),
        created_before: TimestampMillis::new(u64::MAX - 1),
        limit: PageSize::new(100)?,
    })?;
    assert_eq!(cleanup.temporary_publications_removed, 0);
    assert_eq!(cleanup.unreferenced_blobs_removed, 1);
    assert_eq!(
        reopened.begin_publication(&request)?,
        BeginArtifactOutcome::Writable
    );
    reopened.abort_publication(&request.publication)?;
    Ok(())
}

#[test]
fn cleanup_file_delete_fault_boundaries_are_restart_safe() -> Result<(), Box<dyn std::error::Error>>
{
    for (index, point) in [
        FaultPoint::BeforeArtifactCleanupDelete,
        FaultPoint::AfterArtifactCleanupDelete,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let orphan = directory
            .path()
            .join("artifacts/.tmp")
            .join(format!("orphan-{index}.part"));
        std::fs::write(&orphan, b"orphan")?;
        let request = OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(u64::MAX - 1),
            limit: PageSize::new(100)?,
        };
        assert!(store.cleanup_orphans(request).is_err());
        if point == FaultPoint::BeforeArtifactCleanupDelete {
            assert!(orphan.exists());
            assert_eq!(
                store
                    .cleanup_orphans(request)?
                    .temporary_publications_removed,
                1
            );
        } else {
            assert!(!orphan.exists());
            assert_eq!(
                store
                    .cleanup_orphans(request)?
                    .temporary_publications_removed,
                0
            );
        }
    }
    Ok(())
}

#[test]
fn artifact_publication_fault_boundaries_recover_without_dangling_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let points = [
        FaultPoint::BeforeArtifactRename,
        FaultPoint::AfterArtifactRename,
        FaultPoint::BeforeArtifactMetadataCommit,
        FaultPoint::AfterArtifactMetadataCommit,
    ];
    for (index, point) in points.into_iter().enumerate() {
        let directory = TempDir::new()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let bytes = format!("fault-boundary-content-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-fault-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-fault-{index}"))?,
            RunId::new(format!("run-fault-{index}"))?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?;
        assert_eq!(
            store.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        assert!(
            store
                .write_chunk(&request.publication, 0, &bytes)?
                .complete_size
        );
        assert!(store.commit_publication(&request.publication).is_err());

        let committed_after_failure = store.is_committed(metadata.reference())?;
        if point == FaultPoint::AfterArtifactMetadataCommit {
            assert!(committed_after_failure);
        } else {
            assert!(!committed_after_failure);
        }
        let cleanup = store.cleanup_orphans(OrphanCleanupRequest {
            observed_at: TimestampMillis::new(u64::MAX),
            created_before: TimestampMillis::new(0),
            limit: PageSize::new(100)?,
        })?;
        assert_eq!(cleanup.temporary_publications_removed, 0);
        assert_eq!(cleanup.unreferenced_blobs_removed, 0);
        let recovered = store.commit_publication(&request.publication)?;
        assert!(store.is_committed(metadata.reference())?);
        if point == FaultPoint::AfterArtifactMetadataCommit {
            assert!(!recovered.was_published());
        } else {
            assert!(recovered.was_published());
        }
    }
    Ok(())
}

#[test]
fn artifact_rejects_bad_digest_offsets_chunks_and_budget() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let expected = b"expected";
    let actual = b"mismatch";
    let metadata = artifact_metadata("artifact-invalid", expected, ArtifactSensitivity::Public)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-invalid")?,
        RunId::new("run-invalid")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    assert!(matches!(
        store.write_chunk(&request.publication, 1, &actual[..1]),
        Err(PersistenceError::ImmutableConflict { .. })
    ));
    assert!(matches!(
        store.write_chunk(&request.publication, 0, &[]),
        Err(PersistenceError::Bounds { .. })
    ));
    let oversized_chunk = vec![0_u8; milkdrift_persistence::MAX_ARTIFACT_CHUNK_BYTES + 1];
    assert!(matches!(
        store.write_chunk(&request.publication, 0, &oversized_chunk),
        Err(PersistenceError::Bounds { .. })
    ));
    store.write_chunk(&request.publication, 0, actual)?;
    assert!(matches!(
        store.commit_publication(&request.publication),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));

    let too_small_budget = WorkspaceBudget::new(0, 0, 0, 1, 1, 1)?;
    assert!(
        BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-budget")?,
            RunId::new("run-budget")?,
            metadata,
            too_small_budget,
            WorkspaceUsage::EMPTY,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn artifact_publication_resumes_deduplicates_verifies_and_cleans_orphans()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let content = b"durable artifact bytes";
    let metadata = artifact_metadata("artifact-one", content, ArtifactSensitivity::Restricted)?;
    let budget = WorkspaceBudget::new(0, 0, 0, 10, 1024, 4096)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-one")?,
        RunId::new("run-artifact")?,
        metadata.clone(),
        budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        assert_eq!(
            store.begin_publication(&request)?,
            BeginArtifactOutcome::Writable
        );
        assert!(matches!(
            store.write_chunk(&request.publication, 0, &[0_u8; 64]),
            Err(PersistenceError::Bounds { .. })
        ));
        let first = &content[..7];
        let progress = store.write_chunk(&request.publication, 0, first)?;
        assert_eq!(progress.bytes_received, 7);
        assert!(!progress.complete_size);
    }
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.begin_publication(&request)?.next_offset(), Some(7));
    store.write_chunk(&request.publication, 7, &content[7..])?;
    let first_commit = store.commit_publication(&request.publication)?;
    assert!(first_commit.was_published());
    assert_eq!(first_commit.content_deduplicated(), Some(false));
    assert!(store.is_committed(metadata.reference())?);
    assert!(store.is_referenced_by_run(&request.run, metadata.reference())?);
    assert_eq!(
        store.workspace_usage(&request.run)?,
        request.resulting_usage
    );
    assert!(
        !store.is_referenced_by_run(&RunId::new("run-without-artifact")?, metadata.reference())?
    );
    assert!(matches!(
        store.begin_publication(&BeginArtifactPublication::new(
            ArtifactPublicationId::new("publication-reused-artifact")?,
            RunId::new("run-reused-artifact")?,
            metadata.clone(),
            WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
            WorkspaceUsage::EMPTY,
        )?),
        Err(PersistenceError::ImmutableConflict {
            entity: "artifact_publication",
            ..
        })
    ));
    let public_read = ArtifactReadRequest::new(
        metadata.reference().clone(),
        0,
        16,
        ArtifactReadAuthority::PublicOnly,
    )?;
    assert!(store.read_chunk(&public_read).is_err());

    let second_metadata = artifact_metadata("artifact-two", content, ArtifactSensitivity::Public)?;
    let second = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-two")?,
        request.run.clone(),
        second_metadata.clone(),
        budget,
        request.resulting_usage,
    )?;
    assert_eq!(
        store.begin_publication(&second)?,
        BeginArtifactOutcome::Writable
    );
    assert!(
        store
            .write_chunk(&second.publication, 0, content)?
            .complete_size
    );
    let second_commit = store.commit_publication(&second.publication)?;
    assert!(second_commit.was_published());
    assert_eq!(second_commit.content_deduplicated(), Some(true));
    assert!(store.is_referenced_by_run(&request.run, second_metadata.reference())?);
    assert_eq!(store.workspace_usage(&request.run)?, second.resulting_usage);
    let read = ArtifactReadRequest::new(
        second_metadata.reference().clone(),
        0,
        1_024,
        ArtifactReadAuthority::PublicOnly,
    )?;
    let chunk = store.read_chunk(&read)?;
    assert_eq!(chunk.bytes, content);
    assert!(chunk.end_of_artifact);

    let temp_orphan = directory.path().join("artifacts/.tmp/orphan.part");
    std::fs::write(&temp_orphan, b"orphan")?;
    let orphan_bytes = b"unreferenced";
    let orphan_digest = ContentDigest::for_bytes(orphan_bytes).to_hex();
    let orphan_directory = directory.path().join("artifacts").join(&orphan_digest[..2]);
    std::fs::create_dir_all(&orphan_directory)?;
    let orphan_path = orphan_directory.join(&orphan_digest[2..]);
    std::fs::write(&orphan_path, orphan_bytes)?;
    let cleanup = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(u64::MAX),
        created_before: TimestampMillis::new(u64::MAX - 1),
        limit: PageSize::new(100)?,
    })?;
    assert_eq!(cleanup.temporary_publications_removed, 1);
    assert_eq!(cleanup.unreferenced_blobs_removed, 1);
    assert!(!temp_orphan.exists());
    assert!(!orphan_path.exists());
    assert!(store.is_committed(metadata.reference())?);

    let committed_digest = metadata.reference().digest().to_hex();
    let committed_path = directory
        .path()
        .join("artifacts")
        .join(&committed_digest[..2])
        .join(&committed_digest[2..]);
    std::fs::write(&committed_path, b"corrupted artifact bytes")?;
    assert!(matches!(
        store.is_committed(metadata.reference()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

#[cfg(unix)]
#[test]
fn artifact_publication_and_reads_refuse_symlink_redirection()
-> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::symlink;

    let shard_directory = TempDir::new()?;
    let escaped_directory = TempDir::new()?;
    let bytes = b"must remain inside the artifact root";
    let metadata = artifact_metadata("artifact-shard-link", bytes, ArtifactSensitivity::Public)?;
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-shard-link")?,
        RunId::new("run-shard-link")?,
        metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let store = RedbStore::open(shard_directory.path())?;
    store.begin_publication(&request)?;
    store.write_chunk(&request.publication, 0, bytes)?;
    let digest = metadata.reference().digest().to_hex();
    let shard = shard_directory.path().join("artifacts").join(&digest[..2]);
    symlink(escaped_directory.path(), &shard)?;
    assert!(matches!(
        store.commit_publication(&request.publication),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(!escaped_directory.path().join(&digest[2..]).exists());
    assert!(store.metadata(metadata.reference().artifact())?.is_none());

    let content_directory = TempDir::new()?;
    let external_file = tempfile::NamedTempFile::new()?;
    let content_metadata = artifact_metadata(
        "artifact-content-link",
        b"verified content",
        ArtifactSensitivity::Public,
    )?;
    let content_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-content-link")?,
        RunId::new("run-content-link")?,
        content_metadata.clone(),
        WorkspaceBudget::new(0, 0, 0, 1, 1024, 1024)?,
        WorkspaceUsage::EMPTY,
    )?;
    let content_store = RedbStore::open(content_directory.path())?;
    content_store.begin_publication(&content_request)?;
    content_store.write_chunk(&content_request.publication, 0, b"verified content")?;
    content_store.commit_publication(&content_request.publication)?;
    let digest = content_metadata.reference().digest().to_hex();
    let path = content_directory
        .path()
        .join("artifacts")
        .join(&digest[..2])
        .join(&digest[2..]);
    std::fs::remove_file(&path)?;
    symlink(external_file.path(), &path)?;
    assert!(matches!(
        content_store.is_committed(content_metadata.reference()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}
