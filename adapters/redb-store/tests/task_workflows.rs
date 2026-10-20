//! Fresh task storage refuses stale/conflicting writes and protects settlement capacity.

use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    PersistenceError, TimestampMillis, WorkflowCommandAcceptance, WorkflowCommit,
    WorkflowCommitOutcome, WorkflowSnapshot, WorkflowStorageLimits, WorkflowStore,
};
use milkdrift_redb_store::{RedbStore, RedbStoreConfig};

struct FixedClock;
impl milkdrift_redb_store::StoreClock for FixedClock {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        Ok(TimestampMillis::new(10))
    }
}

fn config(root: &std::path::Path) -> RedbStoreConfig {
    RedbStoreConfig::new(root).with_clock(std::sync::Arc::new(FixedClock))
}

fn commit(
    id: &str,
    generation: u64,
    key: Option<&str>,
) -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    Ok(WorkflowCommit {
        project_guard: None,
        project_id: None,
        expected_generation: if generation == 1 {
            None
        } else {
            generation.checked_sub(1)
        },
        snapshot: WorkflowSnapshot {
            workflow: WorkflowId::new(id)?,
            generation,
            title: id.to_owned(),
            description: "Input-dependent task".to_owned(),
            owner_actor: "alice".to_owned(),
            updated_at: TimestampMillis::new(10),
            document: serde_json::json!({"state":"queued"}),
            entered_invocations: Vec::new(),
            managed_executions: Vec::new(),
            artifact_capacity: 0,
            reserved_bytes: 4096,
            reserved_receipts: 1,
        },
        command: key.map(|key| WorkflowCommandAcceptance {
            reserved_action: None,
            actor: "alice".to_owned(),
            key: key.to_owned(),
            canonical_request: br#"{"input":"one"}"#.to_vec(),
            result: serde_json::json!({"workflow":id,"aliases":{"first":"task-1"}}),
        }),
    })
}

#[test]
fn task_output_requires_durable_entry_proof_in_the_same_workflow()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_capability::InvocationId;
    use milkdrift_persistence::{ArtifactPublicationId, ArtifactStore, BeginArtifactPublication};
    use milkdrift_workspace::{
        ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
        ArtifactSensitivity, CausalReference, ContentDigest, MediaType, RunId, WorkspaceBudget,
        WorkspaceUsage,
    };
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    let invocation = InvocationId::new("entered-task")?;
    store.commit_workflow(commit("workflow-provenance", 1, Some("create-proof"))?)?;
    let mut other = commit("workflow-other", 1, None)?;
    other.snapshot.entered_invocations.push(invocation.clone());
    store.commit_workflow(other)?;
    let bytes = b"task report";
    let reference = ArtifactReference::new(
        ArtifactId::new("task-report")?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        bytes.len() as u64,
    );
    let request = BeginArtifactPublication::new(
        ArtifactPublicationId::new("task-report-publication")?,
        RunId::new("workflow-provenance")?,
        ArtifactMetadata::new(
            reference.clone(),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(
                CausalReference::Invocation {
                    invocation: invocation.clone(),
                },
                Vec::new(),
            )?,
        )?,
        WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&request)?;
    store.write_chunk(request.publication(), 0, bytes)?;
    assert!(matches!(
        store.commit_publication(request.publication()),
        Err(PersistenceError::NotFound {
            entity: "task_artifact_provenance_invocation",
            ..
        })
    ));
    assert!(!store.is_committed(&reference)?);
    let mut entry = commit("workflow-provenance", 2, None)?;
    entry.snapshot.entered_invocations.push(invocation.clone());
    store.commit_workflow(entry)?;
    store.commit_publication(request.publication())?;
    assert!(store.is_committed(&reference)?);
    assert!(
        store
            .commit_workflow(commit("workflow-provenance", 3, None)?)
            .is_err()
    );
    let mut duplicate = commit("workflow-provenance", 3, None)?;
    duplicate.snapshot.entered_invocations = vec![invocation.clone(), invocation];
    assert!(store.commit_workflow(duplicate).is_err());
    drop(store);
    let reopened = RedbStore::open_with_config(config(root.path()))?;
    assert!(reopened.is_committed(&reference)?);
    assert_eq!(
        reopened
            .workflow(&WorkflowId::new("workflow-provenance")?)?
            .ok_or("missing task workflow")?
            .entered_invocations
            .len(),
        1
    );
    Ok(())
}

