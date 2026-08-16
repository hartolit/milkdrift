//! Versioned desktop persistence over redb.
//!
//! Application settings are written as `LAS1` version 2 records; exact version 1
//! records remain readable. Model catalogue entries are written as `LAM1` version
//! 3 records, whose optional configuration-declared scalar type uses a separate
//! presence tag. Exact `LAM1` version 1 (mandatory scalar) and version 2 (scalar
//! code `3` means absent) records remain readable without an implicit rewrite.
//! Legacy timestamp bytes decode unchanged as
//! [`ModelRecord::last_resolved_unix_milliseconds`]. Model reads also require the
//! redb table key to equal the embedded [`ModelRecord::name`].

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

const SETTINGS_KEY: &str = "application";
const SETTINGS_MAGIC: [u8; 4] = *b"LAS1";
const MODEL_MAGIC: [u8; 4] = *b"LAM1";
const SETTINGS_VERSION_V1: u16 = 1;
const SETTINGS_VERSION_V2: u16 = 2;
const MODEL_VERSION_V1: u16 = 1;
const MODEL_VERSION_V2: u16 = 2;
const MODEL_VERSION_V3: u16 = 3;
const SETTINGS_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("application_settings_v1");
const MODELS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("model_catalogue_v1");

/// Persisted application device selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredApplicationDevice {
    /// Use the CPU backend.
    Cpu,
    /// Use one CUDA device by ordinal.
    Cuda {
        /// Zero-based CUDA device ordinal.
        ordinal: u32,
    },
}

/// Persisted accelerator-memory admission policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredAcceleratorMemoryPolicy {
    /// Let the runtime select an accelerator-memory limit.
    Automatic,
    /// Enforce an explicit nonzero accelerator-memory limit.
    Limit {
        /// Maximum admitted accelerator memory in bytes.
        bytes: std::num::NonZeroU64,
    },
}

/// Persisted application-level runtime limits and default Hub selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationSettings {
    /// Default Hub repository, or empty before the first repository selection.
    default_repository: String,
    /// Default Hub revision.
    default_revision: String,
    /// Aggregate host-memory admission bound.
    maximum_host_memory_bytes: u64,
    /// Runtime device selected for application work.
    selected_device: StoredApplicationDevice,
    /// Accelerator-memory admission policy.
    accelerator_memory_policy: StoredAcceleratorMemoryPolicy,
    /// Mandatory drain timeout used before forced cancellation.
    drain_timeout_milliseconds: std::num::NonZeroU64,
}

impl ApplicationSettings {
    /// Creates validated persisted application settings.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] when a nonempty repository consists
    /// only of whitespace, the revision is empty or whitespace-only, or the drain
    /// timeout is zero.
    pub fn new(
        default_repository: String,
        default_revision: String,
        maximum_host_memory_bytes: u64,
        selected_device: StoredApplicationDevice,
        accelerator_memory_policy: StoredAcceleratorMemoryPolicy,
        drain_timeout_milliseconds: u64,
    ) -> Result<Self, StorageError> {
        if !default_repository.is_empty() {
            validate_non_empty(&default_repository, Field::Repository)?;
        }
        validate_non_empty(&default_revision, Field::Revision)?;
        let drain_timeout_milliseconds = std::num::NonZeroU64::new(drain_timeout_milliseconds)
            .ok_or(StorageError::InvalidField(Field::DrainTimeout))?;
        Ok(Self {
            default_repository,
            default_revision,
            maximum_host_memory_bytes,
            selected_device,
            accelerator_memory_policy,
            drain_timeout_milliseconds,
        })
    }

    /// Returns the default Hub repository, which may be empty.
    #[must_use]
    pub fn default_repository(&self) -> &str {
        self.default_repository.as_str()
    }

    /// Returns the non-empty default Hub revision.
    #[must_use]
    pub fn default_revision(&self) -> &str {
        self.default_revision.as_str()
    }

