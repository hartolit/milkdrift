//! Focused physical-deletion and lowered-accounting regressions.

use milkdrift_authority::ActorRef;
use milkdrift_blueprint::{BlueprintRevisionDocument, RevisionId, WorkflowId};
use milkdrift_capability::{BoundedJson, InvocationId};
use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactStore, AtomicRunCommitRequest, BeginArtifactPublication,
    CommandDisposition, CommandId, CommandReceipt, CommandResultDocument, EventId, IndexedRunState,
    IntegrityScanFamily, IntegrityScanRequest, OrphanCleanupRequest, PageSize, PersistenceError,
    RevisionStore, RunEventEnvelope, RunEventKind, RunIndexUpdate, RunJournal, RunSequence,
    RunSummaryIndex, SnapshotDocument, SnapshotId, SnapshotLoad, SnapshotStore, StorageAdmin,
    StorageFailureClass, StorageHealthStatus, TimestampMillis, WorkspaceAccounting, WorkspaceStore,
    history_digest,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactRetention, ArtifactSensitivity,
    CausalId, CausalReference, ContentDigest, MediaType, RunId, ScopeId, ScopeReference, ValueKey,
    ValueVersion, WorkspaceBudget, WorkspaceUsage, WorkspaceValueReference,
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
const RUN_ARTIFACT_OWNERSHIP: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.ownership_by_run");
const ARTIFACT_TEMP_OWNERS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.temp_owners");
const ARTIFACT_PUBLICATIONS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.publications");
const ARTIFACT_PUBLICATIONS_BY_AGE: TableDefinition<'static, &'static [u8], &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.writable_by_age");
const ARTIFACT_RESERVATIONS: TableDefinition<'static, &'static str, &'static str> =
    TableDefinition::new("milkdrift.v1.artifacts.reservations_by_run");
const ARTIFACT_PATHS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v2.artifacts.path_inventory");
const ARTIFACT_DELETE_GUARDS: TableDefinition<'static, &'static [u8], u8> =
    TableDefinition::new("milkdrift.v2.artifacts.delete_guards");
const ARTIFACT_DIGEST_RESERVATIONS: TableDefinition<'static, &'static [u8], u8> =
    TableDefinition::new("milkdrift.v1.artifacts.reservations_by_digest");
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
const COMMAND_RESULTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.commands.results");
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
    store.write_chunk(request.publication(), 0, bytes)?;
    store.commit_publication(request.publication())?;
    Ok(())
}

#[test]
fn integrity_pages_preserve_budget_and_resume_inside_delete_guard_phase()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    let mut publications = Vec::new();
    for ordinal in 0..3 {
        let request = publication_request(
            &format!("artifact-cursor-{ordinal}"),
            &format!("publication-cursor-{ordinal}"),
            &format!("run-cursor-{ordinal}"),
            b"pending",
            WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        store.begin_publication(&request)?;
        publications.push(request.publication().clone());
    }
    drop(store);
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    {
        let mut guards = write.open_table(ARTIFACT_DELETE_GUARDS)?;
        for publication in publications {
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"milkdrift.artifact-publication-temp.v1\0");
            hasher.update(publication.as_str().as_bytes());
            let identity = format!("{}.part", hasher.finalize());
            let mut key = Vec::new();
            for component in ["temp", identity.as_str()] {
                key.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
                key.extend_from_slice(component.as_bytes());
            }
            guards.insert(key.as_slice(), 1)?;
        }
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(directory.path())?;
    let complete = store.scan_integrity(IntegrityScanRequest {
        limit: PageSize::new(1_000)?,
        verify_artifact_content: false,
        cursor: None,
    })?;
    assert_eq!(complete.failures.len(), 3);
    assert!(complete.next_cursor.is_none());

    let mut cursor = None;
    let mut paginated_documents = 0_u64;
    let mut paginated_failures = 0_usize;
    let mut resumed_delete_guard = false;
    for _ in 0..10_000 {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(1)?,
            verify_artifact_content: false,
            cursor,
        })?;
        assert!(page.documents_checked <= 1);
        paginated_documents = paginated_documents.saturating_add(page.documents_checked);
        paginated_failures = paginated_failures.saturating_add(page.failures.len());
        let Some(next) = page.next_cursor else {
            assert_eq!(paginated_documents, complete.documents_checked);
            assert_eq!(paginated_failures, complete.failures.len());
            assert!(
                resumed_delete_guard,
                "scan never returned an in-progress phase-34 delete-guard cursor"
            );
            return Ok(());
        };
        if next.family() == IntegrityScanFamily::Indexes
            && next.after_key().get(33) == Some(&34)
            && matches!(next.after_key().get(37), Some(1 | 2))
        {
            resumed_delete_guard = true;
        }
        cursor = Some(next);
    }
    Err("phase-34 integrity scan did not exhaust".into())
}

