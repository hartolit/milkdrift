//! Focused physical-deletion and lowered-accounting regressions.

use milkdrift_blueprint::{BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, IndexedRunState, IntegrityScanRequest, OrphanCleanupRequest, PageSize,
    PersistenceError, RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal,
    RunSequence, RunSummaryIndex, SnapshotDocument, SnapshotId, SnapshotStore, StorageAdmin,
    StorageFailureClass, StorageHealthStatus, TimestampMillis, WorkspaceAccounting, WorkspaceStore,
    history_digest,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, WorkspaceBudget, WorkspaceUsage,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde_json::json;
use tempfile::TempDir;

const ARTIFACT_METADATA: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.metadata_by_id");
const ARTIFACTS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.by_digest_and_id");
const ARTIFACT_REFERENCES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.references");
const ARTIFACT_TEMP_OWNERS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.temp_owners");
const ARTIFACT_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.accounting");
const WORKSPACE_USAGE: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.usage");
const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_id");
const REVISIONS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_digest_and_id");
const RUN_HEADS: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.runs.heads");
const RUN_EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.runs.events");
const EVENT_HISTORY_DIGESTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.runs.event_history_digests");
const RUN_HISTORY_HEADS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v2.runs.history_heads");
const SNAPSHOT_LATEST: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.snapshots.latest_by_run");

fn assert_corruption<T: std::fmt::Debug>(result: Result<T, PersistenceError>) {
    assert!(
        matches!(
            &result,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            }) | Err(PersistenceError::Corruption(_))
        ),
        "expected corruption, got {result:?}"
    );
}

fn exhaustive_integrity_failure_count(store: &RedbStore) -> Result<usize, PersistenceError> {
    let mut cursor = None;
    let mut failures = 0_usize;
    for _ in 0..10_000 {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(7)?,
            verify_artifact_content: false,
            cursor,
        })?;
        failures = failures.saturating_add(page.failures.len());
        let Some(next) = page.next_cursor else {
            return Ok(failures);
        };
        cursor = Some(next);
    }
    Err(PersistenceError::Bounds {
        location: "integrity_regression.exhaustive_scan",
        reason: "integrity scan did not exhaust within 10,000 bounded pages".to_owned(),
    })
}

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn artifact_metadata(
    id: &str,
    bytes: &[u8],
) -> Result<ArtifactMetadata, Box<dyn std::error::Error>> {
    Ok(ArtifactMetadata::new(
        milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new(id)?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("application/octet-stream")?,
            bytes.len() as u64,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(
            CausalReference::External {
                source: CausalId::new("integrity-regression")?,
            },
            Vec::new(),
        )?,
    )?)
}

fn publication_request(
    artifact: &str,
    publication: &str,
    run: &str,
    bytes: &[u8],
    budget: WorkspaceBudget,
    expected_usage: WorkspaceUsage,
) -> Result<BeginArtifactPublication, Box<dyn std::error::Error>> {
    Ok(BeginArtifactPublication::new(
        ArtifactPublicationId::new(publication)?,
        RunId::new(run)?,
        artifact_metadata(artifact, bytes)?,
        budget,
        expected_usage,
    )?)
}

fn publish(
    store: &RedbStore,
    request: &BeginArtifactPublication,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    store.begin_publication(request)?;
    store.write_chunk(&request.publication, 0, bytes)?;
    store.commit_publication(&request.publication)?;
    Ok(())
}

