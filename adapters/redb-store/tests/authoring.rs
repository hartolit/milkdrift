//! Saved unfinished work has bounded replacement, independent positions and atomic admission.

use std::{collections::BTreeMap, sync::Arc};

use milkdrift_blueprint::WorkflowId;
use milkdrift_persistence::{
    MAX_WORKFLOW_AUTHORING_BYTES, PersistenceError, TimestampMillis, WorkflowAuthoringCommit,
    WorkflowCommandAcceptance, WorkflowCommit, WorkflowCommitOutcome, WorkflowPosition,
    WorkflowPositionsCommit, WorkflowSnapshot, WorkflowStorageLimits, WorkflowStore,
};
use milkdrift_redb_store::{FaultPoint, RedbStore, RedbStoreConfig, StoreClock};

#[path = "support/fault.rs"]
mod fault;

struct Clock;
impl StoreClock for Clock {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        Ok(TimestampMillis::new(10))
    }
}

fn config(root: &std::path::Path) -> RedbStoreConfig {
    RedbStoreConfig::new(root).with_clock(Arc::new(Clock))
}

fn workflow() -> Result<WorkflowId, milkdrift_blueprint::IdentityError> {
    WorkflowId::new("workflow:canvas")
}

fn commit(generation: u64) -> Result<WorkflowCommit, Box<dyn std::error::Error>> {
    Ok(WorkflowCommit {
        authoring: None,
        project_id: None,
        project_guard: None,
        expected_generation: generation.checked_sub(1).filter(|value| *value != 0),
        snapshot: WorkflowSnapshot {
            workflow: workflow()?,
            generation,
            title: "Canvas".into(),
            description: "".into(),
            owner_actor: "alice".into(),
            updated_at: TimestampMillis::new(10),
            document: serde_json::json!({"tasks":[]}),
            entered_invocations: Vec::new(),
            managed_executions: Vec::new(),
            artifact_capacity: 0,
            reserved_bytes: 4096,
            reserved_receipts: 1,
        },
        command: Some(WorkflowCommandAcceptance {
            actor: "alice".into(),
            key: format!("command-{generation}"),
            reserved_action: None,
            canonical_request: serde_json::to_vec(&serde_json::json!({"generation":generation}))?,
            result: serde_json::json!({"workflow":"workflow:canvas","generation":generation}),
        }),
    })
}

fn checkpoint(generation: u64, text: &str) -> WorkflowAuthoringCommit {
    WorkflowAuthoringCommit {
        expected_generation: generation,
        document: serde_json::json!({"tasks":[{"id":"draft:a","instructions":text}]}),
        protected_sources: Vec::new(),
    }
}

#[test]
fn checkpoints_preserve_accepted_work_and_receipts_and_positions_are_independent()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    assert!(store.workflow_authoring(&workflow()?)?.is_none());
    assert!(
        store
            .checkpoint_workflow_authoring(&workflow()?, checkpoint(0, ""))
            .is_err()
    );
    store.commit_workflow(commit(1)?)?;
    let accepted = store.workflow(&workflow()?)?;
    let receipt = store.workflow_receipt("alice", "command-1")?;
    let usage = store.workflow_storage_usage()?;
    let empty = store
        .workflow_authoring(&workflow()?)?
        .ok_or("missing empty workflow")?;
    assert_eq!(empty.generation, 0);
    assert!(empty.document.is_null());
    let first = store.checkpoint_workflow_authoring(&workflow()?, checkpoint(0, ""))?;
    let position = store.checkpoint_workflow_positions(
        &workflow()?,
        WorkflowPositionsCommit {
            expected_generation: 0,
            positions: BTreeMap::from([("draft:a".into(), WorkflowPosition { x: -10.5, y: 42.0 })]),
        },
    )?;
    let second = store.checkpoint_workflow_authoring(&workflow()?, checkpoint(1, "Research"))?;
    assert_ne!(first.digest, second.digest);
    assert!(matches!(
        store.checkpoint_workflow_authoring(&workflow()?, checkpoint(1, "Research")),
        Err(PersistenceError::AuthoringGenerationConflict {
            expected: 1,
            actual: 2,
            ..
        })
    ));
    assert!(matches!(
        store.checkpoint_workflow_positions(
            &workflow()?,
            WorkflowPositionsCommit {
                expected_generation: 0,
                positions: BTreeMap::new(),
            }
        ),
        Err(PersistenceError::PositionsGenerationConflict {
            expected: 0,
            actual: 1,
            ..
        })
    ));
    assert_eq!(store.workflow(&workflow()?)?, accepted);
    assert_eq!(store.workflow_receipt("alice", "command-1")?, receipt);
    let after = store.workflow_storage_usage()?;
    assert_eq!(after.receipt_entries, usage.receipt_entries);
    assert_eq!(after.receipt_bytes, usage.receipt_bytes);
    assert_eq!(after.reserved_bytes, usage.reserved_bytes);
    assert_eq!(after.authoring_bytes, after.authoring_replacement_bytes);
    drop(store);
    let store = RedbStore::open_with_config(config(root.path()))?;
    assert_eq!(store.workflow_authoring(&workflow()?)?, Some(second));
    assert_eq!(store.workflow_positions(&workflow()?)?, Some(position));
    Ok(())
}

