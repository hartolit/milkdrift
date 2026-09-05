use super::*;

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
