use super::*;
pub(crate) const DATABASE_FILENAME: &str = "milkdrift.redb";
pub(crate) const ARTIFACT_DIRECTORY: &str = "artifacts";
pub(crate) const TEMP_DIRECTORY: &str = ".tmp";
pub(crate) const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 1_073_741_824;
pub(crate) const DEFAULT_MAX_TOTAL_ARTIFACT_BYTES: u64 = 10_737_418_240;
pub(crate) const DEFAULT_MAX_READ_BYTES: u64 = 1_073_741_824;
pub(crate) const DEFAULT_HOT_APPLICATION_RECEIPTS: u32 = 10_000;
pub(crate) const DEFAULT_APPLICATION_RECEIPT_ARCHIVE_BATCH_SIZE: u32 = 256;
pub(crate) const DEFAULT_MAX_SECURITY_AUDIT_RECORDS: u32 = 100_000;

/// Injected boundary clock for artifact-publication facts that control cleanup.
pub trait ArtifactClock: Send + Sync {
    /// Returns the timestamp durably recorded for a newly accepted publication.
    fn now(&self) -> Result<TimestampMillis, PersistenceError>;
}

/// Production artifact clock backed by the host system clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemArtifactClock;

impl ArtifactClock for SystemArtifactClock {
    fn now(&self) -> Result<TimestampMillis, PersistenceError> {
        let duration = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|cause| error::corruption(format!("system clock precedes epoch: {cause}")))?;
        let millis = u64::try_from(duration.as_millis())
            .map_err(|_| error::corruption("system timestamp exceeds u64 milliseconds"))?;
        Ok(TimestampMillis::new(millis))
    }
}

/// Bounded local-store configuration.
pub struct RedbStoreConfig {
    pub(crate) root: PathBuf,
    pub(crate) max_artifact_bytes: u64,
    pub(crate) max_total_artifact_bytes: u64,
    pub(crate) max_read_bytes: u64,
    pub(crate) hot_application_receipt_bound: u32,
    pub(crate) application_receipt_archive_batch_size: u32,
    pub(crate) max_security_audit_records: u32,
    pub(crate) faults: Arc<dyn FaultInjector>,
    pub(crate) artifact_clock: Arc<dyn ArtifactClock>,
}

impl fmt::Debug for RedbStoreConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedbStoreConfig")
            .field("root", &self.root)
            .field("max_artifact_bytes", &self.max_artifact_bytes)
            .field("max_total_artifact_bytes", &self.max_total_artifact_bytes)
            .field("max_read_bytes", &self.max_read_bytes)
            .field(
                "hot_application_receipt_bound",
                &self.hot_application_receipt_bound,
            )
            .field(
                "application_receipt_archive_batch_size",
                &self.application_receipt_archive_batch_size,
            )
            .field(
                "max_security_audit_records",
                &self.max_security_audit_records,
            )
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
            hot_application_receipt_bound: DEFAULT_HOT_APPLICATION_RECEIPTS,
            application_receipt_archive_batch_size: DEFAULT_APPLICATION_RECEIPT_ARCHIVE_BATCH_SIZE,
            max_security_audit_records: DEFAULT_MAX_SECURITY_AUDIT_RECORDS,
            faults: no_faults(),
            artifact_clock: Arc::new(SystemArtifactClock),
        }
    }

    /// Applies the bounded hot receipt lifecycle; cold exact replay remains lifetime durable.
    #[must_use]
    pub fn with_application_receipt_lifecycle(
        mut self,
        hot_receipt_bound: u32,
        archive_batch_size: u32,
    ) -> Self {
        self.hot_application_receipt_bound = hot_receipt_bound;
        self.application_receipt_archive_batch_size = archive_batch_size;
        self
    }

    /// Applies the independent retained security-audit prefix bound.
    #[must_use]
    pub fn with_security_audit_limit(mut self, max_security_audit_records: u32) -> Self {
        self.max_security_audit_records = max_security_audit_records;
        self
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

    /// Installs the deterministic clock used for accepted publication timestamps.
    #[must_use]
    pub fn with_artifact_clock(mut self, clock: Arc<dyn ArtifactClock>) -> Self {
        self.artifact_clock = clock;
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
    pub(crate) hot_application_receipt_bound: u32,
    pub(crate) application_receipt_archive_batch_size: u32,
    pub(crate) max_security_audit_records: u32,
    pub(crate) faults: Arc<dyn FaultInjector>,
    pub(crate) artifact_clock: Arc<dyn ArtifactClock>,
    pub(crate) artifact_serialization: Mutex<()>,
}
