use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use milkdrift_persistence::{
    ArtifactPublicationId, ArtifactReadChunk, ArtifactReadRequest, ArtifactStore,
    ArtifactWriteProgress, BeginArtifactOutcome, BeginArtifactPublication, CommitArtifactOutcome,
    OrphanCleanupRequest, OrphanCleanupResult, PersistenceError, StorageFailureClass,
    authorize_artifact_read,
};
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, ContentDigest, RunId, WorkspaceUsage,
};
use redb::ReadableTable;
use serde::{Deserialize, Serialize};

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    json,
    schema::{
        ARTIFACT_DIGEST_RESERVATIONS, ARTIFACT_METADATA, ARTIFACT_PUBLICATIONS,
        ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS,
        ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST, METADATA, WORKSPACE_USAGE,
    },
};

const PUBLICATION_SCHEMA_VERSION: u32 = 1;
const GLOBAL_ARTIFACT_BYTES_KEY: &str = "artifact_content_bytes";
const MAX_CHUNK_BYTES: usize = milkdrift_persistence::MAX_ARTIFACT_CHUNK_BYTES;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PublicationState {
    Writable,
    Committed { content_deduplicated: bool },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationRecord {
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

impl ArtifactStore for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.begin_artifact_publication",
        skip_all,
        fields(
            run = %request.run,
            publication = %request.publication,
            artifact = %request.metadata.reference().artifact(),
            size_bytes = request.metadata.reference().size_bytes()
        )
    )]
    fn begin_publication(
        &self,
        request: &BeginArtifactPublication,
    ) -> Result<BeginArtifactOutcome, PersistenceError> {
        validate_publication_request(self, request)?;
        let _serialization = self.lock_artifact_publications()?;
        let temp_name = publication_temp_name(&request.publication);
        let temp_path = self.temp_root.join(&temp_name);
        let write = self.database().begin_write().map_err(error::redb)?;

        {
            let publications = write
                .open_table(ARTIFACT_PUBLICATIONS)
                .map_err(error::redb)?;
            if let Some(bytes) = publications
                .get(request.publication.as_str())
                .map_err(error::redb)?
            {
                let record = decode_publication(bytes.value())?;
                if !record.matches(request) {
                    return Err(PersistenceError::ImmutableConflict {
                        entity: "artifact_publication",
                        identity: request.publication.to_string(),
                    });
                }
                return match record.state {
                    PublicationState::Committed { .. } => {
                        Ok(BeginArtifactOutcome::AlreadyCommitted(record.metadata))
                    }
                    PublicationState::Writable => {
                        validate_writable_publication_indexes(&write, &record)?;
                        let offset = publication_length_or_published(
                            &temp_path,
                            &self.content_path(record.metadata.reference().digest()),
                            record.metadata.reference(),
                            self.max_artifact_bytes,
                        )?;
                        Ok(BeginArtifactOutcome::Resumed {
                            next_offset: offset,
                        })
                    }
                };
            }
        }

        if let Some(existing) =
            metadata_in_transaction(&write, request.metadata.reference().artifact())?
        {
            if existing != request.metadata {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "artifact",
                    identity: request.metadata.reference().artifact().to_string(),
                });
            }
            verify_blob(
                &self.content_path(existing.reference().digest()),
                existing.reference(),
                self.max_artifact_bytes,
            )?;
            return Err(PersistenceError::ImmutableConflict {
                entity: "artifact_publication",
                identity: format!(
                    "artifact {} is already committed under another publication identity",
                    existing.reference().artifact()
                ),
            });
        }

        crate::journal::validate_or_initialize_workspace_budget(
            &write,
            &request.run,
            &request.budget,
        )?;
        validate_usage_in_transaction(&write, &request.run, request.expected_usage)?;
        {
            let reservations = write
                .open_table(ARTIFACT_RESERVATIONS)
                .map_err(error::redb)?;
            if let Some(owner) = reservations
                .get(request.run.as_str())
                .map_err(error::redb)?
            {
                if owner.value() != request.publication.as_str() {
                    return Err(PersistenceError::Storage {
                        class: StorageFailureClass::OwnerBusy,
                        message: format!(
                            "run {} already has an active artifact publication",
                            request.run
                        ),
                    });
                }
            }
        }

        prepare_new_temp(&temp_path, &self.temp_root)?;
        let created_at_millis = modified_millis(&fs::metadata(&temp_path).map_err(error::io)?)?;
        let record = PublicationRecord::from_request(request, created_at_millis);
        let bytes = json::encode(&record, "artifact publication")?;
        let transaction_result = (|| {
            {
                let mut publications = write
                    .open_table(ARTIFACT_PUBLICATIONS)
                    .map_err(error::redb)?;
                publications
                    .insert(request.publication.as_str(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            {
                let age_key = publication_age_key(created_at_millis, &request.publication)?;
                let mut by_age = write
                    .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
                    .map_err(error::redb)?;
                by_age
                    .insert(age_key.as_slice(), request.publication.as_str())
                    .map_err(error::redb)?;
            }
            {
                let mut reservations = write
                    .open_table(ARTIFACT_RESERVATIONS)
                    .map_err(error::redb)?;
                reservations
                    .insert(request.run.as_str(), request.publication.as_str())
                    .map_err(error::redb)?;
            }
            {
                let mut owners = write
                    .open_table(ARTIFACT_TEMP_OWNERS)
                    .map_err(error::redb)?;
                owners
                    .insert(temp_name.as_str(), request.publication.as_str())
                    .map_err(error::redb)?;
            }
            {
                let digest = request.metadata.reference().digest().to_hex();
                let key = codec::pair(&digest, request.publication.as_str())?;
                let mut digest_reservations = write
                    .open_table(ARTIFACT_DIGEST_RESERVATIONS)
                    .map_err(error::redb)?;
                digest_reservations
                    .insert(key.as_slice(), 1)
                    .map_err(error::redb)?;
            }
            self.faults.check(FaultPoint::BeforeArtifactBeginCommit)?;
            write.commit().map_err(error::redb)
        })();
        if let Err(cause) = transaction_result {
            remove_unowned_temp_if_present(self, &request.publication, None)?;
            return Err(cause);
        }
        self.faults.check(FaultPoint::AfterArtifactBeginCommit)?;
        Ok(BeginArtifactOutcome::Writable)
    }

    #[tracing::instrument(
        name = "milkdrift.redb_store.write_artifact_chunk",
        skip_all,
        fields(publication = %publication, offset = offset, chunk_bytes = bytes.len())
    )]
    fn write_chunk(
        &self,
        publication: &ArtifactPublicationId,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ArtifactWriteProgress, PersistenceError> {
        if bytes.is_empty() || bytes.len() > MAX_CHUNK_BYTES {
            return Err(PersistenceError::Bounds {
                location: "artifact.write.chunk",
                reason: format!("must contain 1..={MAX_CHUNK_BYTES} bytes"),
            });
        }
        let _serialization = self.lock_artifact_publications()?;
        // A write transaction provides bounded in-process serialization without
        // leaking a lock or transaction through the public port.
        let write = self.database().begin_write().map_err(error::redb)?;
        let record = publication_in_transaction(&write, publication)?;
        if !matches!(record.state, PublicationState::Writable) {
            return Err(PersistenceError::ImmutableConflict {
                entity: "committed_artifact_publication",
                identity: publication.to_string(),
            });
        }
        validate_writable_publication_indexes(&write, &record)?;
        let expected_size = record.metadata.reference().size_bytes();
        let resulting = offset
            .checked_add(
                u64::try_from(bytes.len()).map_err(|_| PersistenceError::Bounds {
                    location: "artifact.write.chunk",
                    reason: "chunk length does not fit u64".to_owned(),
                })?,
            )
            .ok_or_else(|| PersistenceError::Bounds {
                location: "artifact.write.offset",
                reason: "offset overflow".to_owned(),
            })?;
        if resulting > expected_size {
            return Err(PersistenceError::Bounds {
                location: "artifact.write",
                reason: format!("write would exceed exact size {expected_size}"),
            });
        }
        let path = self.temp_path(publication);
        let mut file = open_regular_for_append(&path)?;
        let actual = file.metadata().map_err(error::io)?.len();
        if actual != offset {
            return Err(PersistenceError::ImmutableConflict {
                entity: "artifact_publication_offset",
                identity: format!("{publication}: expected {actual}, supplied {offset}"),
            });
        }
        self.faults.check(FaultPoint::BeforeArtifactChunkWrite)?;
        file.seek(SeekFrom::Start(offset)).map_err(error::io)?;
        file.write_all(bytes).map_err(error::io)?;
        file.sync_all().map_err(error::io)?;
        self.faults.check(FaultPoint::AfterArtifactChunkSync)?;
        drop(write);
        Ok(ArtifactWriteProgress {
            bytes_received: resulting,
            complete_size: resulting == expected_size,
        })
    }

    #[tracing::instrument(
        name = "milkdrift.redb_store.commit_artifact_publication",
        skip_all,
        fields(publication = %publication)
    )]
    fn commit_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<CommitArtifactOutcome, PersistenceError> {
        let _serialization = self.lock_artifact_publications()?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let mut record = publication_in_transaction(&write, publication)?;
        if let PublicationState::Committed { .. } = record.state {
            verify_blob(
                &self.content_path(record.metadata.reference().digest()),
                record.metadata.reference(),
                self.max_artifact_bytes,
            )?;
            remove_unowned_temp_if_present(self, publication, None)?;
            return Ok(CommitArtifactOutcome::Replayed {
                metadata: record.metadata,
                usage: record.resulting_usage,
            });
        }
        validate_writable_publication_indexes(&write, &record)?;

        crate::journal::validate_or_initialize_workspace_budget(
            &write,
            &record.run,
            &record.budget,
        )?;
        validate_usage_in_transaction(&write, &record.run, record.expected_usage)?;
        let temp_path = self.temp_path(publication);
        let final_path = self.content_path(record.metadata.reference().digest());
        let final_parent = final_path
            .parent()
            .ok_or_else(|| crate::store::internal("artifact path has no parent"))?;
        let parent_existed = final_parent.exists();
        crate::store::prepare_owned_directory(final_parent, "artifact digest shard")?;
        if !parent_existed {
            sync_directory(&self.artifact_root)?;
        }

        let content_deduplicated = if final_path.exists() {
            if temp_path.exists() {
                verify_blob(
                    &temp_path,
                    record.metadata.reference(),
                    self.max_artifact_bytes,
                )?;
            }
            verify_blob(
                &final_path,
                record.metadata.reference(),
                self.max_artifact_bytes,
            )?;
            true
        } else {
            verify_blob(
                &temp_path,
                record.metadata.reference(),
                self.max_artifact_bytes,
            )?;
            open_regular_for_read(&temp_path)?
                .sync_all()
                .map_err(error::io)?;
            self.faults.check(FaultPoint::BeforeArtifactRename)?;
            fs::rename(&temp_path, &final_path).map_err(error::io)?;
            sync_directory(final_parent)?;
            self.faults.check(FaultPoint::AfterArtifactRename)?;
            false
        };
        verify_blob(
            &final_path,
            record.metadata.reference(),
            self.max_artifact_bytes,
        )?;

        commit_artifact_metadata(self, &write, &mut record, content_deduplicated)?;
        self.faults
            .check(FaultPoint::BeforeArtifactMetadataCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterArtifactMetadataCommit)?;

        remove_unowned_temp_if_present(self, publication, None)?;
        Ok(CommitArtifactOutcome::Published {
            metadata: record.metadata,
            content_deduplicated,
            usage: record.resulting_usage,
        })
    }

    #[tracing::instrument(
        name = "milkdrift.redb_store.abort_artifact_publication",
        skip_all,
        fields(publication = %publication)
    )]
    fn abort_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<(), PersistenceError> {
        let _serialization = self.lock_artifact_publications()?;
        let write = self.database().begin_write().map_err(error::redb)?;
        let Some(record) = optional_publication_in_transaction(&write, publication)? else {
            drop(write);
            remove_unowned_temp_if_present(
                self,
                publication,
                Some((
                    FaultPoint::BeforeArtifactAbortDelete,
                    FaultPoint::AfterArtifactAbortDelete,
                )),
            )?;
            return Ok(());
        };
        if matches!(record.state, PublicationState::Committed { .. }) {
            return Ok(());
        }
        release_writable_publication(&write, &record)?;
        self.faults.check(FaultPoint::BeforeArtifactAbortCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterArtifactAbortCommit)?;
        remove_unowned_temp_if_present(
            self,
            publication,
            Some((
                FaultPoint::BeforeArtifactAbortDelete,
                FaultPoint::AfterArtifactAbortDelete,
            )),
        )
    }

    fn metadata(
        &self,
        artifact: &ArtifactId,
    ) -> Result<Option<ArtifactMetadata>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        table
            .get(artifact.as_str())
            .map_err(error::redb)?
            .map(|bytes| {
                let metadata: ArtifactMetadata = json::decode(bytes.value(), "artifact metadata")?;
                if metadata.reference().artifact() != artifact {
                    return Err(error::corruption(
                        "artifact-metadata key does not match its document",
                    ));
                }
                Ok(metadata)
            })
            .transpose()
    }

    fn is_committed(&self, reference: &ArtifactReference) -> Result<bool, PersistenceError> {
        let Some(metadata) = self.metadata(reference.artifact())? else {
            return Ok(false);
        };
        if metadata.reference() != reference {
            return Ok(false);
        }
        verify_blob(
            &self.content_path(reference.digest()),
            reference,
            self.max_artifact_bytes,
        )?;
        Ok(true)
    }

    fn is_referenced_by_run(
        &self,
        run: &RunId,
        reference: &ArtifactReference,
    ) -> Result<bool, PersistenceError> {
        let digest = reference.digest().to_hex();
        let prefix = codec::components(&[&digest, reference.artifact().as_str(), run.as_str()])?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("artifact-reference prefix has no range end"))?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
        let mut found = false;
        for item in table
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
        {
            let (_, bytes) = item.map_err(error::redb)?;
            let stored: ArtifactReference = json::decode(bytes.value(), "artifact reference")?;
            if &stored != reference {
                return Err(error::corruption(
                    "artifact-reference index prefix contradicts its stored document",
                ));
            }
            found = true;
        }
        Ok(found)
    }

    fn read_chunk(
        &self,
        request: &ArtifactReadRequest,
    ) -> Result<ArtifactReadChunk, PersistenceError> {
        let metadata = self
            .metadata(request.reference.artifact())?
            .ok_or_else(|| PersistenceError::NotFound {
                entity: "artifact",
                identity: request.reference.artifact().to_string(),
            })?;
        if metadata.reference() != &request.reference {
            return Err(PersistenceError::ImmutableConflict {
                entity: "artifact_reference",
                identity: request.reference.artifact().to_string(),
            });
        }
        authorize_artifact_read(metadata.sensitivity(), &request.authority)?;
        if request.reference.size_bytes() > self.max_read_bytes {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: format!(
                    "artifact size exceeds verified-read limit {}",
                    self.max_read_bytes
                ),
            });
        }
        if request.offset > request.reference.size_bytes() {
            return Err(PersistenceError::Bounds {
                location: "artifact.read.offset",
                reason: "offset is beyond exact artifact size".to_owned(),
            });
        }
        let path = self.content_path(request.reference.digest());
        verify_blob(&path, &request.reference, self.max_read_bytes)?;
        let remaining = request.reference.size_bytes() - request.offset;
        let count = remaining.min(u64::from(request.maximum_bytes));
        let count = usize::try_from(count).map_err(|_| PersistenceError::Bounds {
            location: "artifact.read.maximum_bytes",
            reason: "read length does not fit usize".to_owned(),
        })?;
        let mut file = open_regular_for_read(&path)?;
        file.seek(SeekFrom::Start(request.offset))
            .map_err(error::io)?;
        let mut bytes = vec![0_u8; count];
        file.read_exact(&mut bytes).map_err(|cause| {
            error::corruption(format!("artifact changed during verified read: {cause}"))
        })?;
        Ok(ArtifactReadChunk {
            offset: request.offset,
            bytes,
            end_of_artifact: request.offset + count as u64 == request.reference.size_bytes(),
        })
    }

    #[tracing::instrument(
        name = "milkdrift.redb_store.cleanup_artifact_orphans",
        skip_all,
        fields(
            observed_at = request.observed_at.get(),
            created_before = request.created_before.get(),
            limit = request.limit.get()
        )
    )]
    fn cleanup_orphans(
        &self,
        request: OrphanCleanupRequest,
    ) -> Result<OrphanCleanupResult, PersistenceError> {
        if request.created_before > request.observed_at {
            return Err(PersistenceError::InvalidDocument(
                "orphan cleanup threshold cannot be after observed_at".to_owned(),
            ));
        }
        let _artifact_serialization = self.lock_artifact_publications()?;
        let mut result = OrphanCleanupResult::default();
        let mut examined = 0_u32;
        expire_writable_publications(self, request, &mut result, &mut examined)?;
        if examined < request.limit.get() {
            // Holding redb's sole writer while deleting unowned files prevents a
            // command or new publication from acquiring a durable reference in
            // the interval between the reference check and directory sync.
            let serialization = self.database().begin_write().map_err(error::redb)?;
            cleanup_temporary_files(self, &serialization, request, &mut result, &mut examined)?;
            if examined < request.limit.get() {
                cleanup_content_files(self, &serialization, request, &mut result, &mut examined)?;
            }
            drop(serialization);
        }
        Ok(result)
    }
}