#[test]
fn task_exact_receipt_cas_and_discovery_survive_reopen() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let request = commit("workflow-a", 1, Some("request-a"))?;
    let first = {
        let store = RedbStore::open_with_config(config(root.path()))?;
        let WorkflowCommitOutcome::Applied(Some(receipt)) =
            store.commit_workflow(request.clone())?
        else {
            return Err("expected acceptance".into());
        };
        receipt
    };
    let store = RedbStore::open_with_config(config(root.path()))?;
    assert_eq!(
        store.commit_workflow(request.clone())?,
        WorkflowCommitOutcome::Replay(first.clone())
    );
    assert_eq!(store.workflow_receipt("alice", "request-a")?, Some(first));
    assert!(store.workflow_receipt("bob", "request-a")?.is_none());
    let mut conflict = request;
    conflict
        .command
        .as_mut()
        .ok_or("missing command")?
        .canonical_request = br#"{"input":"two"}"#.to_vec();
    assert!(matches!(
        store.commit_workflow(conflict),
        Err(PersistenceError::WorkflowIdempotencyConflict { .. })
    ));
    store.commit_workflow(commit("workflow-a", 2, None)?)?;
    assert!(matches!(
        store.commit_workflow(commit("workflow-a", 2, None)?),
        Err(PersistenceError::WorkflowGenerationConflict { .. })
    ));
    store.commit_workflow(commit("workflow-b", 1, Some("request-b"))?)?;
    let page = store.workflows(None, 1)?;
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items.first().ok_or("missing workflow")?.generation, 2);
    let second = store.workflows(page.next.as_deref(), 1)?;
    assert_eq!(
        second
            .items
            .first()
            .ok_or("missing second workflow")?
            .workflow
            .as_str(),
        "workflow-b"
    );
    assert!(second.next.is_none());
    Ok(())
}

#[test]
fn receipt_exhaustion_still_allows_reserved_stop_and_settlement()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()).with_workflow_limits(
        WorkflowStorageLimits {
            history_bytes: 8192,
            receipt_bytes: 16384,
            receipt_entries: 2,
        },
    ))?;
    store.commit_workflow(commit("workflow-a", 1, Some("request-a"))?)?;
    assert!(matches!(
        store.commit_workflow(commit("workflow-b", 1, Some("request-b"))?),
        Err(PersistenceError::Bounds { .. })
    ));
    let ordinary = commit("workflow-a", 2, Some("another-ordinary"))?;
    assert!(store.commit_workflow(ordinary).is_err());
    let mut stop = commit("workflow-a", 2, Some("stop:workflow-a:task-a"))?;
    stop.command
        .as_mut()
        .ok_or("stop missing receipt")?
        .reserved_action = Some("stop:workflow-a:task-a".into());
    stop.snapshot.reserved_receipts = 0;
    stop.snapshot.document = serde_json::json!({"state":"stop_requested"});
    store.commit_workflow(stop)?;
    let mut settled = commit("workflow-a", 3, None)?;
    settled.snapshot.reserved_bytes = 0;
    settled.snapshot.reserved_receipts = 0;
    settled.snapshot.document =
        serde_json::json!({"state":"stopped","evidence":"process wait returned"});
    store.commit_workflow(settled)?;
    assert!(store.workflow_receipt("alice", "request-a")?.is_some());
    Ok(())
}

#[test]
fn old_schema_refusal_preserves_database_bytes_and_modification_time()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("milkdrift.redb");
    let database = redb::Database::create(&path)?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(redb::TableDefinition::<&str, u64>::new(
            "milkdrift.v1.metadata",
        ))?;
        metadata.insert("storage_schema_version", 17)?;
        metadata.insert("internal_document_format_version", 20)?;
    }
    write.commit()?;
    drop(database);
    let before = std::fs::read(&path)?;
    let modified = std::fs::metadata(&path)?.modified()?;
    assert!(matches!(
        RedbStore::open(root.path()),
        Err(PersistenceError::UnsupportedVersion {
            document: "storage",
            found: 17,
            supported: 21
        })
    ));
    assert_eq!(std::fs::read(&path)?, before);
    assert_eq!(std::fs::metadata(&path)?.modified()?, modified);
    assert!(!root.path().join("artifacts").exists());
    Ok(())
}

