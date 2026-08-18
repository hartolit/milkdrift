use crate::{BoundedDetail, PageSize, PersistenceError, TimestampMillis};

/// Current physical persistence schema expected by adapters.
pub const STORAGE_SCHEMA_VERSION_V1: u32 = 1;

/// Compatibility of durable physical schema with this binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageSchemaCompatibility {
    /// Exact current schema; ordinary access is allowed.
    Current,
    /// An older supported schema requires explicit migration before ordinary writes.
    MigrationRequired,
    /// A newer schema must be refused rather than interpreted.
    FutureUnsupported,
}

/// Physical schema facts without adapter/database implementation types.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageSchemaInfo {
    /// Version observed durably.
    pub stored_version: u32,
    /// Version implemented by this binary.
    pub current_version: u32,
    /// Safe compatibility classification.
    pub compatibility: StorageSchemaCompatibility,
}

/// Stable storage health status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageHealthStatus {
    /// Schema and sampled integrity checks are healthy.
    Healthy,
    /// Storage remains readable but a component needs attention.
    Degraded,
    /// Storage was opened read-only after a safe refusal.
    ReadOnly,
    /// Explicit migration is required before ordinary use.
    MigrationRequired,
}

/// One bounded adapter-neutral component observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageComponentHealth {
    /// Stable component family, such as `journal` or `artifact_content`.
    pub component: BoundedDetail,
    /// Component status.
    pub status: StorageHealthStatus,
    /// Redacted bounded observation.
    pub detail: BoundedDetail,
}

/// Immutable health response suitable for a future control API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageHealth {
    /// Overall safe status.
    pub status: StorageHealthStatus,
    /// Physical schema facts.
    pub schema: StorageSchemaInfo,
    /// Boundary-clock observation supplied by the caller.
    pub observed_at: TimestampMillis,
    /// Bounded component detail.
    pub components: Vec<StorageComponentHealth>,
}

/// Bounded incremental integrity scan request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrityScanRequest {
    /// Maximum records/blobs examined in this call.
    pub limit: PageSize,
    /// Whether artifact bytes should be rehashed, not merely metadata-checked.
    pub verify_artifact_content: bool,
}

/// Integrity scan report; corruption remains explicit and never becomes absence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityScanResult {
    /// Number of durable documents checked.
    pub documents_checked: u64,
    /// Number of artifact blobs checked.
    pub artifacts_checked: u64,
    /// Bounded corruption observations.
    pub failures: Vec<StorageComponentHealth>,
    /// Whether more candidates remain for a later bounded scan.
    pub more_remaining: bool,
}

/// Narrow lifecycle/schema/health port for a durable adapter.
pub trait StorageAdmin: Send + Sync {
    /// Returns physical schema compatibility. A future version must not be opened as v1.
    fn schema_info(&self) -> Result<StorageSchemaInfo, PersistenceError>;

    /// Explicitly migrates a supported older schema to the current exact version.
    ///
    /// The implementation verifies `expected_from`, applies each migration atomically
    /// or with durable restart markers, and refuses unknown future schemas.
    fn migrate_to_current(&self, expected_from: u32)
    -> Result<StorageSchemaInfo, PersistenceError>;

    /// Returns bounded health information without mutating/repairing durable history.
    fn health(&self, observed_at: TimestampMillis) -> Result<StorageHealth, PersistenceError>;

    /// Performs bounded read-only integrity verification.
    fn scan_integrity(
        &self,
        request: IntegrityScanRequest,
    ) -> Result<IntegrityScanResult, PersistenceError>;
}
