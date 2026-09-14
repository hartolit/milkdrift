//! Inspect or preserve a stopped generation using operating-system file-owner authority.
//!
//! redb 2.6 opens databases for writing, even for read transactions. This owner instead
//! locks a read-only source handle using redb's own file lock and copies the database to
//! a private temporary directory. Only that copy can receive redb housekeeping writes.
//! No runtime, clock observation, retention, schema initialization, or repair runs.
//! Artifacts are read from the locked source. Source contents and modification times
//! remain unchanged; access times and filesystem audit observations are outside this
//! guarantee. All supported writers must retain the database lock for their lifetime.

mod backup;
mod paths;
#[cfg(any(test, feature = "test-admin"))]
pub(crate) use paths::private_test_directory;
mod records;
#[cfg(test)]
mod tests;

pub use backup::{BackupManifest, BackupProducer};
pub use records::{InspectionFamily, InspectionPage, InspectionRecord, StorageOverview};

use milkdrift_blueprint::{BlueprintRevision, RevisionId};
use milkdrift_persistence::{
    ArtifactStore, IntegrityScanRequest, IntegrityScanResult, PersistenceError, RevisionStore,
    RunQueryStore, StorageAdmin,
};
use milkdrift_workspace::ArtifactReference;
use redb::{Database, StorageBackend, backends::FileBackend};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::{
    RedbStore, RedbStoreConfig, error,
    store::{ARTIFACT_DIRECTORY, DATABASE_FILENAME, TEMP_DIRECTORY},
};

/// Every physical clone carries this guard, which ordinary store opening refuses.
pub const INSPECTION_MARKER: &str = "milkdrift-inspection-only.json";
/// Daemon-owned invocation materialization component preserved by complete-generation
/// backups. It may contain sensitive unfinished process work and is never exported in diagnostics.
pub const EXECUTION_DIRECTORY: &str = "execution";
const COPY_CHUNK_BYTES: usize = 1024 * 1024;
/// Maximum database copied for inspection. Larger generations require a separately
/// reviewed operational limit; reaching this ceiling never yields a partial inspection.
const MAX_OFFLINE_DATABASE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// An exclusive offline read session. Drop it before restarting the source daemon.
/// The embedded mutable store never escapes this read-only facade.
pub struct OfflineStore {
    // Drop redb before deleting its temporary directory, especially on Windows.
    store: RedbStore,
    _scratch: tempfile::TempDir,
    source: FileBackend,
    root: PathBuf,
    database_digest: String,
    database_bytes: u64,
}

impl OfflineStore {
    /// Locks an existing source and creates a verified private database copy. Missing,
    /// unsupported, unsafe-path and already-open stores fail without changing the source.
    /// `scratch_parent` must be a private existing directory owned by the operator.
    pub fn open(root: &Path, scratch_parent: &Path) -> Result<Self, PersistenceError> {
        paths::private_directory(scratch_parent)?;
        paths::directory(root)?;
        let root = fs::canonicalize(root).map_err(error::io)?;
        let scratch_parent = fs::canonicalize(scratch_parent).map_err(error::io)?;
        if scratch_parent.starts_with(&root) {
            return Err(error::corruption(
                "inspection scratch must be outside the source root",
            ));
        }
        let source_file = paths::read_file(&root.join(DATABASE_FILENAME))?;
        let source = FileBackend::new(source_file).map_err(error::database)?;
        let database_bytes = source.len().map_err(error::io)?;
        if database_bytes == 0 || database_bytes > MAX_OFFLINE_DATABASE_BYTES {
            return Err(error::corruption(
                "offline database is empty or exceeds the 64 GiB copy ceiling",
            ));
        }
        let scratch = tempfile::Builder::new()
            .prefix("milkdrift-inspection-")
            .tempdir_in(scratch_parent)
            .map_err(error::io)?;
        // A crashed inspection can leave scratch behind. Guard that incomplete
        // database clone before copying any accepted history into it.
        backup::write_guard(scratch.path(), "private-scratch-not-a-backup")?;
        let copy = scratch.path().join(DATABASE_FILENAME);
        let database_digest = copy_database(&source, &copy, database_bytes)?;
        let database = Database::open(&copy).map_err(error::database)?;
        // Unlike normal open, compatibility checking must not scan all hot receipts.
        crate::store::schema::validate_format(&database)?;
        let config = RedbStoreConfig::new(&root);
        let artifact_root = root.join(ARTIFACT_DIRECTORY);
        paths::directory(&artifact_root)?;
        let temp_root = artifact_root.join(TEMP_DIRECTORY);
        paths::directory(&temp_root)?;
        let store = RedbStore {
            database,
            root: root.clone(),
            artifact_root,
            temp_root,
            max_artifact_bytes: config.max_artifact_bytes,
            max_total_artifact_bytes: config.max_total_artifact_bytes,
            max_read_bytes: config.max_read_bytes,
            hot_application_receipt_bound: config.hot_application_receipt_bound,
            application_receipt_archive_batch_size: config.application_receipt_archive_batch_size,
            max_security_audit_records: config.max_security_audit_records,
            faults: config.faults,
            clock: config.clock,
            artifact_serialization: Mutex::new(()),
        };
        Ok(Self {
            store,
            _scratch: scratch,
            source,
            root,
            database_digest,
            database_bytes,
        })
    }