#[test]
fn lost_checkpoint_reply_is_recovered_from_latest_content_without_a_receipt()
-> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeAuthoringCommit,
        FaultPoint::AfterAuthoringCommit,
    ] {
        let root = tempfile::tempdir()?;
        let store = RedbStore::open_with_config(
            config(root.path()).with_fault_injector(Arc::new(fault::FailOnce::new(point))),
        )?;
        store.commit_workflow(commit(1)?)?;
        let request = checkpoint(0, "Retain my unfinished thought");
        assert!(
            store
                .checkpoint_workflow_authoring(&workflow()?, request.clone())
                .is_err()
        );
        drop(store);
        let store = RedbStore::open_with_config(config(root.path()))?;
        let current = store
            .workflow_authoring(&workflow()?)?
            .ok_or("missing workflow")?;
        if point == FaultPoint::BeforeAuthoringCommit {
            assert_eq!(current.generation, 0);
            store.checkpoint_workflow_authoring(&workflow()?, request)?;
        } else {
            assert_eq!(current.generation, 1);
            assert_eq!(current.document, request.document);
            assert!(matches!(
                store.checkpoint_workflow_authoring(&workflow()?, request),
                Err(PersistenceError::AuthoringGenerationConflict { .. })
            ));
        }
        assert_eq!(store.workflow_storage_usage()?.receipt_entries, 1);
    }
    Ok(())
}

#[test]
fn run_mapping_and_accepted_work_commit_together_and_replay_ignores_later_saves()
-> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeCommandCommit,
        FaultPoint::AfterCommandCommit,
    ] {
        let root = tempfile::tempdir()?;
        let store = RedbStore::open_with_config(config(root.path()))?;
        store.commit_workflow(commit(1)?)?;
        store.checkpoint_workflow_authoring(&workflow()?, checkpoint(0, "Research"))?;
        drop(store);
        let store = RedbStore::open_with_config(
            config(root.path()).with_fault_injector(Arc::new(fault::FailOnce::new(point))),
        )?;
        let mut run = commit(2)?;
        run.snapshot.document =
            serde_json::json!({"tasks":[{"id":"task:accepted","instructions":"Research"}]});
        run.authoring = Some(WorkflowAuthoringCommit {
            expected_generation: 1,
            document: serde_json::json!({"tasks":[],"accepted":{"draft:a":"task:accepted"}}),
            protected_sources: Vec::new(),
        });
        assert!(store.commit_workflow(run.clone()).is_err());
        let current = store
            .workflow_authoring(&workflow()?)?
            .ok_or("missing workflow")?;
        assert_eq!(
            current.generation,
            if point == FaultPoint::BeforeCommandCommit {
                1
            } else {
                2
            }
        );
        assert_eq!(
            store
                .workflow(&workflow()?)?
                .ok_or("missing workflow")?
                .generation,
            current.generation
        );
        let outcome = store.commit_workflow(run.clone())?;
        assert!(matches!(
            outcome,
            WorkflowCommitOutcome::Applied(_) | WorkflowCommitOutcome::Replay(_)
        ));
        store.checkpoint_workflow_authoring(&workflow()?, checkpoint(2, "Next thought"))?;
        let later = store.workflow_authoring(&workflow()?)?;
        assert!(matches!(
            store.commit_workflow(run)?,
            WorkflowCommitOutcome::Replay(_)
        ));
        assert_eq!(store.workflow_authoring(&workflow()?)?, later);
        let mut stale = commit(3)?;
        stale.authoring = Some(checkpoint(2, "Stale mapping"));
        assert!(matches!(
            store.commit_workflow(stale),
            Err(PersistenceError::AuthoringGenerationConflict { .. })
        ));
        assert_eq!(
            store
                .workflow(&workflow()?)?
                .ok_or("missing workflow")?
                .generation,
            2
        );
        assert!(store.workflow_receipt("alice", "command-3")?.is_none());
    }
    Ok(())
}