#[cfg(feature = "test-admin")]
#[test]
fn lost_commit_response_recovers_the_exact_acceptance_without_another_transition()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_redb_store::FaultPoint;
    #[path = "support/fault.rs"]
    mod fault;
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let request = commit("accepted-work", 1, Some("lost-response"))?;
        let store = RedbStore::open_with_config(
            config(root.path())
                .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point))),
        )?;
        assert!(store.commit_workflow(request.clone()).is_err());
        drop(store);
        let reopened = RedbStore::open_with_config(config(root.path()))?;
        let accepted = reopened.workflow_receipt("alice", "lost-response")?;
        assert_eq!(accepted.is_some(), point == FaultPoint::AfterCommandCommit);
        match (point, reopened.commit_workflow(request)?) {
            (FaultPoint::BeforeCommandCommit, WorkflowCommitOutcome::Applied(Some(_)))
            | (FaultPoint::AfterCommandCommit, WorkflowCommitOutcome::Replay(_)) => {}
            _ => return Err("fault boundary changed acceptance/replay disposition".into()),
        }
        assert_eq!(
            reopened
                .workflow(&WorkflowId::new("accepted-work")?)?
                .ok_or("accepted workflow missing")?
                .generation,
            1
        );
    }
    Ok(())
}

fn output_request(
    store: &RedbStore,
    identity: &str,
    owner: &str,
    bytes: &[u8],
    invocation: Option<&str>,
    output_limit: Option<u64>,
) -> Result<milkdrift_persistence::BeginArtifactPublication, Box<dyn std::error::Error>> {
    use milkdrift_persistence::ArtifactStore;
    use milkdrift_workspace::*;
    let run = RunId::new(owner)?;
    let producer = match invocation {
        Some(invocation) => CausalReference::Invocation {
            invocation: milkdrift_capability::InvocationId::new(invocation)?,
        },
        None => CausalReference::External {
            source: CausalId::new("test-input")?,
        },
    };
    let metadata = ArtifactMetadata::new(
        ArtifactReference::new(
            ArtifactId::new(identity)?,
            ContentDigest::for_bytes(bytes),
            MediaType::new("text/plain")?,
            bytes.len() as u64,
        ),
        ArtifactSensitivity::Public,
        ArtifactRetention::WhileReferenced,
        ArtifactProvenance::new(producer, Vec::new())?,
    )?;
    let request = milkdrift_persistence::BeginArtifactPublication::new(
        milkdrift_persistence::ArtifactPublicationId::new(format!("publication-{identity}"))?,
        run.clone(),
        metadata,
        WorkspaceBudget::new(0, 0, 0, 64, 60_000, 60_000)?,
        store.artifact_usage(&ArtifactOwner::from(&run))?,
    )?;
    Ok(match output_limit {
        Some(limit) => request.with_task_output_limit(limit)?,
        None => request,
    })
}