    /// Returns the aggregate host-memory admission bound.
    #[must_use]
    pub const fn maximum_host_memory_bytes(&self) -> u64 {
        self.maximum_host_memory_bytes
    }

    /// Returns the persisted runtime device selection.
    #[must_use]
    pub const fn selected_device(&self) -> StoredApplicationDevice {
        self.selected_device
    }

    /// Returns the accelerator-memory admission policy.
    #[must_use]
    pub const fn accelerator_memory_policy(&self) -> StoredAcceleratorMemoryPolicy {
        self.accelerator_memory_policy
    }

    /// Returns the mandatory non-zero drain timeout in milliseconds.
    #[must_use]
    pub const fn drain_timeout_milliseconds(&self) -> u64 {
        self.drain_timeout_milliseconds.get()
    }

    /// Consumes the record into its stable persisted fields.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        String,
        String,
        u64,
        StoredApplicationDevice,
        StoredAcceleratorMemoryPolicy,
        u64,
    ) {
        (
            self.default_repository,
            self.default_revision,
            self.maximum_host_memory_bytes,
            self.selected_device,
            self.accelerator_memory_policy,
            self.drain_timeout_milliseconds.get(),
        )
    }
}

/// Scalar value retained from recognized model-configuration metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredScalarType {
    /// IEEE-754 32-bit floating point.
    F32,
    /// IEEE-754 16-bit floating point.
    F16,
    /// Brain floating point.
    Bf16,
}

/// Persisted logical model selection. Cache paths are deliberately not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRecord {
    /// User-visible unique catalogue name.
    name: String,
    /// Hugging Face repository.
    repository: String,
    /// Branch, tag, reference, or commit.
    revision: String,
    /// Optional scalar metadata declared by immutable model configuration.
    ///
    /// This is producer intent only. It does not describe per-tensor inventory or
    /// the scalar type selected for execution.
    configuration_declared_scalar_type: Option<StoredScalarType>,
    /// Last successful model resolution in Unix milliseconds, supplied by the application.
    last_resolved_unix_milliseconds: u64,
}

impl ModelRecord {
    /// Creates a validated catalogue identity and Hub selection.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] when the name, repository, or
    /// revision is empty or consists only of whitespace.
    pub fn new(
        name: String,
        repository: String,
        revision: String,
        configuration_declared_scalar_type: Option<StoredScalarType>,
        last_resolved_unix_milliseconds: u64,
    ) -> Result<Self, StorageError> {
        validate_non_empty(&name, Field::ModelName)?;
        validate_non_empty(&repository, Field::Repository)?;
        validate_non_empty(&revision, Field::Revision)?;
        Ok(Self {
            name,
            repository,
            revision,
            configuration_declared_scalar_type,
            last_resolved_unix_milliseconds,
        })
    }

    /// Returns the unique catalogue name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// Returns the Hub repository.
    #[must_use]
    pub fn repository(&self) -> &str {
        self.repository.as_str()
    }

    /// Returns the immutable branch, reference, tag, or commit.
    #[must_use]
    pub fn revision(&self) -> &str {
        self.revision.as_str()
    }

    /// Returns optional scalar metadata declared by model configuration.
    #[must_use]
    pub const fn configuration_declared_scalar_type(&self) -> Option<StoredScalarType> {
        self.configuration_declared_scalar_type
    }

    /// Returns the application-supplied last-resolution timestamp.
    #[must_use]
    pub const fn last_resolved_unix_milliseconds(&self) -> u64 {
        self.last_resolved_unix_milliseconds
    }
}

/// Invalid persisted field category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Model catalogue name.
    ModelName,
    /// Hub repository identifier.
    Repository,
    /// Hub revision identifier.
    Revision,
    /// Drain timeout.
    DrainTimeout,
}

