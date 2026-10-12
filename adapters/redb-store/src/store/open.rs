use super::{
    ARTIFACT_DIRECTORY, DATABASE_FILENAME, Database, Mutex, PathBuf, PersistenceError, RedbStore,
    RedbStoreConfig, TEMP_DIRECTORY, ensure_regular_file_or_absent, error, fmt, initialize_schema,
    prepare_owned_directory, sync_owned_directory, validate_schema,
};
use crate::{clock::require_accepted_clock, schema::STORAGE_SCHEMA_VERSION};
use milkdrift_persistence::ClockWatermarkStore as _;
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
    ///
    /// # Errors
    /// Returns the path, schema, clock, retention and storage failures described by
    /// [`Self::open_with_config`].
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        Self::open_with_config(RedbStoreConfig::new(root))
    }

    /// Opens or creates a durable local store from explicit bounds and hooks.
    ///
    /// Validates the owned paths and exact schema, checks startup time against the
    /// watermark, and restores the configured security audit retention bound. Older/future
    /// schemas and unsafe paths fail; this operation performs no storage migration.
    /// It may initialize storage or trim audit records before returning, so an error does
    /// not imply an untouched directory. Reopening uses those same durable records.
    ///
    /// Workflow recovery and full historical integrity scans are separate operations.
    ///
    /// # Errors
    /// Refuses invalid bounds, unsafe paths, inspection-only clones, unsupported/corrupt
    /// storage and clock rollback. Returns filesystem, database, injected-clock/hook and
    /// retention failures; earlier durable initialization or audit trimming may already have committed.
    #[tracing::instrument(
        name = "milkdrift.redb_store.open",
        skip_all,
        fields(storage_schema = STORAGE_SCHEMA_VERSION)
    )]
    pub fn open_with_config(config: RedbStoreConfig) -> Result<Self, PersistenceError> {
        validate_config(&config)?;
        let startup_observation = config.clock.now()?;
        prepare_owned_directory(&config.root, "storage root")?;
        crate::offline::refuse_inspection_clone(&config.root)?;
        let database_path = config.root.join(DATABASE_FILENAME);
        ensure_regular_file_or_absent(&database_path, "storage database")?;
        crate::offline::preflight_format(&config.root)?;
        let database = Database::create(&database_path).map_err(error::database)?;

        if database_is_uninitialized(&database)? {
            initialize_schema(&database, config.faults.as_ref(), startup_observation)?;
            sync_owned_directory(&config.root)?;
        } else {
            validate_schema(&database)?;
        }

        let artifact_root = config.root.join(ARTIFACT_DIRECTORY);
        let temp_root = artifact_root.join(TEMP_DIRECTORY);
        prepare_owned_directory(&artifact_root, "artifact root")?;
        prepare_owned_directory(&temp_root, "artifact temporary directory")?;

        let store = Self {
            workflow_limits: config.workflow_limits,
            database,
            root: config.root,
            artifact_root,
            temp_root,
            max_artifact_bytes: config.max_artifact_bytes,
            max_total_artifact_bytes: config.max_total_artifact_bytes,
            max_read_bytes: config.max_read_bytes,
            max_security_audit_records: config.max_security_audit_records,
            max_active_installations: config.max_active_installations,
            max_pending_evaluations: config.max_pending_evaluations,
            faults: config.faults,
            clock: config.clock,
            artifact_serialization: Mutex::new(()),
        };
        require_accepted_clock(store.observe_clock(startup_observation)?)?;
        store.reestablish_audit_retention_bound()?;
        Ok(store)
    }

    pub(crate) const fn database(&self) -> &Database {
        &self.database
    }
}

pub(crate) fn validate_config(config: &RedbStoreConfig) -> Result<(), PersistenceError> {
    if config.workflow_limits.history_bytes == 0
        || config.workflow_limits.receipt_bytes == 0
        || config.workflow_limits.receipt_entries == 0
    {
        return Err(PersistenceError::Bounds {
            location: "workflow_storage_limits",
            reason: "task history and receipt capacities must be nonzero".to_owned(),
        });
    }
    if config.max_active_installations == 0 || config.max_pending_evaluations == 0 {
        return Err(PersistenceError::Bounds {
            location: "redb_store_config",
            reason: "active installation and pending evaluation bounds must be nonzero".to_owned(),
        });
    }
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
    if config.max_security_audit_records == 0 {
        return Err(PersistenceError::Bounds {
            location: "security_audit",
            reason: "security audit retention must be nonzero".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn database_is_uninitialized(database: &Database) -> Result<bool, PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    Ok(read.list_tables().map_err(error::redb)?.next().is_none()
        && read
            .list_multimap_tables()
            .map_err(error::redb)?
            .next()
            .is_none())
}
