//! Focused physical-deletion and lowered-accounting regressions.

use milkdrift_blueprint::{BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::BoundedJson;
use milkdrift_persistence::{
    ActorRef, ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest, BeginArtifactOutcome,
    BeginArtifactPublication, CommandDisposition, CommandId, CommandReceipt, CommandResultDocument,
    EventId, IndexedRunState, OrphanCleanupRequest, PageSize, PersistenceError, RevisionStore,
    RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal, RunSequence, RunSummaryIndex,
    SnapshotDocument, SnapshotId, SnapshotStore, StorageAdmin, StorageFailureClass,
    StorageHealthStatus, TimestampMillis, WorkspaceAccounting, WorkspaceStore, history_digest,
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
const ARTIFACT_MANIFEST: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.authoritative_manifest");
const ARTIFACT_PUBLICATIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.publications");
const ARTIFACTS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.by_digest_and_id");
const ARTIFACT_REFERENCES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.references");
const ARTIFACT_TEMP_OWNERS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.temp_owners");
const ARTIFACT_TEMP_MANIFEST: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.authoritative_temp_manifest");
const RUN_ARTIFACT_OWNERSHIP: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.ownership_by_run");
const ARTIFACT_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.accounting");
const WORKSPACE_USAGE: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.usage");
const WORKSPACE_BUDGETS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.budgets");
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
const METADATA: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.metadata");
const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_id");
const REVISIONS_BY_DIGEST: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.revisions.by_digest_and_id");
const RUN_HEADS: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.runs.heads");
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

fn revision_id() -> Result<RevisionId, PersistenceError> {
    serde_json::from_value(json!(format!("rev_{}", "0".repeat(64)))).map_err(PersistenceError::Json)
}

fn legacy_payload(envelope: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    const MARKER: &[u8] = b",\"payload\":";
    let start = envelope
        .windows(MARKER.len())
        .position(|window| window == MARKER)
        .map(|position| position + MARKER.len())
        .ok_or("internal document envelope has no payload")?;
    let end = envelope
        .len()
        .checked_sub(1)
        .filter(|end| envelope.get(*end) == Some(&b'}'))
        .ok_or("internal document envelope has no closing object")?;
    Ok(envelope[start..end].to_vec())
}

fn downgrade_string_documents(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static str, &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    let rows = table
        .iter()?
        .map(|item| {
            let (key, value) = item?;
            Ok((key.value().to_owned(), legacy_payload(value.value())?))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    for (key, value) in rows {
        table.insert(key.as_str(), value.as_slice())?;
    }
    Ok(())
}

fn downgrade_binary_documents(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static [u8], &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    let rows = table
        .iter()?
        .map(|item| {
            let (key, value) = item?;
            Ok((key.value().to_vec(), legacy_payload(value.value())?))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    for (key, value) in rows {
        table.insert(key.as_slice(), value.as_slice())?;
    }
    Ok(())
}

fn clear_string_documents(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static str, &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    let keys = table
        .iter()?
        .map(|item| item.map(|(key, _)| key.value().to_owned()))
        .collect::<Result<Vec<_>, _>>()?;
    for key in keys {
        let _ = table.remove(key.as_str())?;
    }
    Ok(())
}

fn clear_binary_documents(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static [u8], &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    let keys = table
        .iter()?
        .map(|item| item.map(|(key, _)| key.value().to_vec()))
        .collect::<Result<Vec<_>, _>>()?;
    for key in keys {
        let _ = table.remove(key.as_slice())?;
    }
    Ok(())
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
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run: publication.run.clone(),
                workflow: WorkflowId::new("workflow-integrity")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(20),
            }),
            ..RunIndexUpdate::default()
        },
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
        RunIndexUpdate {
            summary: Some(RunSummaryIndex {
                run,
                workflow: WorkflowId::new("workflow-integrity")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: RunSequence::FIRST,
                updated_at: TimestampMillis::new(10),
            }),
            ..RunIndexUpdate::default()
        },
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
fn artifact_only_usage_requires_workspace_value_accounting()
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
        let mut accounting = write.open_table(WORKSPACE_VALUE_ACCOUNTING)?;
        assert!(accounting.remove(first.run.as_str())?.is_some());
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
    }
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
fn legacy_physical_v1_artifact_publication_backfills_integrity_documents_on_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let request = publication_request(
        "artifact-legacy",
        "publication-legacy",
        "run-legacy",
        b"legacy",
        WorkspaceBudget::new(0, 0, 0, 2, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    {
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path()).with_artifact_limits(10, 10, 64),
        )?;
        publish(&store, &request, b"legacy")?;
    }

    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    downgrade_string_documents(&write, ARTIFACT_METADATA)?;
    downgrade_string_documents(&write, ARTIFACT_PUBLICATIONS)?;
    downgrade_binary_documents(&write, ARTIFACTS_BY_DIGEST)?;
    downgrade_binary_documents(&write, ARTIFACT_REFERENCES)?;
    downgrade_string_documents(&write, WORKSPACE_USAGE)?;
    downgrade_string_documents(&write, WORKSPACE_BUDGETS)?;
    clear_string_documents(&write, ARTIFACT_MANIFEST)?;
    clear_string_documents(&write, ARTIFACT_TEMP_MANIFEST)?;
    clear_binary_documents(&write, RUN_ARTIFACT_OWNERSHIP)?;
    clear_string_documents(&write, ARTIFACT_ACCOUNTING)?;
    clear_string_documents(&write, DISCOVERY_ACCOUNTING)?;
    clear_string_documents(&write, WORKSPACE_VALUE_ACCOUNTING)?;
    clear_string_documents(&write, INTEGRITY_ACCOUNTING)?;
    clear_string_documents(&write, INTEGRITY_ROOTS)?;
    clear_binary_documents(&write, INTEGRITY_NODES)?;
    {
        let mut metadata = write.open_table(METADATA)?;
        let _ = metadata.remove("internal_document_format_version")?;
        metadata.insert("artifact_content_bytes", 6)?;
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open_with_config(
        RedbStoreConfig::new(directory.path()).with_artifact_limits(10, 10, 64),
    )
    .map_err(|cause| format!("legacy reopen failed: {cause}"))?;
    assert!(
        store
            .is_committed(request.metadata.reference())
            .map_err(|cause| format!("legacy committed lookup failed: {cause}"))?
    );
    assert!(store.is_referenced_by_run(&request.run, request.metadata.reference())?);
    assert_eq!(
        store.workspace_usage(&request.run)?,
        request.resulting_usage
    );
    assert!(matches!(
        store.begin_publication(&request)?,
        BeginArtifactOutcome::AlreadyCommitted(_)
    ));

    let second = publication_request(
        "artifact-after-legacy",
        "publication-after-legacy",
        "run-after-legacy",
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
            request.receipt.run().clone(),
            RunSequence::FIRST,
            history_digest(&request.events)?,
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
            let _ = latest.remove(request.receipt.run().as_str())?;
        } else {
            let mut heads = write.open_table(RUN_HEADS)?;
            heads.insert(request.receipt.run().as_str(), 0)?;
        }
        write.commit()?;
        drop(database);
        let store = RedbStore::open(directory.path())?;
        assert_corruption(store.latest_snapshot(request.receipt.run()));
        if !delete_pointer {
            assert_corruption(store.put_snapshot(&snapshot));
        }
    }
    Ok(())
}