#[test]
fn scrub_detects_corruption_in_every_artifact_coordination_family()
-> Result<(), Box<dyn std::error::Error>> {
    #[derive(Clone, Copy)]
    enum Family {
        Publications,
        Age,
        RunReservations,
        Paths,
        DeleteGuards,
        DigestReservations,
    }

    for (ordinal, family) in [
        Family::Publications,
        Family::Age,
        Family::RunReservations,
        Family::Paths,
        Family::DeleteGuards,
        Family::DigestReservations,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let request = publication_request(
            &format!("artifact-scrub-family-{ordinal}"),
            &format!("publication-scrub-family-{ordinal}"),
            &format!("run-scrub-family-{ordinal}"),
            b"pending",
            WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        {
            let store = RedbStore::open(directory.path())?;
            store.begin_publication(&request)?;
            assert_eq!(exhaustive_integrity_failure_count(&store)?, 0);
        }
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        match family {
            Family::Publications => {
                let mut table = write.open_table(ARTIFACT_PUBLICATIONS)?;
                assert!(table.remove(request.publication().as_str())?.is_some());
            }
            Family::Age => {
                let mut table = write.open_table(ARTIFACT_PUBLICATIONS_BY_AGE)?;
                let key = table
                    .iter()?
                    .next()
                    .transpose()?
                    .ok_or("age row absent")?
                    .0
                    .value()
                    .to_vec();
                assert!(table.remove(key.as_slice())?.is_some());
            }
            Family::RunReservations => {
                let mut table = write.open_table(ARTIFACT_RESERVATIONS)?;
                assert!(table.remove(request.run().as_str())?.is_some());
            }
            Family::Paths => {
                let mut table = write.open_table(ARTIFACT_PATHS)?;
                let key = table
                    .iter()?
                    .next()
                    .transpose()?
                    .ok_or("path row absent")?
                    .0
                    .value()
                    .to_vec();
                assert!(table.remove(key.as_slice())?.is_some());
            }
            Family::DeleteGuards => {
                let mut key = Vec::new();
                for component in ["temp", "dangling-delete-guard.part"] {
                    key.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
                    key.extend_from_slice(component.as_bytes());
                }
                write
                    .open_table(ARTIFACT_DELETE_GUARDS)?
                    .insert(key.as_slice(), 1)?;
            }
            Family::DigestReservations => {
                let mut table = write.open_table(ARTIFACT_DIGEST_RESERVATIONS)?;
                let key = table
                    .iter()?
                    .next()
                    .transpose()?
                    .ok_or("digest reservation absent")?
                    .0
                    .value()
                    .to_vec();
                assert!(table.remove(key.as_slice())?.is_some());
            }
        }
        write.commit()?;
        drop(database);
        let reopened = RedbStore::open(directory.path())?;
        assert!(
            exhaustive_integrity_failure_count(&reopened)? > 0,
            "artifact coordination corruption family {ordinal} was not detected"
        );
    }
    Ok(())
}

#[test]
fn artifact_commit_rejects_missing_authoritative_provenance_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let missing_artifact = milkdrift_workspace::ArtifactReference::new(
        ArtifactId::new("missing-causal-artifact")?,
        ContentDigest::for_bytes(b"missing"),
        MediaType::new("application/octet-stream")?,
        7,
    );
    let missing_value = WorkspaceValueReference::new(
        ScopeReference::new(
            RunId::new("run-missing-workspace-provenance")?,
            ScopeId::new("scope-missing-workspace-provenance")?,
        ),
        ValueKey::new("value-missing-workspace-provenance")?,
        ValueVersion::FIRST,
    );
    let cases = [
        CausalReference::Artifact {
            reference: missing_artifact,
        },
        CausalReference::WorkspaceValue {
            reference: missing_value,
        },
        CausalReference::RunInput {
            run: RunId::new("run-missing-run-input-provenance")?,
            key: ValueKey::new("input-missing-run-input-provenance")?,
        },
        CausalReference::Invocation {
            invocation: InvocationId::new("invocation-missing-provenance")?,
        },
    ];
    for (ordinal, causal) in cases.into_iter().enumerate() {
        let directory = TempDir::new()?;
        let bytes = format!("target-{ordinal}").into_bytes();
        let reference = milkdrift_workspace::ArtifactReference::new(
            ArtifactId::new(format!("artifact-provenance-target-{ordinal}"))?,
            ContentDigest::for_bytes(&bytes),
            MediaType::new("application/octet-stream")?,
            bytes.len() as u64,
        );
        let metadata = ArtifactMetadata::new(
            reference.clone(),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(causal, Vec::new())?,
        )?;
        let request = BeginArtifactPublication::new(
            ArtifactPublicationId::new(format!("publication-provenance-target-{ordinal}"))?,
            RunId::new(format!("run-provenance-target-{ordinal}"))?,
            metadata,
            WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
            WorkspaceUsage::EMPTY,
        )?;
        let store = RedbStore::open(directory.path())?;
        store.begin_publication(&request)?;
        store.write_chunk(request.publication(), 0, &bytes)?;
        assert!(store.commit_publication(request.publication()).is_err());
        assert!(!store.is_committed(&reference)?);
    }
    Ok(())
}

