//! Complete, create-new generation copies, with completion recorded last.
use super::{EXECUTION_DIRECTORY, INSPECTION_MARKER, OfflineStore, copy_database, paths};
use crate::{
    error,
    schema::{INTERNAL_DOCUMENT_FORMAT_VERSION, STORAGE_SCHEMA_VERSION},
    store::{ARTIFACT_DIRECTORY, DATABASE_FILENAME},
};
use milkdrift_persistence::{IntegrityScanRequest, PageSize, PersistenceError};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

const MANIFEST: &str = "milkdrift-backup.json";
const MAX_FILES: usize = 100_000;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAX_COPY_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const MAX_SCAN_PAGES: usize = 100_000;

/// Producer identity supplied by the application. No credential/configuration files
/// are embedded: the binary digest pins a locally built binary, including dirty builds.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupProducer {
    /// Nonempty product version, at most 128 bytes.
    pub version: String,
    /// BLAKE3 digest of the actual producing executable, in lowercase hexadecimal.
    pub binary_digest: String,
    /// Nonempty compile-time source revision or explicit unavailability, at most 256 bytes.
    pub source_revision: String,
}

impl BackupProducer {
    fn validate(&self) -> Result<(), PersistenceError> {
        if self.version.is_empty()
            || self.version.len() > 128
            || self.source_revision.is_empty()
            || self.source_revision.len() > 256
            || milkdrift_workspace::ContentDigest::from_hex(&self.binary_digest).is_err()
        {
            return Err(error::corruption(
                "backup producer identity is invalid or unbounded",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    path: String,
    bytes: u64,
    blake3: String,
}

/// A completed physical copy, verified by compatible readers without runtime recovery.
/// The manifest is bounded to 100,000 files and 32 MiB. It establishes copy integrity,
/// not authenticity, external-effect settlement, or permission to run the clone.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    /// Backup envelope schema, independent of redb formats.
    pub schema_version: u32,
    /// True only in the final manifest written after copy and reader verification.
    pub complete: bool,
    /// Observed physical schema.
    pub storage_schema: u64,
    /// Exact internal document reader format.
    pub document_format: u64,
    /// Database fingerprint of the preserved source, before housekeeping.
    pub source_database_digest: String,
    /// Original producing binary; restore preserves this provenance.
    pub producer: BackupProducer,
    /// Exact durable clock watermark; restore never lowers it.
    pub clock_high_water_unix_ms: u64,
    /// Documents examined by the explicit compatible-reader integrity scan.
    pub documents_checked: u64,
    /// Artifact contents examined by that scan.
    pub artifacts_checked: u64,
    /// Total failed checks. A nonzero value means preservation of damaged evidence,
    /// not a healthy or execution-ready generation.
    pub integrity_failures: u64,
    files: Vec<FileEntry>,
    directories: Vec<String>,
}

impl OfflineStore {
    /// Copies the complete closed storage generation to a new directory below a
    /// private parent. The held source lock excludes a supported writer throughout.
    /// Unknown root components, links, limits and failed copies are refused. A partial
    /// destination keeps its execution guard and lacks a completion manifest.
    pub fn backup(
        &self,
        destination: &Path,
        scratch_parent: &Path,
        producer: BackupProducer,
    ) -> Result<BackupManifest, PersistenceError> {
        producer.validate()?;
        let (directories, mut files) = inventory(&self.root)?;
        prepare_destination(destination, &self.root)?;
        write_guard(destination, &self.database_digest)?;
        for directory in &directories {
            paths::create_directory(&destination.join(directory))?;
        }
        for entry in &mut files {
            let target = destination.join(&entry.path);
            if entry.path == DATABASE_FILENAME {
                entry.blake3 = copy_database(&self.source, &target, self.database_bytes)?;
            } else {
                let mut source = paths::read_file(&self.root.join(&entry.path))?;
                let mut output = paths::new_file(&target)?;
                let copied = std::io::copy(
                    &mut Read::by_ref(&mut source).take(entry.bytes.saturating_add(1)),
                    &mut output,
                )
                .map_err(error::io)?;
                if copied != entry.bytes {
                    return Err(error::corruption("source file changed during backup"));
                }
                output.sync_all().map_err(error::io)?;
                let original = paths::hash_file(&self.root.join(&entry.path))?;
                let copied = paths::hash_file(&target)?;
                if original != copied {
                    return Err(error::corruption("backup file digest mismatch"));
                }
                entry.blake3 = copied.1;
            }
            #[cfg(test)]
            if COPY_INTERRUPT.with(|hook| hook.get()) {
                return Err(error::corruption("injected interrupted copy"));
            }
        }
        let verified = OfflineStore::open(destination, scratch_parent)?;
        let overview = verified.overview()?;
        let (documents_checked, artifacts_checked, integrity_failures) = verify_readers(&verified)?;
        drop(verified);
        let manifest = BackupManifest {
            schema_version: 1,
            complete: true,
            storage_schema: STORAGE_SCHEMA_VERSION,
            document_format: INTERNAL_DOCUMENT_FORMAT_VERSION,
            source_database_digest: self.database_digest.clone(),
            producer,
            clock_high_water_unix_ms: overview.clock_high_water_unix_ms,
            documents_checked,
            artifacts_checked,
            integrity_failures,
            files,
            directories,
        };
        write_manifest(destination, &manifest)?;
        Ok(manifest)
    }

    /// Verifies a completed backup's bounded manifest, exact inventory, all file
    /// digests and current readers. It acquires the same exclusive source ownership
    /// as inspection. A missing completion marker is never accepted as a backup.
    pub fn verify_backup(
        root: &Path,
        scratch_parent: &Path,
    ) -> Result<BackupManifest, PersistenceError> {
        let store = Self::open(root, scratch_parent)?;
        verify_manifest(&store)
    }

    /// Restores a verified backup into a create-new isolated directory. The result
    /// retains its inspection-only guard and every replay/clock/account fact.
    pub fn restore(
        root: &Path,
        destination: &Path,
        scratch_parent: &Path,
    ) -> Result<BackupManifest, PersistenceError> {
        let store = Self::open(root, scratch_parent)?;
        let manifest = verify_manifest(&store)?;
        store.backup(destination, scratch_parent, manifest.producer)
    }
}

fn prepare_destination(destination: &Path, source: &Path) -> Result<(), PersistenceError> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| error::corruption("backup destination requires an explicit parent"))?;
    paths::private_directory(parent)?;
    let parent = fs::canonicalize(parent).map_err(error::io)?;
    if parent.starts_with(source) {
        return Err(error::corruption(
            "backup destination must be outside the source",
        ));
    }
    paths::create_directory(destination)
}

pub(super) fn write_guard(root: &Path, digest: &str) -> Result<(), PersistenceError> {
    let mut file = paths::new_file(&root.join(INSPECTION_MARKER))?;
    file.write_all(
        format!("{{\"inspection_only\":true,\"source_database_blake3\":\"{digest}\"}}\n")
            .as_bytes(),
    )
    .map_err(error::io)?;
    file.sync_all().map_err(error::io)?;
    crate::store::sync_owned_directory(root)
}

fn write_manifest(root: &Path, manifest: &BackupManifest) -> Result<(), PersistenceError> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(error::corruption("backup manifest exceeds 32 MiB"));
    }
    // The final path is a completion marker. A crash while writing it is detected by
    // strict JSON parsing; no reader treats an incomplete JSON document as complete.
    let mut file = paths::new_file(&root.join(MANIFEST))?;
    file.write_all(&bytes).map_err(error::io)?;
    file.sync_all().map_err(error::io)?;
    crate::store::sync_owned_directory(root)
}

