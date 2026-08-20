use super::*;
#[test]
fn revision_lookup_and_integrity_scan_detect_physical_key_mismatches()
-> Result<(), Box<dyn std::error::Error>> {
    const REVISIONS: TableDefinition<'static, &'static str, &'static [u8]> =
        TableDefinition::new("milkdrift.v1.revisions.by_id");
    const EVENTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
        TableDefinition::new("milkdrift.v1.runs.events");

    let directory = TempDir::new()?;
    let revision_bytes =
        include_bytes!("../../../../crates/blueprint/tests/fixtures/revision-v1.json");
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