#[test]
fn artifact_uploads_share_capacity_with_protected_work_and_pending_writes_survive_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_persistence::ArtifactStore;
    let root = tempfile::tempdir()?;
    let settings = || {
        config(root.path()).with_workflow_limits(WorkflowStorageLimits {
            history_bytes: 60_000,
            receipt_bytes: 50_000,
            receipt_entries: 8,
        })
    };
    let store = RedbStore::open_with_config(settings())?;
    let mut accepted = commit("capacity-workflow", 1, Some("capacity-create"))?;
    accepted.snapshot.artifact_capacity = 30_000;
    accepted
        .snapshot
        .entered_invocations
        .push(milkdrift_capability::InvocationId::new(
            "capacity-invocation",
        )?);
    store.commit_workflow(accepted.clone())?;
    let reserve = store.workflow_storage_usage()?;
    assert_eq!(reserve.artifact_reserved_bytes, 30_000);
    let excessive = output_request(
        &store,
        "excess-input",
        "input-owner",
        &vec![b'x'; 30_000],
        None,
        None,
    )?;
    assert!(matches!(
        store.begin_publication(&excessive),
        Err(PersistenceError::Bounds { .. })
    ));
    assert_eq!(store.workflow_storage_usage()?, reserve);
    let input = output_request(
        &store,
        "pending-input",
        "input-owner",
        &vec![b'i'; 20_000],
        None,
        None,
    )?;
    store.begin_publication(&input)?;
    store.write_chunk(input.publication(), 0, b"prefix")?;
    assert_eq!(
        store.workflow_storage_usage()?.pending_artifact_bytes,
        20_000
    );
    drop(store);
    let store = RedbStore::open_with_config(settings())?;
    assert_eq!(
        store.workflow_storage_usage()?.pending_artifact_bytes,
        20_000
    );
    let report_bytes = vec![b'r'; 16_000];
    let report = output_request(
        &store,
        "bounded-report",
        "capacity-workflow",
        &report_bytes,
        Some("capacity-invocation"),
        Some(22_000),
    )?;
    store.begin_publication(&report)?;
    assert_eq!(
        store.workflow_storage_usage()?.artifact_reserved_bytes,
        14_000
    );
    store.abort_publication(report.publication())?;
    assert_eq!(
        store.workflow_storage_usage()?.artifact_reserved_bytes,
        30_000
    );
    let report = output_request(
        &store,
        "bounded-report-retry",
        "capacity-workflow",
        &report_bytes,
        Some("capacity-invocation"),
        Some(22_000),
    )?;
    store.begin_publication(&report)?;
    store.write_chunk(report.publication(), 0, &report_bytes)?;
    store.commit_publication(report.publication())?;
    let after_output = store.workflow_storage_usage()?;
    assert_eq!(after_output.artifact_content_bytes, 16_000);
    assert_eq!(after_output.pending_artifact_bytes, 20_000);
    assert_eq!(after_output.artifact_reserved_bytes, 14_000);
    let too_much = output_request(
        &store,
        "second-output",
        "capacity-workflow",
        &vec![b's'; 10_000],
        Some("capacity-invocation"),
        Some(22_000),
    )?;
    assert!(matches!(
        store.begin_publication(&too_much),
        Err(PersistenceError::Bounds { .. })
    ));
    store.abort_publication(input.publication())?;
    assert_eq!(store.workflow_storage_usage()?.pending_artifact_bytes, 0);
    let mut settled = accepted;
    settled.expected_generation = Some(1);
    settled.snapshot.generation = 2;
    settled.command = None;
    settled.snapshot.artifact_capacity = 0;
    settled.snapshot.reserved_bytes = 0;
    settled.snapshot.reserved_receipts = 0;
    store.commit_workflow(settled)?;
    let final_usage = store.workflow_storage_usage()?;
    assert_eq!(final_usage.artifact_reserved_bytes, 0);
    assert_eq!(final_usage.artifact_content_bytes, 16_000);
    assert!(store.is_committed(report.metadata().reference())?);
    drop(store);
    assert_eq!(
        RedbStore::open_with_config(settings())?.workflow_storage_usage()?,
        final_usage
    );
    Ok(())
}

#[test]
fn pending_outputs_remain_charged_after_workflow_releases_its_reservation()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_persistence::ArtifactStore;
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    let mut accepted = commit("pending-workflow", 1, None)?;
    accepted.snapshot.artifact_capacity = 1000;
    accepted
        .snapshot
        .entered_invocations
        .push(milkdrift_capability::InvocationId::new(
            "pending-invocation",
        )?);
    store.commit_workflow(accepted.clone())?;
    let report = output_request(
        &store,
        "pending-report",
        "pending-workflow",
        &[b'p'; 500],
        Some("pending-invocation"),
        Some(1000),
    )?;
    store.begin_publication(&report)?;
    accepted.expected_generation = Some(1);
    accepted.snapshot.generation = 2;
    accepted.snapshot.artifact_capacity = 0;
    store.commit_workflow(accepted)?;
    let pending = store.workflow_storage_usage()?;
    assert_eq!(pending.artifact_reserved_bytes, 0);
    assert_eq!(pending.pending_artifact_bytes, 500);
    store.abort_publication(report.publication())?;
    let released = store.workflow_storage_usage()?;
    assert_eq!(released.artifact_reserved_bytes, 0);
    assert_eq!(released.pending_artifact_bytes, 0);
    Ok(())
}

