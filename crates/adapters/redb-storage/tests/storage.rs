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

fn write_raw_model(path: &Path, name: &str, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let database = Database::create(path)?;
    let write = database.begin_write()?;
    {
        let mut table = write.open_table(MODELS_TABLE)?;
        table.insert(name, bytes)?;
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

fn scalar_code(scalar_type: StoredScalarType) -> u8 {
    match scalar_type {
        StoredScalarType::F32 => 0,
        StoredScalarType::F16 => 1,
        StoredScalarType::Bf16 => 2,
    }
}

fn encode_model_identity(
    output: &mut Vec<u8>,
    name: &str,
    repository: &str,
    revision: &str,
) -> Result<(), TryFromIntError> {
    encode_string(output, name)?;
    encode_string(output, repository)?;
    encode_string(output, revision)
}

fn model_v1_bytes(
    name: &str,
    repository: &str,
    revision: &str,
    scalar_code: u8,
    last_resolved_unix_milliseconds: u64,
) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAM1".to_vec();
    output.extend_from_slice(&1_u16.to_le_bytes());
    encode_model_identity(&mut output, name, repository, revision)?;
    output.push(scalar_code);
    output.extend_from_slice(&last_resolved_unix_milliseconds.to_le_bytes());
    Ok(output)
}

fn model_v2_bytes(record: &ModelRecord) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAM1".to_vec();
    output.extend_from_slice(&2_u16.to_le_bytes());
    encode_model_identity(
        &mut output,
        &record.name,
        &record.repository,
        &record.revision,
    )?;
    output.push(
        record
            .configuration_declared_scalar_type
            .map_or(3, scalar_code),
    );
    output.extend_from_slice(&record.last_resolved_unix_milliseconds.to_le_bytes());
    Ok(output)
}

fn model_v3_bytes(record: &ModelRecord) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = b"LAM1".to_vec();
    output.extend_from_slice(&3_u16.to_le_bytes());
    encode_model_identity(
        &mut output,
        &record.name,
        &record.repository,
        &record.revision,
    )?;
    match record.configuration_declared_scalar_type {
        Some(scalar_type) => {
            output.push(1);
            output.push(scalar_code(scalar_type));
        }
        None => output.push(0),
    }
    output.extend_from_slice(&record.last_resolved_unix_milliseconds.to_le_bytes());
    Ok(output)
}

fn model_metadata_offset(record: &ModelRecord) -> usize {
    6 + 4 + record.name.len() + 4 + record.repository.len() + 4 + record.revision.len()
}

