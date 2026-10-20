use super::*;
#[test]
fn reopen_and_single_owner_are_enforced() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    let store = RedbStore::open(directory.path())?;
    assert_eq!(store.schema_info()?.stored_version, 21);
    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            ..
        })
    ));
    drop(store);
    let reopened = RedbStore::open(directory.path())?;
    assert_eq!(reopened.schema_info()?.stored_version, 21);
    Ok(())
}

#[test]
fn older_and_future_storage_schemas_are_refused() -> Result<(), Box<dyn std::error::Error>> {
    const METADATA: TableDefinition<'static, &'static str, u64> =
        TableDefinition::new("milkdrift.v1.metadata");
    for found in [12, 13, 14, 15, 16, 17, 18, 22] {
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
                supported: 21
            }) if u64::from(observed) == found
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
