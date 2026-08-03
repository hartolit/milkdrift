//! Persistence round-trip tests.

use std::error::Error;
use std::fs;
use std::io;
use std::num::{NonZeroU64, TryFromIntError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use redb::{Database, ReadableDatabase, TableDefinition};
use redb_storage::{
    ApplicationSettings, ModelRecord, RedbStorage, StorageError, StoredAcceleratorMemoryPolicy,
    StoredApplicationDevice, StoredScalarType,
};

const SETTINGS_KEY: &str = "application";
const SETTINGS_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("application_settings_v1");
const MODELS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("model_catalogue_v1");

static NEXT_DATABASE: AtomicU64 = AtomicU64::new(0);

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new() -> Self {
        let identifier = NEXT_DATABASE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "llm-app-redb-storage-{}-{identifier}.redb",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_raw_settings(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let database = Database::create(path)?;
    let write = database.begin_write()?;
    {
        let mut table = write.open_table(SETTINGS_TABLE)?;
        table.insert(SETTINGS_KEY, bytes)?;
    }
    write.commit()?;
    Ok(())
}

fn read_raw_record(
    path: &Path,
    definition: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let database = Database::create(path)?;
    let read = database.begin_read()?;
    let table = read.open_table(definition)?;
    let value = table
        .get(key)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "raw record is missing"))?;
    Ok(value.value().to_vec())
}

fn encode_string(output: &mut Vec<u8>, value: &str) -> Result<(), TryFromIntError> {
    let length = u32::try_from(value.len())?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn nonzero(bytes: u64) -> Result<NonZeroU64, io::Error> {
    NonZeroU64::new(bytes)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "test limit must be nonzero"))
}

fn set_byte(bytes: &mut [u8], offset: usize, value: u8) -> Result<(), io::Error> {
    let byte = bytes.get_mut(offset).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "test offset is out of bounds")
    })?;
    *byte = value;
    Ok(())
}

fn insert_bytes(bytes: &mut Vec<u8>, offset: usize, value: [u8; 8]) -> Result<(), io::Error> {
    if bytes.get(offset..).is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "test insertion offset is out of bounds",
        ));
    }
    bytes.splice(offset..offset, value);
    Ok(())
}

fn settings_v1_bytes(
    repository: &str,
    revision: &str,
    maximum_host_memory_bytes: u64,
    maximum_device_memory_bytes: u64,
    drain_timeout_milliseconds: u64,
) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAS1".to_vec();
    output.extend_from_slice(&1_u16.to_le_bytes());
    encode_string(&mut output, repository)?;
    encode_string(&mut output, revision)?;
    output.extend_from_slice(&maximum_host_memory_bytes.to_le_bytes());
    output.extend_from_slice(&maximum_device_memory_bytes.to_le_bytes());
    output.extend_from_slice(&drain_timeout_milliseconds.to_le_bytes());
    Ok(output)
}

fn settings_v2_bytes(settings: &ApplicationSettings) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAS1".to_vec();
    output.extend_from_slice(&2_u16.to_le_bytes());
    encode_string(&mut output, &settings.default_repository)?;
    encode_string(&mut output, &settings.default_revision)?;
    output.extend_from_slice(&settings.maximum_host_memory_bytes.to_le_bytes());
    match settings.selected_device {
        StoredApplicationDevice::Cpu => output.push(0),
        StoredApplicationDevice::Cuda { ordinal } => {
            output.push(1);
            output.extend_from_slice(&ordinal.to_le_bytes());
        }
    }
    match settings.accelerator_memory_policy {
        StoredAcceleratorMemoryPolicy::Automatic => output.push(0),
        StoredAcceleratorMemoryPolicy::Limit { bytes } => {
            output.push(1);
            output.extend_from_slice(&bytes.get().to_le_bytes());
        }
    }
    output.extend_from_slice(&settings.drain_timeout_milliseconds.to_le_bytes());
    Ok(output)
}