fn double_charge_request(
    publication: &BeginArtifactPublication,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let command = CommandId::new("command-double-charge")?;
    let receipt = CommandReceipt::new(
        command.clone(),
        publication.run().clone(),
        ActorRef::new("actor-integrity")?,
        RunSequence::ZERO,
        TimestampMillis::new(20),
        br#"{"schema_version":1,"type":"double_charge"}"#.to_vec(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new("event-double-charge")?,
        publication.run().clone(),
        RunSequence::FIRST,
        TimestampMillis::new(20),
        RunEventKind::ArtifactPublished {
            metadata: publication.metadata().clone(),
        },
    )?;
    let result = CommandResultDocument::new(
        command,
        publication.run().clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        RunSequence::FIRST,
        vec![event.event_id().clone()],
        BoundedJson::new(json!({"accepted": true}))?,
    )?;
    let resulting_usage = publication
        .budget()
        .admit_artifact(&publication.resulting_usage(), publication.metadata())?;
    Ok(AtomicRunCommitRequest::new(
        receipt,
        vec![event],
        Vec::new(),
        Some(WorkspaceAccounting {
            budget: publication.budget().clone(),
            expected_usage: publication.resulting_usage(),
            resulting_usage,
        }),
        vec![publication.metadata().reference().clone()],
        vec![publication.metadata().reference().clone()],
        None,
        result,
        RunIndexUpdate::new(
            Some(RunSummaryIndex {
                run: publication.run().clone(),
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

fn continuation_request(
    run: &RunId,
    sequence: u64,
) -> Result<AtomicRunCommitRequest, Box<dyn std::error::Error>> {
    let expected = RunSequence::new(sequence.checked_sub(1).ok_or("sequence underflow")?);
    let resulting = RunSequence::new(sequence);
    let command = CommandId::new(format!("command-continuity-{sequence}"))?;
    let receipt = CommandReceipt::new(
        command.clone(),
        run.clone(),
        ActorRef::new("actor-integrity")?,
        expected,
        TimestampMillis::new(10 + sequence),
        format!(r#"{{"schema_version":1,"type":"continuity-{sequence}"}}"#).into_bytes(),
    )?;
    let event = RunEventEnvelope::new(
        EventId::new(format!("event-continuity-{sequence}"))?,
        run.clone(),
        resulting,
        TimestampMillis::new(10 + sequence),
        RunEventKind::RunStarted,
    )?;
    let result = CommandResultDocument::new(
        command,
        run.clone(),
        receipt.fingerprint().clone(),
        CommandDisposition::Accepted,
        resulting,
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
                run: run.clone(),
                workflow: WorkflowId::new("workflow-integrity")?,
                revision: revision_id()?,
                state: IndexedRunState::Active,
                through_sequence: resulting,
                updated_at: TimestampMillis::new(10 + sequence),
            }),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    )?)
}

fn pair_key(first: &str, second: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut key = Vec::new();
    for component in [first, second] {
        key.extend_from_slice(&u32::try_from(component.len())?.to_be_bytes());
        key.extend_from_slice(component.as_bytes());
    }
    Ok(key)
}

fn event_key(run: &RunId, sequence: RunSequence) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut key = Vec::new();
    key.extend_from_slice(&u32::try_from(run.as_str().len())?.to_be_bytes());
    key.extend_from_slice(run.as_str().as_bytes());
    key.extend_from_slice(&sequence.get().to_be_bytes());
    Ok(key)
}

#[test]
fn scrub_detects_paired_interior_event_and_command_deletion()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let first = start_request("run-continuity-gap")?;
    let run = first.receipt().run().clone();
    let second = continuation_request(&run, 2)?;
    let third = continuation_request(&run, 3)?;
    {
        let store = RedbStore::open(directory.path())?;
        store.commit_command(&first)?;
        store.commit_command(&second)?;
        store.commit_command(&third)?;
    }
    let database = Database::open(directory.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    assert!(
        write
            .open_table(RUN_EVENTS)?
            .remove(event_key(&run, RunSequence::new(2))?.as_slice())?
            .is_some()
    );
    assert!(
        write
            .open_table(COMMAND_RESULTS)?
            .remove(pair_key(run.as_str(), "command-continuity-2")?.as_slice())?
            .is_some()
    );
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert!(exhaustive_integrity_failure_count(&store)? > 0);
    Ok(())
}

#[test]
fn paired_artifact_reference_loss_cannot_reopen_double_charging()
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
    {
        let mut ownership = write.open_table(RUN_ARTIFACT_OWNERSHIP)?;
        let keys = ownership
            .iter()?
            .map(|item| item.map(|(key, _)| key.value().to_vec()))
            .collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            let _ = ownership.remove(key.as_slice())?;
        }
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.is_referenced_by_run(request.run(), request.metadata().reference()));
    assert_corruption(store.commit_command(&double_charge_request(&request)?));
    assert!(exhaustive_integrity_failure_count(&store)? > 0);
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
        assert!(usage.remove(first.run().as_str())?.is_some());
    }
    write.commit()?;
    drop(database);

    let store = RedbStore::open(directory.path())?;
    assert_corruption(store.workspace_usage(first.run()));
    assert_eq!(
        store.health(TimestampMillis::new(20))?.status,
        StorageHealthStatus::Degraded
    );
    let second = publication_request(
        "artifact-accounted-second",
        "publication-accounted-second",
        first.run().as_str(),
        b"second",
        budget,
        first.resulting_usage(),
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
            let _ = metadata.remove(request.metadata().reference().artifact().as_str())?;
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
        assert_corruption(store.is_committed(request.metadata().reference()));
        assert!(exhaustive_integrity_failure_count(&store)? > 0);
        assert_eq!(
            store.health(TimestampMillis::new(30))?.status,
            StorageHealthStatus::Degraded
        );
    }

    for delete_primary in [true, false] {
        let directory = TempDir::new()?;
        let fixture = include_bytes!("../../../crates/blueprint/tests/fixtures/revision-v2.json");
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
    store.write_chunk(second.publication(), 0, b"shared-content")?;
    assert_corruption(store.commit_publication(second.publication()));
    assert_corruption(store.is_committed(first.metadata().reference()));
    assert!(!store.is_committed(second.metadata().reference())?);
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
        store.write_chunk(request.publication(), 0, b"write")?;
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
    store.write_chunk(second.publication(), 0, b"abcde")?;
    assert!(matches!(
        store.commit_publication(second.publication()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        })
    ));
    Ok(())
}

#[test]
fn snapshot_pointer_deletion_is_rejected_but_lowered_journal_head_is_corruption()
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
        let request = request.with_projection_checkpoint(snapshot.payload_checkpoint()?)?;
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
        if delete_pointer {
            assert!(matches!(
                store.latest_snapshot(request.receipt().run())?,
                SnapshotLoad::Rejected { snapshot: None, .. }
            ));
        } else {
            assert_corruption(store.latest_snapshot(request.receipt().run()));
            assert_corruption(store.put_snapshot(&snapshot));
        }
    }
    Ok(())
}

#[test]
fn missing_history_chain_checkpoint_or_head_is_corruption() -> Result<(), Box<dyn std::error::Error>>
{
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
        assert_corruption(store.history_digest(request.receipt().run(), RunSequence::FIRST));
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
