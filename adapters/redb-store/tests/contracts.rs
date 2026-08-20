//! End-to-end durability, conflict, corruption, and artifact contract tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use milkdrift_blueprint::{
    BlueprintRevisionDocument, ContentDigest as BlueprintContentDigest, PortId, RevisionId,
    WorkflowId,
};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactReadAuthority, ArtifactReadRequest, ArtifactStore,
    AtomicRunCommitOutcome, AtomicRunCommitRequest, AttemptId, BeginArtifactOutcome,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, EventPageQuery, ImmutableRevisionPut, IndexedRunState, IntegrityScanRequest, LeaseId,
    LeaseIndexEntry, LeaseIndexMutation, NodeExecutionId, OrphanCleanupRequest, PageSize,
    PersistenceError, RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal,
    RunQueryStore, RunSequence, RunSummaryFilter, RunSummaryIndex, RunSummaryPageQuery,
    RunnableIndexEntry, RunnableIndexMutation, SnapshotDocument, SnapshotId, SnapshotLoad,
    SnapshotStore, StorageAdmin, StorageFailureClass, StorageHealthStatus, TimerId,
    TimerIndexEntry, TimerIndexMutation, TimestampMillis, WorkerId, WorkspaceAccounting,
    WorkspaceMutation, WorkspaceStore, history_digest,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    BranchId, CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ValueKey,
    WorkspaceBudget, WorkspaceScope, WorkspaceUsage, WorkspaceValue, WorkspaceValueEntry,
    WorkspaceValueReference,
};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde_json::json;
use tempfile::TempDir;

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn revision_digest() -> Result<BlueprintContentDigest, PersistenceError> {
    serde_json::from_value(json!(format!("b3_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn stored_workspace_value_key(
    reference: &WorkspaceValueReference,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoded = Vec::new();
    for component in [
        reference.scope().run().as_str(),
        reference.scope().scope().as_str(),
        reference.key().as_str(),
    ] {
        encoded.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
        encoded.extend_from_slice(component.as_bytes());
    }
    encoded.extend_from_slice(&reference.version().get().to_be_bytes());
    Ok(encoded)
}

fn stored_workspace_scope_key(
    scope: &milkdrift_workspace::ScopeReference,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut encoded = Vec::new();
    for component in [scope.run().as_str(), scope.scope().as_str()] {
        encoded.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
        encoded.extend_from_slice(component.as_bytes());
    }
    Ok(encoded)
}

fn assert_storage_corruption<T: std::fmt::Debug>(result: Result<T, PersistenceError>) {
    assert!(
        matches!(
            &result,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            }) | Err(PersistenceError::Corruption(_))
        ),
        "expected storage corruption, got {result:?}"
    );
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
        None,
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

fn accepted_followup_request(
    run: RunId,
    command: &str,
    event: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let sequence = RunSequence::new(2);
    let command = CommandId::new(command)?;
    let document = br#"{"schema_version":1,"type":"followup"}"#.to_vec();
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        RunSequence::FIRST,
        TimestampMillis::new(11),
        document,
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(event)?,
        run.clone(),
        sequence,
        TimestampMillis::new(11),
        RunEventKind::RunStarted,
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
            budget: WorkspaceBudget::new(0, 0, 0, 0, 0, 0)?,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: WorkspaceUsage::EMPTY,
        }),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: sequence,
                updated_at: TimestampMillis::new(11),
            }),
            ..RunIndexUpdate::default()
        },
    )?)
}

fn accepted_workspace_request(
    run: RunId,
    command: &str,
    event_prefix: &str,
    kinds: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
    accounting: WorkspaceAccounting,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"workspace-test"}"#.to_vec(),
    )?;
    let events = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            RunEventEnvelope::new(
                EventId::new(format!("{event_prefix}-{index}"))?,
                run.clone(),
                RunSequence::new(index as u64 + 1),
                TimestampMillis::new(10),
                kind,
            )
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    let resulting_sequence = RunSequence::new(events.len() as u64);
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        resulting_sequence,
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
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: resulting_sequence,
                updated_at: TimestampMillis::new(10),
            }),
            ..RunIndexUpdate::default()
        },
    )?)
}

fn accepted_workspace_followup_request(
    run: RunId,
    expected_sequence: RunSequence,
    command: &str,
    event_prefix: &str,
    kinds: Vec<RunEventKind>,
    workspace: Vec<WorkspaceMutation>,
    accounting: WorkspaceAccounting,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new(command)?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-test")?,
        expected_sequence,
        TimestampMillis::new(11),
        br#"{"schema_version":1,"type":"workspace-followup-test"}"#.to_vec(),
    )?;
    let events = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let offset = u64::try_from(index)?
                .checked_add(1)
                .ok_or("event offset overflow")?;
            let sequence = expected_sequence
                .get()
                .checked_add(offset)
                .ok_or("event sequence overflow")?;
            Ok(RunEventEnvelope::new(
                EventId::new(format!("{event_prefix}-{index}"))?,
                run.clone(),
                RunSequence::new(sequence),
                TimestampMillis::new(11),
                kind,
            )?)
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    let resulting_sequence = events
        .last()
        .map_or(expected_sequence, RunEventEnvelope::sequence);
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        resulting_sequence,
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
        Some(accounting),
        Vec::new(),
        Vec::new(),
        None,
        result,
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-test")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: resulting_sequence,
                updated_at: TimestampMillis::new(11),
            }),
            ..RunIndexUpdate::default()
        },
    )?)
}

fn run_created_kind(
    root_scope: WorkspaceScope,
    workspace_budget: WorkspaceBudget,
    inputs: Vec<milkdrift_workspace::WorkspaceValueReference>,
) -> Result<RunEventKind, Box<dyn std::error::Error>> {
    Ok(RunEventKind::RunCreated {
        workflow: WorkflowId::new("workflow-test")?,
        revision: revision_id()?,
        revision_digest: revision_digest()?,
        root_scope,
        workspace_budget,
        inputs,
    })
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
        request.expected_lease_catalog,
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
        request.expected_lease_catalog,
        request.result,
        indexes,
    )?)
}

#[derive(Clone, Copy)]
enum DiscoveryIndexKind {
    Runnable,
    Timer,
    Lease,
}