    /// Source database fingerprint, taken before redb opens the private copy. This is
    /// provenance for this observation, not a new workflow or authenticated store identity.
    pub fn database_digest(&self) -> &str {
        &self.database_digest
    }

    /// Read-only, bounded journal discovery and event queries. No execution is implied.
    pub fn queries(&self) -> &dyn RunQueryStore {
        &self.store
    }

    /// Reads the exact immutable governing revision for a historical attempt.
    pub fn revision(&self, id: &RevisionId) -> Result<Option<BlueprintRevision>, PersistenceError> {
        self.store.revision(id)
    }

    /// Reads one bounded, digest-verified artifact under OS-owner authority. Callers
    /// must redact protected evidence before exporting diagnostics. No workflow grant
    /// or fabricated actor is used by offline administration.
    pub fn artifact_bytes(
        &self,
        reference: &ArtifactReference,
        maximum: u64,
    ) -> Result<Vec<u8>, PersistenceError> {
        if reference.size_bytes() > maximum || maximum > self.store.max_read_bytes {
            return Err(PersistenceError::Bounds {
                location: "offline_artifact",
                reason: "artifact exceeds the selected read bound".into(),
            });
        }
        let metadata = self.store.metadata(reference.artifact())?.ok_or_else(|| {
            PersistenceError::Storage {
                class: milkdrift_persistence::StorageFailureClass::Unavailable,
                message: "artifact metadata is unavailable".into(),
            }
        })?;
        if metadata.reference() != reference {
            return Err(error::corruption(
                "artifact metadata contradicts the frozen reference",
            ));
        }
        let path = self.store.content_path(reference.digest());
        let mut file = paths::read_file(&path)?;
        let mut bytes = Vec::new();
        use std::io::Read as _;
        std::io::Read::by_ref(&mut file)
            .take(maximum.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(error::io)?;
        if bytes.len() as u64 != reference.size_bytes()
            || milkdrift_workspace::ContentDigest::for_bytes(&bytes) != reference.digest()
        {
            return Err(error::corruption(
                "artifact content is missing, truncated or corrupt",
            ));
        }
        Ok(bytes)
    }

    /// Continues the existing integrity scanner without repair. Content hashing is
    /// an explicit request choice and remains bound into its resumable cursor.
    pub fn scan_integrity(
        &self,
        request: IntegrityScanRequest,
    ) -> Result<IntegrityScanResult, PersistenceError> {
        self.store.scan_integrity(request)
    }
}

pub(crate) fn refuse_inspection_clone(root: &Path) -> Result<(), PersistenceError> {
    match fs::symlink_metadata(root.join(INSPECTION_MARKER)) {
        Ok(_) => Err(error::corruption(
            "inspection-only generation: ordinary execution is blocked; preserve the marker until explicit offline activation after fencing the original and reviewing outstanding effects",
        )),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(error::io(cause)),
    }
}

fn copy_database(
    source: &FileBackend,
    target: &Path,
    length: u64,
) -> Result<String, PersistenceError> {
    let mut output = paths::new_file(target)?;
    let mut digest = blake3::Hasher::new();
    let mut offset = 0;
    while offset < length {
        let count = (length - offset).min(COPY_CHUNK_BYTES as u64) as usize;
        let bytes = source.read(offset, count).map_err(error::io)?;
        output.write_all(&bytes).map_err(error::io)?;
        digest.update(&bytes);
        offset += count as u64;
    }
    output.sync_all().map_err(error::io)?;
    let digest = digest.finalize().to_hex().to_string();
    if paths::hash_file(target)?.1 != digest {
        return Err(error::corruption(
            "database copy digest verification failed",
        ));
    }
    Ok(digest)
}