fn double_charge_request(
    publication: &BeginArtifactPublication,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new("command-double-charge")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        publication.run.clone(),
        ActorRef::new("actor-integrity")?,
        RunSequence::ZERO,
        TimestampMillis::new(20),
        br#"{"schema_version":1,"type":"double_charge"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new("event-double-charge")?,
        publication.run.clone(),
        RunSequence::FIRST,
        TimestampMillis::new(20),
        RunEventKind::ArtifactPublished {
            metadata: publication.metadata.clone(),
        },
    )?;
    let result = CommandResultDocument::new(
        command,
        publication.run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let resulting_usage = publication
        .budget
        .admit_artifact(&publication.resulting_usage, &publication.metadata)?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: publication.budget.clone(),
            expected_usage: publication.resulting_usage,
            resulting_usage,
        }),
        vec![publication.metadata.reference().clone()],
        vec![publication.metadata.reference().clone()],
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run: publication.run.clone(),
                workflow: WorkflowId::new("workflow-integrity")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(20),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn start_request(run: &str) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let run = RunId::new(run)?;
    let command = CommandId::new(format!("command-{run}"))?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-integrity")?,
        RunSequence::ZERO,
        TimestampMillis::new(10),
        br#"{"schema_version":1,"type":"start"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(format!("event-{run}"))?,
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
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-integrity")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

#[test]
fn deleted_artifact_reference_rows_cannot_reopen_double_charging()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let budget = WorkspaceBudget::new(0, 0, 0, 10, 1_024, 1_024)?;
    let request = publication_request(
        "artifact-owned",
        "publication-owned",
        "run-owned",
        b"owned",
        budget,
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        publish(&store, &request, b"owned")?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut references = write.open_table(ARTIFACT_REFERENCES)?;
        let keys = references
            .iter()?
            .map(|item| item.map(|(key, _)| key.value().to_vec()))
            .collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            let _ = references.remove(key.as_slice())?;
        }
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.is_referenced_by_run(&request.run, request.metadata.reference()));
    assert_corruption(store.commit_command(&double_charge_request(&request)?));
    Ok(())
}

#[test]
fn artifact_only_usage_requires_a_complete_workspace_domain()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let budget = WorkspaceBudget::new(0, 0, 0, 3, 64, 64)?;
    let first = publication_request(
        "artifact-accounted-first",
        "publication-accounted-first",
        "run-artifact-accounted",
        b"first",
        budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        publish(&store, &first, b"first")?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut usage = write.open_table(WORKSPACE_USAGE)?;
        assert!(usage.remove(first.run.as_str())?.is_some());
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.workspace_usage(&first.run));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    let second = publication_request(
        "artifact-accounted-second",
        "publication-accounted-second",
        first.run.as_str(),
        b"second",
        budget,
        first.resulting_usage,
    )?;
    assert_corruption(store.begin_publication(&second));
    Ok(())
}

