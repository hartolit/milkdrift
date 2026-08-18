use std::{
    fmt, fs,
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use milkdrift_persistence::{PersistenceError, StorageFailureClass};
use redb::Database;

use crate::{
    error,
    fault::{FaultInjector, no_faults},
    schema::{
        ARTIFACT_DIGEST_RESERVATIONS, ARTIFACT_METADATA, ARTIFACT_PUBLICATIONS,
        ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS,
        ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, COMMAND_RESULTS, EVENT_CHECKSUMS, LEASE_ENTRIES,
        LEASE_INDEX, METADATA, NONTERMINAL_RUNS, REVISIONS, REVISIONS_BY_DIGEST, ROOT_SCOPES,
        RUN_EVENTS, RUN_HEADS, RUN_SUMMARIES, RUNNABLE_ENTRIES, RUNNABLE_INDEX, SCHEMA_VERSION_KEY,
        SCOPES, SNAPSHOT_LATEST, SNAPSHOTS, STORAGE_SCHEMA_VERSION, TIMER_ENTRIES, TIMER_INDEX,
        VALUES, WORKSPACE_BUDGETS, WORKSPACE_USAGE,
    },
};

const DATABASE_FILENAME: &str = "milkdrift.redb";
const ARTIFACT_DIRECTORY: &str = "artifacts";
const TEMP_DIRECTORY: &str = ".tmp";
const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 1_073_741_824;
const DEFAULT_MAX_TOTAL_ARTIFACT_BYTES: u64 = 10_737_418_240;
const DEFAULT_MAX_READ_BYTES: u64 = 1_073_741_824;

/// Bounded local-store configuration.
pub struct RedbStoreConfig {
    pub(crate) root: PathBuf,
    pub(crate) max_artifact_bytes: u64,
    pub(crate) max_total_artifact_bytes: u64,
    pub(crate) max_read_bytes: u64,
    pub(crate) faults: Arc<dyn FaultInjector>,
}

impl fmt::Debug for RedbStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedbStoreConfig")
            .field("root", &self.root)
            .field("max_artifact_bytes", &self.max_artifact_bytes)
            .field("max_total_artifact_bytes", &self.max_total_artifact_bytes)
            .field("max_read_bytes", &self.max_read_bytes)
            .finish_non_exhaustive()
    }
}

impl RedbStoreConfig {
    /// Creates a configuration rooted at an owned local data directory.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            max_total_artifact_bytes: DEFAULT_MAX_TOTAL_ARTIFACT_BYTES,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
            faults: no_faults(),
        }
    }

    /// Applies adapter-wide content and verified-read bounds.
    #[must_use]
    pub fn with_artifact_limits(
        mut self,
        max_artifact_bytes: u64,
        max_total_artifact_bytes: u64,
        max_read_bytes: u64,
    ) -> Self {
        self.max_artifact_bytes = max_artifact_bytes;
        self.max_total_artifact_bytes = max_total_artifact_bytes;
        self.max_read_bytes = max_read_bytes;
        self
    }

    /// Installs a synchronous deterministic fault hook for durability tests.
    #[must_use]
    pub fn with_fault_injector(mut self, faults: Arc<dyn FaultInjector>) -> Self {
        self.faults = faults;
        self
    }
}

/// Production local persistence and content-addressed artifact owner.
///
/// The embedded database remains private; callers interact only through the
/// `milkdrift-persistence` ports implemented by this type.
pub struct RedbStore {
    pub(crate) database: Database,
    pub(crate) root: PathBuf,
    pub(crate) artifact_root: PathBuf,
    pub(crate) temp_root: PathBuf,
    pub(crate) max_artifact_bytes: u64,
    pub(crate) max_total_artifact_bytes: u64,
    pub(crate) max_read_bytes: u64,
    pub(crate) faults: Arc<dyn FaultInjector>,
    pub(crate) artifact_serialization: Mutex<()>,
}