fn validate_publication_request(
    store: &RedbStore,
    request: &BeginArtifactPublication,
) -> Result<(), PersistenceError> {
    request
        .budget
        .validate_usage(&request.expected_usage)
        .map_err(|cause| PersistenceError::InvalidDocument(cause.to_string()))?;
    let calculated = request
        .budget
        .admit_artifact(&request.expected_usage, &request.metadata)
        .map_err(|cause| PersistenceError::InvalidDocument(cause.to_string()))?;
    if calculated != request.resulting_usage {
        return Err(PersistenceError::InvalidDocument(
            "artifact resulting usage does not match its budget charge".to_owned(),
        ));
    }
    let size = request.metadata.reference().size_bytes();
    if size > store.max_artifact_bytes
        || request.resulting_usage.artifact_bytes() > store.max_total_artifact_bytes
    {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            message: "artifact publication exceeds configured local-store limits".to_owned(),
        });
    }
    Ok(())
}

fn decode_publication(bytes: &[u8]) -> Result<PublicationRecord, PersistenceError> {
    let record: PublicationRecord = json::decode(bytes, "artifact publication")?;
    if record.schema_version != PUBLICATION_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "artifact_publication",
            found: record.schema_version,
            supported: PUBLICATION_SCHEMA_VERSION,
        });
    }
    record
        .budget
        .validate_usage(&record.expected_usage)
        .map_err(|cause| {
            error::corruption(format!(
                "artifact publication expected usage is invalid: {cause}"
            ))
        })?;
    let resulting = record
        .budget
        .admit_artifact(&record.expected_usage, &record.metadata)
        .map_err(|cause| {
            error::corruption(format!(
                "artifact publication budget transition is invalid: {cause}"
            ))
        })?;
    if resulting != record.resulting_usage {
        return Err(error::corruption(
            "artifact publication resulting usage does not match its immutable budget transition",
        ));
    }
    Ok(record)
}