#[test]
fn deleted_task_accounting_refuses_reopen_before_a_new_admission()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    store.commit_workflow(commit("accounted", 1, Some("accounted-create"))?)?;
    drop(store);
    let database = redb::Database::open(root.path().join("milkdrift.redb"))?;
    let write = database.begin_write()?;
    write
        .open_table(redb::TableDefinition::<&str, &[u8]>::new(
            "milkdrift.v1.tasks.accounting",
        ))?
        .remove("global")?;
    write.commit()?;
    drop(database);
    assert!(RedbStore::open_with_config(config(root.path())).is_err());
    Ok(())
}

#[test]
fn mandatory_stop_reservation_has_one_durable_actor_owner() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    let mut first = commit("workflow-stops", 1, Some("create-stops"))?;
    first.snapshot.reserved_receipts = 2;
    store.commit_workflow(first)?;
    let mut stop = commit("workflow-stops", 2, Some("stop:workflow-stops:first"))?;
    stop.command
        .as_mut()
        .ok_or("receipt missing")?
        .reserved_action = Some("stop:workflow-stops:first".into());
    let accepted = store.commit_workflow(stop.clone())?;
    assert!(matches!(accepted, WorkflowCommitOutcome::Applied(Some(_))));
    drop(store);
    let store = RedbStore::open_with_config(config(root.path()))?;
    assert!(matches!(
        store.commit_workflow(stop.clone())?,
        WorkflowCommitOutcome::Replay(_)
    ));
    let before = store.workflow_storage_usage()?;
    let mut duplicate = stop;
    duplicate.expected_generation = Some(2);
    duplicate.snapshot.generation = 3;
    duplicate.snapshot.reserved_receipts = 0;
    duplicate.command.as_mut().ok_or("receipt missing")?.actor = "bob".into();
    assert!(store.commit_workflow(duplicate.clone()).is_err());
    assert_eq!(store.workflow_storage_usage()?, before);
    // The second actor may retain an ordinary receipt, but cannot consume the other task's slot.
    duplicate.snapshot.reserved_receipts = 1;
    duplicate
        .command
        .as_mut()
        .ok_or("receipt missing")?
        .reserved_action = None;
    store.commit_workflow(duplicate)?;
    assert_eq!(store.workflow_storage_usage()?.reserved_receipts, 1);
    assert!(
        store
            .workflow_receipt("bob", "stop:workflow-stops:first")?
            .is_some()
    );
    Ok(())
}

#[test]
fn workflow_discovery_bounds_page_bytes_without_losing_large_records()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    for (id, size) in [
        ("page-a", 2 * 1024 * 1024),
        ("page-b", 2 * 1024 * 1024),
        ("page-c", 5 * 1024 * 1024),
    ] {
        let mut value = commit(id, 1, None)?;
        value.snapshot.document = serde_json::json!({"large_retained_document": "x".repeat(size)});
        store.commit_workflow(value)?;
    }
    let first = store.workflows(None, 200)?;
    assert_eq!(first.items.len(), 1);
    assert_eq!(
        first
            .items
            .first()
            .ok_or("first row missing")?
            .workflow
            .as_str(),
        "page-a"
    );
    assert_eq!(first.next.as_deref(), Some("page-a"));
    let second = store.workflows(first.next.as_deref(), 200)?;
    assert_eq!(second.items.len(), 1);
    assert_eq!(
        second
            .items
            .first()
            .ok_or("second row missing")?
            .workflow
            .as_str(),
        "page-b"
    );
    let last = store.workflows(second.next.as_deref(), 200)?;
    assert_eq!(last.items.len(), 1);
    assert_eq!(
        last.items
            .first()
            .ok_or("last row missing")?
            .workflow
            .as_str(),
        "page-c"
    );
    assert!(last.next.is_none());
    Ok(())
}