/// Stable storage adapter failure.
#[derive(Debug)]
pub enum StorageError {
    /// redb operation failed.
    Database(redb::Error),
    /// A required semantic field was empty or invalid.
    InvalidField(Field),
    /// Record header did not match its table.
    InvalidRecordKind,
    /// Record version is newer or otherwise unsupported.
    UnsupportedVersion(u16),
    /// Record ended before all declared values were available.
    TruncatedRecord,
    /// A string length exceeded the binary record limit.
    StringTooLong {
        /// UTF-8 byte length that did not fit the persistent format.
        length: usize,
    },
    /// Persisted bytes were not valid UTF-8.
    InvalidUtf8(std::string::FromUtf8Error),
    /// Persisted application-device tag is unknown.
    InvalidApplicationDeviceTag(u8),
    /// Persisted accelerator-memory policy tag is unknown.
    InvalidAcceleratorMemoryPolicyTag(u8),
    /// Persisted accelerator-memory limit is invalid.
    InvalidAcceleratorMemoryLimit(u64),
    /// Persisted configuration-declared scalar metadata code is unknown.
    InvalidScalarType(u8),
    /// Persisted configuration-declared scalar metadata presence tag is unknown.
    InvalidScalarTypePresenceTag(u8),
    /// A redb model key disagreed with the logical name embedded in its record.
    ModelNameMismatch {
        /// Model catalogue table key.
        key: String,
        /// Model name embedded in the persisted record.
        embedded_name: String,
    },
    /// Bytes remained after a complete versioned record was decoded.
    TrailingBytes,
}

impl Display for StorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database operation failed: {error}"),
            Self::InvalidField(field) => write!(formatter, "invalid storage field: {field:?}"),
            Self::InvalidRecordKind => formatter.write_str("invalid persistent record kind"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported persistent record version: {version}"
                )
            }
            Self::TruncatedRecord => formatter.write_str("persistent record is truncated"),
            Self::StringTooLong { length } => {
                write!(
                    formatter,
                    "persistent string has {length} bytes and exceeds u32"
                )
            }
            Self::InvalidUtf8(error) => {
                write!(formatter, "persistent string is invalid UTF-8: {error}")
            }
            Self::InvalidApplicationDeviceTag(tag) => {
                write!(
                    formatter,
                    "unknown persistent application-device tag: {tag}"
                )
            }
            Self::InvalidAcceleratorMemoryPolicyTag(tag) => {
                write!(
                    formatter,
                    "unknown persistent accelerator-memory policy tag: {tag}"
                )
            }
            Self::InvalidAcceleratorMemoryLimit(bytes) => {
                write!(
                    formatter,
                    "invalid persistent accelerator-memory limit: {bytes} bytes"
                )
            }
            Self::InvalidScalarType(code) => {
                write!(
                    formatter,
                    "unknown persistent configuration-declared scalar type: {code}"
                )
            }
            Self::InvalidScalarTypePresenceTag(tag) => {
                write!(
                    formatter,
                    "unknown persistent configuration-declared scalar presence tag: {tag}"
                )
            }
            Self::ModelNameMismatch { key, embedded_name } => {
                write!(
                    formatter,
                    "model catalogue key {key:?} does not match embedded model name {embedded_name:?}"
                )
            }
            Self::TrailingBytes => formatter.write_str("persistent record contains trailing bytes"),
        }
    }
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::InvalidUtf8(error) => Some(error),
            Self::InvalidField(_)
            | Self::InvalidRecordKind
            | Self::UnsupportedVersion(_)
            | Self::TruncatedRecord
            | Self::StringTooLong { .. }
            | Self::InvalidApplicationDeviceTag(_)
            | Self::InvalidAcceleratorMemoryPolicyTag(_)
            | Self::InvalidAcceleratorMemoryLimit(_)
            | Self::InvalidScalarType(_)
            | Self::InvalidScalarTypePresenceTag(_)
            | Self::ModelNameMismatch { .. }
            | Self::TrailingBytes => None,
        }
    }
}

/// Open redb-backed desktop state.
pub struct RedbStorage {
    database: Database,
}