fn accepted_request_with_discovery_index(
    kind: DiscoveryIndexKind,
    suffix: &str,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let run_text = format!("run-discovery-{suffix}");
    let command = format!("command-discovery-{suffix}");
    let event = format!("event-discovery-{suffix}");
    let request = accepted_request(&run_text, &command, &event, "start")?;
    let run = request.receipt.run().clone();
    let mut indexes = request.indexes.clone();
    match kind {
        DiscoveryIndexKind::Runnable => {
            indexes.runnable.push(RunnableIndexMutation::Upsert {
                entry: RunnableIndexEntry {
                    run,
                    execution: NodeExecutionId::new(format!("execution-{suffix}"))?,
                    eligible_at: TimestampMillis::new(10),
                    priority: 1,
                    through_sequence: RunSequence::FIRST,
                },
            });
        }
        DiscoveryIndexKind::Timer => {
            indexes.timers.push(TimerIndexMutation::Upsert {
                entry: TimerIndexEntry {
                    run,
                    timer: TimerId::new(format!("timer-{suffix}"))?,
                    fire_at: TimestampMillis::new(10),
                    through_sequence: RunSequence::FIRST,
                },
            });
        }
        DiscoveryIndexKind::Lease => {
            indexes.leases.push(LeaseIndexMutation::Upsert {
                entry: LeaseIndexEntry {
                    run,
                    lease: LeaseId::new(format!("lease-{suffix}"))?,
                    attempt: AttemptId::new(format!("attempt-{suffix}"))?,
                    worker: WorkerId::new("worker-discovery")?,
                    expires_at: TimestampMillis::new(10),
                    through_sequence: RunSequence::FIRST,
                },
            });
        }
    }
    Ok(AtomicRunCommitRequest::new(
        request.receipt,
        request.events,
        request.workspace,
        request.workspace_accounting,
        request.required_artifacts,
        request.newly_referenced_artifacts,
        request.expected_lease_catalog,
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
    let mut cursor = None;
    let mut pages = 0_u32;
    let mut failures = 0_usize;
    loop {
        let scan = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(1)?,
            verify_artifact_content: false,
            cursor,
        })?;
        pages += 1;
        if pages == 1 {
            assert!(scan.failures.is_empty());
            assert!(scan.next_cursor.is_some());
            assert!(matches!(
                store.scan_integrity(IntegrityScanRequest {
                    limit: PageSize::new(1)?,
                    verify_artifact_content: true,
                    cursor: scan.next_cursor.clone(),
                }),
                Err(PersistenceError::InvalidCursor(_))
            ));
        }
        failures += scan.failures.len();
        let Some(next) = scan.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    assert!(pages >= 4);
    assert!(failures >= 2);
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
    let mut runnable = Vec::new();
    let mut cursor = None;
    for _ in 0..16 {
        let page =
            store.runnable_page(TimestampMillis::new(10), cursor.as_ref(), PageSize::new(2)?)?;
        runnable.extend(page.entries);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
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
    let mut one_at_a_time = Vec::new();
    let mut cursor = None;
    for _ in 0..16 {
        let page =
            store.runnable_page(TimestampMillis::new(10), cursor.as_ref(), PageSize::new(1)?)?;
        one_at_a_time.extend(page.entries);
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(one_at_a_time.len(), 2);
    assert_ne!(one_at_a_time[0].run, one_at_a_time[1].run);
    assert!(one_at_a_time.iter().any(|entry| entry.run == quiet_run));
    Ok(())
}

#[test]
fn runnable_pages_advance_across_future_rows_and_removed_anchors()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..10 {
        let run = RunId::new(format!("run-future-{ordinal:02}"))?;
        store.commit_command(&accepted_request_with_runnable(
            run.as_str(),
            &format!("command-future-{ordinal:02}"),
            &format!("event-future-{ordinal:02}"),
            vec![RunnableIndexEntry {
                run: run.clone(),
                execution: NodeExecutionId::new(format!("execution-future-{ordinal:02}"))?,
                eligible_at: TimestampMillis::new(1_000),
                priority: 1,
                through_sequence: RunSequence::FIRST,
            }],
        )?)?;
    }
    let eligible_run = RunId::new("run-later-eligible")?;
    store.commit_command(&accepted_request_with_runnable(
        eligible_run.as_str(),
        "command-later-eligible",
        "event-later-eligible",
        vec![RunnableIndexEntry {
            run: eligible_run.clone(),
            execution: NodeExecutionId::new("execution-later-eligible")?,
            eligible_at: TimestampMillis::new(1),
            priority: 1,
            through_sequence: RunSequence::FIRST,
        }],
    )?)?;

    let first = store.runnable_page(TimestampMillis::new(10), None, PageSize::new(1)?)?;
    assert!(first.entries.is_empty());
    assert!(first.next.is_some());
    let second = store.runnable_page(
        TimestampMillis::new(20),
        first.next.as_ref(),
        PageSize::new(1)?,
    )?;
    assert_eq!(second.entries.len(), 1);
    assert_eq!(second.entries[0].run, eligible_run);

    let anchor_directory = TempDir::new()?;
    let anchor_store = RedbStore::open(anchor_directory.path())?;
    let first_run = RunId::new("run-anchor-a")?;
    let second_run = RunId::new("run-anchor-b")?;
    let first_execution = NodeExecutionId::new("execution-anchor-a")?;
    let second_execution = NodeExecutionId::new("execution-anchor-b")?;
    for (run, execution, suffix) in [
        (&first_run, &first_execution, "a"),
        (&second_run, &second_execution, "b"),
    ] {
        anchor_store.commit_command(&accepted_request_with_runnable(
            run.as_str(),
            &format!("command-anchor-{suffix}"),
            &format!("event-anchor-{suffix}"),
            vec![RunnableIndexEntry {
                run: run.clone(),
                execution: execution.clone(),
                eligible_at: TimestampMillis::new(1),
                priority: 1,
                through_sequence: RunSequence::FIRST,
            }],
        )?)?;
    }
    let first_page =
        anchor_store.runnable_page(TimestampMillis::new(10), None, PageSize::new(1)?)?;
    assert_eq!(first_page.entries.len(), 1);
    assert_eq!(first_page.entries[0].run, first_run);
    let mut removal = accepted_followup_request(
        first_run.clone(),
        "command-remove-anchor",
        "event-remove-anchor",
    )?;
    removal
        .indexes
        .runnable
        .push(RunnableIndexMutation::Remove {
            run: first_run,
            execution: first_execution,
        });
    anchor_store.commit_command(&removal)?;
    let second_page = anchor_store.runnable_page(
        TimestampMillis::new(20),
        first_page.next.as_ref(),
        PageSize::new(1)?,
    )?;
    assert_eq!(second_page.entries.len(), 1);
    assert_eq!(second_page.entries[0].run, second_run);
    assert!(second_page.next.is_none());
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
fn summary_and_nonterminal_cursors_advance_by_last_scanned_head()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    for ordinal in 0..10 {
        let mut request = accepted_request(
            &format!("run-filter-{ordinal:02}"),
            &format!("command-filter-{ordinal:02}"),
            &format!("event-filter-{ordinal:02}"),
            "start",
        )?;
        request
            .indexes
            .summary
            .as_mut()
            .ok_or("summary missing")?
            .workflow = WorkflowId::new("workflow-nonmatch")?;
        store.commit_command(&request)?;
    }
    let matching_run = RunId::new("run-filter-z-match")?;
    let mut matching = accepted_request(
        matching_run.as_str(),
        "command-filter-match",
        "event-filter-match",
        "start",
    )?;
    matching
        .indexes
        .summary
        .as_mut()
        .ok_or("summary missing")?
        .workflow = WorkflowId::new("workflow-match")?;
    store.commit_command(&matching)?;

    let filter = RunSummaryFilter {
        state: None,
        workflow: Some(WorkflowId::new("workflow-match")?),
    };
    let first = store.run_summaries(&RunSummaryPageQuery {
        filter: filter.clone(),
        cursor: None,
        limit: PageSize::new(1)?,
    })?;
    assert!(first.runs.is_empty());
    assert!(first.next.is_some());
    let second = store.run_summaries(&RunSummaryPageQuery {
        filter: filter.clone(),
        cursor: first.next.clone(),
        limit: PageSize::new(1)?,
    })?;
    assert_eq!(second.runs.len(), 1);
    assert_eq!(second.runs[0].run, matching_run);
    assert!(matches!(
        store.run_summaries(&RunSummaryPageQuery {
            filter: RunSummaryFilter {
                state: None,
                workflow: Some(WorkflowId::new("workflow-other")?),
            },
            cursor: first.next,
            limit: PageSize::new(1)?,
        }),
        Err(PersistenceError::InvalidCursor(_))
    ));

    let terminal_directory = TempDir::new()?;
    let terminal_store = RedbStore::open(terminal_directory.path())?;
    for ordinal in 0..3 {
        let mut request = accepted_request(
            &format!("run-terminal-{ordinal}"),
            &format!("command-terminal-{ordinal}"),
            &format!("event-terminal-{ordinal}"),
            "start",
        )?;
        request
            .indexes
            .summary
            .as_mut()
            .ok_or("summary missing")?
            .state = IndexedRunState::Terminal;
        terminal_store.commit_command(&request)?;
    }
    terminal_store.commit_command(&accepted_request(
        "run-terminal-z-active",
        "command-terminal-active",
        "event-terminal-active",
        "start",
    )?)?;
    let terminal_page = terminal_store.nonterminal_run_page(None, PageSize::new(2)?)?;
    assert!(terminal_page.runs.is_empty());
    assert!(terminal_page.next.is_some());
    let active_page =
        terminal_store.nonterminal_run_page(terminal_page.next.as_ref(), PageSize::new(2)?)?;
    assert_eq!(active_page.runs.len(), 1);
    assert_eq!(active_page.runs[0].run.as_str(), "run-terminal-z-active");
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

    assert_eq!(store.active_leases(PageSize::new(2)?)?.entries.len(), 2);
    assert_eq!(store.active_leases(PageSize::new(4)?)?.entries.len(), 3);
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
    let child_request = accepted_workspace_request(
        child_root.reference().run().clone(),
        "command-child-import",
        "event-child-import",
        vec![run_created_kind(
            child_root.clone(),
            child_budget.clone(),
            vec![child_value.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: child_root.clone(),
            },
            WorkspaceMutation::PutValue {
                entry: child_value.clone(),
            },
        ],
        WorkspaceAccounting {
            budget: child_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: child_usage,
        },
    )?;
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
    let parent_request = accepted_workspace_request(
        parent_root.reference().run().clone(),
        "command-parent-import",
        "event-parent-import",
        vec![run_created_kind(
            parent_root.clone(),
            parent_budget.clone(),
            vec![imported.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope { scope: parent_root },
            WorkspaceMutation::PutValue {
                entry: imported.clone(),
            },
        ],
        WorkspaceAccounting {
            budget: parent_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: parent_usage,
        },
    )?;
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
    let altered_request = accepted_workspace_request(
        altered_root.reference().run().clone(),
        "command-altered-import",
        "event-altered-import",
        vec![run_created_kind(
            altered_root.clone(),
            altered_budget.clone(),
            vec![altered_import.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: altered_root,
            },
            WorkspaceMutation::PutValue {
                entry: altered_import,
            },
        ],
        WorkspaceAccounting {
            budget: altered_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: altered_usage,
        },
    )?;
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
    let missing_request = accepted_workspace_request(
        missing_root.reference().run().clone(),
        "command-missing-import",
        "event-missing-import",
        vec![run_created_kind(
            missing_root.clone(),
            missing_budget.clone(),
            vec![missing_import.reference().clone()],
        )?],
        vec![
            WorkspaceMutation::CreateScope {
                scope: missing_root,
            },
            WorkspaceMutation::PutValue {
                entry: missing_import,
            },
        ],
        WorkspaceAccounting {
            budget: missing_budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage: missing_usage,
        },
    )?;
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
fn workspace_scope_and_value_envelopes_detect_valid_json_payload_tampering()
-> Result<(), Box<dyn std::error::Error>> {
    const SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.scopes");
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-envelope-tamper")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("scope-root")?);
    let child = WorkspaceScope::branch(
        ScopeId::new("scope-child")?,
        &root,
        BranchId::new("branch-a")?,
    )?;
    let entry = WorkspaceValueEntry::initial(
        child.reference().clone(),
        ValueKey::new("answer")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"answer": 42}))?),
    );
    let budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let usage = budget.admit_value(&WorkspaceUsage::EMPTY, entry.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-envelope-tamper",
            "event-envelope-tamper",
            vec![
                run_created_kind(root.clone(), budget.clone(), Vec::new())?,
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-envelope-tamper")?,
                    port: PortId::new("branch-a")?,
                    branch: BranchId::new("branch-a")?,
                    scope: child.clone(),
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("output-envelope-tamper")?,
                    value: entry.reference().clone(),
                    artifact: None,
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: root },
                WorkspaceMutation::CreateScope {
                    scope: child.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: entry.clone(),
                },
            ],
            WorkspaceAccounting {
                budget,
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage,
            },
        )?)?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut scopes = write.open_table(SCOPES)?;
        let (key, bytes) = {
            let mut found = None;
            for item in scopes.iter()? {
                let (key, bytes) = item?;
                if bytes
                    .value()
                    .windows(b"branch-a".len())
                    .any(|part| part == b"branch-a")
                {
                    found = Some((key.value().to_vec(), bytes.value().to_vec()));
                    break;
                }
            }
            found.ok_or("child scope envelope was not found")?
        };
        let tampered = String::from_utf8(bytes)?.replace("branch-a", "branch-b");
        scopes.insert(key.as_slice(), tampered.as_bytes())?;
    }
    {
        let mut values = write.open_table(VALUES)?;
        let (key, bytes) = {
            let mut rows = values.iter()?;
            let (key, bytes) = rows
                .next()
                .transpose()?
                .ok_or("workspace value is absent")?;
            (key.value().to_vec(), bytes.value().to_vec())
        };
        let tampered = String::from_utf8(bytes)?.replace("\"answer\":42", "\"answer\":43");
        values.insert(key.as_slice(), tampered.as_bytes())?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let scope_result = store.scope(&run, child.reference().scope());
    assert_storage_corruption(scope_result);
    let value_result = store.value(entry.reference());
    assert_storage_corruption(value_result);
    Ok(())
}

#[test]
fn durable_workspace_inheritance_preserves_exact_ancestor_content()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let root =
        WorkspaceScope::run_root(RunId::new("run-inherited-content")?, ScopeId::new("root")?);
    let branch_id = BranchId::new("branch-a")?;
    let branch = WorkspaceScope::branch(ScopeId::new("branch-scope")?, &root, branch_id.clone())?;
    let root_value = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("request")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"prompt": "original"}))?),
    );
    let altered_inheritance = WorkspaceValueEntry::inherited(
        branch.reference().clone(),
        ValueKey::new("request")?,
        root_value.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"prompt": "altered"}))?),
    )?;
    let budget = WorkspaceBudget::new(2, 1024, 2048, 0, 0, 0)?;
    let root_usage = budget.admit_value(&WorkspaceUsage::EMPTY, root_value.value())?;
    let resulting_usage = budget.admit_value(&root_usage, altered_inheritance.value())?;
    let request = accepted_workspace_request(
        root.reference().run().clone(),
        "command-inherited-content",
        "event-inherited-content",
        vec![
            run_created_kind(
                root.clone(),
                budget.clone(),
                vec![root_value.reference().clone()],
            )?,
            RunEventKind::BranchScopeCreated {
                fork_execution: NodeExecutionId::new("fork-inherited-content")?,
                port: PortId::new("branch-a")?,
                branch: branch_id,
                scope: branch.clone(),
            },
            RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("output-inherited-content")?,
                value: altered_inheritance.reference().clone(),
                artifact: None,
            },
        ],
        vec![
            WorkspaceMutation::CreateScope {
                scope: root.clone(),
            },
            WorkspaceMutation::CreateScope { scope: branch },
            WorkspaceMutation::PutValue { entry: root_value },
            WorkspaceMutation::PutValue {
                entry: altered_inheritance,
            },
        ],
        WorkspaceAccounting {
            budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage,
        },
    )?;

    assert!(matches!(
        store.commit_command(&request),
        Err(PersistenceError::InvalidDocument(reason))
            if reason.contains("preserve its exact ancestor content")
    ));
    assert_eq!(store.head(request.receipt.run())?, RunSequence::ZERO);
    assert!(
        store
            .scope(request.receipt.run(), root.reference().scope())?
            .is_none()
    );
    Ok(())
}