#[test]
fn artifact_and_revision_primary_digest_pairs_fail_closed_after_deletion()
-> Result<(), Box<dyn std::error::Error>> {
    for delete_primary in [true, false] {
        let directory = TempDir::new()?;
        let request = publication_request(
            "artifact-paired",
            "publication-paired",
            "run-paired",
            b"paired",
            WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            publish(&store, &request, b"paired")?;
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        if delete_primary {
            let mut metadata = write.open_table(ARTIFACT_METADATA)?;
            let _ = metadata.remove(request.metadata.reference().artifact().as_str())?;
        } else {
            let mut by_digest = write.open_table(ARTIFACTS_BY_DIGEST)?;
            let key = by_digest
                .iter()?
                .next()
                .transpose()?
                .ok_or("artifact digest index is empty")?
                .0
                .value()
                .to_vec();
            let _ = by_digest.remove(key.as_slice())?;
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open(directory.path())?;
        assert_corruption(store.is_committed(request.metadata.reference()));
        assert!(exhaustive_integrity_failure_count(&store)? > 0);
        assert_eq!(
            store.health(TimestampMillis::new(30))?.status,
            StorageHealthStatus::Degraded
        );
    }

    for delete_primary in [true, false] {
        let directory = TempDir::new()?;
        let fixture = include_bytes!("../../../crates/blueprint/tests/fixtures/revision-v1.json");
        let (_, revision) = BlueprintRevisionDocument::from_json(fixture)?;
        {
            let store = RedbStore::open(directory.path())?;
            store.put_revision(&revision)?;
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        if delete_primary {
            let mut revisions = write.open_table(REVISIONS)?;
            let _ = revisions.remove(revision.id().as_str())?;
        } else {
            let mut by_digest = write.open_table(REVISIONS_BY_DIGEST)?;
            let key = by_digest
                .iter()?
                .next()
                .transpose()?
                .ok_or("revision digest index is empty")?
                .0
                .value()
                .to_vec();
            let _ = by_digest.remove(key.as_slice())?;
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open(directory.path())?;
        assert_corruption(store.revision(revision.id()));
        assert!(exhaustive_integrity_failure_count(&store)? > 0);
        assert_eq!(
            store.health(TimestampMillis::new(30))?.status,
            StorageHealthStatus::Degraded
        );
    }
    Ok(())
}

#[test]
fn missing_digest_index_cannot_turn_existing_content_into_a_second_charge()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let budget = WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?;
    let first = publication_request(
        "artifact-digest-first",
        "publication-digest-first",
        "run-digest-first",
        b"shared-content",
        budget.clone(),
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        publish(&store, &first, b"shared-content")?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut by_digest = write.open_table(ARTIFACTS_BY_DIGEST)?;
        let key = by_digest
            .iter()?
            .next()
            .transpose()?
            .ok_or("artifact digest index is empty")?
            .0
            .value()
            .to_vec();
        assert!(by_digest.remove(key.as_slice())?.is_some());
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    let second = publication_request(
        "artifact-digest-second",
        "publication-digest-second",
        "run-digest-second",
        b"shared-content",
        budget,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&second)?;
    store.write_chunk(&second.publication, 0, b"shared-content")?;
    assert_corruption(store.commit_publication(&second.publication));
    assert_corruption(store.is_committed(first.metadata.reference()));
    assert!(!store.is_committed(second.metadata.reference())?);
    Ok(())
}

#[test]
fn missing_temp_owner_cannot_delete_a_live_writable_publication()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = publication_request(
        "artifact-writable",
        "publication-writable",
        "run-writable",
        b"writable",
        WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open(directory.path())?;
        store.begin_publication(&request)?;
        store.write_chunk(&request.publication, 0, b"write")?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut owners = write.open_table(ARTIFACT_TEMP_OWNERS)?;
        let key = owners
            .iter()?
            .next()
            .transpose()?
            .ok_or("temporary owner is absent")?
            .0
            .value()
            .to_owned();
        let _ = owners.remove(key.as_str())?;
    }
    write.commit()?;
    drop(database);

    let temporary = directory.path().join("artifacts/.tmp");
    let before = std::fs::read_dir(&temporary)?.count();
    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(u64::MAX),
        created_before: TimestampMillis::new(u64::MAX - 1),
        limit: PageSize::new(10)?,
        cursor: None,
    }));
    assert_eq!(std::fs::read_dir(temporary)?.count(), before);
    Ok(())
}

#[test]
fn aggregate_artifact_counter_deletion_lowering_and_real_limit_are_fail_closed()
-> Result<(), Box<dyn std::error::Error>> {
    for lower_instead_of_delete in [false, true] {
        let directory = TempDir::new()?;
        let config = RedbStoreConfig::new(directory.path()).with_artifact_limits(10, 10, 64);
        let first = publication_request(
            "artifact-counter-one",
            "publication-counter-one",
            "run-counter-one",
            b"123456",
            WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        {
            let store = RedbStore::open_with_config(config)?;
            publish(&store, &first, b"123456")?;
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut accounting = write.open_table(ARTIFACT_ACCOUNTING)?;
            if lower_instead_of_delete {
                let bytes = accounting
                    .get("artifact_content_bytes")?
                    .ok_or("artifact accounting is absent")?
                    .value()
                    .to_vec();
                let mut envelope: serde_json::Value = serde_json::from_slice(&bytes)?;
                envelope["payload"]["committed_content_bytes"] = json!(0);
                let lowered = serde_json::to_vec(&envelope)?;
                accounting.insert("artifact_content_bytes", lowered.as_slice())?;
            } else {
                let _ = accounting.remove("artifact_content_bytes")?;
            }
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path()).with_artifact_limits(10, 10, 64),
        )?;
        let second = publication_request(
            "artifact-counter-two",
            "publication-counter-two",
            "run-counter-two",
            b"abcde",
            WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        assert_corruption(store.begin_publication(&second));
    }

    let directory = TempDir::new()?;
    let config = RedbStoreConfig::new(directory.path()).with_artifact_limits(10, 10, 64);
    let store = RedbStore::open_with_config(config)?;
    let first = publication_request(
        "artifact-limit-one",
        "publication-limit-one",
        "run-limit-one",
        b"123456",
        WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    publish(&store, &first, b"123456")?;
    let second = publication_request(
        "artifact-limit-two",
        "publication-limit-two",
        "run-limit-two",
        b"abcde",
        WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&second)?;
    store.write_chunk(&second.publication, 0, b"abcde")?;
    assert!(matches!(
        store.commit_publication(&second.publication),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        })
    ));
    Ok(())
}