fn model_record(
    name: &str,
    scalar_type: Option<StoredScalarType>,
    last_resolved_unix_milliseconds: u64,
) -> ModelRecord {
    ModelRecord {
        name: name.to_owned(),
        repository: "acme/model".to_owned(),
        revision: "immutable-commit".to_owned(),
        configuration_declared_scalar_type: scalar_type,
        last_resolved_unix_milliseconds,
    }
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
fn model_v1_scalar_codes_decode_as_present_without_rewrite() -> Result<(), Box<dyn Error>> {
    let cases = [
        (0, StoredScalarType::F32, "legacy-v1-f32", 40),
        (1, StoredScalarType::F16, "legacy-v1-f16", 41),
        (2, StoredScalarType::Bf16, "legacy-v1-bf16", 42),
    ];

    for (code, scalar_type, name, timestamp) in cases {
        let database = TestDatabase::new();
        let record = model_record(name, Some(scalar_type), timestamp);
        let raw = model_v1_bytes(name, &record.repository, &record.revision, code, timestamp)?;
        write_raw_model(database.path(), name, &raw)?;

        {
            let storage = RedbStorage::open(database.path())?;
            assert_eq!(storage.load_model(name)?, Some(record.clone()));
            assert_eq!(storage.list_models()?, vec![record]);
        }

        assert_eq!(read_raw_record(database.path(), MODELS_TABLE, name)?, raw);
    }
    Ok(())
}

#[test]
fn model_v2_present_and_absent_codes_decode_without_rewrite() -> Result<(), Box<dyn Error>> {
    let cases = [
        (Some(StoredScalarType::F32), "legacy-v2-f32", 50),
        (Some(StoredScalarType::F16), "legacy-v2-f16", 51),
        (Some(StoredScalarType::Bf16), "legacy-v2-bf16", 52),
        (None, "legacy-v2-none", 53),
    ];

    for (scalar_type, name, timestamp) in cases {
        let database = TestDatabase::new();
        let record = model_record(name, scalar_type, timestamp);
        let raw = model_v2_bytes(&record)?;
        write_raw_model(database.path(), name, &raw)?;

        {
            let storage = RedbStorage::open(database.path())?;
            assert_eq!(storage.load_model(name)?, Some(record.clone()));
            assert_eq!(storage.list_models()?, vec![record]);
        }

        assert_eq!(read_raw_record(database.path(), MODELS_TABLE, name)?, raw);
    }
    Ok(())
}

#[test]
fn model_v3_present_metadata_writes_exact_presence_and_scalar_bytes() -> Result<(), Box<dyn Error>>
{
    let database = TestDatabase::new();
    let models = vec![
        model_record("model-0-f32", Some(StoredScalarType::F32), 60),
        model_record("model-1-f16", Some(StoredScalarType::F16), 61),
        model_record("model-2-bf16", Some(StoredScalarType::Bf16), 62),
    ];

    {
        let storage = RedbStorage::open(database.path())?;
        for model in &models {
            storage.upsert_model(model)?;
            assert_eq!(storage.load_model(&model.name)?, Some(model.clone()));
        }
        assert_eq!(storage.list_models()?, models);
    }

    for model in &models {
        assert_eq!(
            read_raw_record(database.path(), MODELS_TABLE, &model.name)?,
            model_v3_bytes(model)?,
            "unexpected v3 bytes for {}",
            model.name
        );
    }

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.list_models()?, models);
    assert!(reopened.remove_model("model-0-f32")?);
    assert!(reopened.load_model("model-0-f32")?.is_none());
    Ok(())
}

#[test]
fn model_v3_absent_metadata_writes_only_the_absence_tag() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let model = model_record("model-none", None, 0x0302_0100_0302_0100);

    {
        let storage = RedbStorage::open(database.path())?;
        storage.upsert_model(&model)?;
        assert_eq!(storage.load_model(&model.name)?, Some(model.clone()));
    }

    let raw = read_raw_record(database.path(), MODELS_TABLE, &model.name)?;
    assert_eq!(raw, model_v3_bytes(&model)?);
    assert_eq!(raw.get(model_metadata_offset(&model)), Some(&0));
    assert_eq!(raw.len(), model_metadata_offset(&model) + 1 + 8);

    let reopened = RedbStorage::open(database.path())?;
    assert_eq!(reopened.load_model(&model.name)?, Some(model.clone()));
    assert_eq!(reopened.list_models()?, vec![model]);
    Ok(())
}

#[test]
fn explicit_upsert_after_old_read_rewrites_as_v3() -> Result<(), Box<dyn Error>> {
    let v1_record = model_record("upsert-v1", Some(StoredScalarType::F16), 1_700_000_000_001);
    let v2_record = model_record("upsert-v2", None, 1_700_000_000_002);
    let cases = [
        (
            model_v1_bytes(
                &v1_record.name,
                &v1_record.repository,
                &v1_record.revision,
                1,
                v1_record.last_resolved_unix_milliseconds,
            )?,
            v1_record,
        ),
        (model_v2_bytes(&v2_record)?, v2_record),
    ];

    for (old_bytes, expected_record) in cases {
        let database = TestDatabase::new();
        write_raw_model(database.path(), &expected_record.name, &old_bytes)?;

        let loaded = {
            let storage = RedbStorage::open(database.path())?;
            storage
                .load_model(&expected_record.name)?
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "old model is missing"))?
        };
        assert_eq!(loaded, expected_record);
        assert_eq!(
            read_raw_record(database.path(), MODELS_TABLE, &loaded.name)?,
            old_bytes
        );

        {
            let storage = RedbStorage::open(database.path())?;
            storage.upsert_model(&loaded)?;
        }
        assert_eq!(
            read_raw_record(database.path(), MODELS_TABLE, &loaded.name)?,
            model_v3_bytes(&loaded)?
        );
    }
    Ok(())
}