fn model_v1_bytes(record: &ModelRecord) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAM1".to_vec();
    output.extend_from_slice(&1_u16.to_le_bytes());
    encode_string(&mut output, &record.name)?;
    encode_string(&mut output, &record.repository)?;
    encode_string(&mut output, &record.revision)?;
    output.push(match record.scalar_type {
        StoredScalarType::F32 => 0,
        StoredScalarType::F16 => 1,
        StoredScalarType::Bf16 => 2,
    });
    output.extend_from_slice(&record.last_used_unix_milliseconds.to_le_bytes());
    Ok(output)
}

#[test]
fn settings_v1_zero_device_memory_migrates_to_cpu_automatic() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let raw = settings_v1_bytes("acme/legacy-zero", "main", 8_192, 0, 2_000)?;
    write_raw_settings(database.path(), &raw)?;

    {
        let storage = RedbStorage::open(database.path())?;
        assert_eq!(
            storage.load_settings()?,
            Some(ApplicationSettings {
                default_repository: "acme/legacy-zero".to_owned(),
                default_revision: "main".to_owned(),
                maximum_host_memory_bytes: 8_192,
                selected_device: StoredApplicationDevice::Cpu,
                accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Automatic,
                drain_timeout_milliseconds: 2_000,
            })
        );
    }

    assert_eq!(
        read_raw_record(database.path(), SETTINGS_TABLE, SETTINGS_KEY)?,
        raw
    );
    Ok(())
}

#[test]
fn settings_v1_nonzero_device_memory_migrates_to_cpu_limit() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let raw = settings_v1_bytes("acme/legacy-limit", "release", 16_384, 4_096, 3_000)?;
    write_raw_settings(database.path(), &raw)?;

    {
        let storage = RedbStorage::open(database.path())?;
        assert_eq!(
            storage.load_settings()?,
            Some(ApplicationSettings {
                default_repository: "acme/legacy-limit".to_owned(),
                default_revision: "release".to_owned(),
                maximum_host_memory_bytes: 16_384,
                selected_device: StoredApplicationDevice::Cpu,
                accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Limit {
                    bytes: nonzero(4_096)?,
                },
                drain_timeout_milliseconds: 3_000,
            })
        );
    }

    assert_eq!(
        read_raw_record(database.path(), SETTINGS_TABLE, SETTINGS_KEY)?,
        raw
    );
    Ok(())
}

#[test]
fn settings_v2_empty_default_repository_with_cuda_survives_reopen() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let settings = ApplicationSettings {
        default_repository: String::new(),
        default_revision: "main".to_owned(),
        maximum_host_memory_bytes: 24_576,
        selected_device: StoredApplicationDevice::Cuda { ordinal: 3 },
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Automatic,
        drain_timeout_milliseconds: 3_500,
    };

    {
        let storage = RedbStorage::open(database.path())?;
        storage.save_settings(&settings)?;
    }

    assert_eq!(
        read_raw_record(database.path(), SETTINGS_TABLE, SETTINGS_KEY)?,
        settings_v2_bytes(&settings)?
    );

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.load_settings()?, Some(settings));
    Ok(())
}

#[test]
fn settings_v2_cpu_round_trip_survives_reopen() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let settings = ApplicationSettings {
        default_repository: "acme/cpu-model".to_owned(),
        default_revision: "cpu-revision".to_owned(),
        maximum_host_memory_bytes: 32_768,
        selected_device: StoredApplicationDevice::Cpu,
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Automatic,
        drain_timeout_milliseconds: 4_000,
    };

    {
        let storage = RedbStorage::open(database.path())?;
        storage.save_settings(&settings)?;
    }

    assert_eq!(
        read_raw_record(database.path(), SETTINGS_TABLE, SETTINGS_KEY)?,
        settings_v2_bytes(&settings)?
    );

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.load_settings()?, Some(settings));
    Ok(())
}

