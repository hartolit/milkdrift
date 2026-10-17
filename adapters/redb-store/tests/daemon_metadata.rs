//! Durable shared names preserve exact actor-scoped recovery and finite receipt capacity.
use milkdrift_authority::ActorRef;
use milkdrift_capability::PeerId;
use milkdrift_persistence::{
    DaemonDisplayName, DaemonMetadataStore, DaemonRename, DaemonRenameOutcome,
    MAX_DAEMON_RENAME_RECEIPTS, PersistenceError, StorageFailureClass,
};
use milkdrift_redb_store::RedbStore;

fn rename(
    actor: &str,
    key: &str,
    generation: u64,
    name: &str,
) -> Result<DaemonRename, Box<dyn std::error::Error>> {
    Ok(DaemonRename {
        installation: PeerId::new("host:studio-a")?,
        actor: ActorRef::new(actor)?,
        key: key.to_owned(),
        expected_generation: generation,
        display_name: DaemonDisplayName::new(name)?,
        canonical_request: serde_json::to_vec(
            &serde_json::json!({"request_id":key,"expected_generation":generation,"name":name}),
        )?,
    })
}

#[test]
fn initialization_and_reopen_preserve_accepted_name_and_stable_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let host = PeerId::new("host:studio-a")?;
    let store = RedbStore::open(root.path())?;
    assert!(store.daemon_metadata(&host)?.is_none());
    let initial = store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
    assert_eq!(initial.generation, 1);
    assert_eq!(initial.display_name.as_str(), "Milkdrift");
    assert_eq!(
        store.initialize_daemon_metadata(&host, &DaemonDisplayName::new("Ignored")?)?,
        initial
    );
    let command = rename("alice", "first", 1, "  Shared studio  ")?;
    let DaemonRenameOutcome::Applied(receipt) = store.rename_daemon(command.clone())? else {
        return Err("new rename was replayed".into());
    };
    assert_eq!(receipt.result.display_name.as_str(), "Shared studio");
    assert_eq!(receipt.result.generation, 2);
    drop(store);
    let store = RedbStore::open(root.path())?;
    assert_eq!(
        store.initialize_daemon_metadata(
            &host,
            &DaemonDisplayName::new("Old configuration seed")?
        )?,
        receipt.result
    );
    assert_eq!(
        store.rename_daemon(command)?,
        DaemonRenameOutcome::Replay(receipt.clone())
    );
    assert_eq!(
        store.daemon_rename_receipt(&host, &ActorRef::new("alice")?, "first")?,
        Some(receipt)
    );
    let other = PeerId::new("host:studio-b")?;
    assert!(
        store
            .initialize_daemon_metadata(&other, &DaemonDisplayName::default())
            .is_err()
    );
    assert!(store.daemon_metadata(&other).is_err());
    assert!(
        store
            .daemon_rename_receipt(&other, &ActorRef::new("alice")?, "first")
            .is_err()
    );
    Ok(())
}

#[test]
fn replay_precedes_generation_but_raw_request_changes_and_other_actors_do_not_replay()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let host = PeerId::new("host:studio-a")?;
    store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
    let original = rename("alice", "shared-key", 1, " Studio ")?;
    let DaemonRenameOutcome::Applied(first) = store.rename_daemon(original.clone())? else {
        return Err("missing acceptance".into());
    };
    assert!(
        store
            .daemon_rename_receipt(&host, &ActorRef::new("bob")?, "shared-key")?
            .is_none()
    );
    store.rename_daemon(rename("bob", "shared-key", 2, "Same key, another actor")?)?;
    assert_eq!(
        store.rename_daemon(original)?,
        DaemonRenameOutcome::Replay(first)
    );
    assert!(matches!(
        store.rename_daemon(rename("alice", "shared-key", 1, "Studio")?),
        Err(PersistenceError::DaemonIdempotencyConflict { .. })
    ));
    assert!(matches!(
        store.rename_daemon(rename("alice", "new-key", 1, "Stale draft")?),
        Err(PersistenceError::DaemonGenerationConflict {
            expected: 1,
            actual: 3
        })
    ));
    assert_eq!(
        store
            .daemon_metadata(&host)?
            .ok_or("missing metadata")?
            .display_name
            .as_str(),
        "Same key, another actor"
    );
    Ok(())
}

#[test]
fn canonical_request_and_result_share_the_receipt_byte_limit()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = RedbStore::open(root.path())?;
    let host = PeerId::new("host:studio-a")?;
    store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
    let mut oversized = rename("alice", "large", 1, "New")?;
    // The request alone fits exactly; its required saved result makes the pair too large.
    let overhead = serde_json::to_vec(&serde_json::json!({"padding":""}))?.len();
    oversized.canonical_request = serde_json::to_vec(
        &serde_json::json!({"padding":"x".repeat((16_usize * 1024).checked_sub(overhead).ok_or("bad overhead")?)}),
    )?;
    assert_eq!(oversized.canonical_request.len(), 16 * 1024);
    assert!(matches!(
        store.rename_daemon(oversized),
        Err(PersistenceError::Bounds { .. })
    ));
    let mut noncanonical = rename("alice", "noncanonical", 1, "New")?;
    noncanonical.canonical_request = b"{ \"name\" : \"New\" }".to_vec();
    assert!(matches!(
        store.rename_daemon(noncanonical),
        Err(PersistenceError::InvalidDocument(_))
    ));
    assert_eq!(
        store
            .daemon_metadata(&host)?
            .ok_or("missing metadata")?
            .generation,
        1
    );
    Ok(())
}