#[test]
fn unknown_record_versions_are_rejected() -> Result<(), Box<dyn Error>> {
    for version in [0, 4, u16::MAX] {
        let model_database = TestDatabase::new();
        let mut model_bytes = b"LAM1".to_vec();
        model_bytes.extend_from_slice(&version.to_le_bytes());
        write_raw_model(model_database.path(), "unknown-version", &model_bytes)?;
        let storage = RedbStorage::open(model_database.path())?;
        assert!(matches!(
            storage.load_model("unknown-version"),
            Err(StorageError::UnsupportedVersion(found)) if found == version
        ));
        drop(storage);

        let settings_database = TestDatabase::new();
        let mut settings_bytes = b"LAS1".to_vec();
        settings_bytes.extend_from_slice(&version.to_le_bytes());
        write_raw_settings(settings_database.path(), &settings_bytes)?;
        let storage = RedbStorage::open(settings_database.path())?;
        assert!(matches!(
            storage.load_settings(),
            Err(StorageError::UnsupportedVersion(found)) if found == version
        ));
    }
    Ok(())
}

#[test]
fn wrong_record_magic_is_rejected() -> Result<(), Box<dyn Error>> {
    let model_database = TestDatabase::new();
    write_raw_model(model_database.path(), "wrong-magic", b"LAS1\x02\x00")?;
    let storage = RedbStorage::open(model_database.path())?;
    assert!(matches!(
        storage.load_model("wrong-magic"),
        Err(StorageError::InvalidRecordKind)
    ));
    drop(storage);

    let settings_database = TestDatabase::new();
    write_raw_settings(settings_database.path(), b"LAM1\x03\x00")?;
    let storage = RedbStorage::open(settings_database.path())?;
    assert!(matches!(
        storage.load_settings(),
        Err(StorageError::InvalidRecordKind)
    ));
    Ok(())
}

#[test]
fn every_v3_model_prefix_is_rejected_as_truncated() -> Result<(), Box<dyn Error>> {
    let records = [
        model_record("truncated-present", Some(StoredScalarType::Bf16), 70),
        model_record("truncated-absent", None, 71),
    ];

    for record in records {
        let raw = model_v3_bytes(&record)?;
        for length in 0..raw.len() {
            let database = TestDatabase::new();
            let prefix = raw.get(..length).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "test prefix is out of bounds")
            })?;
            write_raw_model(database.path(), &record.name, prefix)?;
            let storage = RedbStorage::open(database.path())?;
            assert!(
                matches!(
                    storage.load_model(&record.name),
                    Err(StorageError::TruncatedRecord)
                ),
                "prefix length {length} unexpectedly decoded"
            );
        }
    }
    Ok(())
}

#[test]
fn all_model_versions_and_settings_reject_trailing_bytes() -> Result<(), Box<dyn Error>> {
    let record = model_record("trailing", Some(StoredScalarType::F32), 80);
    let model_records = [
        model_v1_bytes(
            &record.name,
            &record.repository,
            &record.revision,
            0,
            record.last_resolved_unix_milliseconds,
        )?,
        model_v2_bytes(&record)?,
        model_v3_bytes(&record)?,
    ];

    for mut raw in model_records {
        raw.push(0xff);
        let database = TestDatabase::new();
        write_raw_model(database.path(), &record.name, &raw)?;
        let storage = RedbStorage::open(database.path())?;
        assert!(matches!(
            storage.load_model(&record.name),
            Err(StorageError::TrailingBytes)
        ));
    }

    let settings = ApplicationSettings {
        default_repository: "acme/trailing".to_owned(),
        default_revision: "main".to_owned(),
        maximum_host_memory_bytes: 1_024,
        selected_device: StoredApplicationDevice::Cpu,
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy::Automatic,
        drain_timeout_milliseconds: 1_000,
    };
    let mut raw = settings_v2_bytes(&settings)?;
    raw.push(0xff);
    let database = TestDatabase::new();
    write_raw_settings(database.path(), &raw)?;
    let storage = RedbStorage::open(database.path())?;
    assert!(matches!(
        storage.load_settings(),
        Err(StorageError::TrailingBytes)
    ));
    Ok(())
}

