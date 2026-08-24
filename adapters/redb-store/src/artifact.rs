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
    ArtifactId, ArtifactMetadata, ArtifactReference, ContentDigest, RunId, WorkspaceUsage,
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
        ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, RUN_ARTIFACT_OWNERSHIP,
        WORKSPACE_USAGE,
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
            publication: request.publication.clone(),
            run: request.run.clone(),
            metadata: request.metadata.clone(),
            budget: request.budget.clone(),
            expected_usage: request.expected_usage,
            resulting_usage: request.resulting_usage,
            created_at_millis,
            state: PublicationState::Writable,
        }
    }

    fn matches(&self, request: &BeginArtifactPublication) -> bool {
        self.publication == request.publication
            && self.run == request.run
            && self.metadata == request.metadata
            && self.budget == request.budget
            && self.expected_usage == request.expected_usage
            && self.resulting_usage == request.resulting_usage
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