impl RedbStorage {
    /// Opens or creates the database file and initializes the versioned tables.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if the database cannot be created or
    /// opened, either table cannot be initialized, or the initialization
    /// transaction cannot be committed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let database =
            Database::create(path).map_err(|error| StorageError::Database(error.into()))?;
        let write = database
            .begin_write()
            .map_err(|error| StorageError::Database(error.into()))?;
        {
            let _settings = write
                .open_table(SETTINGS_TABLE)
                .map_err(|error| StorageError::Database(error.into()))?;
            let _models = write
                .open_table(MODELS_TABLE)
                .map_err(|error| StorageError::Database(error.into()))?;
        }
        write
            .commit()
            .map_err(|error| StorageError::Database(error.into()))?;
        Ok(Self { database })
    }

    /// Atomically replaces the application settings record.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] if `settings` fails validation,
    /// [`StorageError::StringTooLong`] if a persisted string exceeds the binary
    /// format's length limit, or [`StorageError::Database`] if the write fails.
    pub fn save_settings(&self, settings: &ApplicationSettings) -> Result<(), StorageError> {
        let encoded = encode_settings(settings)?;
        self.insert(SETTINGS_TABLE, SETTINGS_KEY, encoded.as_slice())
    }

    /// Reads the application settings when present.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if the read fails. Returns a record
    /// decoding or validation error if the stored settings bytes are malformed,
    /// unsupported, or contain invalid fields.
    pub fn load_settings(&self) -> Result<Option<ApplicationSettings>, StorageError> {
        let Some(bytes) = self.get(SETTINGS_TABLE, SETTINGS_KEY)? else {
            return Ok(None);
        };
        decode_settings(bytes.as_slice()).map(Some)
    }

    /// Atomically inserts or replaces one model catalogue entry as `LAM1` version 3.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] if `record` fails validation,
    /// [`StorageError::StringTooLong`] if a persisted string exceeds the binary
    /// format's length limit, or [`StorageError::Database`] if the write fails.
    pub fn upsert_model(&self, record: &ModelRecord) -> Result<(), StorageError> {
        let encoded = encode_model(record)?;
        self.insert(MODELS_TABLE, record.name.as_str(), encoded.as_slice())
    }

    /// Reads one model catalogue entry by its exact logical name.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] if `name` is empty or consists
    /// only of whitespace, [`StorageError::Database`] if the read fails, or
    /// [`StorageError::ModelNameMismatch`] if `name` differs from the name embedded
    /// in the stored record. Returns a record decoding or validation error if the
    /// stored model bytes are malformed, unsupported, or contain invalid fields.
    pub fn load_model(&self, name: &str) -> Result<Option<ModelRecord>, StorageError> {
        validate_non_empty(name, Field::ModelName)?;
        let Some(bytes) = self.get(MODELS_TABLE, name)? else {
            return Ok(None);
        };
        validate_model_key(name, decode_model(bytes.as_slice())?).map(Some)
    }

    /// Removes one catalogue entry and reports whether it existed.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidField`] if `name` is empty or consists
    /// only of whitespace. Returns [`StorageError::Database`] if the removal or
    /// its transaction fails.
    pub fn remove_model(&self, name: &str) -> Result<bool, StorageError> {
        validate_non_empty(name, Field::ModelName)?;
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Database(error.into()))?;
        let removed = {
            let mut table = write
                .open_table(MODELS_TABLE)
                .map_err(|error| StorageError::Database(error.into()))?;
            table
                .remove(name)
                .map_err(|error| StorageError::Database(error.into()))?
                .is_some()
        };
        write
            .commit()
            .map_err(|error| StorageError::Database(error.into()))?;
        Ok(removed)
    }

    /// Reads all catalogue entries in redb key order.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if opening or iterating the table fails,
    /// or [`StorageError::ModelNameMismatch`] if a table key differs from the name
    /// embedded in its record. Returns a record decoding or validation error if
    /// any stored model is malformed, unsupported, or contains invalid fields.
    pub fn list_models(&self) -> Result<Vec<ModelRecord>, StorageError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| StorageError::Database(error.into()))?;
        let table = read
            .open_table(MODELS_TABLE)
            .map_err(|error| StorageError::Database(error.into()))?;
        let mut records = Vec::new();
        let iterator = table
            .iter()
            .map_err(|error| StorageError::Database(error.into()))?;
        for entry in iterator {
            let (key, value) = entry.map_err(|error| StorageError::Database(error.into()))?;
            records.push(validate_model_key(
                key.value(),
                decode_model(value.value())?,
            )?);
        }
        Ok(records)
    }

    fn insert(
        &self,
        definition: TableDefinition<&str, &[u8]>,
        key: &str,
        value: &[u8],
    ) -> Result<(), StorageError> {
        let write = self
            .database
            .begin_write()
            .map_err(|error| StorageError::Database(error.into()))?;
        {
            let mut table = write
                .open_table(definition)
                .map_err(|error| StorageError::Database(error.into()))?;
            table
                .insert(key, value)
                .map_err(|error| StorageError::Database(error.into()))?;
        }
        write
            .commit()
            .map_err(|error| StorageError::Database(error.into()))
    }

    fn get(
        &self,
        definition: TableDefinition<&str, &[u8]>,
        key: &str,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let read = self
            .database
            .begin_read()
            .map_err(|error| StorageError::Database(error.into()))?;
        let table = read
            .open_table(definition)
            .map_err(|error| StorageError::Database(error.into()))?;
        let value = table
            .get(key)
            .map_err(|error| StorageError::Database(error.into()))?;
        Ok(value.map(|guard| guard.value().to_vec()))
    }
}

