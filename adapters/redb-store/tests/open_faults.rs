//! Process-style schema initialization and internal-format migration fault tests.

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
use redb::{Database, ReadableTable, TableDefinition};
use tempfile::TempDir;

const DATABASE_FILENAME: &str = "milkdrift.redb";
const METADATA: TableDefinition<'static, &'static str, u64> =
    TableDefinition::new("milkdrift.v1.metadata");
const WORKSPACE_VALUE_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.workspace.value_accounting");
const ARTIFACT_ACCOUNTING: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.artifacts.accounting");
const INTEGRITY_ROOTS: TableDefinition<'static, &'static str, &'static [u8]> =
    TableDefinition::new("milkdrift.v1.integrity.roots");
const INTEGRITY_NODES: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("milkdrift.v1.integrity.trie_nodes");
const INTERNAL_FORMAT_KEY: &str = "internal_document_format_version";

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

fn clear_string_table(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static str, &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    loop {
        let key = table.first()?.map(|(key, _)| key.value().to_owned());
        let Some(key) = key else {
            break;
        };
        let _ = table.remove(key.as_str())?;
    }
    Ok(())
}

fn clear_binary_table(
    write: &redb::WriteTransaction,
    definition: TableDefinition<'static, &'static [u8], &'static [u8]>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut table = write.open_table(definition)?;
    loop {
        let key = table.first()?.map(|(key, _)| key.value().to_vec());
        let Some(key) = key else {
            break;
        };
        let _ = table.remove(key.as_slice())?;
    }
    Ok(())
}

fn internal_envelope(family: &str, payload: &[u8]) -> Result<Vec<u8>, serde_json::Error> {
    const DOMAIN: &[u8] = b"milkdrift.redb.internal-document.v1\0";
    let mut hasher = blake3::Hasher::new();
    hasher.update(DOMAIN);
    hasher.update(&(family.len() as u64).to_be_bytes());
    hasher.update(family.as_bytes());
    hasher.update(&(payload.len() as u64).to_be_bytes());
    hasher.update(payload);
    let family_json = serde_json::to_string(family)?;
    let mut encoded = format!(
        "{{\"schema_version\":1,\"family\":{family_json},\"checksum\":\"{}\",\"payload\":",
        hasher.finalize().to_hex()
    )
    .into_bytes();
    encoded.extend_from_slice(payload);
    encoded.push(b'}');
    Ok(encoded)
}

fn downgrade_empty_current_store_to_v2(
    directory: &TempDir,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        let store = RedbStore::open(directory.path())?;
        assert_eq!(
            store.schema_info()?.compatibility,
            StorageSchemaCompatibility::Current
        );
    }

    let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
    let write = database.begin_write()?;
    {
        let mut metadata = write.open_table(METADATA)?;
        metadata.insert(INTERNAL_FORMAT_KEY, 2)?;
    }
    clear_string_table(&write, INTEGRITY_ROOTS)?;
    clear_binary_table(&write, INTEGRITY_NODES)?;
    {
        let mut accounting = write.open_table(WORKSPACE_VALUE_ACCOUNTING)?;
        let bytes = internal_envelope(
            "workspace value accounting",
            br#"{"schema_version":1,"value_versions":0,"inline_bytes":0}"#,
        )?;
        accounting.insert("", bytes.as_slice())?;
    }
    {
        let mut accounting = write.open_table(ARTIFACT_ACCOUNTING)?;
        let bytes = internal_envelope(
            "artifact accounting",
            br#"{"schema_version":1,"committed_content_bytes":0}"#,
        )?;
        accounting.insert("artifact_content_bytes", bytes.as_slice())?;
    }
    write.commit()?;
    Ok(())
}

fn internal_format(directory: &TempDir) -> Result<u64, Box<dyn std::error::Error>> {
    let database = Database::open(directory.path().join(DATABASE_FILENAME))?;
    let read = database.begin_read()?;
    let metadata = read.open_table(METADATA)?;
    Ok(metadata
        .get(INTERNAL_FORMAT_KEY)?
        .ok_or("internal format marker is absent")?
        .value())
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
fn migration_faults_reopen_as_v2_or_fully_migrated_v3() -> Result<(), Box<dyn std::error::Error>> {
    for point in [
        FaultPoint::BeforeMigrationCommit,
        FaultPoint::AfterMigrationCommit,
    ] {
        let directory = TempDir::new()?;
        downgrade_empty_current_store_to_v2(&directory)?;
        assert_eq!(internal_format(&directory)?, 2);

        assert!(
            RedbStore::open_with_config(
                RedbStoreConfig::new(directory.path())
                    .with_fault_injector(Arc::new(FailOnce::new(point))),
            )
            .is_err()
        );
        assert_eq!(
            internal_format(&directory)?,
            if point == FaultPoint::BeforeMigrationCommit {
                2
            } else {
                3
            }
        );

        let reopened = RedbStore::open(directory.path())?;
        assert_eq!(
            reopened.schema_info()?.compatibility,
            StorageSchemaCompatibility::Current
        );
        drop(reopened);
        assert_eq!(internal_format(&directory)?, 3);
    }
    Ok(())
}