#[test]
fn snapshot_pointer_deletion_and_lowered_journal_head_are_corruption()
-> Result<(), Box<dyn std::error::Error>> {
    for delete_pointer in [true, false] {
        let directory = TempDir::new()?;
        let request = start_request(if delete_pointer {
            "run-snapshot-pointer"
        } else {
            "run-snapshot-head"
        })?;
        let snapshot = SnapshotDocument::new(
            SnapshotId::new(if delete_pointer {
                "snapshot-pointer"
            } else {
                "snapshot-head"
            })?,
            request.receipt().run().clone(),
            RunSequence::FIRST,
            history_digest(request.events())?,
            1,
            b"projection".to_vec(),
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            store.commit_command(&request)?;
            store.put_snapshot(&snapshot)?;
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        if delete_pointer {
            let mut latest = write.open_table(SNAPSHOT_LATEST)?;
            let _ = latest.remove(request.receipt().run().as_str())?;
        } else {
            let mut heads = write.open_table(RUN_HEADS)?;
            heads.insert(request.receipt().run().as_str(), 0)?;
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open(directory.path())?;
        assert_corruption(store.latest_snapshot(request.receipt().run()));
        if !delete_pointer {
            assert_corruption(store.put_snapshot(&snapshot));
        }
    }
    Ok(())
}

#[test]
fn missing_history_chain_checkpoint_or_head_is_corruption()
-> Result<(), Box<dyn std::error::Error>> {
    for delete_checkpoint in [true, false] {
        let directory = TempDir::new()?;
        let request = start_request(if delete_checkpoint {
            "run-missing-history-checkpoint"
        } else {
            "run-missing-history-head"
        })?;
        {
            let store = RedbStore::open(directory.path())?;
            store.commit_command(&request)?;
        }

        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        if delete_checkpoint {
            let mut checkpoints = write.open_table(EVENT_HISTORY_DIGESTS)?;
            let key = checkpoints
                .iter()?
                .next()
                .transpose()?
                .ok_or("history checkpoint is absent")?
                .0
                .value()
                .to_vec();
            assert!(checkpoints.remove(key.as_slice())?.is_some());
        } else {
            let mut heads = write.open_table(RUN_HISTORY_HEADS)?;
            assert!(heads.remove(request.receipt().run().as_str())?.is_some());
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        assert_corruption(store.history_digest(
            request.receipt().run(),
            RunSequence::FIRST,
        ));
        assert!(exhaustive_integrity_failure_count(&store)? > 0);
    }
    Ok(())
}

#[test]
fn missing_authoritative_event_is_corruption() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = start_request("run-missing-event")?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&request)?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut events = write.open_table(RUN_EVENTS)?;
        let key = events
            .iter()?
            .next()
            .transpose()?
            .ok_or("run event is absent")?
            .0
            .value()
            .to_vec();
        assert!(events.remove(key.as_slice())?.is_some());
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.head(request.receipt().run()));
    assert!(exhaustive_integrity_failure_count(&store)? > 0);
    Ok(())
}