fn encode_settings(settings: &ApplicationSettings) -> Result<Vec<u8>, StorageError> {
    let mut output = Vec::new();
    output.extend_from_slice(&SETTINGS_MAGIC);
    output.extend_from_slice(&SETTINGS_VERSION_V2.to_le_bytes());
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
    output.extend_from_slice(&settings.drain_timeout_milliseconds.get().to_le_bytes());
    Ok(output)
}

fn decode_settings(bytes: &[u8]) -> Result<ApplicationSettings, StorageError> {
    let (mut decoder, version) = Decoder::new(bytes, SETTINGS_MAGIC)?;
    let settings = match version {
        SETTINGS_VERSION_V1 => decode_settings_v1(&mut decoder)?,
        SETTINGS_VERSION_V2 => decode_settings_v2(&mut decoder)?,
        version => return Err(StorageError::UnsupportedVersion(version)),
    };
    decoder.finish()?;
    Ok(settings)
}

fn decode_settings_v1(decoder: &mut Decoder<'_>) -> Result<ApplicationSettings, StorageError> {
    let default_repository = decoder.string()?;
    let default_revision = decoder.string()?;
    let maximum_host_memory_bytes = decoder.u64()?;
    let maximum_device_memory_bytes = decoder.u64()?;
    let accelerator_memory_policy = match std::num::NonZeroU64::new(maximum_device_memory_bytes) {
        Some(bytes) => StoredAcceleratorMemoryPolicy::Limit { bytes },
        None => StoredAcceleratorMemoryPolicy::Automatic,
    };
    let drain_timeout_milliseconds = decoder.u64()?;

    ApplicationSettings::new(
        default_repository,
        default_revision,
        maximum_host_memory_bytes,
        StoredApplicationDevice::Cpu,
        accelerator_memory_policy,
        drain_timeout_milliseconds,
    )
}