#[test]
fn replacement_releases_old_bytes_and_cannot_spend_accepted_settlement_reserves()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    store.commit_workflow(commit(1)?)?;
    store.checkpoint_workflow_authoring(&workflow()?, checkpoint(0, &"a".repeat(4000)))?;
    let before = store.workflow_storage_usage()?;
    let limit = before.history_bytes + before.authoring_replacement_bytes + before.reserved_bytes;
    drop(store);
    let store = RedbStore::open_with_config(config(root.path()).with_workflow_limits(
        WorkflowStorageLimits {
            history_bytes: limit,
            ..WorkflowStorageLimits::default()
        },
    ))?;
    // Same-sized replacement fits its reserved room even at the configured limit.
    store.checkpoint_workflow_authoring(&workflow()?, checkpoint(1, &"b".repeat(4000)))?;
    assert!(matches!(
        store.checkpoint_workflow_authoring(&workflow()?, checkpoint(2, &"c".repeat(5000))),
        Err(PersistenceError::Bounds { .. })
    ));
    let after = store.workflow_storage_usage()?;
    assert_eq!(after, before);
    let mut settlement = commit(2)?;
    settlement.command = None;
    settlement.snapshot.document = serde_json::json!({"tasks":[],"settled":"x".repeat(1000)});
    settlement.snapshot.reserved_bytes = 0;
    settlement.snapshot.reserved_receipts = 0;
    store.commit_workflow(settlement)?;
    store.checkpoint_workflow_authoring(&workflow()?, checkpoint(2, ""))?;
    let smaller = store.workflow_storage_usage()?;
    assert!(smaller.authoring_bytes < after.authoring_bytes);
    assert!(smaller.authoring_replacement_bytes < after.authoring_replacement_bytes);
    Ok(())
}

#[test]
fn oversize_content_invalid_positions_and_missing_sources_leave_latest_unchanged()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    store.commit_workflow(commit(1)?)?;
    let first = store.checkpoint_workflow_authoring(&workflow()?, checkpoint(0, "Keep"))?;
    let usage = store.workflow_storage_usage()?;
    assert!(
        store
            .checkpoint_workflow_authoring(
                &workflow()?,
                checkpoint(1, &"a".repeat(MAX_WORKFLOW_AUTHORING_BYTES))
            )
            .is_err()
    );
    let mut missing = checkpoint(1, "Replace");
    missing.protected_sources.push("artifact:missing".into());
    assert!(matches!(
        store.checkpoint_workflow_authoring(&workflow()?, missing),
        Err(PersistenceError::NotFound {
            entity: "authoring source artifact",
            ..
        })
    ));
    assert!(
        store
            .checkpoint_workflow_positions(
                &workflow()?,
                WorkflowPositionsCommit {
                    expected_generation: 0,
                    positions: BTreeMap::from([(
                        "draft:a".into(),
                        WorkflowPosition {
                            x: f64::NAN,
                            y: 0.0
                        }
                    )]),
                }
            )
            .is_err()
    );
    let too_many = (0..601)
        .map(|index| (format!("card:{index}"), WorkflowPosition { x: 0.0, y: 0.0 }))
        .collect();
    assert!(
        store
            .checkpoint_workflow_positions(
                &workflow()?,
                WorkflowPositionsCommit {
                    expected_generation: 0,
                    positions: too_many
                }
            )
            .is_err()
    );
    assert_eq!(store.workflow_authoring(&workflow()?)?, Some(first));
    assert_eq!(store.workflow_storage_usage()?, usage);
    Ok(())
}

