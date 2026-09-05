//! Physical membership tests bypass the host filesystem durability boundary.
use milkdrift_persistence::{PersistenceError, StorageFailureClass};
use redb::{Database, MultimapTableDefinition, TableDefinition, TableHandle as _};
use std::path::Path;
use tempfile::TempDir;
const DATABASE_FILENAME: &str = "schema.redb";

fn initialize(path: &Path) -> Result<Database, Box<dyn std::error::Error>> {
    let db = Database::create(path.join(DATABASE_FILENAME))?;
    let write = db.begin_write()?;
    super::initialize_tables(&write)?;
    write.commit()?;
    super::validate_tables(&db.begin_read()?)?;
    Ok(db)
}

fn validate(path: &Path) -> Result<(), PersistenceError> {
    let db = Database::open(path.join(DATABASE_FILENAME)).map_err(crate::error::database)?;
    super::validate_tables(&db.begin_read().map_err(crate::error::redb)?)
}

fn assert_corruption(result: Result<(), PersistenceError>) {
    assert!(
        matches!(
            &result,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            })
        ),
        "expected corruption, got {result:?}"
    );
}
#[test]
fn every_initialized_table_is_required_and_type_checked_on_reopen()
-> Result<(), Box<dyn std::error::Error>> {
    let baseline = TempDir::new()?;
    drop(initialize(baseline.path())?);
    let database = Database::open(baseline.path().join(DATABASE_FILENAME))?;
    let names: Vec<_> = database
        .begin_read()?
        .list_tables()?
        .map(|table| table.name().to_owned())
        .collect();
    // Independent physical-format expectation, never generated from the declaration macro.
    assert_eq!(names.len(), 63);
    drop(database);
    for name in names {
        for malformed in [false, true] {
            let directory = TempDir::new()?;
            drop(initialize(directory.path())?);
            let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
            let write = database.begin_write()?;
            let table = write
                .list_tables()?
                .find(|table| table.name() == name)
                .ok_or("table absent from initialization")?;
            assert!(write.delete_table(table)?);
            if malformed {
                // No supported table uses this key/value pair.
                drop(write.open_table(TableDefinition::<u128, u128>::new(&name))?);
            }
            write.commit()?;
            drop(database);
            assert_corruption(validate(directory.path()));
        }
    }
    Ok(())
}

#[test]
fn extra_physical_tables_are_refused_without_repair() -> Result<(), Box<dyn std::error::Error>> {
    for multimap in [false, true] {
        let directory = TempDir::new()?;
        drop(initialize(directory.path())?);
        let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
        let write = database.begin_write()?;
        if multimap {
            drop(
                write
                    .open_multimap_table(MultimapTableDefinition::<u64, u64>::new("unexpected"))?,
            );
        } else {
            drop(write.open_table(TableDefinition::<u64, u64>::new("unexpected"))?);
        }
        write.commit()?;
        drop(database);
        assert_corruption(validate(directory.path()));
        let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
        let read = database.begin_read()?;
        assert_eq!(read.list_tables()?.count(), 63 + usize::from(!multimap));
        assert_eq!(read.list_multimap_tables()?.count(), usize::from(multimap));
    }
    Ok(())
}
