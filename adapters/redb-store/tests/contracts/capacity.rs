//! Physical loss of operational membership cannot silently manufacture fresh capacity.
use super::*;

#[test]
fn active_capacity_reopen_refuses_missing_counts_and_index_drift()
-> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    for (index, key) in [
        (
            "milkdrift.v1.managed.active_installations",
            "managed_active_count",
        ),
        (
            "milkdrift.v1.managed.pending_evaluations",
            "evaluation_pending_count",
        ),
        (
            "milkdrift.v1.published.active_methods",
            "publication_active_count",
        ),
    ] {
        for corruption in ["missing-count", "missing-row", "extra-row"] {
            let root = TempDir::new()?;
            drop(RedbStore::open(root.path())?);
            let database = Database::open(root.path().join("milkdrift.redb"))?;
            let write = database.begin_write()?;
            match corruption {
                "missing-count" => {
                    write.open_table(METADATA)?.remove(key)?;
                }
                "missing-row" => {
                    write.open_table(METADATA)?.insert(key, 1)?;
                }
                _ => {
                    write
                        .open_table(TableDefinition::<&str, u64>::new(index))?
                        .insert("orphan", 1)?;
                }
            }
            write.commit()?;
            drop(database);
            assert!(
                matches!(RedbStore::open(root.path()), Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption, message,
            }) if message.contains("active capacity")),
                "{index}: {corruption}"
            );
        }
    }
    Ok(())
}

#[test]
fn active_capacity_configuration_refuses_zero_without_creating_a_database()
-> Result<(), Box<dyn std::error::Error>> {
    for (installations, evaluations, publications) in [(0, 1, 1), (1, 0, 1), (1, 1, 0)] {
        let root = TempDir::new()?;
        assert!(matches!(
            RedbStore::open_with_config(
                RedbStoreConfig::new(root.path())
                    .with_managed_limits(installations, evaluations)
                    .with_publication_limit(publications)
            ),
            Err(PersistenceError::Bounds {
                location: "redb_store_config",
                ..
            })
        ));
        assert!(!root.path().join("milkdrift.redb").exists());
    }
    Ok(())
}
