use super::*;
pub(crate) const DATABASE_FILENAME: &str = "milkdrift.redb";
pub(crate) const ARTIFACT_DIRECTORY: &str = "artifacts";
pub(crate) const TEMP_DIRECTORY: &str = ".tmp";
pub(crate) const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 1_073_741_824;
pub(crate) const DEFAULT_MAX_TOTAL_ARTIFACT_BYTES: u64 = 10_737_418_240;
pub(crate) const DEFAULT_MAX_READ_BYTES: u64 = 1_073_741_824;

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
