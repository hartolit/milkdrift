use super::*;
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
        let database = Database::create(&database_path).map_err(error::database)?;

        if database_is_uninitialized(&database)? {
            initialize_schema(&database, config.faults.as_ref())?;
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
            artifact_clock: config.artifact_clock,
            artifact_serialization: Mutex::new(()),
        })
    }

    pub(crate) const fn database(&self) -> &Database {
        &self.database
    }
}

pub(crate) fn validate_config(config: &RedbStoreConfig) -> Result<(), PersistenceError> {
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

pub(crate) fn database_is_uninitialized(database: &Database) -> Result<bool, PersistenceError> {
    let read = database.begin_read().map_err(error::redb)?;
    let mut tables = read.list_tables().map_err(error::redb)?;
    Ok(tables.next().is_none())
}