fn decode_settings_v2(decoder: &mut Decoder<'_>) -> Result<ApplicationSettings, StorageError> {
    let default_repository = decoder.string()?;
    let default_revision = decoder.string()?;
    let maximum_host_memory_bytes = decoder.u64()?;
    let selected_device = match decoder.u8()? {
        0 => StoredApplicationDevice::Cpu,
        1 => StoredApplicationDevice::Cuda {
            ordinal: decoder.u32()?,
        },
        tag => return Err(StorageError::InvalidApplicationDeviceTag(tag)),
    };
    let accelerator_memory_policy = match decoder.u8()? {
        0 => StoredAcceleratorMemoryPolicy::Automatic,
        1 => {
            let bytes = decoder.u64()?;
            let bytes = std::num::NonZeroU64::new(bytes)
                .ok_or(StorageError::InvalidAcceleratorMemoryLimit(bytes))?;
            StoredAcceleratorMemoryPolicy::Limit { bytes }
        }
        tag => return Err(StorageError::InvalidAcceleratorMemoryPolicyTag(tag)),
    };
    let drain_timeout_milliseconds = decoder.u64()?;

    ApplicationSettings::new(
        default_repository,
        default_revision,
        maximum_host_memory_bytes,
        selected_device,
        accelerator_memory_policy,
        drain_timeout_milliseconds,
    )
}

fn encode_model(record: &ModelRecord) -> Result<Vec<u8>, StorageError> {
    let mut output = Vec::new();
    output.extend_from_slice(&MODEL_MAGIC);
    output.extend_from_slice(&MODEL_VERSION_V3.to_le_bytes());
    encode_string(&mut output, &record.name)?;
    encode_string(&mut output, &record.repository)?;
    encode_string(&mut output, &record.revision)?;
    match record.configuration_declared_scalar_type {
        Some(scalar_type) => {
            output.push(1);
            output.push(stored_scalar_type_code(scalar_type));
        }
        None => output.push(0),
    }
    output.extend_from_slice(&record.last_resolved_unix_milliseconds.to_le_bytes());
    Ok(output)
}

fn decode_model(bytes: &[u8]) -> Result<ModelRecord, StorageError> {
    let (mut decoder, version) = Decoder::new(bytes, MODEL_MAGIC)?;
    let record = match version {
        MODEL_VERSION_V1 => decode_model_v1(&mut decoder)?,
        MODEL_VERSION_V2 => decode_model_v2(&mut decoder)?,
        MODEL_VERSION_V3 => decode_model_v3(&mut decoder)?,
        version => return Err(StorageError::UnsupportedVersion(version)),
    };
    decoder.finish()?;
    Ok(record)
}

fn decode_model_v1(decoder: &mut Decoder<'_>) -> Result<ModelRecord, StorageError> {
    ModelRecord::new(
        decoder.string()?,
        decoder.string()?,
        decoder.string()?,
        Some(decode_stored_scalar_type(decoder.u8()?)?),
        decoder.u64()?,
    )
}

fn decode_model_v2(decoder: &mut Decoder<'_>) -> Result<ModelRecord, StorageError> {
    let name = decoder.string()?;
    let repository = decoder.string()?;
    let revision = decoder.string()?;
    let configuration_declared_scalar_type = match decoder.u8()? {
        3 => None,
        code => Some(decode_stored_scalar_type(code)?),
    };
    let last_resolved_unix_milliseconds = decoder.u64()?;

    ModelRecord::new(
        name,
        repository,
        revision,
        configuration_declared_scalar_type,
        last_resolved_unix_milliseconds,
    )
}

fn decode_model_v3(decoder: &mut Decoder<'_>) -> Result<ModelRecord, StorageError> {
    let name = decoder.string()?;
    let repository = decoder.string()?;
    let revision = decoder.string()?;
    let configuration_declared_scalar_type = match decoder.u8()? {
        0 => None,
        1 => Some(decode_stored_scalar_type(decoder.u8()?)?),
        tag => return Err(StorageError::InvalidScalarTypePresenceTag(tag)),
    };
    let last_resolved_unix_milliseconds = decoder.u64()?;

    ModelRecord::new(
        name,
        repository,
        revision,
        configuration_declared_scalar_type,
        last_resolved_unix_milliseconds,
    )
}