fn publication_in_transaction(
    write: &redb::WriteTransaction,
    publication: &ArtifactPublicationId,
) -> Result<PublicationRecord, PersistenceError> {
    optional_publication_in_transaction(write, publication)?.ok_or_else(|| {
        PersistenceError::NotFound {
            entity: "artifact_publication",
            identity: publication.to_string(),
        }
    })
}

fn optional_publication_in_transaction(
    write: &redb::WriteTransaction,
    publication: &ArtifactPublicationId,
) -> Result<Option<PublicationRecord>, PersistenceError> {
    let table = write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let Some(bytes) = table.get(publication.as_str()).map_err(error::redb)? else {
        return Ok(None);
    };
    let record = decode_publication(bytes.value())?;
    if record.publication != *publication {
        return Err(error::corruption(
            "artifact-publication key does not match its document",
        ));
    }
    Ok(Some(record))
}

fn metadata_in_transaction(
    write: &redb::WriteTransaction,
    artifact: &ArtifactId,
) -> Result<Option<ArtifactMetadata>, PersistenceError> {
    let table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    table
        .get(artifact.as_str())
        .map_err(error::redb)?
        .map(|bytes| {
            let metadata: ArtifactMetadata = json::decode(bytes.value(), "artifact metadata")?;
            if metadata.reference().artifact() != artifact {
                return Err(error::corruption(
                    "artifact-metadata key does not match its document",
                ));
            }
            Ok(metadata)
        })
        .transpose()
}