#[test]
fn removing_saved_source_protection_does_not_release_accepted_artifact_ownership()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_capability::InvocationId;
    use milkdrift_persistence::{
        ArtifactPublicationId, ArtifactStore, BeginArtifactPublication, OrphanCleanupRequest,
        PageSize,
    };
    use milkdrift_workspace::{
        ArtifactId, ArtifactMetadata, ArtifactProvenance, ArtifactReference, ArtifactRetention,
        ArtifactSensitivity, CausalReference, ContentDigest, MediaType, RunId, WorkspaceBudget,
        WorkspaceUsage,
    };
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    let invocation = InvocationId::new("source-producer")?;
    let mut creation = commit(1)?;
    creation
        .snapshot
        .entered_invocations
        .push(invocation.clone());
    store.commit_workflow(creation)?;
    let bytes = b"retained source";
    let reference = ArtifactReference::new(
        ArtifactId::new("artifact:source")?,
        ContentDigest::for_bytes(bytes),
        MediaType::new("text/plain")?,
        u64::try_from(bytes.len())?,
    );
    let publication = BeginArtifactPublication::new(
        ArtifactPublicationId::new("publication:source")?,
        RunId::new(workflow()?.as_str())?,
        ArtifactMetadata::new(
            reference.clone(),
            ArtifactSensitivity::Public,
            ArtifactRetention::WhileReferenced,
            ArtifactProvenance::new(CausalReference::Invocation { invocation }, Vec::new())?,
        )?,
        WorkspaceBudget::new(0, 0, 0, 1, 64, 64)?,
        WorkspaceUsage::EMPTY,
    )?;
    store.begin_publication(&publication)?;
    store.write_chunk(publication.publication(), 0, bytes)?;
    store.commit_publication(publication.publication())?;
    let mut saved = checkpoint(0, "Use this source later");
    saved
        .protected_sources
        .push(reference.artifact().to_string());
    let saved = store.checkpoint_workflow_authoring(&workflow()?, saved)?;
    assert_eq!(
        saved.protected_sources,
        vec![reference.artifact().to_string()]
    );
    drop(store);
    let store = RedbStore::open_with_config(config(root.path()))?;
    assert_eq!(store.workflow_authoring(&workflow()?)?, Some(saved));
    store
        .checkpoint_workflow_authoring(&workflow()?, checkpoint(1, "Source removed from canvas"))?;
    let current = store
        .workflow_authoring(&workflow()?)?
        .ok_or("missing workflow")?;
    assert!(current.protected_sources.is_empty());
    let cleanup = store.cleanup_orphans(OrphanCleanupRequest {
        observed_at: TimestampMillis::new(10),
        created_before: TimestampMillis::new(10),
        limit: PageSize::new(100)?,
        cursor: None,
    })?;
    assert_eq!(cleanup.temporary_publications_removed, 0);
    assert!(store.is_committed(&reference)?);
    assert!(store.is_referenced_by_run(&RunId::new(workflow()?.as_str())?, &reference)?);
    Ok(())
}

#[test]
fn bounded_integrity_pages_include_authoring_and_positions_without_losing_cursors()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_persistence::{
        IntegrityScanFamily, IntegrityScanRequest, PageSize, StorageAdmin,
    };
    let root = tempfile::tempdir()?;
    let store = RedbStore::open_with_config(config(root.path()))?;
    store.commit_workflow(commit(1)?)?;
    store.checkpoint_workflow_authoring(&workflow()?, checkpoint(0, "Saved"))?;
    store.checkpoint_workflow_positions(
        &workflow()?,
        WorkflowPositionsCommit {
            expected_generation: 0,
            positions: BTreeMap::from([("draft:a".into(), WorkflowPosition { x: 1.0, y: 2.0 })]),
        },
    )?;
    let complete = store.scan_integrity(IntegrityScanRequest {
        limit: PageSize::new(100)?,
        verify_artifact_content: false,
        cursor: None,
    })?;
    assert!(complete.failures.is_empty());
    assert!(complete.next_cursor.is_none());
    let mut cursor = None;
    let mut checked = 0u64;
    let mut authoring = false;
    let mut positions = false;
    for _ in 0..100 {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(1)?,
            verify_artifact_content: false,
            cursor,
        })?;
        assert!(page.failures.is_empty());
        assert!(page.documents_checked <= 1);
        checked = checked.saturating_add(page.documents_checked);
        let Some(next) = page.next_cursor else {
            assert_eq!(checked, complete.documents_checked);
            assert!(authoring && positions);
            return Ok(());
        };
        if next.family() == IntegrityScanFamily::Indexes {
            authoring |= next.after_key().get(33) == Some(&5);
            positions |= next.after_key().get(33) == Some(&6);
        }
        cursor = Some(next);
    }
    Err("integrity scan did not finish within its record bound".into())
}
