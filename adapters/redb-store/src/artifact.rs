use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadChunk, ArtifactReadRequest, ArtifactStore,
    ArtifactWriteProgress, BeginArtifactOutcome, BeginArtifactPublication, CommitArtifactOutcome,
    OrphanCleanupCursor, OrphanCleanupFamily, OrphanCleanupRequest, OrphanCleanupResult,
    PersistenceError, StorageFailureClass, authorize_artifact_read,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, CausalReference, ContentDigest, RunId,
    WorkspaceUsage, WorkspaceValueEntry,
};
use redb::{ReadableTable, ReadableTableMetadata};
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        ARTIFACT_ACCOUNTING, ARTIFACT_DELETE_GUARDS, ARTIFACT_DIGEST_RESERVATIONS,
        ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_PATHS, ARTIFACT_PUBLICATIONS,
        ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS,
        ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, ROOT_SCOPES,
        RUN_ARTIFACT_OWNERSHIP, RUN_EVENTS, SCOPES, VALUES, WORKSPACE_USAGE,
    },
};

const PUBLICATION_SCHEMA_VERSION: u32 = 1;
const ARTIFACT_ACCOUNTING_SCHEMA_VERSION: u32 = 3;
pub(crate) const GLOBAL_ARTIFACT_BYTES_KEY: &str = "artifact_content_bytes";
const MAX_CHUNK_BYTES: usize = milkdrift_persistence::MAX_ARTIFACT_CHUNK_BYTES;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PublicationState {
    Writable,
    Committed { content_deduplicated: bool },
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PublicationRecord {
    schema_version: u32,
    publication: ArtifactPublicationId,
    run: milkdrift_workspace::RunId,
    metadata: ArtifactMetadata,
    budget: milkdrift_workspace::WorkspaceBudget,
    expected_usage: WorkspaceUsage,
    resulting_usage: WorkspaceUsage,
    created_at_millis: u64,
    state: PublicationState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArtifactAccountingRecord {
    pub(crate) schema_version: u32,
    pub(crate) committed_content_bytes: u64,
}

impl ArtifactAccountingRecord {
    pub(crate) const EMPTY: Self = Self {
        schema_version: ARTIFACT_ACCOUNTING_SCHEMA_VERSION,
        committed_content_bytes: 0,
    };
}

impl PublicationRecord {
    fn from_request(request: &BeginArtifactPublication, created_at_millis: u64) -> Self {
        Self {
            schema_version: PUBLICATION_SCHEMA_VERSION,
            publication: request.publication().clone(),
            run: request.run().clone(),
            metadata: request.metadata().clone(),
            budget: request.budget().clone(),
            expected_usage: request.expected_usage(),
            resulting_usage: request.resulting_usage(),
            created_at_millis,
            state: PublicationState::Writable,
        }
    }

    fn matches(&self, request: &BeginArtifactPublication) -> bool {
        self.publication == *request.publication()
            && self.run == *request.run()
            && self.metadata == *request.metadata()
            && self.budget == *request.budget()
            && self.expected_usage == request.expected_usage()
            && self.resulting_usage == request.resulting_usage()
    }
}

mod accounting;
mod cleanup;
mod path;
mod publication;

pub(crate) use accounting::{
    persist_artifact_reference_occurrence, persist_run_artifact_ownership, validate_artifact_state,
    validated_run_artifact_reference_in_transaction,
};
pub(crate) use path::verify_blob;

pub(crate) fn validate_publication_scrub(
    read: &redb::ReadTransaction,
    key: &str,
    bytes: &[u8],
) -> Result<(), PersistenceError> {
    let record = publication::decode_publication(bytes)?;
    if record.schema_version != PUBLICATION_SCHEMA_VERSION || record.publication.as_str() != key {
        return Err(error::corruption(
            "artifact publication key or schema disagrees with its document",
        ));
    }
    let age = read
        .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
        .map_err(error::redb)?;
    let reservations = read
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    let owners = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    let manifests = read
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    let digest_reservations = read
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;
    let paths = read.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let guards = read
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?;
    let age_key = path::publication_age_key(record.created_at_millis, &record.publication)?;
    let temp_name = path::publication_temp_name(&record.publication);
    let digest = record.metadata.reference().digest().to_hex();
    let digest_key = codec::pair(&digest, record.publication.as_str())?;
    let pending = path::artifact_path_entry(&record, path::ArtifactPathKind::TempPending)?;
    let ready = path::artifact_path_entry(&record, path::ArtifactPathKind::TempReady)?;
    let content = path::artifact_path_entry(&record, path::ArtifactPathKind::ContentIntent)?;
    let has_path = |entry: &path::ArtifactPathEntry| -> Result<bool, PersistenceError> {
        match paths
            .get(entry.storage_key.as_slice())
            .map_err(error::redb)?
        {
            None => Ok(false),
            Some(value) if value.value() == entry.logical_key.as_slice() => Ok(true),
            Some(_) => Err(error::corruption(
                "artifact path inventory value is mismatched",
            )),
        }
    };
    let pending = has_path(&pending)?;
    let ready = has_path(&ready)?;
    let content = has_path(&content)?;
    let age_owner = age
        .get(age_key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let run_owner = reservations
        .get(record.run.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let temp_owner = owners
        .get(temp_name.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let manifest = manifests
        .get(temp_name.as_str())
        .map_err(error::redb)?
        .map(|value| {
            json::decode::<ArtifactPublicationId>(value.value(), "artifact temporary manifest")
        })
        .transpose()?;
    let digest_reserved = digest_reservations
        .get(digest_key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value());
    let temp_guard = guards
        .get(
            path::artifact_delete_guard_key(path::ArtifactPathKind::TempReady, &temp_name)?
                .as_slice(),
        )
        .map_err(error::redb)?
        .map(|value| value.value());
    match record.state {
        PublicationState::Writable => {
            if age_owner.as_deref() != Some(key)
                || run_owner.as_deref() != Some(key)
                || temp_owner.as_deref() != Some(key)
                || manifest.as_ref() != Some(&record.publication)
                || digest_reserved != Some(1)
                || pending == ready
                || temp_guard.is_some()
            {
                return Err(error::corruption(
                    "writable publication has incomplete or conflicting coordination indexes",
                ));
            }
        }
        PublicationState::Committed { .. } => {
            if age_owner.is_some()
                || run_owner.as_deref() == Some(key)
                || temp_owner.is_some()
                || manifest.is_some()
                || digest_reserved.is_some()
                || content
                || (pending && ready)
                || (temp_guard.is_some() && !pending && !ready)
            {
                return Err(error::corruption(
                    "committed publication retains invalid writable or path coordination state",
                ));
            }
        }
        PublicationState::Released => {
            if age_owner.is_some()
                || run_owner.as_deref() == Some(key)
                || temp_owner.is_some()
                || manifest.is_some()
                || digest_reserved.is_some()
                || (pending && ready)
                || (!pending && !ready && !content)
                || (temp_guard.is_some() && !pending && !ready)
            {
                return Err(error::corruption(
                    "released publication has invalid residual path coordination state",
                ));
            }
        }
    }
    Ok(())
}

fn scrub_publication(
    read: &redb::ReadTransaction,
    publication: &str,
) -> Result<PublicationRecord, PersistenceError> {
    let publication = ArtifactPublicationId::new(publication)
        .map_err(|cause| error::corruption(format!("invalid artifact publication: {cause}")))?;
    let table = read
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let bytes = table
        .get(publication.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("artifact coordination index is dangling"))?;
    let record = publication::decode_publication(bytes.value())?;
    if record.publication != publication {
        return Err(error::corruption(
            "artifact coordination index names a mismatched publication",
        ));
    }
    Ok(record)
}

pub(crate) fn validate_publication_age_scrub(
    read: &redb::ReadTransaction,
    key: &[u8],
    publication: &str,
) -> Result<(), PersistenceError> {
    let record = scrub_publication(read, publication)?;
    if !matches!(record.state, PublicationState::Writable)
        || path::publication_age_key(record.created_at_millis, &record.publication)?.as_slice()
            != key
    {
        return Err(error::corruption(
            "publication-age index disagrees with its writable publication",
        ));
    }
    Ok(())
}

pub(crate) fn validate_publication_reservation_scrub(
    read: &redb::ReadTransaction,
    run: &str,
    publication: &str,
) -> Result<(), PersistenceError> {
    let record = scrub_publication(read, publication)?;
    if !matches!(record.state, PublicationState::Writable) || record.run.as_str() != run {
        return Err(error::corruption(
            "artifact run reservation disagrees with its writable publication",
        ));
    }
    Ok(())
}

pub(crate) fn validate_digest_reservation_scrub(
    read: &redb::ReadTransaction,
    key: &[u8],
    marker: u8,
) -> Result<(), PersistenceError> {
    let components = codec::decode_components(key, 2)?;
    let record = scrub_publication(read, components[1])?;
    if marker != 1
        || !matches!(record.state, PublicationState::Writable)
        || record.metadata.reference().digest().to_hex() != components[0]
    {
        return Err(error::corruption(
            "artifact digest reservation disagrees with its writable publication",
        ));
    }
    Ok(())
}

pub(crate) fn validate_path_scrub(
    read: &redb::ReadTransaction,
    key: &[u8],
    value: &[u8],
) -> Result<(), PersistenceError> {
    let entry = path::decode_artifact_path_entry(key, value)?;
    let record = scrub_publication(read, entry.publication.as_str())?;
    let expected = path::artifact_path_entry(&record, entry.kind)?;
    if expected != entry {
        return Err(error::corruption(
            "artifact path inventory disagrees with its publication",
        ));
    }
    match (&record.state, entry.kind) {
        (PublicationState::Writable, _) => {}
        (
            PublicationState::Committed { .. },
            path::ArtifactPathKind::TempPending | path::ArtifactPathKind::TempReady,
        ) => {}
        (PublicationState::Released, _) => {}
        _ => {
            return Err(error::corruption(
                "artifact path is invalid for publication state",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_delete_guard_scrub(key: &[u8], marker: u8) -> Result<(), PersistenceError> {
    let components = codec::decode_components(key, 2)?;
    if marker != 1 || !matches!(components[0], "temp" | "content") || components[1].is_empty() {
        return Err(error::corruption("artifact delete guard is malformed"));
    }
    Ok(())
}

pub(crate) fn artifact_path_guard_key(
    key: &[u8],
    value: &[u8],
) -> Result<Vec<u8>, PersistenceError> {
    let entry = path::decode_artifact_path_entry(key, value)?;
    path::artifact_delete_guard_key(entry.kind, &entry.identity)
}