fn validate_usage_in_transaction(
    write: &redb::WriteTransaction,
    run: &milkdrift_workspace::RunId,
    expected: WorkspaceUsage,
) -> Result<(), PersistenceError> {
    let table = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    let actual = table
        .get(run.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "workspace usage"))
        .transpose()?
        .unwrap_or(WorkspaceUsage::EMPTY);
    if actual != expected {
        return Err(PersistenceError::WorkspaceUsageConflict { run: run.clone() });
    }
    Ok(())
}

fn commit_artifact_metadata(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    record: &mut PublicationRecord,
    content_deduplicated: bool,
) -> Result<(), PersistenceError> {
    let metadata_bytes = json::encode(&record.metadata, "artifact metadata")?;
    {
        let mut metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        if let Some(existing) = metadata
            .get(record.metadata.reference().artifact().as_str())
            .map_err(error::redb)?
        {
            let existing: ArtifactMetadata = json::decode(existing.value(), "artifact metadata")?;
            if existing != record.metadata {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "artifact",
                    identity: record.metadata.reference().artifact().to_string(),
                });
            }
        } else {
            metadata
                .insert(
                    record.metadata.reference().artifact().as_str(),
                    metadata_bytes.as_slice(),
                )
                .map_err(error::redb)?;
        }
    }
    let digest = record.metadata.reference().digest().to_hex();
    let digest_key = codec::pair(&digest, record.metadata.reference().artifact().as_str())?;
    let digest_was_known = {
        let by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
        let prefix = codec::component(&digest)?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("artifact digest prefix has no range end"))?;
        by_digest
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .is_some()
    };
    {
        let mut by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
        by_digest
            .insert(digest_key.as_slice(), metadata_bytes.as_slice())
            .map_err(error::redb)?;
    }
    if !digest_was_known {
        let mut metadata = write.open_table(METADATA).map_err(error::redb)?;
        let current = metadata
            .get(GLOBAL_ARTIFACT_BYTES_KEY)
            .map_err(error::redb)?
            .map_or(0, |value| value.value());
        let resulting = current
            .checked_add(record.metadata.reference().size_bytes())
            .ok_or_else(|| PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: "global artifact-byte accounting overflow".to_owned(),
            })?;
        if resulting > store.max_total_artifact_bytes {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: "global artifact-byte limit exceeded".to_owned(),
            });
        }
        metadata
            .insert(GLOBAL_ARTIFACT_BYTES_KEY, resulting)
            .map_err(error::redb)?;
    }
    {
        let usage_bytes = json::encode(&record.resulting_usage, "workspace usage")?;
        let mut usage = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        usage
            .insert(record.run.as_str(), usage_bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let key = codec::components(&[
            &digest,
            record.metadata.reference().artifact().as_str(),
            record.run.as_str(),
            "publication",
            record.publication.as_str(),
        ])?;
        let reference_bytes = json::encode(record.metadata.reference(), "artifact reference")?;
        let mut references = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
        references
            .insert(key.as_slice(), reference_bytes.as_slice())
            .map_err(error::redb)?;
    }
    remove_publication_age_index(write, record)?;
    record.state = PublicationState::Committed {
        content_deduplicated,
    };
    let record_bytes = json::encode(record, "artifact publication")?;
    {
        let mut publications = write
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
        publications
            .insert(record.publication.as_str(), record_bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let mut reservations = write
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
        let _removed = reservations
            .remove(record.run.as_str())
            .map_err(error::redb)?;
    }
    {
        let temp_name = publication_temp_name(&record.publication);
        let mut owners = write
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?;
        let _removed = owners.remove(temp_name.as_str()).map_err(error::redb)?;
    }
    {
        let key = codec::pair(&digest, record.publication.as_str())?;
        let mut digest_reservations = write
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
        let _removed = digest_reservations
            .remove(key.as_slice())
            .map_err(error::redb)?;
    }
    Ok(())
}

impl RedbStore {
    fn lock_artifact_publications(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, ()>, PersistenceError> {
        self.artifact_serialization
            .lock()
            .map_err(|_| PersistenceError::Storage {
                class: StorageFailureClass::Internal,
                message: "artifact publication serialization lock was poisoned".to_owned(),
            })
    }

    fn temp_path(&self, publication: &ArtifactPublicationId) -> PathBuf {
        self.temp_root.join(publication_temp_name(publication))
    }

    pub(crate) fn content_path(&self, digest: ContentDigest) -> PathBuf {
        let hex = digest.to_hex();
        self.artifact_root.join(&hex[..2]).join(&hex[2..])
    }
}

fn publication_temp_name(publication: &ArtifactPublicationId) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"milkdrift.artifact-publication-temp.v1\0");
    hasher.update(publication.as_str().as_bytes());
    format!("{}.part", hasher.finalize())
}