#[test]
fn model_v3_rejects_invalid_utf8_in_each_string() -> Result<(), Box<dyn Error>> {
    let record = ModelRecord {
        name: "n".to_owned(),
        repository: "r".to_owned(),
        revision: "v".to_owned(),
        configuration_declared_scalar_type: None,
        last_resolved_unix_milliseconds: 90,
    };
    let name_byte_offset = 10;
    let repository_byte_offset = name_byte_offset + record.name.len() + 4;
    let revision_byte_offset = repository_byte_offset + record.repository.len() + 4;

    for (offset, field) in [
        (name_byte_offset, "name"),
        (repository_byte_offset, "repository"),
        (revision_byte_offset, "revision"),
    ] {
        let database = TestDatabase::new();
        let mut raw = model_v3_bytes(&record)?;
        set_byte(&mut raw, offset, 0xff)?;
        write_raw_model(database.path(), &record.name, &raw)?;
        let storage = RedbStorage::open(database.path())?;
        assert!(
            matches!(
                storage.load_model(&record.name),
                Err(StorageError::InvalidUtf8(_))
            ),
            "invalid UTF-8 in {field} was accepted"
        );
    }
    Ok(())
}

#[test]
fn model_versions_reject_invalid_presence_tags_and_scalars() -> Result<(), Box<dyn Error>> {
    let v1_database = TestDatabase::new();
    let v1 = model_record("invalid-v1-scalar", Some(StoredScalarType::F32), 100);
    let raw = model_v1_bytes(&v1.name, &v1.repository, &v1.revision, 3, 100)?;
    write_raw_model(v1_database.path(), &v1.name, &raw)?;
    let storage = RedbStorage::open(v1_database.path())?;
    assert!(matches!(
        storage.load_model(&v1.name),
        Err(StorageError::InvalidScalarType(3))
    ));
    drop(storage);

    let v2_database = TestDatabase::new();
    let v2 = model_record("invalid-v2-scalar", Some(StoredScalarType::F16), 101);
    let mut raw = model_v2_bytes(&v2)?;
    set_byte(&mut raw, model_metadata_offset(&v2), 4)?;
    write_raw_model(v2_database.path(), &v2.name, &raw)?;
    let storage = RedbStorage::open(v2_database.path())?;
    assert!(matches!(
        storage.load_model(&v2.name),
        Err(StorageError::InvalidScalarType(4))
    ));
    drop(storage);

    let presence_database = TestDatabase::new();
    let v3 = model_record("invalid-v3-presence", Some(StoredScalarType::Bf16), 102);
    let mut raw = model_v3_bytes(&v3)?;
    set_byte(&mut raw, model_metadata_offset(&v3), 2)?;
    write_raw_model(presence_database.path(), &v3.name, &raw)?;
    let storage = RedbStorage::open(presence_database.path())?;
    assert!(matches!(
        storage.load_model(&v3.name),
        Err(StorageError::InvalidScalarTypePresenceTag(2))
    ));
    drop(storage);

    let scalar_database = TestDatabase::new();
    let v3 = model_record("invalid-v3-scalar", Some(StoredScalarType::F32), 103);
    let mut raw = model_v3_bytes(&v3)?;
    set_byte(&mut raw, model_metadata_offset(&v3) + 1, 3)?;
    write_raw_model(scalar_database.path(), &v3.name, &raw)?;
    let storage = RedbStorage::open(scalar_database.path())?;
    assert!(matches!(
        storage.load_model(&v3.name),
        Err(StorageError::InvalidScalarType(3))
    ));
    Ok(())
}

#[test]
fn model_key_mismatch_is_rejected_by_load_and_list() -> Result<(), Box<dyn Error>> {
    let database = TestDatabase::new();
    let record = model_record("embedded-name", None, 110);
    let raw = model_v3_bytes(&record)?;
    write_raw_model(database.path(), "table-key", &raw)?;

    let storage = RedbStorage::open(database.path())?;
    assert!(matches!(
        storage.load_model("table-key"),
        Err(StorageError::ModelNameMismatch { key, embedded_name })
            if key == "table-key" && embedded_name == "embedded-name"
    ));
    assert!(matches!(
        storage.list_models(),
        Err(StorageError::ModelNameMismatch { key, embedded_name })
            if key == "table-key" && embedded_name == "embedded-name"
    ));
    Ok(())
}
