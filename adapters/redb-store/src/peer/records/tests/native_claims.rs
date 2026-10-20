//! Serving lifecycle and native-directory ownership change in the same store transaction.
use crate::{RedbStore, schema::PEER_EXECUTION_LOCATIONS};
use milkdrift_persistence::{
    NativePathClaim, NativePathClaimStore, NativePathRoot, PeerDispatchClaim, PeerEntryEvidence,
    PeerExecutionPhase, PeerExecutionRecord, WorkerId,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn claimed() -> TestResult<PeerExecutionRecord> {
    let mut record = super::serving::record()?;
    record.revision = 2;
    record.phase = PeerExecutionPhase::DispatchClaimed {
        claim: PeerDispatchClaim {
            worker: WorkerId::new("worker:one")?,
            generation: 2,
            claimed_at_unix_ms: 110,
            lease_expires_at_unix_ms: 1000,
        },
    };
    Ok(record)
}

fn save(store: &RedbStore, record: &PeerExecutionRecord) -> TestResult {
    let write = store.database().begin_write()?;
    write
        .open_table(PEER_EXECUTION_LOCATIONS)?
        .insert(record.execution.as_str(), 1)?;
    super::super::put_execution(&write, record)?;
    write.commit()?;
    Ok(())
}

fn claim(record: &PeerExecutionRecord, path: &std::path::Path) -> TestResult<NativePathClaim> {
    Ok(NativePathClaim {
        owner: record.managed_invocation()?.to_string(),
        roots: vec![NativePathRoot {
            requested: path.into(),
            physical: path.into(),
            identity: "physical-directory".into(),
            write: true,
        }],
    })
}

#[test]
fn current_serving_claim_is_required_before_replay_and_terminal_release_is_atomic() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let mut record = claimed()?;
    save(&store, &record)?;
    let native = claim(&record, &directory.path().join("project"))?;
    store.claim_serving_native_paths(&record, &native)?;
    store.claim_serving_native_paths(&record, &native)?;
    let mut stale = record.clone();
    if let PeerExecutionPhase::DispatchClaimed { claim } = &mut stale.phase {
        claim.generation += 1;
    }
    assert!(store.claim_serving_native_paths(&stale, &native).is_err());
    let mut changed = record.clone();
    changed.request.deadline_unix_ms -= 1;
    assert!(store.claim_serving_native_paths(&changed, &native).is_err());
    let mut unrelated = native.clone();
    unrelated.owner = "task:other".into();
    assert!(
        store
            .claim_serving_native_paths(&record, &unrelated)
            .is_err()
    );
    assert!(store.claim_native_paths(&unrelated).is_err());

    record.last_observation_sequence = 1;
    record.accounting.observations = 1;
    record.phase = PeerExecutionPhase::Terminal {
        sequence: 1,
        terminal_at_unix_ms: 150,
    };
    // A failed transition cannot release the path while leaving the original claim live.
    let write = store.database().begin_write()?;
    super::super::put_execution(&write, &record)?;
    drop(write);
    assert_eq!(
        store.native_path_claim(&native.owner)?,
        Some(native.clone())
    );
    save(&store, &record)?;
    assert!(store.native_path_claim(&native.owner)?.is_none());
    assert!(
        store
            .claim_serving_native_paths(&claimed()?, &native)
            .is_err()
    );
    assert!(store.native_path_claim(&native.owner)?.is_none());
    store.claim_native_paths(&unrelated)?;
    Ok(())
}

#[test]
fn pre_entry_requeue_releases_but_entered_and_uncertain_paths_remain_protected() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = RedbStore::open(directory.path())?;
    let mut record = claimed()?;
    save(&store, &record)?;
    let original = record.clone();
    let native = claim(&record, &directory.path().join("project"))?;
    store.claim_serving_native_paths(&record, &native)?;
    record.phase = PeerExecutionPhase::DispatchAvailable {
        available_at_unix_ms: 160,
    };
    save(&store, &record)?;
    assert!(store.native_path_claim(&native.owner)?.is_none());
    assert!(
        store
            .claim_serving_native_paths(&original, &native)
            .is_err()
    );
    record = claimed()?;
    record.revision = 3;
    if let PeerExecutionPhase::DispatchClaimed { claim } = &mut record.phase {
        claim.generation = 3;
    }
    save(&store, &record)?;
    assert!(
        store
            .claim_serving_native_paths(&original, &native)
            .is_err()
    );
    store.claim_serving_native_paths(&record, &native)?;
    let dispatch = record.phase.claim().ok_or("dispatch claim absent")?.clone();
    record.phase = PeerExecutionPhase::Entered {
        claim: dispatch.clone(),
        evidence: PeerEntryEvidence {
            worker: dispatch.worker,
            claim_generation: dispatch.generation,
            entered_at_unix_ms: 170,
            authority: record.authority.clone(),
        },
    };
    save(&store, &record)?;
    assert_eq!(
        store.native_path_claim(&native.owner)?,
        Some(native.clone())
    );
    record.phase = PeerExecutionPhase::Uncertain {
        uncertain_at_unix_ms: 180,
        reason: "terminal evidence lost".into(),
    };
    save(&store, &record)?;
    assert_eq!(
        store.native_path_claim(&native.owner)?,
        Some(native.clone())
    );
    drop(store);
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.native_path_claim(&native.owner)?, Some(native));
    Ok(())
}