fn publication_age_key(
    created_at_millis: u64,
    publication: &ArtifactPublicationId,
) -> Result<Vec<u8>, PersistenceError> {
    let publication = codec::component(publication.as_str())?;
    let mut key = Vec::with_capacity(std::mem::size_of::<u64>() + publication.len());
    key.extend_from_slice(&created_at_millis.to_be_bytes());
    key.extend_from_slice(&publication);
    Ok(key)
}

fn prepare_new_temp(path: &Path, directory: &Path) -> Result<(), PersistenceError> {
    crate::store::prepare_owned_directory(directory, "artifact temporary directory")?;
    if path.exists() {
        let metadata = fs::symlink_metadata(path).map_err(error::io)?;
        if !metadata.file_type().is_file() {
            return Err(error::corruption(
                "artifact temporary path is not a regular file",
            ));
        }
        fs::remove_file(path).map_err(error::io)?;
    }
    let file = create_private_file(path)?;
    file.sync_all().map_err(error::io)?;
    sync_directory(directory)
}

fn create_private_file(path: &Path) -> Result<File, PersistenceError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(error::io)
}

fn open_regular_for_append(path: &Path) -> Result<File, PersistenceError> {
    ensure_regular(path)?;
    open_regular_no_follow(path, true)
}

fn open_regular_for_read(path: &Path) -> Result<File, PersistenceError> {
    ensure_regular(path)?;
    open_regular_no_follow(path, false)
}

