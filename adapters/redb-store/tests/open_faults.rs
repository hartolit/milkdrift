//! Process-style schema initialization and exact-current-format refusal tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use milkdrift_persistence::{
    PersistenceError, StorageAdmin, StorageFailureClass, StorageSchemaCompatibility,
};
use milkdrift_redb_store::{
    FaultInjector, FaultPoint, RedbStore, RedbStoreConfig, injected_failure,
};
use redb::{Database, TableDefinition};
use tempfile::TempDir;

const DATABASE_FILENAME: &str = "milkdrift.redb";
const METADATA: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.metadata");

struct FailOnce {
    point: FaultPoint,
    remaining: AtomicUsize,
}

impl FailOnce {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(1),
        }
    }
}

impl FaultInjector for FailOnce {
    fn check(&self, point: FaultPoint) -> Result<(), PersistenceError> {
        if point == self.point && self.remaining.swap(0, Ordering::SeqCst) == 1 {
            Err(injected_failure(point))
        } else {
            Ok(())
        }
    }
}

fn assert_corruption<T: std::fmt::Debug>(result: Result<T, PersistenceError>) {
    assert!(
        matches!(
            &result,
            Err(PersistenceError::Storage {
                class: StorageFailureClass::Corruption,
                ..
            }) | Err(PersistenceError::Corruption(_))
        ),
        "expected corruption, got {result:?}"
    );
}

#[test]
fn schema_initialization_faults_reopen_as_empty_or_fully_initialized()
-> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeSchemaCommit,
        FaultPoint::AfterSchemaCommit,
    ] {
        let directory = TempDir::new()?;
        assert!(
            RedbStore::open_with_config(
                RedbStoreConfig::new(directory.path())
                    .with_fault_injector(Arc::new(FailOnce::new(point))),
            )
            .is_err()
        );

        let reopened = RedbStore::open(directory.path())?;
        assert_eq!(
            reopened.schema_info()?.compatibility,
            StorageSchemaCompatibility::Current
        );
    }
    Ok(())
}

#[test]
fn a_nonempty_partially_initialized_database_is_refused() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TempDir::new()?;
    let database = Database::create(directory.path().join(DATABASE_FILENAME))?;
    let write = database.begin_write()?;
    {
        let _metadata = write.open_table(METADATA)?;
    }
    write.commit()?;
    drop(database);

    assert_corruption(RedbStore::open(directory.path()));
    Ok(())
}

#[test]
fn unpublished_internal_document_formats_are_refused_without_migration()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TempDir::new()?;
    drop(RedbStore::open(directory.path())?);

    let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(METADATA)?;
        metadata.insert("internal_document_format_version", 3)?;
    }
    write.commit()?;
    drop(database);

    assert!(matches!(
        RedbStore::open(directory.path()),
        Err(PersistenceError::UnsupportedVersion {
            document: "redb internal document envelope",
            found: 3,
            supported: 4,
        })
    ));
    Ok(())
}
