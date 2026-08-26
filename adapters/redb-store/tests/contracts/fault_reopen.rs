use super::*;
#[test]
fn reopen_and_single_owner_are_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.schema_info()?.stored_version, 3);
    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            ..
        })
    ));
    drop(store);
    let reopened = RedbStore::open(directory.path())?;
    assert_eq!(reopened.schema_info()?.stored_version, 3);
    Ok(())
}

#[test]
fn older_and_future_storage_schemas_are_refused() -> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    for found in [2, 4] {
        let directory = TempDir::new()?;
        drop(RedbStore::open(directory.path())?);
        let database = Database::open(directory.path().join("milkdrift.redb"))?;
        let write = database.begin_write()?;
        {
            let mut metadata = write.open_table(METADATA)?;
            metadata.insert("storage_schema_version", found)?;
        }
        write.commit()?;
        drop(database);
        assert!(matches!(
            RedbStore::open(directory.path()),
            Err(PersistenceError::UnsupportedVersion {
                document: "storage",
                found: observed,
                supported: 3
            }) if observed == found as u32
        ));
    }
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
fn command_fault_boundaries_are_atomic_and_replayable() -> Result<(), Box<dyn std::error::Error>> {
    for (index, point) in [
        FaultPoint::BeforeEventInsert,
        FaultPoint::AfterEventInsert,
        FaultPoint::AfterHistoryChainUpdate,
        FaultPoint::BeforeCommandCommit,
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TempDir::new()?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        let request = accepted_request(
            &format!("run-before-{index}"),
            &format!("command-before-{index}"),
            &format!("event-before-{index}"),
            "start",
        )?;
        assert!(store.commit_command(&request).is_err());
        assert_eq!(store.head(request.receipt().run())?, RunSequence::ZERO);
        assert!(
            store
                .command_result(request.receipt().run(), request.receipt().command())?
                .is_none()
        );
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        assert_eq!(reopened.head(request.receipt().run())?, RunSequence::ZERO);
        assert!(matches!(
            reopened.commit_command(&request)?,
            AtomicRunCommitOutcome::Committed(_)
        ));
    }

    let after_directory = TempDir::new()?;
    let after = RedbStore::open_with_config(
        RedbStoreConfig::new(after_directory.path())
            .with_fault_injector(Arc::new(FailOnce::new(FaultPoint::AfterCommandCommit))),
    )?;
    let request = accepted_request("run-after", "command-after", "event-after", "start")?;
    assert!(after.commit_command(&request).is_err());
    assert_eq!(after.head(request.receipt().run())?, RunSequence::FIRST);
    assert!(matches!(
        after.commit_command(&request)?,
        AtomicRunCommitOutcome::Replayed(_)
    ));
    Ok(())
}

#[test]
fn revision_fault_boundaries_are_atomic_and_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let revision_bytes =
        include_bytes!("../../../../crates/blueprint/tests/fixtures/revision-v2.json");
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
            request.receipt().run().clone(),
            RunSequence::FIRST,
            history_digest(request.events())?,
            1,
            b"projection".to_vec(),
        )?;
        let request = request.with_projection_checkpoint(snapshot.payload_checkpoint()?)?;
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
                reopened.latest_snapshot(request.receipt().run())?,
                SnapshotLoad::Absent
            );
        } else {
            assert_eq!(
                reopened.latest_snapshot(request.receipt().run())?,
                SnapshotLoad::Verified(snapshot.clone())
            );
        }
        reopened.put_snapshot(&snapshot)?;
        assert_eq!(
            reopened.latest_snapshot(request.receipt().run())?,
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
            request.receipt().run().clone(),
            RunSequence::FIRST,
            history_digest(request.events())?,
            1,
            b"projection".to_vec(),
        )?;
        let request = request.with_projection_checkpoint(snapshot.payload_checkpoint()?)?;
        let store = RedbStore::open_with_config(
            RedbStoreConfig::new(directory.path())
                .with_fault_injector(Arc::new(FailOnce::new(point))),
        )?;
        store.commit_command(&request)?;
        store.put_snapshot(&snapshot)?;
        assert!(
            store
                .discard_snapshot(request.receipt().run(), snapshot.snapshot())
                .is_err()
        );
        drop(store);

        let reopened = RedbStore::open(directory.path())?;
        if point == FaultPoint::BeforeSnapshotDiscardCommit {
            assert_eq!(
                reopened.latest_snapshot(request.receipt().run())?,
                SnapshotLoad::Verified(snapshot.clone())
            );
        } else {
            assert_eq!(
                reopened.latest_snapshot(request.receipt().run())?,
                SnapshotLoad::Absent
            );
        }
        reopened.discard_snapshot(request.receipt().run(), snapshot.snapshot())?;
        assert_eq!(
            reopened.latest_snapshot(request.receipt().run())?,
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
    let run = request.receipt().run().clone();
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
    assert_storage_corruption(error);
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
        let _removed = heads.remove(missing.receipt().run().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(missing_directory.path())?;
    let missing_head = store.head(missing.receipt().run());
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
            missing.receipt().run().clone(),
            None,
            PageSize::new(1)?,
        )?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.command_result(missing.receipt().run(), missing.receipt().command()),
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
        first.receipt().run().clone(),
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
        heads.insert(first.receipt().run().as_str(), RunSequence::FIRST.get())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(lowered_directory.path())?;
    assert!(matches!(
        store.head(first.receipt().run()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.events(&EventPageQuery::new(
            first.receipt().run().clone(),
            None,
            PageSize::new(1)?,
        )?),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::Corruption,
            ..
        })
    ));
    assert!(matches!(
        store.command_result(second.receipt().run(), second.receipt().command()),
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
        first.receipt().run().clone(),
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
        key.extend_from_slice(&(first.receipt().run().as_str().len() as u32).to_be_bytes());
        key.extend_from_slice(first.receipt().run().as_str().as_bytes());
        key.extend_from_slice(&RunSequence::FIRST.get().to_be_bytes());
        let _removed = events.remove(key.as_slice())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(missing_event_directory.path())?;
    // RUN_HEADS remains the sole bounded sequence authority. Exact history reads,
    // command replay, and new commits must still refuse the missing interior fact.
    assert_eq!(store.head(first.receipt().run())?, RunSequence::new(2));
    assert!(matches!(
        store.command_result(first.receipt().run(), first.receipt().command()),
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
        let _removed = summaries.remove(summary_request.receipt().run().as_str())?;
    }
    write.commit()?;
    drop(database);
    let store = RedbStore::open(summary_directory.path())?;
    assert!(matches!(
        store.run_summary(summary_request.receipt().run()),
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
            let _removed = rows.remove(request.receipt().run().as_str())?;
        }
        write.commit()?;
        drop(database);

        let store = RedbStore::open(directory.path())?;
        if suffix == "usage" {
            assert!(matches!(
                store.workspace_usage(request.receipt().run()),
                Err(PersistenceError::Storage {
                    class: StorageFailureClass::Corruption,
                    ..
                })
            ));
        }
        let followup = accepted_followup_request(
            request.receipt().run().clone(),
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