#[test]
fn settings_v2_cuda_limit_round_trip_survives_reopen() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let settings = ApplicationSettings {
        default_repository: "acme/cuda-model".to_owned(),
        default_revision: "cuda-revision".to_owned(),
        maximum_host_memory_bytes: 65_536,
        selected_device: StoredApplicationDevice::Cuda { ordinal: 7 },
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Limit {
            bytes: nonzero(12_288)?,
        },
        drain_timeout_milliseconds: 5_000,
    };

    {
        let storage = RedbStorage::open(database.path())?;
        storage.save_settings(&settings)?;
    }

    assert_eq!(
        read_raw_record(database.path(), SETTINGS_TABLE, SETTINGS_KEY)?,
        settings_v2_bytes(&settings)?
    );

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.load_settings()?, Some(settings));
    Ok(())
}

#[test]
fn settings_v2_rejects_invalid_tags_and_zero_limit() -> Result<(), Box<dyn Error>> {
    let settings = ApplicationSettings {
        default_repository: "acme/invalid-settings".to_owned(),
        default_revision: "main".to_owned(),
        maximum_host_memory_bytes: 1_024,
        selected_device: StoredApplicationDevice::Cpu,
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Automatic,
        drain_timeout_milliseconds: 1_000,
    };
    let device_tag_offset =
        6 + 4 + settings.default_repository.len() + 4 + settings.default_revision.len() + 8;
    let policy_tag_offset = device_tag_offset + 1;

    let unknown_device_database = TestDatabase::new();
    let mut unknown_device = settings_v2_bytes(&settings)?;
    set_byte(&mut unknown_device, device_tag_offset, 2)?;
    write_raw_settings(unknown_device_database.path(), &unknown_device)?;
    let storage = RedbStorage::open(unknown_device_database.path())?;
    assert!(matches!(
        storage.load_settings(),
        Err(StorageError::InvalidApplicationDeviceTag(2))
    ));
    drop(storage);

    let unknown_policy_database = TestDatabase::new();
    let mut unknown_policy = settings_v2_bytes(&settings)?;
    set_byte(&mut unknown_policy, policy_tag_offset, 2)?;
    write_raw_settings(unknown_policy_database.path(), &unknown_policy)?;
    let storage = RedbStorage::open(unknown_policy_database.path())?;
    assert!(matches!(
        storage.load_settings(),
        Err(StorageError::InvalidAcceleratorMemoryPolicyTag(2))
    ));
    drop(storage);

    let zero_limit_database = TestDatabase::new();
    let mut zero_limit = settings_v2_bytes(&settings)?;
    set_byte(&mut zero_limit, policy_tag_offset, 1)?;
    let limit_value_offset = policy_tag_offset.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "test offset arithmetic overflow",
        )
    })?;
    insert_bytes(&mut zero_limit, limit_value_offset, [0; 8])?;
    write_raw_settings(zero_limit_database.path(), &zero_limit)?;
    let storage = RedbStorage::open(zero_limit_database.path())?;
    assert!(matches!(
        storage.load_settings(),
        Err(StorageError::InvalidAcceleratorMemoryLimit(0))
    ));
    Ok(())
}

#[test]
fn model_v1_format_and_operations_are_unaffected() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let model = ModelRecord {
        name: "local-llama".to_owned(),
        repository: "acme/llama".to_owned(),
        revision: "main".to_owned(),
        scalar_type: StoredScalarType::F32,
        last_used_unix_milliseconds: 42,
    };

    {
        let storage = RedbStorage::open(database.path())?;
        storage.upsert_model(&model)?;
        assert_eq!(storage.load_model("local-llama")?, Some(model.clone()));
        assert_eq!(storage.list_models()?, vec![model.clone()]);
    }

    assert_eq!(
        read_raw_record(database.path(), MODELS_TABLE, "local-llama")?,
        model_v1_bytes(&model)?
    );

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.load_model("local-llama")?, Some(model.clone()));
    assert_eq!(reopened.list_models()?, vec![model]);
    assert!(reopened.remove_model("local-llama")?);
    assert!(reopened.load_model("local-llama")?.is_none());
    Ok(())
}