impl fmt::Debug for RedbStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedbStore")
            .field("root", &self.root)
            .field("max_artifact_bytes", &self.max_artifact_bytes)
            .field("max_total_artifact_bytes", &self.max_total_artifact_bytes)
            .field("max_read_bytes", &self.max_read_bytes)
            .finish_non_exhaustive()
    }
}

impl RedbStore {
    /// Opens or creates a durable local store with default bounds.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        Self::open_with_config(RedbStoreConfig::new(root))
    }

    /// Opens or creates a durable local store from explicit bounds and hooks.
    #[tracing::instrument(
        name = "milkdrift.redb_store.open",
        skip_all,
        fields(storage_schema = STORAGE_SCHEMA_VERSION)
    )]
    pub fn open_with_config(config: RedbStoreConfig) -> Result<Self, PersistenceError> {
        validate_config(&config)?;
        prepare_owned_directory(&config.root, "storage root")?;
        let database_path = config.root.join(DATABASE_FILENAME);
        ensure_regular_file_or_absent(&database_path, "storage database")?;
        let is_new = file_is_new(&database_path)?;
        let database = Database::create(&database_path).map_err(error::database)?;

        if is_new {
            initialize_schema(&database)?;
            sync_owned_directory(&config.root)?;
        } else {
            validate_schema(&database)?;
        }

        let artifact_root = config.root.join(ARTIFACT_DIRECTORY);
        let temp_root = artifact_root.join(TEMP_DIRECTORY);
        prepare_owned_directory(&artifact_root, "artifact root")?;
        prepare_owned_directory(&temp_root, "artifact temporary directory")?;

        Ok(Self {
            database,
            root: config.root,
            artifact_root,
            temp_root,
            max_artifact_bytes: config.max_artifact_bytes,
            max_total_artifact_bytes: config.max_total_artifact_bytes,
            max_read_bytes: config.max_read_bytes,
            faults: config.faults,
            artifact_serialization: Mutex::new(()),
        })
    }

    pub(crate) const fn database(&self) -> &Database {
        &self.database
    }
}

fn validate_config(config: &RedbStoreConfig) -> Result<(), PersistenceError> {
    if config.max_artifact_bytes > config.max_total_artifact_bytes {
        return Err(PersistenceError::Bounds {
            location: "redb_store_config",
            reason: "per-artifact bytes cannot exceed aggregate artifact bytes".to_owned(),
        });
    }
    if config.max_read_bytes == 0 {
        return Err(PersistenceError::Bounds {
            location: "redb_store_config",
            reason: "verified read limit must be nonzero".to_owned(),
        });
    }
    Ok(())
}

fn file_is_new(path: &Path) -> Result<bool, PersistenceError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len() == 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error::io(error)),
    }
}

pub(crate) fn prepare_owned_directory(
    path: &Path,
    family: &'static str,
) -> Result<(), PersistenceError> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owned_directory_type(&metadata, family)?;
            true
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => false,
        Err(cause) => return Err(error::io(cause)),
    };
    if !existed {
        fs::create_dir_all(path).map_err(error::io)?;
        let metadata = fs::symlink_metadata(path).map_err(error::io)?;
        validate_owned_directory_type(&metadata, family)?;
        if let Some(parent) = path.parent() {
            sync_owned_directory(parent)?;
        }
    }
    Ok(())
}

pub(crate) fn sync_owned_directory(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path).map_err(error::io)?;
    validate_owned_directory_type(&metadata, "storage directory")?;
    let directory = open_directory_no_follow(path)?;
    verify_opened_identity(path, &directory, true)?;
    directory.sync_all().map_err(error::io)
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> Result<File, PersistenceError> {
    use rustix::fs::{Mode, OFlags};

    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|cause| {
        if cause == rustix::io::Errno::LOOP {
            error::corruption("storage directory changed into a symlink while opening")
        } else {
            error::io(cause.into())
        }
    })
}