#[cfg(unix)]
fn open_regular_no_follow(path: &Path, writable: bool) -> Result<File, PersistenceError> {
    use rustix::fs::{Mode, OFlags};

    let access = if writable {
        OFlags::RDWR
    } else {
        OFlags::RDONLY
    };
    let file = rustix::fs::open(
        path,
        access | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|cause| {
        if cause == rustix::io::Errno::LOOP {
            error::corruption("artifact path changed into a symlink while opening")
        } else {
            error::io(cause.into())
        }
    })?;
    verify_opened_regular_identity(path, &file)?;
    Ok(file)
}

#[cfg(not(unix))]
fn open_regular_no_follow(path: &Path, writable: bool) -> Result<File, PersistenceError> {
    let file = OpenOptions::new()
        .read(true)
        .write(writable)
        .open(path)
        .map_err(error::io)?;
    verify_opened_regular_identity(path, &file)?;
    Ok(file)
}

fn verify_opened_regular_identity(path: &Path, file: &File) -> Result<(), PersistenceError> {
    let opened = file.metadata().map_err(error::io)?;
    let path_metadata = fs::symlink_metadata(path).map_err(error::io)?;
    if !opened.is_file()
        || !path_metadata.file_type().is_file()
        || path_metadata.file_type().is_symlink()
    {
        return Err(error::corruption(
            "artifact path changed type or became a symlink while opening",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if opened.dev() != path_metadata.dev() || opened.ino() != path_metadata.ino() {
            return Err(error::corruption(
                "artifact path identity changed while opening",
            ));
        }
    }
    Ok(())
}

fn ensure_regular(path: &Path) -> Result<(), PersistenceError> {
    let metadata = fs::symlink_metadata(path).map_err(|cause| {
        if cause.kind() == std::io::ErrorKind::NotFound {
            error::corruption("artifact content file is missing")
        } else {
            error::io(cause)
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(error::corruption(
            "artifact content path is not a regular file",
        ));
    }
    Ok(())
}

fn publication_length_or_published(
    temp: &Path,
    published: &Path,
    reference: &ArtifactReference,
    maximum: u64,
) -> Result<u64, PersistenceError> {
    if temp.exists() {
        ensure_regular(temp)?;
        let size = fs::metadata(temp).map_err(error::io)?.len();
        if size > reference.size_bytes() || size > maximum {
            return Err(error::corruption(
                "artifact temporary stream exceeds its declared bound",
            ));
        }
        return Ok(size);
    }
    if published.exists() {
        verify_blob(published, reference, maximum)?;
        return Ok(reference.size_bytes());
    }
    Err(error::corruption(
        "writable artifact publication has neither temporary nor published content",
    ))
}

pub(crate) fn verify_blob(
    path: &Path,
    reference: &ArtifactReference,
    maximum: u64,
) -> Result<(), PersistenceError> {
    if reference.size_bytes() > maximum {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            message: format!("artifact verification exceeds configured bound {maximum}"),
        });
    }
    let mut file = open_regular_for_read(path)?;
    let size = file.metadata().map_err(error::io)?.len();
    if size != reference.size_bytes() {
        return Err(error::corruption(format!(
            "artifact size mismatch: expected {}, stored {size}",
            reference.size_bytes()
        )));
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; MAX_CHUNK_BYTES];
    let mut read_total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(error::io)?;
        if count == 0 {
            break;
        }
        read_total = read_total
            .checked_add(count as u64)
            .ok_or_else(|| error::corruption("artifact read length overflow"))?;
        if read_total > maximum || read_total > reference.size_bytes() {
            return Err(error::corruption(
                "artifact changed size during verification",
            ));
        }
        hasher.update(&buffer[..count]);
    }
    if read_total != reference.size_bytes()
        || hasher.finalize().as_bytes() != reference.digest().as_bytes()
    {
        return Err(error::corruption("artifact content digest mismatch"));
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), PersistenceError> {
    crate::store::sync_owned_directory(path)
}

fn expire_writable_publications(
    store: &RedbStore,
    request: OrphanCleanupRequest,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
) -> Result<(), PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    let mut expired = Vec::new();
    {
        let by_age = write
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
        for item in by_age.iter().map_err(error::redb)? {
            if *examined >= request.limit.get() {
                break;
            }
            let (age_key, publication) = item.map_err(error::redb)?;
            let age_key = age_key.value();
            let created_at = age_key
                .get(..std::mem::size_of::<u64>())
                .and_then(|bytes| bytes.try_into().ok())
                .map(u64::from_be_bytes)
                .ok_or_else(|| error::corruption("invalid publication-age index key"))?;
            if created_at >= request.created_before.get() {
                break;
            }
            let publication = ArtifactPublicationId::new(publication.value()).map_err(|cause| {
                error::corruption(format!("invalid publication-age identity: {cause}"))
            })?;
            let record = publication_in_transaction(&write, &publication)?;
            if !matches!(record.state, PublicationState::Writable) {
                return Err(error::corruption(
                    "publication-age index points to a committed publication",
                ));
            }
            if record.created_at_millis != created_at
                || publication_age_key(created_at, &record.publication)?.as_slice() != age_key
            {
                return Err(error::corruption(
                    "publication-age index key does not match its document",
                ));
            }
            *examined += 1;
            expired.push(record);
        }
    }
    if expired.is_empty() {
        drop(write);
        return Ok(());
    }

    for record in &expired {
        release_writable_publication(&write, record)?;
    }
    store
        .faults
        .check(FaultPoint::BeforeArtifactCleanupCommit)?;
    write.commit().map_err(error::redb)?;
    store.faults.check(FaultPoint::AfterArtifactCleanupCommit)?;

    for record in expired {
        let path = store.temp_path(&record.publication);
        if let Some(size) = remove_cleanup_file_if_present(store, &path, &store.temp_root)? {
            result.temporary_publications_removed =
                result.temporary_publications_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(())
}

fn validate_writable_publication_indexes(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    if !matches!(record.state, PublicationState::Writable) {
        return Err(error::corruption(
            "writable publication indexes point to a committed record",
        ));
    }
    let age_key = publication_age_key(record.created_at_millis, &record.publication)?;
    let by_age = write
        .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
        .map_err(error::redb)?;
    if by_age
        .get(age_key.as_slice())
        .map_err(error::redb)?
        .is_none_or(|value| value.value() != record.publication.as_str())
    {
        return Err(error::corruption(
            "writable publication has an inconsistent age index",
        ));
    }
    let reservations = write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    if reservations
        .get(record.run.as_str())
        .map_err(error::redb)?
        .is_none_or(|value| value.value() != record.publication.as_str())
    {
        return Err(error::corruption(
            "writable publication has an inconsistent run reservation",
        ));
    }
    let temp_name = publication_temp_name(&record.publication);
    let owners = write
        .open_table(ARTIFACT_TEMP_OWNERS)
        .map_err(error::redb)?;
    if owners
        .get(temp_name.as_str())
        .map_err(error::redb)?
        .is_none_or(|value| value.value() != record.publication.as_str())
    {
        return Err(error::corruption(
            "writable publication has an inconsistent temporary-file owner",
        ));
    }
    let digest = record.metadata.reference().digest().to_hex();
    let key = codec::pair(&digest, record.publication.as_str())?;
    let digest_reservations = write
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;
    if digest_reservations
        .get(key.as_slice())
        .map_err(error::redb)?
        .is_none()
    {
        return Err(error::corruption(
            "writable publication has no digest reservation",
        ));
    }
    Ok(())
}

fn release_writable_publication(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    validate_writable_publication_indexes(write, record)?;
    {
        let mut publications = write
            .open_table(ARTIFACT_PUBLICATIONS)
            .map_err(error::redb)?;
        let stored = publications
            .get(record.publication.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("artifact publication disappeared during cleanup"))?;
        if decode_publication(stored.value())? != *record {
            return Err(error::corruption(
                "artifact publication changed during serialized cleanup",
            ));
        }
        drop(stored);
        let _removed = publications
            .remove(record.publication.as_str())
            .map_err(error::redb)?;
    }
    remove_publication_age_index(write, record)?;
    {
        let mut reservations = write
            .open_table(ARTIFACT_RESERVATIONS)
            .map_err(error::redb)?;
        let owner = reservations
            .get(record.run.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        if owner.as_deref() != Some(record.publication.as_str()) {
            return Err(error::corruption(
                "writable publication has an inconsistent run reservation",
            ));
        }
        let _removed = reservations
            .remove(record.run.as_str())
            .map_err(error::redb)?;
    }
    {
        let temp_name = publication_temp_name(&record.publication);
        let mut owners = write
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?;
        let owner = owners
            .get(temp_name.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        if owner.as_deref() != Some(record.publication.as_str()) {
            return Err(error::corruption(
                "writable publication has an inconsistent temporary-file owner",
            ));
        }
        let _removed = owners.remove(temp_name.as_str()).map_err(error::redb)?;
    }
    {
        let digest = record.metadata.reference().digest().to_hex();
        let key = codec::pair(&digest, record.publication.as_str())?;
        let mut reservations = write
            .open_table(ARTIFACT_DIGEST_RESERVATIONS)
            .map_err(error::redb)?;
        if reservations
            .get(key.as_slice())
            .map_err(error::redb)?
            .is_none()
        {
            return Err(error::corruption(
                "writable publication has no digest reservation",
            ));
        }
        let _removed = reservations.remove(key.as_slice()).map_err(error::redb)?;
    }
    Ok(())
}

fn remove_publication_age_index(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    let age_key = publication_age_key(record.created_at_millis, &record.publication)?;
    let mut by_age = write
        .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
        .map_err(error::redb)?;
    let indexed = by_age
        .get(age_key.as_slice())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    if indexed.as_deref() != Some(record.publication.as_str()) {
        return Err(error::corruption(
            "writable publication has an inconsistent age index",
        ));
    }
    let _removed = by_age.remove(age_key.as_slice()).map_err(error::redb)?;
    Ok(())
}

fn cleanup_temporary_files(
    store: &RedbStore,
    transaction: &redb::WriteTransaction,
    request: OrphanCleanupRequest,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
) -> Result<(), PersistenceError> {
    for entry in fs::read_dir(&store.temp_root).map_err(error::io)? {
        if *examined >= request.limit.get() {
            break;
        }
        let entry = entry.map_err(error::io)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(error::io)?;
        if !metadata.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let owners = transaction
            .open_table(ARTIFACT_TEMP_OWNERS)
            .map_err(error::redb)?;
        let owner = owners
            .get(name.as_str())
            .map_err(error::redb)?
            .map(|value| value.value().to_owned());
        drop(owners);
        if let Some(owner) = owner {
            let publication = ArtifactPublicationId::new(owner).map_err(|cause| {
                error::corruption(format!("invalid temporary-file owner identity: {cause}"))
            })?;
            let record = publication_in_transaction(transaction, &publication)?;
            if publication_temp_name(&record.publication) != name
                || !matches!(record.state, PublicationState::Writable)
            {
                return Err(error::corruption(
                    "temporary-file owner does not identify its writable publication",
                ));
            }
            validate_writable_publication_indexes(transaction, &record)?;
            continue;
        }
        *examined += 1;
        if modified_millis(&metadata)? >= request.created_before.get() {
            continue;
        }
        if let Some(size) = remove_cleanup_file_if_present(store, &entry.path(), &store.temp_root)?
        {
            result.temporary_publications_removed =
                result.temporary_publications_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(())
}

fn cleanup_content_files(
    store: &RedbStore,
    transaction: &redb::WriteTransaction,
    request: OrphanCleanupRequest,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
) -> Result<(), PersistenceError> {
    for shard in fs::read_dir(&store.artifact_root).map_err(error::io)? {
        if *examined >= request.limit.get() {
            break;
        }
        let shard = shard.map_err(error::io)?;
        let shard_name = shard.file_name().to_string_lossy().into_owned();
        if shard_name == ".tmp" || !shard.file_type().map_err(error::io)?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(shard.path()).map_err(error::io)? {
            if *examined >= request.limit.get() {
                break;
            }
            let entry = entry.map_err(error::io)?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(error::io)?;
            if !metadata.file_type().is_file() {
                continue;
            }
            *examined += 1;
            if modified_millis(&metadata)? >= request.created_before.get() {
                continue;
            }
            let tail = entry.file_name().to_string_lossy().into_owned();
            let digest_text = format!("{shard_name}{tail}");
            if ContentDigest::from_hex(&digest_text).is_err()
                || digest_has_metadata_or_references(transaction, &digest_text)?
            {
                continue;
            }
            if let Some(size) = remove_cleanup_file_if_present(store, &entry.path(), &shard.path())?
            {
                result.unreferenced_blobs_removed =
                    result.unreferenced_blobs_removed.saturating_add(1);
                result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
            }
        }
    }
    Ok(())
}

fn digest_has_metadata_or_references(
    transaction: &redb::WriteTransaction,
    digest: &str,
) -> Result<bool, PersistenceError> {
    let prefix = codec::component(digest)?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("artifact digest prefix has no range end"))?;
    let metadata = transaction
        .open_table(ARTIFACTS_BY_DIGEST)
        .map_err(error::redb)?;
    if metadata
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some()
    {
        return Ok(true);
    }
    let references = transaction
        .open_table(ARTIFACT_REFERENCES)
        .map_err(error::redb)?;
    if references
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some()
    {
        return Ok(true);
    }
    let reservations = transaction
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;
    Ok(reservations
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some())
}

fn remove_cleanup_file_if_present(
    store: &RedbStore,
    path: &Path,
    parent: &Path,
) -> Result<Option<u64>, PersistenceError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return Err(error::corruption(
                "artifact cleanup target is not a regular file",
            ));
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(cause) => return Err(error::io(cause)),
    };
    store
        .faults
        .check(FaultPoint::BeforeArtifactCleanupDelete)?;
    fs::remove_file(path).map_err(error::io)?;
    sync_directory(parent)?;
    store.faults.check(FaultPoint::AfterArtifactCleanupDelete)?;
    Ok(Some(metadata.len()))
}

fn remove_unowned_temp_if_present(
    store: &RedbStore,
    publication: &ArtifactPublicationId,
    fault_boundary: Option<(FaultPoint, FaultPoint)>,
) -> Result<(), PersistenceError> {
    let name = publication_temp_name(publication);
    let read = store.database().begin_read().map_err(error::redb)?;
    let owners = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    if owners.get(name.as_str()).map_err(error::redb)?.is_some() {
        return Err(error::corruption(
            "missing publication still owns a temporary artifact stream",
        ));
    }
    drop(owners);
    drop(read);
    let path = store.temp_root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if let Some((before, _after)) = fault_boundary {
                store.faults.check(before)?;
            }
            fs::remove_file(path).map_err(error::io)?;
            sync_directory(&store.temp_root)?;
            if let Some((_before, after)) = fault_boundary {
                store.faults.check(after)?;
            }
            Ok(())
        }
        Ok(_) => Err(error::corruption(
            "artifact abort target is not a regular file",
        )),
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(cause) => Err(error::io(cause)),
    }
}

fn modified_millis(metadata: &fs::Metadata) -> Result<u64, PersistenceError> {
    let duration = metadata
        .modified()
        .map_err(error::io)?
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| error::corruption(format!("invalid filesystem timestamp: {cause}")))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| error::corruption("filesystem timestamp exceeds u64 milliseconds"))
}