const fn stored_scalar_type_code(scalar_type: StoredScalarType) -> u8 {
    match scalar_type {
        StoredScalarType::F32 => 0,
        StoredScalarType::F16 => 1,
        StoredScalarType::Bf16 => 2,
    }
}

fn decode_stored_scalar_type(code: u8) -> Result<StoredScalarType, StorageError> {
    match code {
        0 => Ok(StoredScalarType::F32),
        1 => Ok(StoredScalarType::F16),
        2 => Ok(StoredScalarType::Bf16),
        code => Err(StorageError::InvalidScalarType(code)),
    }
}

fn validate_model_key(key: &str, record: ModelRecord) -> Result<ModelRecord, StorageError> {
    if record.name == key {
        Ok(record)
    } else {
        Err(StorageError::ModelNameMismatch {
            key: key.to_owned(),
            embedded_name: record.name,
        })
    }
}

fn validate_non_empty(value: &str, field: Field) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        Err(StorageError::InvalidField(field))
    } else {
        Ok(())
    }
}

fn encode_string(output: &mut Vec<u8>, value: &str) -> Result<(), StorageError> {
    let length = u32::try_from(value.len()).map_err(|_| StorageError::StringTooLong {
        length: value.len(),
    })?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

struct Decoder<'record> {
    bytes: &'record [u8],
    offset: usize,
}

impl<'record> Decoder<'record> {
    fn new(bytes: &'record [u8], expected_magic: [u8; 4]) -> Result<(Self, u16), StorageError> {
        let magic = bytes.get(..4).ok_or(StorageError::TruncatedRecord)?;
        if magic != expected_magic {
            return Err(StorageError::InvalidRecordKind);
        }
        let version_bytes: [u8; 2] = bytes
            .get(4..6)
            .ok_or(StorageError::TruncatedRecord)?
            .try_into()
            .map_err(|_| StorageError::TruncatedRecord)?;
        let version = u16::from_le_bytes(version_bytes);
        Ok((Self { bytes, offset: 6 }, version))
    }

    fn u8(&mut self) -> Result<u8, StorageError> {
        let value = *self
            .bytes
            .get(self.offset)
            .ok_or(StorageError::TruncatedRecord)?;
        self.offset = self
            .offset
            .checked_add(1)
            .ok_or(StorageError::TruncatedRecord)?;
        Ok(value)
    }

    fn u32(&mut self) -> Result<u32, StorageError> {
        let end = self
            .offset
            .checked_add(4)
            .ok_or(StorageError::TruncatedRecord)?;
        let value: [u8; 4] = self
            .bytes
            .get(self.offset..end)
            .ok_or(StorageError::TruncatedRecord)?
            .try_into()
            .map_err(|_| StorageError::TruncatedRecord)?;
        self.offset = end;
        Ok(u32::from_le_bytes(value))
    }

    fn u64(&mut self) -> Result<u64, StorageError> {
        let end = self
            .offset
            .checked_add(8)
            .ok_or(StorageError::TruncatedRecord)?;
        let value: [u8; 8] = self
            .bytes
            .get(self.offset..end)
            .ok_or(StorageError::TruncatedRecord)?
            .try_into()
            .map_err(|_| StorageError::TruncatedRecord)?;
        self.offset = end;
        Ok(u64::from_le_bytes(value))
    }

    fn string(&mut self) -> Result<String, StorageError> {
        let length = usize::try_from(self.u32()?).map_err(|_| StorageError::TruncatedRecord)?;
        let end = self
            .offset
            .checked_add(length)
            .ok_or(StorageError::TruncatedRecord)?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(StorageError::TruncatedRecord)?;
        self.offset = end;
        String::from_utf8(bytes.to_vec()).map_err(StorageError::InvalidUtf8)
    }

    const fn finish(self) -> Result<(), StorageError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(StorageError::TrailingBytes)
        }
    }
}