fn inventory(root: &Path) -> Result<(Vec<String>, Vec<FileEntry>), PersistenceError> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut pending = vec![String::new()];
    let mut total = 0u64;
    while let Some(relative) = pending.pop() {
        for child in fs::read_dir(root.join(&relative)).map_err(error::io)? {
            let child = child.map_err(error::io)?;
            let name = child
                .file_name()
                .into_string()
                .map_err(|_| error::corruption("backup path is not UTF-8"))?;
            if relative.is_empty() && [MANIFEST, INSPECTION_MARKER].contains(&name.as_str()) {
                continue;
            }
            if relative.is_empty()
                && ![DATABASE_FILENAME, ARTIFACT_DIRECTORY, EXECUTION_DIRECTORY]
                    .contains(&name.as_str())
            {
                return Err(error::corruption(
                    "unknown data-root component; preserve separately and use compatible readers; backup never imports credentials or working files",
                ));
            }
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            paths::relative(&path)?;
            if directories.len() + files.len() >= MAX_FILES {
                return Err(error::corruption("backup exceeds 100000 files/directories"));
            }
            let metadata = fs::symlink_metadata(child.path()).map_err(error::io)?;
            if metadata.is_dir() {
                paths::directory(&child.path())?;
                let maximum_depth = if path.starts_with(EXECUTION_DIRECTORY) {
                    32
                } else {
                    3
                };
                if path.matches('/').count() >= maximum_depth {
                    return Err(error::corruption(
                        "data-root directory depth exceeds backup policy",
                    ));
                }
                directories.push(path.clone());
                pending.push(path);
            } else {
                drop(paths::read_file(&child.path())?);
                total = total
                    .checked_add(metadata.len())
                    .ok_or_else(|| error::corruption("backup size overflow"))?;
                if total > MAX_COPY_BYTES {
                    return Err(error::corruption("backup exceeds 1 TiB"));
                }
                files.push(FileEntry {
                    path,
                    bytes: metadata.len(),
                    blake3: String::new(),
                });
            }
        }
    }
    directories.sort();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((directories, files))
}