#[test]
fn deleted_successor_history_blocks_reads_latest_and_next_version()
-> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-successor-history")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let key = ValueKey::new("stream")?;
    let first = WorkspaceValueEntry::initial(
        root.reference().clone(),
        key.clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 1}))?),
    );
    let second = WorkspaceValueEntry::successor(
        first.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 2}))?),
    )?;
    let third = WorkspaceValueEntry::successor(
        second.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"version": 3}))?),
    )?;
    let budget = WorkspaceBudget::new(3, 1024, 3072, 0, 0, 0)?;
    let usage_one = budget.admit_value(&WorkspaceUsage::EMPTY, first.value())?;
    let usage_two = budget.admit_value(&usage_one, second.value())?;
    let usage_three = budget.admit_value(&usage_two, third.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-successor-one",
            "event-successor-one",
            vec![run_created_kind(
                root.clone(),
                budget.clone(),
                vec![first.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope {
                    scope: root.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: first.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            run.clone(),
            RunSequence::FIRST,
            "command-successor-two",
            "event-successor-two",
            vec![RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-successor-two")?,
                value: second.reference().clone(),
                artifact: None,
            }],
            vec![WorkspaceMutation::PutValue {
                entry: second.clone(),
            }],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: usage_one,
                resulting_usage: usage_two,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(first.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(second.reference()));
    assert_storage_corruption(store.latest_value(root.reference(), &key));
    let next = accepted_workspace_followup_request(
        run,
        RunSequence::new(2),
        "command-successor-three",
        "event-successor-three",
        vec![RunEventKind::DeterministicOutputPublished {
            execution: NodeExecutionId::new("execution-successor-three")?,
            value: third.reference().clone(),
            artifact: None,
        }],
        vec![WorkspaceMutation::PutValue { entry: third }],
        WorkspaceAccounting {
            budget,
            expected_usage: usage_two,
            resulting_usage: usage_three,
        },
    )?;
    assert_storage_corruption(store.commit_command(&next));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_inherited_source_history_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-inherited-history")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let branch =
        WorkspaceScope::branch(ScopeId::new("branch")?, &root, BranchId::new("branch-a")?)?;
    let source_one = WorkspaceValueEntry::initial(
        root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 1}))?),
    );
    let source_two = WorkspaceValueEntry::successor(
        source_one.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 2}))?),
    )?;
    let inherited = WorkspaceValueEntry::inherited(
        branch.reference().clone(),
        ValueKey::new("inherited")?,
        source_two.reference().clone(),
        source_two.value().clone(),
    )?;
    let budget = WorkspaceBudget::new(3, 1024, 3072, 0, 0, 0)?;
    let usage_one = budget.admit_value(&WorkspaceUsage::EMPTY, source_one.value())?;
    let usage_two = budget.admit_value(&usage_one, source_two.value())?;
    let usage_three = budget.admit_value(&usage_two, inherited.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-inherited-source-one",
            "event-inherited-source-one",
            vec![run_created_kind(
                root.clone(),
                budget.clone(),
                vec![source_one.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope {
                    scope: root.clone(),
                },
                WorkspaceMutation::PutValue {
                    entry: source_one.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            run,
            RunSequence::FIRST,
            "command-inherited-source-two",
            "event-inherited-source-two",
            vec![
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-inherited-history")?,
                    port: PortId::new("branch-a")?,
                    branch: BranchId::new("branch-a")?,
                    scope: branch.clone(),
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("execution-source-two")?,
                    value: source_two.reference().clone(),
                    artifact: None,
                },
                RunEventKind::DeterministicOutputPublished {
                    execution: NodeExecutionId::new("execution-inherited")?,
                    value: inherited.reference().clone(),
                    artifact: None,
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: branch },
                WorkspaceMutation::PutValue { entry: source_two },
                WorkspaceMutation::PutValue {
                    entry: inherited.clone(),
                },
            ],
            WorkspaceAccounting {
                budget,
                expected_usage: usage_one,
                resulting_usage: usage_three,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(source_one.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(inherited.reference()));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_imported_source_history_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    const VALUES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.values");

    let directory = TempDir::new()?;
    let child_run = RunId::new("run-import-source-history")?;
    let child_root = WorkspaceScope::run_root(child_run.clone(), ScopeId::new("child-root")?);
    let source_one = WorkspaceValueEntry::initial(
        child_root.reference().clone(),
        ValueKey::new("source")?,
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 1}))?),
    );
    let source_two = WorkspaceValueEntry::successor(
        source_one.reference().clone(),
        WorkspaceValue::Json(BoundedJson::new(json!({"value": 2}))?),
    )?;
    let child_budget = WorkspaceBudget::new(2, 1024, 2048, 0, 0, 0)?;
    let child_usage_one = child_budget.admit_value(&WorkspaceUsage::EMPTY, source_one.value())?;
    let child_usage_two = child_budget.admit_value(&child_usage_one, source_two.value())?;

    let parent_run = RunId::new("run-import-target-history")?;
    let parent_root = WorkspaceScope::run_root(parent_run.clone(), ScopeId::new("parent-root")?);
    let imported = WorkspaceValueEntry::imported(
        parent_root.reference().clone(),
        ValueKey::new("imported")?,
        source_two.reference().clone(),
        source_two.value().clone(),
    )?;
    let parent_budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    let parent_usage = parent_budget.admit_value(&WorkspaceUsage::EMPTY, imported.value())?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            child_run.clone(),
            "command-import-source-one",
            "event-import-source-one",
            vec![run_created_kind(
                child_root.clone(),
                child_budget.clone(),
                vec![source_one.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope { scope: child_root },
                WorkspaceMutation::PutValue {
                    entry: source_one.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: child_budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: child_usage_one,
            },
        )?)?;
        store.commit_command(&accepted_workspace_followup_request(
            child_run,
            RunSequence::FIRST,
            "command-import-source-two",
            "event-import-source-two",
            vec![RunEventKind::DeterministicOutputPublished {
                execution: NodeExecutionId::new("execution-import-source-two")?,
                value: source_two.reference().clone(),
                artifact: None,
            }],
            vec![WorkspaceMutation::PutValue { entry: source_two }],
            WorkspaceAccounting {
                budget: child_budget,
                expected_usage: child_usage_one,
                resulting_usage: child_usage_two,
            },
        )?)?;
        store.commit_command(&accepted_workspace_request(
            parent_run,
            "command-import-target",
            "event-import-target",
            vec![run_created_kind(
                parent_root.clone(),
                parent_budget.clone(),
                vec![imported.reference().clone()],
            )?],
            vec![
                WorkspaceMutation::CreateScope { scope: parent_root },
                WorkspaceMutation::PutValue {
                    entry: imported.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: parent_budget,
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: parent_usage,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut values = write.open_table(VALUES)?;
        assert!(
            values
                .remove(stored_workspace_value_key(source_one.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.value(imported.reference()));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn deleted_parent_scope_blocks_child_reads_writes_and_health()
-> Result<(), Box<dyn std::error::Error>> {
    const SCOPES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.scopes");

    let directory = TempDir::new()?;
    let run = RunId::new("run-deleted-parent-scope")?;
    let root = WorkspaceScope::run_root(run.clone(), ScopeId::new("root")?);
    let parent = WorkspaceScope::branch(
        ScopeId::new("parent")?,
        &root,
        BranchId::new("branch-parent")?,
    )?;
    let child = WorkspaceScope::branch(
        ScopeId::new("child")?,
        &parent,
        BranchId::new("branch-child")?,
    )?;
    let budget = WorkspaceBudget::new(1, 1024, 1024, 0, 0, 0)?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_workspace_request(
            run.clone(),
            "command-parent-scope",
            "event-parent-scope",
            vec![
                run_created_kind(root.clone(), budget.clone(), Vec::new())?,
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-parent")?,
                    port: PortId::new("branch-parent")?,
                    branch: BranchId::new("branch-parent")?,
                    scope: parent.clone(),
                },
                RunEventKind::BranchScopeCreated {
                    fork_execution: NodeExecutionId::new("fork-child")?,
                    port: PortId::new("branch-child")?,
                    branch: BranchId::new("branch-child")?,
                    scope: child.clone(),
                },
            ],
            vec![
                WorkspaceMutation::CreateScope { scope: root },
                WorkspaceMutation::CreateScope {
                    scope: parent.clone(),
                },
                WorkspaceMutation::CreateScope {
                    scope: child.clone(),
                },
            ],
            WorkspaceAccounting {
                budget: budget.clone(),
                expected_usage: WorkspaceUsage::EMPTY,
                resulting_usage: WorkspaceUsage::EMPTY,
            },
        )?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut scopes = write.open_table(SCOPES)?;
        assert!(
            scopes
                .remove(stored_workspace_scope_key(parent.reference())?.as_slice())?
                .is_some()
        );
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_storage_corruption(store.scope(&run, child.reference().scope()));
    assert_storage_corruption(
        store.latest_value(child.reference(), &ValueKey::new("never-written")?),
    );
    let value = WorkspaceValueEntry::initial(
        child.reference().clone(),
        ValueKey::new("blocked")?,
        WorkspaceValue::Json(BoundedJson::new(json!(true))?),
    );
    let resulting_usage = budget.admit_value(&WorkspaceUsage::EMPTY, value.value())?;
    let write_request = accepted_workspace_followup_request(
        run,
        RunSequence::new(3),
        "command-child-after-parent-delete",
        "event-child-after-parent-delete",
        vec![RunEventKind::DeterministicOutputPublished {
            execution: NodeExecutionId::new("execution-child-after-parent-delete")?,
            value: value.reference().clone(),
            artifact: None,
        }],
        vec![WorkspaceMutation::PutValue { entry: value }],
        WorkspaceAccounting {
            budget,
            expected_usage: WorkspaceUsage::EMPTY,
            resulting_usage,
        },
    )?;
    assert_storage_corruption(store.commit_command(&write_request));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
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
fn missing_or_lowered_journal_heads_are_never_interpreted_as_empty()
-> Result<(), Box<dyn std::error::Error>> {
    const HEADS: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.runs.heads");
    const EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.runs.events");

    let missing_directory = TempDir::new()?;
    let missing = accepted_request(
        "run-missing-head",
        "command-missing-head",
        "event-missing-head",
        "start",
    )?;
    {
        let store = RedbStore::open(missing_directory.path())?;
        store.commit_command(&missing)?;
    }
    let database = Database::open(missing_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut heads = write.open_table(HEADS)?;
        let _removed = heads.remove(missing.receipt.run().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(missing_directory.path())?;
    let missing_head = store.head(missing.receipt.run());
    assert!(
        matches!(
            missing_head,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            })
        ),
        "unexpected missing-head result: {missing_head:?}"
    );
    assert!(matches!(
        store.events(&EventPageQuery::new(
            missing.receipt.run().clone(),
            None,
            PageSize::new(1)?,
        )?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.command_result(missing.receipt.run(), missing.receipt.command()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.commit_command(&missing),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    drop(store);

    let lowered_directory = TempDir::new()?;
    let first = accepted_request(
        "run-lowered-head",
        "command-lowered-head",
        "event-lowered-head",
        "start",
    )?;
    let second = accepted_followup_request(
        first.receipt.run().clone(),
        "command-beyond-head",
        "event-beyond-head",
    )?;
    {
        let store = RedbStore::open(lowered_directory.path())?;
        store.commit_command(&first)?;
        store.commit_command(&second)?;
    }
    let database = Database::open(lowered_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut heads = write.open_table(HEADS)?;
        heads.insert(first.receipt.run().as_str(), RunSequence::FIRST.get())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(lowered_directory.path())?;
    assert!(matches!(
        store.head(first.receipt.run()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.events(&EventPageQuery::new(
            first.receipt.run().clone(),
            None,
            PageSize::new(1)?,
        )?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.command_result(second.receipt.run(), second.receipt.command()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.commit_command(&second),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));

    let missing_event_directory = TempDir::new()?;
    let first = accepted_request(
        "run-missing-command-event",
        "command-before-missing-event",
        "event-to-remove",
        "start",
    )?;
    let second = accepted_followup_request(
        first.receipt.run().clone(),
        "command-after-missing-event",
        "event-head-remains",
    )?;
    {
        let store = RedbStore::open(missing_event_directory.path())?;
        store.commit_command(&first)?;
        store.commit_command(&second)?;
    }
    let database = Database::open(missing_event_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut events = write.open_table(EVENTS)?;
        let mut key = Vec::new();
        key.extend_from_slice(&(first.receipt.run().as_str().len() as u32).to_be_bytes());
        key.extend_from_slice(first.receipt.run().as_str().as_bytes());
        key.extend_from_slice(&RunSequence::FIRST.get().to_be_bytes());
        let _removed = events.remove(key.as_slice())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(missing_event_directory.path())?;
    assert_storage_corruption(store.head(first.receipt.run()));
    assert!(matches!(
        store.command_result(first.receipt.run(), first.receipt.command()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.commit_command(&first),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

#[test]
fn missing_summary_usage_or_budget_rows_are_classified_as_corruption()
-> Result<(), Box<dyn std::error::Error>> {
    const SUMMARIES: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.run_summaries");
    const USAGE: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.usage");
    const BUDGETS: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.budgets");

    let summary_directory = TempDir::new()?;
    let summary_request = accepted_request(
        "run-missing-summary",
        "command-missing-summary",
        "event-missing-summary",
        "start",
    )?;
    {
        let store = RedbStore::open(summary_directory.path())?;
        store.commit_command(&summary_request)?;
    }
    let database = Database::open(summary_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut summaries = write.open_table(SUMMARIES)?;
        let _removed = summaries.remove(summary_request.receipt.run().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(summary_directory.path())?;
    assert!(matches!(
        store.run_summary(summary_request.receipt.run()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    drop(store);

    for (suffix, table) in [("usage", USAGE), ("budget", BUDGETS)] {
        let directory = TempDir::new()?;
        let request = accepted_request(
            &format!("run-missing-{suffix}"),
            &format!("command-missing-{suffix}"),
            &format!("event-missing-{suffix}"),
            "start",
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            store.commit_command(&request)?;
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut rows = write.open_table(table)?;
            let _removed = rows.remove(request.receipt.run().as_str())?;
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        if suffix == "usage" {
            assert!(matches!(
                store.workspace_usage(request.receipt.run()),
                Err(PersistenceError::Storage {
                    class: StorageFailureClass::Corruption,
                    ..
                })
            ));
        }
        let followup = accepted_followup_request(
            request.receipt.run().clone(),
            &format!("command-after-missing-{suffix}"),
            &format!("event-after-missing-{suffix}"),
        )?;
        assert!(matches!(
            store.commit_command(&followup),
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            })
        ));
    }
    Ok(())
}

#[test]
fn deleted_discovery_and_lease_rows_refuse_recovery_and_admission()
-> Result<(), Box<dyn std::error::Error>> {
    const NONTERMINAL: TableDefinition<'static, &'static str, u8> =
        TableDefinition::new("milkdrift.v1.discovery.nonterminal_runs");
    const ORDERED_LEASES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.leases");

    let recovery_directory = TempDir::new()?;
    let recovery = accepted_request(
        "run-hidden-recovery",
        "command-hidden-recovery",
        "event-hidden-recovery",
        "start",
    )?;
    {
        let store = RedbStore::open(recovery_directory.path())?;
        store.commit_command(&recovery)?;
    }
    let database = Database::open(recovery_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut nonterminal = write.open_table(NONTERMINAL)?;
        let _removed = nonterminal.remove(recovery.receipt.run().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(recovery_directory.path())?;
    assert!(matches!(
        store.nonterminal_run_page(None, PageSize::new(10)?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    drop(store);

    let lease_directory = TempDir::new()?;
    let run = RunId::new("run-hidden-lease")?;
    let lease_request = accepted_request_with_lease(
        run.as_str(),
        "command-hidden-lease",
        "event-hidden-lease",
        LeaseIndexEntry {
            run: run.clone(),
            lease: LeaseId::new("lease-hidden")?,
            attempt: AttemptId::new("attempt-hidden")?,
            worker: WorkerId::new("worker-hidden")?,
            expires_at: TimestampMillis::new(100),
            through_sequence: RunSequence::FIRST,
        },
    )?;
    {
        let store = RedbStore::open(lease_directory.path())?;
        store.commit_command(&lease_request)?;
    }
    let database = Database::open(lease_directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut leases = write.open_table(ORDERED_LEASES)?;
        let key = {
            let mut rows = leases.iter()?;
            let (key, _) = rows.next().transpose()?.ok_or("ordered lease is absent")?;
            key.value().to_vec()
        };
        let _removed = leases.remove(key.as_slice())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(lease_directory.path())?;
    assert!(matches!(
        store.active_leases(PageSize::new(10)?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.expired_leases(TimestampMillis::new(u64::MAX), PageSize::new(10)?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    Ok(())
}

fn assert_symmetric_discovery_pair_deletion_is_corruption(
    kind: DiscoveryIndexKind,
    suffix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    const RUNNABLE_IDENTITIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.runnable_by_identity");
    const RUNNABLE_ORDERED: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.runnable");
    const TIMER_IDENTITIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.timers_by_identity");
    const TIMER_ORDERED: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.timers");
    const LEASE_IDENTITIES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.leases_by_identity");
    const LEASE_ORDERED: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.leases");

    let (identity_definition, ordered_definition) = match kind {
        DiscoveryIndexKind::Runnable => (RUNNABLE_IDENTITIES, RUNNABLE_ORDERED),
        DiscoveryIndexKind::Timer => (TIMER_IDENTITIES, TIMER_ORDERED),
        DiscoveryIndexKind::Lease => (LEASE_IDENTITIES, LEASE_ORDERED),
    };
    let directory = TempDir::new()?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&accepted_request_with_discovery_index(kind, suffix)?)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut identities = write.open_table(identity_definition)?;
        let key = {
            let mut rows = identities.iter()?;
            let (key, _) = rows.next().transpose()?.ok_or("identity row is absent")?;
            key.value().to_vec()
        };
        assert!(identities.remove(key.as_slice())?.is_some());
        let mut ordered = write.open_table(ordered_definition)?;
        let key = {
            let mut rows = ordered.iter()?;
            let (key, _) = rows.next().transpose()?.ok_or("ordered row is absent")?;
            key.value().to_vec()
        };
        assert!(ordered.remove(key.as_slice())?.is_some());
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let query = match kind {
        DiscoveryIndexKind::Runnable => store
            .runnable_page(TimestampMillis::new(10), None, PageSize::new(10)?)
            .map(|_| ()),
        DiscoveryIndexKind::Timer => store
            .due_timers(TimestampMillis::new(10), PageSize::new(10)?)
            .map(|_| ()),
        DiscoveryIndexKind::Lease => store.active_leases(PageSize::new(10)?).map(|_| ()),
    };
    assert!(matches!(
        query,
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    Ok(())
}

#[test]
fn symmetric_runnable_pair_deletion_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    assert_symmetric_discovery_pair_deletion_is_corruption(
        DiscoveryIndexKind::Runnable,
        "symmetric-runnable",
    )
}

#[test]
fn symmetric_timer_pair_deletion_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    assert_symmetric_discovery_pair_deletion_is_corruption(
        DiscoveryIndexKind::Timer,
        "symmetric-timer",
    )
}

#[test]
fn symmetric_lease_pair_deletion_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    assert_symmetric_discovery_pair_deletion_is_corruption(
        DiscoveryIndexKind::Lease,
        "symmetric-lease",
    )
}

#[test]
fn enveloped_v1_store_backfills_discovery_accounting_atomically()
-> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    const DISCOVERY_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.discovery.accounting");
    const WORKSPACE_VALUE_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.workspace.value_accounting");
    const INTEGRITY_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.integrity.accounting");
    const INTEGRITY_ROOTS: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.integrity.roots");
    const INTEGRITY_NODES: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.integrity.trie_nodes");
    const ARTIFACT_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.artifacts.accounting");

    let directory = TempDir::new()?;
    {
        let store = RedbStore::open(directory.path())?;
        for (kind, suffix) in [
            (DiscoveryIndexKind::Runnable, "migrate-runnable"),
            (DiscoveryIndexKind::Timer, "migrate-timer"),
            (DiscoveryIndexKind::Lease, "migrate-lease"),
        ] {
            store.commit_command(&accepted_request_with_discovery_index(kind, suffix)?)?;
        }
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(METADATA)?;
        metadata.insert("internal_document_format_version", 1)?;
        let mut accounting = write.open_table(DISCOVERY_ACCOUNTING)?;
        drop(accounting.remove("active_index_counts")?);
        drop(accounting);
        let mut value_accounting = write.open_table(WORKSPACE_VALUE_ACCOUNTING)?;
        let keys = value_accounting
            .iter()?
            .map(|row| row.map(|(key, _)| key.value().to_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            drop(value_accounting.remove(key.as_str())?);
        }
        drop(value_accounting);
        for definition in [INTEGRITY_ACCOUNTING, INTEGRITY_ROOTS, ARTIFACT_ACCOUNTING] {
            let mut table = write.open_table(definition)?;
            let keys = table
                .iter()?
                .map(|row| row.map(|(key, _)| key.value().to_owned()))
                .collect::<Result<Vec<_>, _>>()?;
            for key in keys {
                drop(table.remove(key.as_str())?);
            }
        }
        let mut nodes = write.open_table(INTEGRITY_NODES)?;
        let keys = nodes
            .iter()?
            .map(|row| row.map(|(key, _)| key.value().to_vec()))
            .collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            drop(nodes.remove(key.as_slice())?);
        }
    }
    write.commit()?;
    drop(database);

    {
        let store = RedbStore::open(directory.path())?;
        assert_eq!(
            store
                .runnable_page(TimestampMillis::new(10), None, PageSize::new(10)?)?
                .entries
                .len(),
            1
        );
        assert_eq!(
            store
                .due_timers(TimestampMillis::new(10), PageSize::new(10)?)?
                .len(),
            1
        );
        assert_eq!(store.active_leases(PageSize::new(10)?)?.entries.len(), 1);
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let read = database.begin_read()?;
    let metadata = read.open_table(METADATA)?;
    assert_eq!(
        metadata
            .get("internal_document_format_version")?
            .ok_or("internal document format marker is absent")?
            .value(),
        3
    );
    let accounting = read.open_table(DISCOVERY_ACCOUNTING)?;
    assert!(accounting.get("active_index_counts")?.is_some());
    let value_accounting = read.open_table(WORKSPACE_VALUE_ACCOUNTING)?;
    assert!(value_accounting.get("")?.is_some());
    assert_eq!(value_accounting.len()?, 4);
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
            cursor: None,
        };
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..3])?;
        assert!(store.cleanup_orphans(cleanup_request.clone()).is_err());
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
        cursor: None,
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
            cursor: None,
        };
        assert!(store.cleanup_orphans(request.clone()).is_err());
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
fn orphan_cleanup_cursors_visit_every_family_without_starvation()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let artifact_budget = WorkspaceBudget::new(0, 0, 0, 1, 4096, 4096)?;

    let referenced_bytes = b"durably-referenced-content";
    let referenced = artifact_metadata(
        "artifact-cleanup-referenced",
        referenced_bytes,
        ArtifactSensitivity::Public,
    )?;
    let referenced_request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication-cleanup-referenced")?,
        RunId::new("run-cleanup-referenced")?,
        referenced.clone(),
        artifact_budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&referenced_request)?;
    store.write_chunk(&referenced_request.publication, 0, referenced_bytes)?;
    store.commit_publication(&referenced_request.publication)?;

    for index in 0..3 {
        let bytes = format!("abandoned-publication-{index}").into_bytes();
        let metadata = artifact_metadata(
            &format!("artifact-abandoned-{index}"),
            &bytes,
            ArtifactSensitivity::Public,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-abandoned-{index}"))?,
            RunId::new(format!("run-abandoned-{index}"))?,
            metadata,
            artifact_budget.clone(),
            WorkspaceUsage::EMPTY,
        )?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, &bytes[..1])?;
    }

    for index in 0..4 {
        std::fs::write(
            directory
                .path()
                .join("artifacts/.tmp")
                .join(format!("unowned-{index}.part")),
            format!("temporary-{index}"),
        )?;
        let bytes = format!("unowned-content-{index}").into_bytes();
        let digest = ContentDigest::for_bytes(&bytes).to_hex();
        let shard = directory.path().join("artifacts").join(&digest[..2]);
        std::fs::create_dir_all(&shard)?;
        std::fs::write(shard.join(&digest[2..]), bytes)?;
    }

    let observed_at = TimestampMillis::new(u64::MAX);
    let created_before = TimestampMillis::new(u64::MAX - 1);
    let mut page = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at,
        created_before,
        limit: PageSize::new(2)?,
        cursor: None,
    })?;
    let first_cursor = page
        .next_cursor
        .clone()
        .ok_or("first cleanup page must have a continuation")?;
    assert!(matches!(
        store.cleanup_orphans(OrphanCleanupRequest {
            observed_at,
            created_before: TimestampMillis::new(u64::MAX - 2),
            limit: PageSize::new(2)?,
            cursor: Some(first_cursor),
        }),
        Err(PersistenceError::InvalidCursor(_))
    ));

    let mut pages = 1_u32;
    let mut temporary_removed = page.temporary_publications_removed;
    let mut content_removed = page.unreferenced_blobs_removed;
    while let Some(cursor) = page.next_cursor {
        page = store.cleanup_orphans(OrphanCleanupRequest {
            observed_at,
            created_before,
            limit: PageSize::new(2)?,
            cursor: Some(cursor),
        })?;
        pages += 1;
        assert!(pages < 20, "cleanup cursor failed to converge");
        temporary_removed = temporary_removed.saturating_add(page.temporary_publications_removed);
        content_removed = content_removed.saturating_add(page.unreferenced_blobs_removed);
    }

    assert!(pages >= 6);
    assert_eq!(temporary_removed, 7);
    assert_eq!(content_removed, 4);
    assert!(store.is_committed(referenced.reference())?);
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
            cursor: None,
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
        cursor: None,
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