#[cfg(not(unix))]
fn open_directory_no_follow(path: &Path) -> Result<File, PersistenceError> {
    File::open(path).map_err(error::io)
}

fn verify_opened_identity(
    path: &Path,
    opened: &File,
    expect_directory: bool,
) -> Result<(), PersistenceError> {
    let opened_metadata = opened.metadata().map_err(error::io)?;
    let path_metadata = fs::symlink_metadata(path).map_err(error::io)?;
    let expected_type = if expect_directory {
        opened_metadata.is_dir() && path_metadata.file_type().is_dir()
    } else {
        opened_metadata.is_file() && path_metadata.file_type().is_file()
    };
    if !expected_type || path_metadata.file_type().is_symlink() {
        return Err(error::corruption(
            "storage path changed type or became a symlink while opening",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if opened_metadata.dev() != path_metadata.dev()
            || opened_metadata.ino() != path_metadata.ino()
        {
            return Err(error::corruption(
                "storage path identity changed while opening",
            ));
        }
    }
    Ok(())
}

fn validate_owned_directory_type(
    metadata: &fs::Metadata,
    family: &'static str,
) -> Result<(), PersistenceError> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(error::corruption(format!(
            "{family} must be an owned directory, not a symlink or special file"
        )));
    }
    Ok(())
}

fn ensure_regular_file_or_absent(
    path: &Path,
    family: &'static str,
) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(error::corruption(format!(
            "{family} must be a regular file, not a symlink or special file"
        ))),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(error::io(cause)),
    }
}

fn initialize_schema(database: &Database) -> Result<(), PersistenceError> {
    let write = database.begin_write().map_err(error::redb)?;
    {
        let mut table = write.open_table(METADATA).map_err(error::redb)?;
        table
            .insert(SCHEMA_VERSION_KEY, STORAGE_SCHEMA_VERSION)
            .map_err(error::redb)?;
    }
    // Opening each definition records its exact key/value encoding in redb.
    {
        let _table = write.open_table(REVISIONS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_EVENTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(TIMER_INDEX).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(LEASE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(SNAPSHOTS).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(SCOPES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(VALUES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?;
    }
    {
        let _table = write
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    }
    {
        let _table = write.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    }
    write.commit().map_err(error::redb)
}

fn validate_schema(database: &Database) -> Result<(), PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let found = {
        let table = read.open_table(METADATA).map_err(error::redb)?;
        table
            .get(SCHEMA_VERSION_KEY)
            .map_err(error::redb)?
            .map(|value| value.value())
            .ok_or_else(|| error::corruption("storage schema version is missing"))?
    };
    if found > STORAGE_SCHEMA_VERSION {
        let found = u32::try_from(found).unwrap_or(u32::MAX);
        return Err(PersistenceError::UnsupportedVersion {
            document: "storage",
            found,
            supported: STORAGE_SCHEMA_VERSION as u32,
        });
    }
    if found < STORAGE_SCHEMA_VERSION {
        return Err(PersistenceError::MigrationRequired {
            found: found as u32,
            target: STORAGE_SCHEMA_VERSION as u32,
        });
    }

    // A successful typed open is the schema's physical type check.
    {
        let _table = read.open_table(REVISIONS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_HEADS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_EVENTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(EVENT_CHECKSUMS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(COMMAND_RESULTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUN_SUMMARIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(NONTERMINAL_RUNS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUNNABLE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(RUNNABLE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(TIMER_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(TIMER_INDEX).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(LEASE_ENTRIES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(LEASE_INDEX).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(SNAPSHOTS).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(SNAPSHOT_LATEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(SCOPES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ROOT_SCOPES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(VALUES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    }
    {
        let _table = read
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    }
    {
        let _table = read.open_table(WORKSPACE_BUDGETS).map_err(error::redb)?;
    }
    Ok(())
}

pub(crate) fn internal(message: impl Into<String>) -> PersistenceError {
    PersistenceError::Storage {
        class: StorageFailureClass::Internal,
        message: message.into(),
    }
}
