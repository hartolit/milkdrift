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
            reserved_bytes: 4096,
            reserved_receipts: 1,
        },
        command: key.map(|key| WorkflowCommandAcceptance {
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
    let mut stop = commit("workflow-a", 2, Some("stop-workflow-a"))?;
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
            supported: 18
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