fn verify_manifest(store: &OfflineStore) -> Result<BackupManifest, PersistenceError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Guard {
        inspection_only: bool,
        source_database_blake3: String,
    }
    let mut guard = Vec::new();
    paths::read_file(&store.root.join(INSPECTION_MARKER))?
        .take(1025)
        .read_to_end(&mut guard)
        .map_err(error::io)?;
    if guard.len() > 1024 {
        return Err(error::corruption(
            "backup execution guard exceeds its bound",
        ));
    }
    let guard: Guard = serde_json::from_slice(&guard)?;
    if !guard.inspection_only || guard.source_database_blake3 != store.database_digest {
        return Err(error::corruption(
            "backup inspection guard is missing or contradicts its source",
        ));
    }
    let mut bytes = Vec::new();
    paths::read_file(&store.root.join(MANIFEST))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(error::io)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(error::corruption("backup manifest exceeds 32 MiB"));
    }
    let manifest: BackupManifest = serde_json::from_slice(&bytes)?;
    manifest.producer.validate()?;
    if manifest.schema_version != 1
        || !manifest.complete
        || manifest.storage_schema != STORAGE_SCHEMA_VERSION
        || manifest.document_format != INTERNAL_DOCUMENT_FORMAT_VERSION
    {
        return Err(error::corruption(
            "backup is incomplete or has unsupported formats",
        ));
    }
    if manifest.source_database_digest != store.database_digest {
        return Err(error::corruption(
            "backup source database fingerprint mismatch",
        ));
    }
    let (directories, files) = inventory(&store.root)?;
    if directories != manifest.directories || files.len() != manifest.files.len() {
        return Err(error::corruption(
            "backup inventory differs from completion manifest",
        ));
    }
    for (actual, expected) in files.iter().zip(&manifest.files) {
        paths::relative(&expected.path)?;
        if actual.path != expected.path || actual.bytes != expected.bytes {
            return Err(error::corruption("backup file inventory mismatch"));
        }
        let digest = if actual.path == DATABASE_FILENAME {
            store.database_digest.clone()
        } else {
            paths::hash_file(&store.root.join(&actual.path))?.1
        };
        if digest != expected.blake3 {
            return Err(error::corruption("backup file digest mismatch"));
        }
    }
    if store.overview()?.clock_high_water_unix_ms != manifest.clock_high_water_unix_ms {
        return Err(error::corruption(
            "backup durable clock differs from manifest",
        ));
    }
    let (documents, artifacts, failures) = verify_readers(store)?;
    if (documents, artifacts, failures)
        != (
            manifest.documents_checked,
            manifest.artifacts_checked,
            manifest.integrity_failures,
        )
    {
        return Err(error::corruption(
            "backup compatible-reader verification differs from manifest",
        ));
    }
    Ok(manifest)
}

fn verify_readers(store: &OfflineStore) -> Result<(u64, u64, u64), PersistenceError> {
    let mut cursor = None;
    let (mut documents, mut artifacts, mut failures) = (0, 0, 0);
    for _ in 0..MAX_SCAN_PAGES {
        let page = store.scan_integrity(IntegrityScanRequest {
            limit: PageSize::new(256)?,
            verify_artifact_content: true,
            cursor,
        })?;
        documents += page.documents_checked;
        artifacts += page.artifacts_checked;
        failures += page.failures.len() as u64;
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok((documents, artifacts, failures));
        }
    }
    Err(error::corruption(
        "backup reader verification exceeded 100000 pages; copy remains incomplete",
    ))
}

#[cfg(test)]
thread_local! { pub(super) static COPY_INTERRUPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
