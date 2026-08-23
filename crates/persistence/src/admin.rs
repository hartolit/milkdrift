use crate::{BoundedDetail, PageSize, PersistenceError, TimestampMillis};

/// Current physical persistence schema expected by adapters.
pub const STORAGE_SCHEMA_VERSION_V1: u32 = 1;
/// Maximum opaque key bytes retained by one resumable integrity-scan cursor.
pub const MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES: usize = 512;

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
    /// Schema and the bounded checks actually sampled are healthy. This is not a
    /// complete historical or artifact-content integrity proof.
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

/// Closed durable-record family traversed by an integrity scan.
///
/// The ordering is part of the cursor contract: revisions precede run events,
/// which precede artifact metadata/content, derived-index consistency, and finally
/// the authenticated catalogs that prove physical membership and absence.
/// Adapter-specific table identities do not cross this boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IntegrityScanFamily {
    /// Immutable workflow revision documents.
    Revisions,
    /// Append-only run-event envelopes and their integrity indexes.
    RunEvents,
    /// Artifact metadata and, when requested, content bytes.
    Artifacts,
    /// Authoritative run heads and rebuildable discovery/accounting indexes.
    Indexes,
    /// Root-bound authenticated membership catalogs and their physical documents.
    AuthenticatedCatalogs,
}

/// Stable exclusive resume point for one bounded integrity scan.
///
/// `after_key` is an opaque, adapter-defined, lexicographically ordered key. It
/// is bounded and can only be constructed through validation, so persistence
/// callers can retain and return it without learning physical table details. The
/// artifact-content verification mode is bound into the cursor so a continuation
/// cannot silently weaken or strengthen the logical scan halfway through.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityScanCursor {
    family: IntegrityScanFamily,
    after_key: Vec<u8>,
    verify_artifact_content: bool,
}

impl IntegrityScanCursor {
    /// Constructs a validated exclusive resume point.
    pub fn new(
        family: IntegrityScanFamily,
        after_key: Vec<u8>,
        verify_artifact_content: bool,
    ) -> Result<Self, PersistenceError> {
        if after_key.is_empty() || after_key.len() > MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES {
            return Err(PersistenceError::InvalidCursor(format!(
                "integrity cursor key must contain 1..={MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES} bytes"
            )));
        }
        Ok(Self {
            family,
            after_key,
            verify_artifact_content,
        })
    }

    /// Durable-record family containing the exclusive resume key.
    #[must_use]
    pub const fn family(&self) -> IntegrityScanFamily {
        self.family
    }

    /// Opaque adapter-defined exclusive resume key.
    #[must_use]
    pub fn after_key(&self) -> &[u8] {
        &self.after_key
    }

    /// Whether this logical scan verifies artifact content bytes.
    #[must_use]
    pub const fn verify_artifact_content(&self) -> bool {
        self.verify_artifact_content
    }
}

/// Bounded incremental integrity scan request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityScanRequest {
    /// Maximum records/blobs examined in this call.
    pub limit: PageSize,
    /// Whether artifact bytes should be rehashed, not merely metadata-checked.
    pub verify_artifact_content: bool,
    /// Exclusive cursor returned by the prior call; absent starts at revisions.
    pub cursor: Option<IntegrityScanCursor>,
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
    /// Exclusive resume point for the next bounded scan; absent when exhausted.
    pub next_cursor: Option<IntegrityScanCursor>,
}

/// Narrow lifecycle/schema/health port for a durable adapter.
pub trait StorageAdmin: Send + Sync {
    /// Returns physical schema compatibility. A future version must not be opened as v1.
    fn schema_info(&self) -> Result<StorageSchemaInfo, PersistenceError>;

    /// Returns bounded health information without mutating/repairing durable history.
    fn health(&self, observed_at: TimestampMillis) -> Result<StorageHealth, PersistenceError>;

    /// Performs one bounded page of an explicit, read-only administrative scrub.
    /// The returned cursor resumes the same authenticated root, family, and
    /// artifact-content verification mode; callers decide whether to continue.
    fn scan_integrity(
        &self,
        request: IntegrityScanRequest,
    ) -> Result<IntegrityScanResult, PersistenceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrity_cursor_is_bounded_and_opaque() -> Result<(), PersistenceError> {
        assert!(
            IntegrityScanCursor::new(IntegrityScanFamily::Revisions, Vec::new(), false).is_err()
        );
        assert!(
            IntegrityScanCursor::new(
                IntegrityScanFamily::RunEvents,
                vec![0; MAX_INTEGRITY_SCAN_CURSOR_KEY_BYTES + 1],
                false,
            )
            .is_err()
        );

        let cursor = IntegrityScanCursor::new(
            IntegrityScanFamily::Artifacts,
            vec![0, 0xff, b'/', b'\0'],
            true,
        )?;
        assert_eq!(cursor.family(), IntegrityScanFamily::Artifacts);
        assert_eq!(cursor.after_key(), &[0, 0xff, b'/', b'\0']);
        assert!(cursor.verify_artifact_content());
        Ok(())
    }
}