#[test]
fn concurrent_renames_accept_only_one_observed_generation() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;
    let store = std::sync::Arc::new(RedbStore::open(root.path())?);
    let host = PeerId::new("host:studio-a")?;
    store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
    let commands = [
        rename("alice", "a", 1, "Studio Alice")?,
        rename("bob", "b", 1, "Studio Bob")?,
    ];
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let outcomes = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for command in commands {
            let store = std::sync::Arc::clone(&store);
            let barrier = std::sync::Arc::clone(&barrier);
            workers.push(scope.spawn(move || {
                barrier.wait();
                store.rename_daemon(command)
            }));
        }
        workers
            .into_iter()
            .map(std::thread::ScopedJoinHandle::join)
            .collect::<Vec<_>>()
    });
    let mut applied = 0;
    let mut conflicts = 0;
    for outcome in outcomes {
        match outcome.map_err(|_| "rename worker panicked")? {
            Ok(DaemonRenameOutcome::Applied(_)) => applied += 1,
            Err(PersistenceError::DaemonGenerationConflict {
                expected: 1,
                actual: 2,
            }) => conflicts += 1,
            other => return Err(format!("unexpected concurrent rename result: {other:?}").into()),
        }
    }
    assert_eq!((applied, conflicts), (1, 1));
    assert_eq!(
        store
            .daemon_metadata(&host)?
            .ok_or("missing metadata")?
            .generation,
        2
    );
    Ok(())
}

#[test]
fn full_receipt_capacity_preserves_reads_replay_and_conflicts_across_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let host = PeerId::new("host:studio-a")?;
    let store = RedbStore::open(root.path())?;
    store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
    let first = rename("alice", "rename-1", 1, "Studio")?;
    for generation in 1..=MAX_DAEMON_RENAME_RECEIPTS {
        assert!(matches!(
            store.rename_daemon(rename(
                "alice",
                &format!("rename-{generation}"),
                generation,
                "Studio"
            )?)?,
            DaemonRenameOutcome::Applied(_)
        ));
    }
    drop(store);
    let store = RedbStore::open(root.path())?;
    assert_eq!(
        store
            .daemon_metadata(&host)?
            .ok_or("missing metadata")?
            .generation,
        1025
    );
    assert!(matches!(
        store.rename_daemon(first)?,
        DaemonRenameOutcome::Replay(_)
    ));
    assert!(matches!(
        store.rename_daemon(rename("alice", "new", 1025, "Over capacity")?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            ..
        })
    ));
    assert!(matches!(
        store.rename_daemon(rename("alice", "rename-1", 1, "Changed")?),
        Err(PersistenceError::DaemonIdempotencyConflict { .. })
    ));
    assert!(
        store
            .daemon_rename_receipt(&host, &ActorRef::new("bob")?, "rename-1")?
            .is_none()
    );
    Ok(())
}

#[cfg(feature = "test-admin")]
#[test]
fn lost_reply_and_failed_commit_preserve_atomic_metadata_and_receipts()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_redb_store::{FaultPoint, RedbStoreConfig};
    for point in [
        FaultPoint::BeforeDaemonMetadataCommit,
        FaultPoint::AfterDaemonMetadataCommit,
    ] {
        let root = tempfile::tempdir()?;
        let host = PeerId::new("host:studio-a")?;
        let store = RedbStore::open(root.path())?;
        store.initialize_daemon_metadata(&host, &DaemonDisplayName::default())?;
        drop(store);
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(root.path())
                .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point))),
        )?;
        let command = rename("alice", "lost", 1, "Accepted")?;
        assert!(store.rename_daemon(command.clone()).is_err());
        drop(store);
        let store = RedbStore::open(root.path())?;
        let saved = store.daemon_rename_receipt(&host, &ActorRef::new("alice")?, "lost")?;
        assert_eq!(
            saved.is_some(),
            point == FaultPoint::AfterDaemonMetadataCommit
        );
        assert_eq!(
            store
                .daemon_metadata(&host)?
                .ok_or("missing metadata")?
                .generation,
            if saved.is_some() { 2 } else { 1 }
        );
        let outcome = store.rename_daemon(command)?;
        assert_eq!(
            matches!(outcome, DaemonRenameOutcome::Replay(_)),
            saved.is_some()
        );
    }
    Ok(())
}

#[cfg(feature = "test-admin")]
#[test]
fn initialization_binds_identity_and_seed_in_the_same_commit()
-> Result<(), Box<dyn std::error::Error>> {
    use milkdrift_redb_store::{FaultPoint, RedbStoreConfig};
    for point in [
        FaultPoint::BeforeDaemonMetadataCommit,
        FaultPoint::AfterDaemonMetadataCommit,
    ] {
        let root = tempfile::tempdir()?;
        let host = PeerId::new("host:studio-a")?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(root.path())
                .with_fault_injector(std::sync::Arc::new(fault::FailOnce::new(point))),
        )?;
        assert!(
            store
                .initialize_daemon_metadata(&host, &DaemonDisplayName::new("First seed")?)
                .is_err()
        );
        drop(store);
        let store = RedbStore::open(root.path())?;
        assert_eq!(
            store.daemon_metadata(&host)?.is_some(),
            point == FaultPoint::AfterDaemonMetadataCommit
        );
        let initialized =
            store.initialize_daemon_metadata(&host, &DaemonDisplayName::new("Second seed")?)?;
        assert_eq!(
            initialized.display_name.as_str(),
            if point == FaultPoint::AfterDaemonMetadataCommit {
                "First seed"
            } else {
                "Second seed"
            }
        );
    }
    Ok(())
}

#[cfg(feature = "test-admin")]
#[path = "support/fault.rs"]
mod fault;
