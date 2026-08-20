use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
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
        ARTIFACT_ACCOUNTING, ARTIFACT_DIGEST_RESERVATIONS, ARTIFACT_MANIFEST, ARTIFACT_METADATA,
        ARTIFACT_PUBLICATIONS, ARTIFACT_PUBLICATIONS_BY_AGE, ARTIFACT_REFERENCES,
        ARTIFACT_RESERVATIONS, ARTIFACT_TEMP_MANIFEST, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST,
        RUN_ARTIFACT_OWNERSHIP, WORKSPACE_USAGE,
    },
    trie::{self, CatalogFamily},
};

const PUBLICATION_SCHEMA_VERSION: u32 = 1;
const ARTIFACT_ACCOUNTING_SCHEMA_VERSION: u32 = 3;
const GLOBAL_ARTIFACT_BYTES_KEY: &str = "artifact_content_bytes";
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyArtifactAccountingRecord {
    schema_version: u32,
    committed_content_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyArtifactAccountingRecordV2 {
    schema_version: u32,
    artifact_count: u64,
    committed_content_bytes: u64,
    reference_occurrence_count: u64,
    writable_publication_count: u64,
    committed_publication_count: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
enum LegacyArtifactAccountingDocument {
    V2(LegacyArtifactAccountingRecordV2),
    V1(LegacyArtifactAccountingRecord),
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
        let artifact_accounting = validate_artifact_catalog(&write)?;
        if artifact_accounting.committed_content_bytes > self.max_total_artifact_bytes {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: "committed artifact bytes exceed the configured aggregate limit"
                    .to_owned(),
            });
        }

        let existing_publication = optional_publication_in_transaction(&write, &request.publication)?;
        if let Some(record) = existing_publication {
            if !record.matches(request) {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "artifact_publication",
                    identity: request.publication.to_string(),
                });
            }
            return match record.state {
                PublicationState::Committed { .. } => {
                    let actual = crate::journal::validate_workspace_domain_in_transaction(
                        &write,
                        &record.run,
                        &record.budget,
                    )?;
                    if !usage_covers(actual, record.resulting_usage) {
                        return Err(error::corruption(
                            "committed publication is beyond current workspace usage",
                        ));
                    }
                    if !validated_run_artifact_reference_in_transaction(
                        &write,
                        &record.run,
                        record.metadata.reference(),
                    )? {
                        return Err(error::corruption(
                            "committed publication has no run artifact ownership",
                        ));
                    }
                    match validated_artifact_metadata_in_transaction(
                        &write,
                        record.metadata.reference().artifact(),
                    )? {
                        Some(metadata) if metadata == record.metadata => {
                            verify_blob(
                                &self.content_path(record.metadata.reference().digest()),
                                record.metadata.reference(),
                                self.max_artifact_bytes,
                            )?;
                            Ok(BeginArtifactOutcome::AlreadyCommitted(record.metadata))
                        }
                        _ => Err(error::corruption(
                            "committed publication disagrees with its artifact manifest",
                        )),
                    }
                }
                PublicationState::Writable => {
                    let actual = crate::journal::validate_workspace_domain_in_transaction(
                        &write,
                        &record.run,
                        &record.budget,
                    )?;
                    if actual != record.expected_usage {
                        return Err(error::corruption(
                            "writable publication disagrees with workspace usage",
                        ));
                    }
                    validate_writable_publication_indexes(&write, &record)?;
                    drop(write);
                    ensure_temp_inventory_ready(self, &record.publication)?;
                    let verify = self.database().begin_write().map_err(error::redb)?;
                    let verified = publication_in_transaction(&verify, &record.publication)?;
                    if verified != record {
                        return Err(error::corruption(
                            "artifact publication changed while materializing its temp path",
                        ));
                    }
                    require_temp_inventory_ready(&verify, &verified)?;
                    let offset = publication_length_or_published(
                        &temp_path,
                        &self.content_path(verified.metadata.reference().digest()),
                        verified.metadata.reference(),
                        self.max_artifact_bytes,
                    )?;
                    Ok(BeginArtifactOutcome::Resumed {
                        next_offset: offset,
                    })
                }
                PublicationState::Released => Err(PersistenceError::ImmutableConflict {
                    entity: "released_artifact_publication",
                    identity: record.publication.to_string(),
                }),
            };
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

        let actual = crate::journal::validate_or_initialize_workspace_domain(
            &write,
            &request.run,
            &request.budget,
        )?;
        if actual != request.expected_usage {
            return Err(PersistenceError::WorkspaceUsageConflict {
                run: request.run.clone(),
            });
        }
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

        let created_at_millis = now_millis()?;
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
            persist_publication_catalog(&write, &record, None)?;
            let pending = artifact_path_entry(&record, ArtifactPathKind::TempPending)?;
            put_artifact_path(&write, &pending, None)?;
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
                let bytes = json::encode(&request.publication, "artifact temporary manifest")?;
                let mut manifest = write
                    .open_table(ARTIFACT_TEMP_MANIFEST)
                    .map_err(error::redb)?;
                manifest
                    .insert(temp_name.as_str(), bytes.as_slice())
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
            validate_artifact_catalog(&write)?;
            self.faults.check(FaultPoint::BeforeArtifactBeginCommit)?;
            write.commit().map_err(error::redb)
        })();
        transaction_result?;
        self.faults.check(FaultPoint::AfterArtifactBeginCommit)?;
        ensure_temp_inventory_ready(self, &request.publication)?;
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
        validate_artifact_catalog(&write)?;
        let record = publication_in_transaction(&write, publication)?;
        if !matches!(record.state, PublicationState::Writable) {
            return Err(PersistenceError::ImmutableConflict {
                entity: "committed_artifact_publication",
                identity: publication.to_string(),
            });
        }
        let actual = crate::journal::validate_workspace_domain_in_transaction(
            &write,
            &record.run,
            &record.budget,
        )?;
        if actual != record.expected_usage {
            return Err(error::corruption(
                "writable publication disagrees with workspace usage",
            ));
        }
        validate_writable_publication_indexes(&write, &record)?;
        require_temp_inventory_ready(&write, &record)?;
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
        let artifact_accounting = validate_artifact_catalog(&write)?;
        if artifact_accounting.committed_content_bytes > self.max_total_artifact_bytes {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: "committed artifact bytes exceed the configured aggregate limit"
                    .to_owned(),
            });
        }
        let mut record = publication_in_transaction(&write, publication)?;
        if let PublicationState::Committed { .. } = record.state {
            let actual = crate::journal::validate_workspace_domain_in_transaction(
                &write,
                &record.run,
                &record.budget,
            )?;
            if !usage_covers(actual, record.resulting_usage) {
                return Err(error::corruption(
                    "committed publication is beyond current workspace usage",
                ));
            }
            if !validated_run_artifact_reference_in_transaction(
                &write,
                &record.run,
                record.metadata.reference(),
            )? {
                return Err(error::corruption(
                    "committed publication has no run artifact ownership",
                ));
            }
            if validated_artifact_metadata_in_transaction(
                &write,
                record.metadata.reference().artifact(),
            )?
            .as_ref()
                != Some(&record.metadata)
            {
                return Err(error::corruption(
                    "committed publication disagrees with its artifact manifest",
                ));
            }
            verify_blob(
                &self.content_path(record.metadata.reference().digest()),
                record.metadata.reference(),
                self.max_artifact_bytes,
            )?;
            drop(write);
            finalize_released_publication_paths(self, &record, None)?;
            return Ok(CommitArtifactOutcome::Replayed {
                metadata: record.metadata,
                usage: record.resulting_usage,
            });
        }
        if matches!(record.state, PublicationState::Released) {
            return Err(PersistenceError::ImmutableConflict {
                entity: "released_artifact_publication",
                identity: record.publication.to_string(),
            });
        }
        validate_writable_publication_indexes(&write, &record)?;
        require_temp_inventory_ready(&write, &record)?;

        let actual = crate::journal::validate_workspace_domain_in_transaction(
            &write,
            &record.run,
            &record.budget,
        )?;
        if actual != record.expected_usage {
            return Err(error::corruption(
                "writable publication disagrees with workspace usage",
            ));
        }
        let content_intent_preexisted = content_intent_state(&write, &record)?;
        let content_identity = record.metadata.reference().digest().to_hex();
        if artifact_delete_guard_exists(
            &write,
            ArtifactPathKind::ContentIntent,
            &content_identity,
        )? {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::OwnerBusy,
                message: "artifact content path is being durably finalized".to_owned(),
            });
        }
        if !content_intent_preexisted {
            let intent = artifact_path_entry(&record, ArtifactPathKind::ContentIntent)?;
            put_artifact_path(&write, &intent, None)?;
            self.faults
                .check(FaultPoint::BeforeArtifactContentIntentCommit)?;
            write.commit().map_err(error::redb)?;
            self.faults
                .check(FaultPoint::AfterArtifactContentIntentCommit)?;
        } else {
            drop(write);
        }
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

        let digest_was_cataloged = {
            let read = self.database().begin_write().map_err(error::redb)?;
            let digest = record.metadata.reference().digest().to_hex();
            let prefix = codec::component(&digest)?;
            let end = codec::prefix_end(prefix.clone()).ok_or_else(|| {
                error::corruption("artifact digest prefix has no range end")
            })?;
            let cataloged = read
                .open_table(ARTIFACTS_BY_DIGEST)
                .map_err(error::redb)?
                .range(prefix.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .is_some();
            require_content_intent(&read, &record)?;
            cataloged
        };
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
            digest_was_cataloged
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

        let write = self.database().begin_write().map_err(error::redb)?;
        let reloaded = publication_in_transaction(&write, publication)?;
        if reloaded != record || !matches!(reloaded.state, PublicationState::Writable) {
            return Err(error::corruption(
                "artifact publication changed while its final path was prepared",
            ));
        }
        validate_writable_publication_indexes(&write, &record)?;
        require_temp_inventory_ready(&write, &record)?;
        require_content_intent(&write, &record)?;
        if artifact_delete_guard_exists(
            &write,
            ArtifactPathKind::ContentIntent,
            &record.metadata.reference().digest().to_hex(),
        )? {
            return Err(PersistenceError::Storage {
                class: StorageFailureClass::OwnerBusy,
                message: "artifact content path is being durably finalized".to_owned(),
            });
        }
        let actual = crate::journal::validate_workspace_domain_in_transaction(
            &write,
            &record.run,
            &record.budget,
        )?;
        if actual != record.expected_usage {
            return Err(error::corruption(
                "writable publication changed workspace usage before metadata commit",
            ));
        }
        commit_artifact_metadata(self, &write, &mut record, content_deduplicated)?;
        self.faults
            .check(FaultPoint::BeforeArtifactMetadataCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterArtifactMetadataCommit)?;

        finalize_released_publication_paths(self, &record, None)?;
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
        validate_artifact_catalog(&write)?;
        let Some(record) = optional_publication_in_transaction(&write, publication)? else {
            drop(write);
            return Ok(());
        };
        let expected_usage = match record.state {
            PublicationState::Writable => record.expected_usage,
            PublicationState::Committed { .. } => record.resulting_usage,
            PublicationState::Released => record.expected_usage,
        };
        let actual = crate::journal::validate_workspace_domain_in_transaction(
            &write,
            &record.run,
            &record.budget,
        )?;
        if (matches!(record.state, PublicationState::Writable | PublicationState::Released)
            && actual != expected_usage)
            || (matches!(record.state, PublicationState::Committed { .. })
                && !usage_covers(actual, expected_usage))
        {
            return Err(error::corruption(
                "artifact publication disagrees with workspace usage",
            ));
        }
        if matches!(record.state, PublicationState::Committed { .. }) {
            return Ok(());
        }
        if matches!(record.state, PublicationState::Released) {
            drop(write);
            finalize_released_publication_paths(
                self,
                &record,
                Some((
                    FaultPoint::BeforeArtifactAbortDelete,
                    FaultPoint::AfterArtifactAbortDelete,
                )),
            )?;
            return Ok(());
        }
        release_writable_publication(&write, &record)?;
        self.faults.check(FaultPoint::BeforeArtifactAbortCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterArtifactAbortCommit)?;
        let _removed = finalize_released_publication_paths(
            self,
            &record,
            Some((
                FaultPoint::BeforeArtifactAbortDelete,
                FaultPoint::AfterArtifactAbortDelete,
            )),
        )?;
        Ok(())
    }

    fn metadata(
        &self,
        artifact: &ArtifactId,
    ) -> Result<Option<ArtifactMetadata>, PersistenceError> {
        let write = self.database().begin_write().map_err(error::redb)?;
        validated_artifact_metadata_in_transaction(&write, artifact)
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
        let write = self.database().begin_write().map_err(error::redb)?;
        validated_run_artifact_reference_in_transaction(&write, run, reference)
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
        if request
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.created_before() != request.created_before)
        {
            return Err(PersistenceError::InvalidCursor(
                "orphan-cleanup cursor belongs to a different age threshold".to_owned(),
            ));
        }
        let _artifact_serialization = self.lock_artifact_publications()?;
        let mut result = OrphanCleanupResult::default();
        let mut examined = 0_u32;
        let start_family = request.cursor.as_ref().map_or(
            OrphanCleanupFamily::WritablePublications,
            OrphanCleanupCursor::family,
        );
        let mut last_cursor = None;

        if start_family <= OrphanCleanupFamily::WritablePublications {
            let after = request
                .cursor
                .as_ref()
                .filter(|cursor| cursor.family() == OrphanCleanupFamily::WritablePublications)
                .map(OrphanCleanupCursor::after_key);
            if expire_writable_publications(
                self,
                &request,
                after,
                &mut result,
                &mut examined,
                &mut last_cursor,
            )? {
                result.next_cursor = last_cursor;
                return Ok(result);
            }
        }

        if start_family <= OrphanCleanupFamily::TemporaryFiles {
            let after = request
                .cursor
                .as_ref()
                .filter(|cursor| cursor.family() == OrphanCleanupFamily::TemporaryFiles)
                .map(OrphanCleanupCursor::after_key);
            if cleanup_temporary_files(
                self,
                &request,
                after,
                &mut result,
                &mut examined,
                &mut last_cursor,
            )? {
                result.next_cursor = last_cursor;
                return Ok(result);
            }
        }
        if start_family <= OrphanCleanupFamily::ContentFiles
            && cleanup_content_files(
                self,
                &request,
                request
                    .cursor
                    .as_ref()
                    .filter(|cursor| cursor.family() == OrphanCleanupFamily::ContentFiles)
                    .map(OrphanCleanupCursor::after_key),
                &mut result,
                &mut examined,
                &mut last_cursor,
            )?
        {
            result.next_cursor = last_cursor;
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
    validate_artifact_catalog(write)?;
    let table = write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let stored = table
        .get(publication.as_str())
        .map_err(error::redb)?
        .map(|bytes| bytes.value().to_vec());
    drop(table);
    let family = CatalogFamily::ArtifactPublication;
    let logical_key = publication.as_str().as_bytes();
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, logical_key),
        logical_key,
    )?;
    match (stored, witness) {
        (None, None) => Ok(None),
        (Some(bytes), Some(witness)) => {
            if witness != trie::digest_payload(family, &bytes) {
                return Err(error::corruption(
                    "artifact publication disagrees with its authenticated catalog",
                ));
            }
            let record = decode_publication(&bytes)?;
            if record.publication != *publication {
                return Err(error::corruption(
                    "artifact-publication key does not match its document",
                ));
            }
            Ok(Some(record))
        }
        _ => Err(error::corruption(
            "artifact publication and authenticated catalog are incomplete",
        )),
    }
}

fn metadata_in_transaction(
    write: &redb::WriteTransaction,
    artifact: &ArtifactId,
) -> Result<Option<ArtifactMetadata>, PersistenceError> {
    validated_artifact_metadata_in_transaction(write, artifact)
}

pub(crate) fn validated_artifact_metadata_in_transaction(
    write: &redb::WriteTransaction,
    artifact: &ArtifactId,
) -> Result<Option<ArtifactMetadata>, PersistenceError> {
    validate_artifact_catalog(write)?;
    let metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let stored = metadata
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
        .transpose()?;
    drop(metadata);
    let manifest = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let manifested = manifest
        .get(artifact.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode::<ArtifactMetadata>(bytes.value(), "artifact manifest"))
        .transpose()?;
    drop(manifest);
    if stored != manifested {
        return Err(error::corruption(
            "artifact metadata disagrees with its authoritative manifest",
        ));
    }
    let family = CatalogFamily::Artifact;
    let logical_key = artifact.as_str().as_bytes();
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, logical_key),
        logical_key,
    )?;
    let Some(stored) = stored else {
        return if witness.is_none() {
            Ok(None)
        } else {
            Err(error::corruption(
                "authenticated artifact is missing its metadata and manifest",
            ))
        };
    };
    let digest = stored.reference().digest().to_hex();
    let digest_key = codec::pair(&digest, artifact.as_str())?;
    let by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let indexed = by_digest
        .get(digest_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("artifact metadata is absent from its digest index"))?;
    let indexed: ArtifactMetadata = json::decode(indexed.value(), "artifact metadata")?;
    if indexed != stored {
        return Err(error::corruption(
            "artifact digest index disagrees with authoritative metadata",
        ));
    }
    let expected = artifact_catalog_payload(&stored)?;
    if witness != Some(expected) {
        return Err(error::corruption(
            "artifact metadata disagrees with its authenticated catalog",
        ));
    }
    Ok(Some(stored))
}

fn artifact_catalog_payload(
    metadata: &ArtifactMetadata,
) -> Result<[u8; 32], PersistenceError> {
    let bytes = json::encode(metadata, "artifact metadata")?;
    Ok(trie::digest_payload(CatalogFamily::Artifact, &bytes))
}

fn publication_catalog_path(publication: &ArtifactPublicationId) -> [u8; 32] {
    let family = CatalogFamily::ArtifactPublication;
    trie::hashed_path(family, publication.as_str().as_bytes())
}

fn persist_publication_catalog(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
    previous: Option<[u8; 32]>,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactPublication;
    let bytes = json::encode(record, "artifact publication")?;
    let replaced = trie::put(
        write,
        family,
        publication_catalog_path(&record.publication),
        record.publication.as_str().as_bytes(),
        trie::digest_payload(family, &bytes),
    )?;
    if replaced != previous {
        return Err(error::corruption(
            "artifact publication changed outside its authoritative transaction",
        ));
    }
    Ok(())
}

fn remove_publication_catalog(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactPublication;
    let bytes = json::encode(record, "artifact publication")?;
    let removed = trie::remove(
        write,
        family,
        publication_catalog_path(&record.publication),
        record.publication.as_str().as_bytes(),
    )?;
    if removed != Some(trie::digest_payload(family, &bytes)) {
        return Err(error::corruption(
            "artifact publication catalog is incomplete during removal",
        ));
    }
    Ok(())
}

pub(crate) fn validate_artifact_catalog(
    write: &redb::WriteTransaction,
) -> Result<ArtifactAccountingRecord, PersistenceError> {
    let accounting = write.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
    if accounting.len().map_err(error::redb)? != 1 {
        return Err(error::corruption(
            "artifact accounting must contain exactly one checked document",
        ));
    }
    let stored = accounting
        .get(GLOBAL_ARTIFACT_BYTES_KEY)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("artifact accounting document is missing"))?;
    let stored: ArtifactAccountingRecord = json::decode(stored.value(), "artifact accounting")?;
    if stored.schema_version != ARTIFACT_ACCOUNTING_SCHEMA_VERSION {
        return Err(PersistenceError::UnsupportedVersion {
            document: "artifact_accounting",
            found: stored.schema_version,
            supported: ARTIFACT_ACCOUNTING_SCHEMA_VERSION,
        });
    }
    crate::trie::validate_roots_in_transaction(write)?;
    Ok(stored)
}

pub(crate) fn persist_artifact_accounting(
    write: &redb::WriteTransaction,
    accounting: &ArtifactAccountingRecord,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(accounting, "artifact accounting")?;
    write
        .open_table(ARTIFACT_ACCOUNTING)
        .map_err(error::redb)?
        .insert(GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn validated_global_artifact_bytes(
    write: &redb::WriteTransaction,
) -> Result<u64, PersistenceError> {
    Ok(validate_artifact_catalog(write)?.committed_content_bytes)
}

pub(crate) fn validated_run_artifact_reference_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    validate_artifact_catalog(write)?;
    let indexed = indexed_run_artifact_reference(write, run, reference)?;
    let authoritative = manifested_run_artifact_reference(write, run, reference)?;
    if indexed != authoritative {
        return Err(error::corruption(format!(
            "artifact-reference index disagrees with authoritative ownership for run {run} and artifact {}",
            reference.artifact()
        )));
    }
    Ok(authoritative)
}

fn indexed_run_artifact_reference(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let digest = reference.digest().to_hex();
    let prefix = codec::components(&[&digest, reference.artifact().as_str(), run.as_str()])?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("artifact-reference prefix has no range end"))?;
    let table = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    let item = table
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?;
    if let Some((key, bytes)) = item {
        let key = key.value().to_vec();
        let bytes = bytes.value().to_vec();
        let family = CatalogFamily::ArtifactReferenceOccurrence;
        let witness = trie::verify_member_in_transaction(
            write,
            family,
            trie::hashed_path(family, &key),
            &key,
        )?;
        if witness != Some(trie::digest_payload(family, &bytes)) {
            return Err(error::corruption(
                "artifact-reference occurrence disagrees with its authenticated catalog",
            ));
        }
        let stored: ArtifactReference = json::decode(&bytes, "artifact reference")?;
        if &stored != reference {
            return Err(error::corruption(
                "artifact-reference index prefix contradicts its stored document",
            ));
        }
        Ok(true)
    } else {
        Ok(false)
    }
}

fn manifested_run_artifact_reference(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let digest = reference.digest().to_hex();
    let key = codec::components(&[run.as_str(), &digest, reference.artifact().as_str()])?;
    let ownership = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    let stored = ownership
        .get(key.as_slice())
        .map_err(error::redb)?
        .map(|bytes| json::decode::<ArtifactReference>(bytes.value(), "run artifact ownership"))
        .transpose()?;
    drop(ownership);
    let family = CatalogFamily::RunArtifactOwnership;
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
    )?;
    match (stored, witness) {
        (None, None) => Ok(false),
        (Some(stored), Some(witness)) if &stored == reference => {
            let bytes = json::encode(&stored, "run artifact ownership")?;
            if witness != trie::digest_payload(family, &bytes) {
                return Err(error::corruption(
                    "run artifact ownership disagrees with its authenticated catalog",
                ));
            }
            Ok(true)
        }
        (Some(_), Some(_)) => Err(error::corruption(
            "run artifact-ownership key contradicts its stored document",
        )),
        _ => Err(error::corruption(
            "run artifact ownership and authenticated catalog are incomplete",
        )),
    }
}

pub(crate) fn persist_artifact_reference_occurrence(
    write: &redb::WriteTransaction,
    key: &[u8],
    reference: &ArtifactReference,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(reference, "artifact reference")?;
    let prior = write
        .open_table(ARTIFACT_REFERENCES)
        .map_err(error::redb)?
        .get(key)
        .map_err(error::redb)?
        .map(|stored| stored.value().to_vec());
    if prior.is_some() {
        return Err(error::corruption(
            "artifact reference occurrence already exists before its authoritative append",
        ));
    }
    write
        .open_table(ARTIFACT_REFERENCES)
        .map_err(error::redb)?
        .insert(key, bytes.as_slice())
        .map_err(error::redb)?;
    let family = CatalogFamily::ArtifactReferenceOccurrence;
    if trie::put(
        write,
        family,
        trie::hashed_path(family, key),
        key,
        trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "artifact reference occurrence catalog already contains the new identity",
        ));
    }
    Ok(())
}

pub(crate) fn persist_run_artifact_ownership(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<(), PersistenceError> {
    let digest = reference.digest().to_hex();
    let key = codec::components(&[run.as_str(), &digest, reference.artifact().as_str()])?;
    let bytes = json::encode(reference, "run artifact ownership")?;
    let previous = {
        let table = write
            .open_table(RUN_ARTIFACT_OWNERSHIP)
            .map_err(error::redb)?;
        table
            .get(key.as_slice())
            .map_err(error::redb)?
            .map(|stored| stored.value().to_vec())
    };
    let family = CatalogFamily::RunArtifactOwnership;
    let prior_witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
    )?;
    match (previous.as_deref(), prior_witness) {
        (Some(stored), Some(witness)) => {
            let decoded: ArtifactReference = json::decode(stored, "run artifact ownership")?;
            if decoded != *reference || witness != trie::digest_payload(family, stored) {
                return Err(error::corruption(
                    "existing run artifact ownership disagrees with its catalog",
                ));
            }
            return Ok(());
        }
        (None, None) => {}
        _ => {
            return Err(error::corruption(
                "run artifact ownership and authenticated catalog are incomplete",
            ));
        }
    }
    write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    if trie::put(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
        trie::digest_payload(family, &bytes),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "run artifact ownership catalog already contains the new identity",
        ));
    }
    Ok(())
}

const fn usage_covers(current: WorkspaceUsage, historical: WorkspaceUsage) -> bool {
    current.value_versions() >= historical.value_versions()
        && current.inline_bytes() >= historical.inline_bytes()
        && current.artifacts() >= historical.artifacts()
        && current.artifact_bytes() >= historical.artifact_bytes()
}

pub(crate) fn materialize_legacy_writable_workspace_domains(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let publications = write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let mut usages = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    for item in publications.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let record = decode_publication(bytes.value())?;
        if record.publication.as_str() != key.value() {
            return Err(error::corruption(
                "legacy artifact publication key disagrees with its document",
            ));
        }
        let budgets = write
            .open_table(crate::schema::WORKSPACE_BUDGETS)
            .map_err(error::redb)?;
        let budget_bytes = budgets
            .get(record.run.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption("legacy artifact publication is missing its workspace budget")
            })?;
        let budget: milkdrift_workspace::WorkspaceBudget =
            json::decode(budget_bytes.value(), "workspace budget")?;
        if budget != record.budget {
            return Err(error::corruption(
                "legacy artifact publication budget disagrees with its workspace domain",
            ));
        }
        let actual = usages
            .get(record.run.as_str())
            .map_err(error::redb)?
            .map(|bytes| json::decode::<WorkspaceUsage>(bytes.value(), "workspace usage"))
            .transpose()?;
        let expected = match record.state {
            PublicationState::Writable => {
                validate_writable_publication_indexes(write, &record)?;
                record.expected_usage
            }
            PublicationState::Committed { .. } => record.resulting_usage,
            PublicationState::Released => {
                return Err(error::corruption(
                    "legacy storage unexpectedly contains a released publication tombstone",
                ));
            }
        };
        match actual {
            Some(actual)
                if matches!(record.state, PublicationState::Writable) && actual == expected => {}
            Some(actual)
                if matches!(record.state, PublicationState::Committed { .. })
                    && usage_covers(actual, expected) => {}
            None if matches!(record.state, PublicationState::Writable)
                && expected == WorkspaceUsage::EMPTY =>
            {
                let bytes = json::encode(&WorkspaceUsage::EMPTY, "workspace usage")?;
                usages
                    .insert(record.run.as_str(), bytes.as_slice())
                    .map_err(error::redb)?;
            }
            Some(_) => {
                return Err(error::corruption(
                    "legacy artifact publication usage disagrees with its state",
                ));
            }
            None => {
                return Err(error::corruption(
                    "legacy artifact publication is missing workspace usage",
                ));
            }
        }
    }
    drop(usages);
    drop(publications);

    let budgets = write
        .open_table(crate::schema::WORKSPACE_BUDGETS)
        .map_err(error::redb)?;
    let usages = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
    for item in budgets.iter().map_err(error::redb)? {
        let (run, _) = item.map_err(error::redb)?;
        if usages.get(run.value()).map_err(error::redb)?.is_none() {
            return Err(error::corruption(
                "legacy workspace budget has no usage domain or writable publication",
            ));
        }
    }
    for item in usages.iter().map_err(error::redb)? {
        let (run, _) = item.map_err(error::redb)?;
        if budgets.get(run.value()).map_err(error::redb)?.is_none() {
            return Err(error::corruption(
                "legacy workspace usage has no immutable budget",
            ));
        }
    }
    Ok(())
}

pub(crate) fn upgrade_artifact_accounting(
    write: &redb::WriteTransaction,
) -> Result<(), PersistenceError> {
    let metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let manifest = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let artifact_count = metadata.len().map_err(error::redb)?;
    if manifest.len().map_err(error::redb)? != artifact_count
        || by_digest.len().map_err(error::redb)? != artifact_count
    {
        return Err(error::corruption(
            "legacy artifact catalog tables have different cardinality",
        ));
    }
    let mut current_digest: Option<String> = None;
    let mut current_size = 0_u64;
    let mut committed_content_bytes = 0_u64;
    for item in by_digest.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let components = codec::decode_components(key.value(), 2)?;
        let document: ArtifactMetadata = json::decode(bytes.value(), "artifact metadata")?;
        if components[0] != document.reference().digest().to_hex()
            || components[1] != document.reference().artifact().as_str()
        {
            return Err(error::corruption(
                "legacy artifact digest key disagrees with its metadata",
            ));
        }
        if current_digest.as_deref() == Some(components[0]) {
            if current_size != document.reference().size_bytes() {
                return Err(error::corruption(
                    "legacy artifacts disagree on the size of one digest",
                ));
            }
        } else {
            committed_content_bytes = committed_content_bytes
                .checked_add(document.reference().size_bytes())
                .ok_or_else(|| error::corruption("legacy artifact byte count overflowed"))?;
            current_digest = Some(components[0].to_owned());
            current_size = document.reference().size_bytes();
        }
        let primary = metadata
            .get(document.reference().artifact().as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy artifact digest row has no primary row"))?;
        let manifested = manifest
            .get(document.reference().artifact().as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("legacy artifact digest row has no manifest"))?;
        if json::decode::<ArtifactMetadata>(primary.value(), "artifact metadata")? != document
            || json::decode::<ArtifactMetadata>(manifested.value(), "artifact manifest")?
                != document
        {
            return Err(error::corruption("legacy artifact catalog rows disagree"));
        }
        let family = CatalogFamily::Artifact;
        let logical_key = document.reference().artifact().as_str().as_bytes();
        if trie::put(
            write,
            family,
            trie::hashed_path(family, logical_key),
            logical_key,
            artifact_catalog_payload(&document)?,
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy artifact catalog contains a duplicate authenticated artifact",
            ));
        }
    }
    drop(by_digest);
    drop(manifest);
    drop(metadata);

    let publications = write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let mut writable_publication_count = 0_u64;
    let mut committed_publication_count = 0_u64;
    for item in publications.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let record = decode_publication(bytes.value())?;
        if record.publication.as_str() != key.value() {
            return Err(error::corruption(
                "legacy artifact publication key disagrees with its document",
            ));
        }
        persist_publication_catalog(write, &record, None)?;
        match record.state {
            PublicationState::Writable => {
                validate_writable_publication_indexes(write, &record)?;
                put_artifact_path(
                    write,
                    &artifact_path_entry(&record, ArtifactPathKind::TempReady)?,
                    None,
                )?;
                writable_publication_count = writable_publication_count
                    .checked_add(1)
                    .ok_or_else(|| error::corruption("writable publication count overflowed"))?;
            }
            PublicationState::Committed { .. } => {
                validate_legacy_committed_publication(write, &record)?;
                committed_publication_count = committed_publication_count
                    .checked_add(1)
                    .ok_or_else(|| error::corruption("committed publication count overflowed"))?;
            }
            PublicationState::Released => {
                return Err(error::corruption(
                    "legacy storage unexpectedly contains a released publication tombstone",
                ));
            }
        }
    }
    if committed_publication_count != artifact_count {
        return Err(error::corruption(
            "legacy committed publications disagree with artifact catalog cardinality",
        ));
    }
    drop(publications);
    let references = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    for item in references.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let components = match codec::decode_components(key.value(), 4) {
            Ok(components) => components,
            Err(_) => {
                let components = codec::decode_components(key.value(), 5)?;
                if components[3] != "publication" {
                    return Err(error::corruption(
                        "legacy artifact occurrence has an unknown five-part identity",
                    ));
                }
                components
            }
        };
        let reference: ArtifactReference = json::decode(bytes.value(), "artifact reference")?;
        if components[0] != reference.digest().to_hex()
            || components[1] != reference.artifact().as_str()
        {
            return Err(error::corruption(
                "legacy artifact occurrence key disagrees with its reference",
            ));
        }
        let metadata = {
            let table = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
            table
                .get(reference.artifact().as_str())
                .map_err(error::redb)?
                .map(|stored| stored.value().to_vec())
                .ok_or_else(|| error::corruption("legacy artifact occurrence has no metadata"))?
        };
        let metadata: ArtifactMetadata = json::decode(&metadata, "artifact metadata")?;
        if metadata.reference() != &reference {
            return Err(error::corruption(
                "legacy artifact occurrence disagrees with artifact metadata",
            ));
        }
        let ownership_key = codec::components(&[
            components[2],
            &reference.digest().to_hex(),
            reference.artifact().as_str(),
        ])?;
        let ownership = {
            let table = write
                .open_table(RUN_ARTIFACT_OWNERSHIP)
                .map_err(error::redb)?;
            table
                .get(ownership_key.as_slice())
                .map_err(error::redb)?
                .map(|stored| stored.value().to_vec())
                .ok_or_else(|| {
                    error::corruption("legacy artifact occurrence has no run ownership")
                })?
        };
        if json::decode::<ArtifactReference>(&ownership, "run artifact ownership")?
            != reference
        {
            return Err(error::corruption(
                "legacy artifact occurrence disagrees with run ownership",
            ));
        }
        let family = CatalogFamily::ArtifactReferenceOccurrence;
        if trie::put(
            write,
            family,
            trie::hashed_path(family, key.value()),
            key.value(),
            trie::digest_payload(family, bytes.value()),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy artifact occurrence catalog contains a duplicate identity",
            ));
        }
    }
    drop(references);
    let ownership = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    for item in ownership.iter().map_err(error::redb)? {
        let (key, bytes) = item.map_err(error::redb)?;
        let components = codec::decode_components(key.value(), 3)?;
        let reference: ArtifactReference = json::decode(bytes.value(), "run artifact ownership")?;
        if components[1] != reference.digest().to_hex()
            || components[2] != reference.artifact().as_str()
        {
            return Err(error::corruption(
                "legacy run artifact ownership key disagrees with its reference",
            ));
        }
        let occurrence_prefix = codec::components(&[
            components[1],
            components[2],
            components[0],
        ])?;
        let end = codec::prefix_end(occurrence_prefix.clone()).ok_or_else(|| {
            error::corruption("legacy artifact occurrence prefix has no range end")
        })?;
        if write
            .open_table(ARTIFACT_REFERENCES)
            .map_err(error::redb)?
            .range(occurrence_prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .is_none()
        {
            return Err(error::corruption(
                "legacy run artifact ownership has no reference occurrence",
            ));
        }
        let family = CatalogFamily::RunArtifactOwnership;
        if trie::put(
            write,
            family,
            trie::hashed_path(family, key.value()),
            key.value(),
            trie::digest_payload(family, bytes.value()),
        )?
        .is_some()
        {
            return Err(error::corruption(
                "legacy run artifact ownership catalog contains a duplicate identity",
            ));
        }
    }
    drop(ownership);
    let mut accounting = write.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
    if let Some(bytes) = accounting
        .get(GLOBAL_ARTIFACT_BYTES_KEY)
        .map_err(error::redb)?
    {
        let legacy: LegacyArtifactAccountingDocument =
            json::decode(bytes.value(), "artifact accounting")?;
        let (schema_version, stored_content_bytes) = match legacy {
            LegacyArtifactAccountingDocument::V1(record) => {
                (record.schema_version, record.committed_content_bytes)
            }
            LegacyArtifactAccountingDocument::V2(record) => {
                if record.artifact_count != artifact_count
                    || record.reference_occurrence_count
                        != write
                            .open_table(ARTIFACT_REFERENCES)
                            .map_err(error::redb)?
                            .len()
                            .map_err(error::redb)?
                    || record.writable_publication_count != writable_publication_count
                    || record.committed_publication_count != committed_publication_count
                {
                    return Err(error::corruption(
                        "legacy artifact integrity counters disagree with their tables",
                    ));
                }
                (record.schema_version, record.committed_content_bytes)
            }
        };
        if !matches!(schema_version, 1 | 2) || stored_content_bytes != committed_content_bytes {
            return Err(error::corruption(
                "legacy artifact accounting disagrees with committed content",
            ));
        }
    } else if committed_content_bytes != 0 {
        return Err(error::corruption(
            "legacy committed artifacts have no aggregate accounting",
        ));
    }
    if accounting.len().map_err(error::redb)? > 1 {
        return Err(error::corruption(
            "legacy artifact accounting contains unknown rows",
        ));
    }
    let upgraded = ArtifactAccountingRecord {
        schema_version: ARTIFACT_ACCOUNTING_SCHEMA_VERSION,
        committed_content_bytes,
    };
    let bytes = json::encode(&upgraded, "artifact accounting")?;
    accounting
        .insert(GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

fn validate_legacy_committed_publication(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    let artifact = record.metadata.reference().artifact();
    let digest = record.metadata.reference().digest().to_hex();
    {
        let metadata = write.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
        let stored = metadata
            .get(artifact.as_str())
            .map_err(error::redb)?
            .ok_or_else(|| error::corruption("committed publication has no artifact metadata"))?;
        if json::decode::<ArtifactMetadata>(stored.value(), "artifact metadata")? != record.metadata
        {
            return Err(error::corruption(
                "committed publication disagrees with artifact metadata",
            ));
        }
    }

    let occurrence_key = codec::components(&[
        &digest,
        artifact.as_str(),
        record.run.as_str(),
        "publication",
        record.publication.as_str(),
    ])?;
    {
        let occurrences = write.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
        let occurrence = occurrences
            .get(occurrence_key.as_slice())
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption("committed publication has no reference occurrence")
            })?;
        if json::decode::<ArtifactReference>(occurrence.value(), "artifact reference")?
            != *record.metadata.reference()
        {
            return Err(error::corruption(
                "committed publication reference occurrence disagrees with its metadata",
            ));
        }
    }

    let ownership_key = codec::components(&[record.run.as_str(), &digest, artifact.as_str()])?;
    let ownership = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    let owned = ownership
        .get(ownership_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("committed publication has no run ownership"))?;
    if json::decode::<ArtifactReference>(owned.value(), "run artifact ownership")?
        != *record.metadata.reference()
    {
        return Err(error::corruption(
            "committed publication ownership disagrees with its metadata",
        ));
    }
    Ok(())
}

fn commit_artifact_metadata(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    record: &mut PublicationRecord,
    content_deduplicated: bool,
) -> Result<(), PersistenceError> {
    let mut artifact_accounting = validate_artifact_catalog(write)?;
    let previous_publication_bytes = json::encode(record, "artifact publication")?;
    let publication_family = CatalogFamily::ArtifactPublication;
    let previous_publication = trie::digest_payload(
        publication_family,
        &previous_publication_bytes,
    );
    let previous_artifact = validated_artifact_metadata_in_transaction(
        write,
        record.metadata.reference().artifact(),
    )?;
    if previous_artifact
        .as_ref()
        .is_some_and(|metadata| metadata != &record.metadata)
    {
        return Err(PersistenceError::ImmutableConflict {
            entity: "artifact",
            identity: record.metadata.reference().artifact().to_string(),
        });
    }
    let current_content_bytes = artifact_accounting.committed_content_bytes;
    crate::journal::advance_workspace_global_usage_in_transaction(
        write,
        &record.run,
        record.expected_usage,
        record.resulting_usage,
    )?;
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
        let existing = by_digest
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?;
        if let Some((_, bytes)) = existing {
            let existing: ArtifactMetadata = json::decode(bytes.value(), "artifact metadata")?;
            if existing.reference().digest() != record.metadata.reference().digest()
                || existing.reference().size_bytes() != record.metadata.reference().size_bytes()
            {
                return Err(error::corruption(
                    "one artifact digest is associated with contradictory content sizes",
                ));
            }
            true
        } else {
            false
        }
    };
    {
        let mut by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
        by_digest
            .insert(digest_key.as_slice(), metadata_bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let bytes = json::encode(&record.metadata, "artifact manifest")?;
        let mut manifest = write.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
        if let Some(existing) = manifest
            .get(record.metadata.reference().artifact().as_str())
            .map_err(error::redb)?
        {
            let existing: ArtifactMetadata = json::decode(existing.value(), "artifact manifest")?;
            if existing != record.metadata {
                return Err(error::corruption(
                    "artifact manifest conflicts with committed metadata",
                ));
            }
        } else {
            manifest
                .insert(
                    record.metadata.reference().artifact().as_str(),
                    bytes.as_slice(),
                )
                .map_err(error::redb)?;
        }
    }
    {
        let family = CatalogFamily::Artifact;
        let logical_key = record.metadata.reference().artifact().as_str().as_bytes();
        let replaced = trie::put(
            write,
            family,
            trie::hashed_path(family, logical_key),
            logical_key,
            artifact_catalog_payload(&record.metadata)?,
        )?;
        let expected = previous_artifact
            .as_ref()
            .map(artifact_catalog_payload)
            .transpose()?;
        if replaced != expected {
            return Err(error::corruption(
                "artifact catalog changed outside its authoritative transaction",
            ));
        }
    }
    let resulting_content_bytes = if digest_was_known {
        current_content_bytes
    } else {
        current_content_bytes
            .checked_add(record.metadata.reference().size_bytes())
            .ok_or_else(|| PersistenceError::Storage {
                class: StorageFailureClass::ResourceExhausted,
                message: "global artifact-byte accounting overflow".to_owned(),
            })?
    };
    if resulting_content_bytes > store.max_total_artifact_bytes {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::ResourceExhausted,
            message: "global artifact-byte limit exceeded".to_owned(),
        });
    }
    {
        artifact_accounting.committed_content_bytes = resulting_content_bytes;
        let bytes = json::encode(&artifact_accounting, "artifact accounting")?;
        let mut accounting = write.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;
        accounting
            .insert(GLOBAL_ARTIFACT_BYTES_KEY, bytes.as_slice())
            .map_err(error::redb)?;
    }
    {
        let usage_bytes = json::encode(&record.resulting_usage, "workspace usage")?;
        let mut usage = write.open_table(WORKSPACE_USAGE).map_err(error::redb)?;
        usage
            .insert(record.run.as_str(), usage_bytes.as_slice())
            .map_err(error::redb)?;
    }
    crate::journal::persist_workspace_value_usage_accounting_in_transaction(
        write,
        &record.run,
        record.resulting_usage,
    )?;
    {
        let key = codec::components(&[
            &digest,
            record.metadata.reference().artifact().as_str(),
            record.run.as_str(),
            "publication",
            record.publication.as_str(),
        ])?;
        persist_artifact_reference_occurrence(write, &key, record.metadata.reference())?;
    }
    persist_run_artifact_ownership(write, &record.run, record.metadata.reference())?;
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
    persist_publication_catalog(write, record, Some(previous_publication))?;
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
        let removed = owners.remove(temp_name.as_str()).map_err(error::redb)?;
        drop(removed);
        drop(owners);
        remove_temporary_manifest(write, &temp_name, &record.publication)?;
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
    remove_artifact_path(
        write,
        &artifact_path_entry(record, ArtifactPathKind::ContentIntent)?,
    )?;
    validate_artifact_catalog(write)?;
    crate::journal::validate_workspace_value_accounting_in_transaction(write)?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArtifactPathKind {
    TempPending,
    TempReady,
    ContentIntent,
}

impl ArtifactPathKind {
    const fn label(self) -> &'static str {
        match self {
            Self::TempPending => "temp_pending",
            Self::TempReady => "temp_ready",
            Self::ContentIntent => "content_intent",
        }
    }

    const fn ordered_tag(self) -> u8 {
        match self {
            Self::TempPending | Self::TempReady => 0,
            Self::ContentIntent => 1,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactPathEntry {
    kind: ArtifactPathKind,
    created_at_millis: u64,
    publication: ArtifactPublicationId,
    identity: String,
    logical_key: Vec<u8>,
    path: [u8; 32],
}

fn artifact_path_entry(
    record: &PublicationRecord,
    kind: ArtifactPathKind,
) -> Result<ArtifactPathEntry, PersistenceError> {
    let identity = match kind {
        ArtifactPathKind::TempPending | ArtifactPathKind::TempReady => {
            publication_temp_name(&record.publication)
        }
        ArtifactPathKind::ContentIntent => record.metadata.reference().digest().to_hex(),
    };
    let created_at = format!("{:020}", record.created_at_millis);
    let logical_key = codec::components(&[
        kind.label(),
        &created_at,
        &identity,
        record.publication.as_str(),
    ])?;
    let mut ordered = [0_u8; 9];
    ordered[0] = kind.ordered_tag();
    ordered[1..].copy_from_slice(&record.created_at_millis.to_be_bytes());
    Ok(ArtifactPathEntry {
        kind,
        created_at_millis: record.created_at_millis,
        publication: record.publication.clone(),
        identity,
        path: trie::ordered_path(CatalogFamily::ArtifactPath, &ordered, &logical_key)?,
        logical_key,
    })
}

fn decode_artifact_path_entry(leaf: &trie::TrieLeaf) -> Result<ArtifactPathEntry, PersistenceError> {
    let components = codec::decode_components(&leaf.logical_key, 4)?;
    let kind = match components[0] {
        "temp_pending" => ArtifactPathKind::TempPending,
        "temp_ready" => ArtifactPathKind::TempReady,
        "content_intent" => ArtifactPathKind::ContentIntent,
        _ => return Err(error::corruption("artifact path catalog contains an unknown kind")),
    };
    if components[1].len() != 20 || !components[1].bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error::corruption(
            "artifact path catalog contains an invalid timestamp",
        ));
    }
    let created_at_millis = components[1]
        .parse::<u64>()
        .map_err(|cause| error::corruption(format!("invalid artifact path timestamp: {cause}")))?;
    let publication = ArtifactPublicationId::new(components[3]).map_err(|cause| {
        error::corruption(format!("invalid artifact path publication identity: {cause}"))
    })?;
    let mut ordered = [0_u8; 9];
    ordered[0] = kind.ordered_tag();
    ordered[1..].copy_from_slice(&created_at_millis.to_be_bytes());
    let expected_path =
        trie::ordered_path(CatalogFamily::ArtifactPath, &ordered, &leaf.logical_key)?;
    if leaf.path != expected_path
        || leaf.payload_digest
            != trie::digest_payload(CatalogFamily::ArtifactPath, &leaf.logical_key)
    {
        return Err(error::corruption(
            "artifact path entry disagrees with its authenticated leaf",
        ));
    }
    Ok(ArtifactPathEntry {
        kind,
        created_at_millis,
        publication,
        identity: components[2].to_owned(),
        logical_key: leaf.logical_key.clone(),
        path: expected_path,
    })
}

pub(crate) fn validate_catalog_leaf(
    read: &redb::ReadTransaction,
    family: CatalogFamily,
    leaf: &trie::TrieLeaf,
) -> Result<(), PersistenceError> {
    match family {
        CatalogFamily::Artifact => {
            let key = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("artifact catalog identity is not UTF-8"))?;
            let bytes = read
                .open_table(ARTIFACT_METADATA)
                .map_err(error::redb)?
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact catalog leaf is dangling"))?
                .value()
                .to_vec();
            let metadata: ArtifactMetadata = json::decode(&bytes, "artifact metadata")?;
            if metadata.reference().artifact().as_str() != key
                || leaf.path != trie::hashed_path(family, &leaf.logical_key)
                || leaf.payload_digest != artifact_catalog_payload(&metadata)?
            {
                return Err(error::corruption(
                    "artifact catalog leaf disagrees with its metadata",
                ));
            }
            let manifest = read
                .open_table(ARTIFACT_MANIFEST)
                .map_err(error::redb)?
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact catalog has no manifest row"))?;
            if json::decode::<ArtifactMetadata>(manifest.value(), "artifact manifest")? != metadata {
                return Err(error::corruption("artifact manifest disagrees with its catalog"));
            }
            let digest_key = codec::pair(&metadata.reference().digest().to_hex(), key)?;
            let indexed = read
                .open_table(ARTIFACTS_BY_DIGEST)
                .map_err(error::redb)?
                .get(digest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact catalog has no digest row"))?;
            if json::decode::<ArtifactMetadata>(indexed.value(), "artifact metadata")? != metadata {
                return Err(error::corruption("artifact digest row disagrees with its catalog"));
            }
            Ok(())
        }
        CatalogFamily::ArtifactPublication => {
            let key = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("publication catalog identity is not UTF-8"))?;
            let bytes = read
                .open_table(ARTIFACT_PUBLICATIONS)
                .map_err(error::redb)?
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("publication catalog leaf is dangling"))?
                .value()
                .to_vec();
            let record = decode_publication(&bytes)?;
            if record.publication.as_str() != key
                || leaf.path != trie::hashed_path(family, &leaf.logical_key)
                || leaf.payload_digest != trie::digest_payload(family, &bytes)
            {
                return Err(error::corruption(
                    "publication catalog leaf disagrees with its document",
                ));
            }
            Ok(())
        }
        CatalogFamily::ArtifactPath => decode_artifact_path_entry(leaf).map(|_| ()),
        CatalogFamily::RunArtifactOwnership => {
            validate_binary_artifact_leaf(read, RUN_ARTIFACT_OWNERSHIP, family, leaf, "run artifact ownership")
        }
        CatalogFamily::ArtifactReferenceOccurrence => {
            validate_binary_artifact_leaf(read, ARTIFACT_REFERENCES, family, leaf, "artifact reference")
        }
        CatalogFamily::ArtifactDeleteGuard => {
            let components = codec::decode_components(&leaf.logical_key, 2)?;
            if !matches!(components[0], "temp" | "content")
                || components[1].is_empty()
                || leaf.path != trie::hashed_path(family, &leaf.logical_key)
                || leaf.payload_digest != trie::digest_payload(family, &leaf.logical_key)
            {
                return Err(error::corruption("artifact delete guard is malformed"));
            }
            Ok(())
        }
        _ => Err(error::corruption(
            "artifact catalog validator received another family's leaf",
        )),
    }
}

fn validate_binary_artifact_leaf(
    read: &redb::ReadTransaction,
    table: redb::TableDefinition<'static, &'static [u8], &'static [u8]>,
    family: CatalogFamily,
    leaf: &trie::TrieLeaf,
    label: &'static str,
) -> Result<(), PersistenceError> {
    let bytes = read
        .open_table(table)
        .map_err(error::redb)?
        .get(leaf.logical_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption(format!("{label} catalog leaf is dangling")))?
        .value()
        .to_vec();
    if leaf.path != trie::hashed_path(family, &leaf.logical_key)
        || leaf.payload_digest != trie::digest_payload(family, &bytes)
    {
        return Err(error::corruption(format!(
            "{label} catalog leaf disagrees with its physical row"
        )));
    }
    Ok(())
}

fn put_artifact_path(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
    expected_previous: Option<[u8; 32]>,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactPath;
    let replaced = trie::put(
        write,
        family,
        entry.path,
        &entry.logical_key,
        trie::digest_payload(family, &entry.logical_key),
    )?;
    if replaced != expected_previous {
        return Err(error::corruption(
            "artifact path inventory changed outside its authoritative transaction",
        ));
    }
    Ok(())
}

fn remove_artifact_path(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactPath;
    let removed = trie::remove(write, family, entry.path, &entry.logical_key)?;
    if removed != Some(trie::digest_payload(family, &entry.logical_key)) {
        return Err(error::corruption(
            "artifact path inventory is absent during finalization",
        ));
    }
    Ok(())
}

fn artifact_path_exists(
    write: &redb::WriteTransaction,
    entry: &ArtifactPathEntry,
) -> Result<bool, PersistenceError> {
    let family = CatalogFamily::ArtifactPath;
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        entry.path,
        &entry.logical_key,
    )?;
    match witness {
        None => Ok(false),
        Some(witness) if witness == trie::digest_payload(family, &entry.logical_key) => Ok(true),
        Some(_) => Err(error::corruption(
            "artifact path inventory payload is invalid",
        )),
    }
}

fn artifact_delete_guard_key(kind: ArtifactPathKind, identity: &str) -> Result<Vec<u8>, PersistenceError> {
    let label = match kind {
        ArtifactPathKind::TempPending | ArtifactPathKind::TempReady => "temp",
        ArtifactPathKind::ContentIntent => "content",
    };
    codec::components(&[label, identity])
}

fn artifact_delete_guard_exists(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<bool, PersistenceError> {
    let family = CatalogFamily::ArtifactDeleteGuard;
    let key = artifact_delete_guard_key(kind, identity)?;
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
    )?;
    match witness {
        None => Ok(false),
        Some(witness) if witness == trie::digest_payload(family, &key) => Ok(true),
        Some(_) => Err(error::corruption("artifact delete guard payload is invalid")),
    }
}

fn put_artifact_delete_guard(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactDeleteGuard;
    let key = artifact_delete_guard_key(kind, identity)?;
    if trie::put(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
        trie::digest_payload(family, &key),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "artifact delete guard was created twice",
        ));
    }
    Ok(())
}

fn remove_artifact_delete_guard(
    write: &redb::WriteTransaction,
    kind: ArtifactPathKind,
    identity: &str,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::ArtifactDeleteGuard;
    let key = artifact_delete_guard_key(kind, identity)?;
    let removed = trie::remove(
        write,
        family,
        trie::hashed_path(family, &key),
        &key,
    )?;
    if removed != Some(trie::digest_payload(family, &key)) {
        return Err(error::corruption("artifact delete guard is absent at finalization"));
    }
    Ok(())
}

fn now_millis() -> Result<u64, PersistenceError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|cause| error::corruption(format!("system clock precedes epoch: {cause}")))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| error::corruption("system timestamp exceeds u64 milliseconds"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TempInventoryState {
    Pending,
    Ready,
}

fn temp_inventory_state(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<TempInventoryState, PersistenceError> {
    let pending = artifact_path_entry(record, ArtifactPathKind::TempPending)?;
    let ready = artifact_path_entry(record, ArtifactPathKind::TempReady)?;
    match (
        artifact_path_exists(write, &pending)?,
        artifact_path_exists(write, &ready)?,
    ) {
        (true, false) => Ok(TempInventoryState::Pending),
        (false, true) => Ok(TempInventoryState::Ready),
        (false, false) => Err(error::corruption(
            "artifact publication is missing its temporary-path inventory",
        )),
        (true, true) => Err(error::corruption(
            "artifact publication has conflicting temporary-path inventory states",
        )),
    }
}

fn ensure_temp_inventory_ready(
    store: &RedbStore,
    publication: &ArtifactPublicationId,
) -> Result<(), PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    let record = publication_in_transaction(&write, publication)?;
    if !matches!(record.state, PublicationState::Writable) {
        return Err(error::corruption(
            "only a writable publication may materialize a temporary path",
        ));
    }
    match temp_inventory_state(&write, &record)? {
        TempInventoryState::Ready => return Ok(()),
        TempInventoryState::Pending => {}
    }
    let temp_name = publication_temp_name(publication);
    if artifact_delete_guard_exists(&write, ArtifactPathKind::TempReady, &temp_name)? {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            message: "temporary artifact path is being durably finalized".to_owned(),
        });
    }
    let path = store.temp_path(publication);
    store.faults.check(FaultPoint::BeforeArtifactTempCreate)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if metadata.len() != 0 {
                return Err(error::corruption(
                    "pending temporary artifact path contains nonempty bytes",
                ));
            }
            open_regular_for_read(&path)?.sync_all().map_err(error::io)?;
            sync_directory(&store.temp_root)?;
        }
        Ok(_) => {
            return Err(error::corruption(
                "pending temporary artifact path is not a regular file",
            ));
        }
        Err(cause) if cause.kind() == std::io::ErrorKind::NotFound => {
            prepare_new_temp(&path, &store.temp_root)?;
        }
        Err(cause) => return Err(error::io(cause)),
    }
    store.faults.check(FaultPoint::AfterArtifactTempCreate)?;
    let pending = artifact_path_entry(&record, ArtifactPathKind::TempPending)?;
    let ready = artifact_path_entry(&record, ArtifactPathKind::TempReady)?;
    remove_artifact_path(&write, &pending)?;
    put_artifact_path(&write, &ready, None)?;
    store
        .faults
        .check(FaultPoint::BeforeArtifactTempReadyCommit)?;
    write.commit().map_err(error::redb)?;
    store
        .faults
        .check(FaultPoint::AfterArtifactTempReadyCommit)
}

fn require_temp_inventory_ready(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    if temp_inventory_state(write, record)? != TempInventoryState::Ready {
        return Err(error::corruption(
            "writable artifact publication has not materialized its temporary path",
        ));
    }
    let temp_name = publication_temp_name(&record.publication);
    if artifact_delete_guard_exists(write, ArtifactPathKind::TempReady, &temp_name)? {
        return Err(PersistenceError::Storage {
            class: StorageFailureClass::OwnerBusy,
            message: "temporary artifact path is being durably finalized".to_owned(),
        });
    }
    Ok(())
}

fn content_intent_state(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<bool, PersistenceError> {
    artifact_path_exists(
        write,
        &artifact_path_entry(record, ArtifactPathKind::ContentIntent)?,
    )
}

fn require_content_intent(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    if !content_intent_state(write, record)? {
        return Err(error::corruption(
            "artifact publication is missing final-content-path intent",
        ));
    }
    Ok(())
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
    request: &OrphanCleanupRequest,
    after: Option<&[u8]>,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
    last_cursor: &mut Option<OrphanCleanupCursor>,
) -> Result<bool, PersistenceError> {
    let write = store.database().begin_write().map_err(error::redb)?;
    let mut expired = Vec::new();
    let mut has_more = false;
    {
        let by_age = write
            .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
            .map_err(error::redb)?;
        let lower = after.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
        for item in by_age
            .range::<&[u8]>((lower, std::ops::Bound::Unbounded))
            .map_err(error::redb)?
        {
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
            if *examined >= request.limit.get() {
                has_more = true;
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
            *last_cursor = Some(OrphanCleanupCursor::new(
                OrphanCleanupFamily::WritablePublications,
                age_key.to_vec(),
                request.created_before,
            )?);
            expired.push(record);
        }
    }
    if expired.is_empty() {
        drop(write);
        return Ok(has_more);
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
        let removed = finalize_released_publication_paths(store, &record, None)?;
        if let Some(size) = removed.temporary {
            result.temporary_publications_removed =
                result.temporary_publications_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
        if let Some(size) = removed.content {
            result.unreferenced_blobs_removed =
                result.unreferenced_blobs_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(has_more)
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
    drop(owners);
    validate_temporary_manifest(write, &temp_name, &record.publication)?;
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

fn validate_temporary_manifest(
    write: &redb::WriteTransaction,
    temp_name: &str,
    publication: &ArtifactPublicationId,
) -> Result<(), PersistenceError> {
    let manifest = write
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    let stored = manifest
        .get(temp_name)
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("writable publication has no temporary-file manifest"))?;
    let stored: ArtifactPublicationId =
        json::decode(stored.value(), "artifact temporary manifest")?;
    if &stored != publication {
        return Err(error::corruption(
            "temporary-file manifest disagrees with its publication",
        ));
    }
    Ok(())
}

fn remove_temporary_manifest(
    write: &redb::WriteTransaction,
    temp_name: &str,
    publication: &ArtifactPublicationId,
) -> Result<(), PersistenceError> {
    validate_temporary_manifest(write, temp_name, publication)?;
    let mut manifest = write
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    let _removed = manifest.remove(temp_name).map_err(error::redb)?;
    Ok(())
}

fn release_writable_publication(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    validate_artifact_catalog(write)?;
    validate_writable_publication_indexes(write, record)?;
    let previous_bytes = json::encode(record, "artifact publication")?;
    let previous_payload = trie::digest_payload(CatalogFamily::ArtifactPublication, &previous_bytes);
    {
        let publications = write
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
    }
    let mut released = record.clone();
    released.state = PublicationState::Released;
    let released_bytes = json::encode(&released, "artifact publication")?;
    write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?
        .insert(released.publication.as_str(), released_bytes.as_slice())
        .map_err(error::redb)?;
    persist_publication_catalog(write, &released, Some(previous_payload))?;
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
        let removed = owners.remove(temp_name.as_str()).map_err(error::redb)?;
        drop(removed);
        drop(owners);
        remove_temporary_manifest(write, &temp_name, &record.publication)?;
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
    validate_artifact_catalog(write)?;
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
    request: &OrphanCleanupRequest,
    after: Option<&[u8]>,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
    last_cursor: &mut Option<OrphanCleanupCursor>,
) -> Result<bool, PersistenceError> {
    let remaining = request.limit.get().saturating_sub(*examined) as usize;
    if remaining == 0 {
        return Ok(true);
    }
    let after = decode_cleanup_path_cursor(after)?;
    let read = store.database().begin_read().map_err(error::redb)?;
    trie::validate_roots(&read)?;
    let page = trie::page(
        &read,
        CatalogFamily::ArtifactPath,
        None,
        after,
        remaining.saturating_add(1),
    )?;
    let mut entries = Vec::new();
    let mut has_more = false;
    for leaf in page.leaves {
        let entry = decode_artifact_path_entry(&leaf)?;
        if entry.kind == ArtifactPathKind::ContentIntent {
            break;
        }
        if entries.len() == remaining {
            has_more = true;
            break;
        }
        entries.push(entry);
    }
    drop(read);
    for entry in entries {
        *examined += 1;
        *last_cursor = Some(OrphanCleanupCursor::new(
            OrphanCleanupFamily::TemporaryFiles,
            entry.path.to_vec(),
            request.created_before,
        )?);
        if let Some(size) = cleanup_temporary_inventory_entry(store, &entry, request)? {
            result.temporary_publications_removed =
                result.temporary_publications_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(has_more)
}

fn temporary_manifest_publication(
    write: &redb::WriteTransaction,
    name: &str,
) -> Result<Option<ArtifactPublicationId>, PersistenceError> {
    let manifest = write
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    manifest
        .get(name)
        .map_err(error::redb)?
        .map(|bytes| json::decode(bytes.value(), "artifact temporary manifest"))
        .transpose()
}

fn cleanup_content_files(
    store: &RedbStore,
    request: &OrphanCleanupRequest,
    after: Option<&[u8]>,
    result: &mut OrphanCleanupResult,
    examined: &mut u32,
    last_cursor: &mut Option<OrphanCleanupCursor>,
) -> Result<bool, PersistenceError> {
    let remaining = request.limit.get().saturating_sub(*examined) as usize;
    if remaining == 0 {
        return Ok(true);
    }
    let after = match decode_cleanup_path_cursor(after)? {
        Some(after) => Some(after),
        None => {
            let mut before_content = [u8::MAX; 32];
            before_content[0] = 0;
            Some(before_content)
        }
    };
    let read = store.database().begin_read().map_err(error::redb)?;
    trie::validate_roots(&read)?;
    let page = trie::page(
        &read,
        CatalogFamily::ArtifactPath,
        None,
        after,
        remaining.saturating_add(1),
    )?;
    let mut entries = Vec::new();
    let mut has_more = false;
    for leaf in page.leaves {
        let entry = decode_artifact_path_entry(&leaf)?;
        if entry.kind != ArtifactPathKind::ContentIntent {
            return Err(error::corruption(
                "artifact content cleanup encountered an out-of-phase path entry",
            ));
        }
        if entries.len() == remaining {
            has_more = true;
            break;
        }
        entries.push(entry);
    }
    drop(read);
    for entry in entries {
        *examined += 1;
        *last_cursor = Some(OrphanCleanupCursor::new(
            OrphanCleanupFamily::ContentFiles,
            entry.path.to_vec(),
            request.created_before,
        )?);
        if let Some(size) = cleanup_content_inventory_entry(store, &entry, request)? {
            result.unreferenced_blobs_removed = result.unreferenced_blobs_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(has_more)
}

fn decode_cleanup_path_cursor(
    after: Option<&[u8]>,
) -> Result<Option<[u8; 32]>, PersistenceError> {
    after
        .map(|bytes| {
            bytes.try_into().map_err(|_| {
                PersistenceError::InvalidCursor(
                    "artifact cleanup cursor does not contain an authenticated path".to_owned(),
                )
            })
        })
        .transpose()
}

fn cleanup_temporary_inventory_entry(
    store: &RedbStore,
    entry: &ArtifactPathEntry,
    request: &OrphanCleanupRequest,
) -> Result<Option<u64>, PersistenceError> {
    if !matches!(
        entry.kind,
        ArtifactPathKind::TempPending | ArtifactPathKind::TempReady
    ) {
        return Err(error::corruption(
            "temporary cleanup received a non-temporary path entry",
        ));
    }
    let write = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&write)?;
    if !artifact_path_exists(&write, entry)? {
        return Err(error::corruption(
            "artifact cleanup path disappeared after authenticated enumeration",
        ));
    }
    let record = optional_publication_in_transaction(&write, &entry.publication)?;
    let owner = write
        .open_table(ARTIFACT_TEMP_OWNERS)
        .map_err(error::redb)?
        .get(entry.identity.as_str())
        .map_err(error::redb)?
        .map(|value| value.value().to_owned());
    let manifested = temporary_manifest_publication(&write, &entry.identity)?;
    match record {
        Some(record) if matches!(record.state, PublicationState::Writable) => {
            if record.created_at_millis != entry.created_at_millis
                || record.publication != entry.publication
                || publication_temp_name(&record.publication) != entry.identity
            {
                return Err(error::corruption(
                    "temporary path inventory disagrees with its writable publication",
                ));
            }
            validate_writable_publication_indexes(&write, &record)?;
            let expected_state = match entry.kind {
                ArtifactPathKind::TempPending => TempInventoryState::Pending,
                ArtifactPathKind::TempReady => TempInventoryState::Ready,
                ArtifactPathKind::ContentIntent => {
                    return Err(error::corruption("invalid temporary inventory kind"));
                }
            };
            if temp_inventory_state(&write, &record)? != expected_state {
                return Err(error::corruption(
                    "temporary path inventory contains a stale state",
                ));
            }
            return Ok(None);
        }
        Some(record) if matches!(record.state, PublicationState::Committed { .. }) => {
            if owner.is_some() || manifested.is_some() {
                return Err(error::corruption(
                    "committed publication retains writable temporary indexes",
                ));
            }
        }
        Some(record) if matches!(record.state, PublicationState::Released) => {
            if owner.is_some() || manifested.is_some() {
                return Err(error::corruption(
                    "released publication retains writable temporary indexes",
                ));
            }
        }
        Some(_) => return Err(error::corruption("temporary path inventory state is invalid")),
        None => {
            if owner.is_some() || manifested.is_some() {
                return Err(error::corruption(
                    "orphan temporary path retains a publication owner",
                ));
            }
        }
    }
    if entry.created_at_millis >= request.created_before.get() {
        return Ok(None);
    }
    let guarded = artifact_delete_guard_exists(&write, entry.kind, &entry.identity)?;
    if !guarded {
        put_artifact_delete_guard(&write, entry.kind, &entry.identity)?;
        store
            .faults
            .check(FaultPoint::BeforeArtifactCleanupCommit)?;
        write.commit().map_err(error::redb)?;
        store.faults.check(FaultPoint::AfterArtifactCleanupCommit)?;
    } else {
        drop(write);
    }
    let path = store.temp_root.join(&entry.identity);
    let removed = remove_cleanup_file_if_present(store, &path, &store.temp_root)?;
    let finalize = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&finalize)?;
    if !artifact_delete_guard_exists(&finalize, entry.kind, &entry.identity)?
        || !artifact_path_exists(&finalize, entry)?
    {
        return Err(error::corruption(
            "temporary cleanup guard or inventory disappeared before finalization",
        ));
    }
    if optional_publication_in_transaction(&finalize, &entry.publication)?
        .is_some_and(|record| matches!(record.state, PublicationState::Writable))
    {
        return Err(error::corruption(
            "temporary cleanup target became writable while delete-guarded",
        ));
    }
    remove_artifact_path(&finalize, entry)?;
    remove_artifact_delete_guard(&finalize, entry.kind, &entry.identity)?;
    remove_released_publication_if_uninventoried(&finalize, &entry.publication)?;
    store
        .faults
        .check(FaultPoint::BeforeArtifactPathFinalizeCommit)?;
    finalize.commit().map_err(error::redb)?;
    store
        .faults
        .check(FaultPoint::AfterArtifactPathFinalizeCommit)?;
    Ok(removed)
}

fn cleanup_content_inventory_entry(
    store: &RedbStore,
    entry: &ArtifactPathEntry,
    request: &OrphanCleanupRequest,
) -> Result<Option<u64>, PersistenceError> {
    if entry.kind != ArtifactPathKind::ContentIntent {
        return Err(error::corruption(
            "content cleanup received a non-content path entry",
        ));
    }
    let digest = ContentDigest::from_hex(&entry.identity).map_err(|cause| {
        error::corruption(format!("artifact content inventory has invalid digest: {cause}"))
    })?;
    let write = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&write)?;
    if !artifact_path_exists(&write, entry)? {
        return Err(error::corruption(
            "artifact content path disappeared after authenticated enumeration",
        ));
    }
    match optional_publication_in_transaction(&write, &entry.publication)? {
        Some(record) if matches!(record.state, PublicationState::Writable) => {
            if record.created_at_millis != entry.created_at_millis
                || record.metadata.reference().digest() != digest
            {
                return Err(error::corruption(
                    "content-path intent disagrees with its writable publication",
                ));
            }
            validate_writable_publication_indexes(&write, &record)?;
            require_content_intent(&write, &record)?;
            return Ok(None);
        }
        Some(record) if matches!(record.state, PublicationState::Committed { .. }) => {
            return Err(error::corruption(
                "committed publication retains final-content-path intent",
            ));
        }
        Some(record) if matches!(record.state, PublicationState::Released) => {}
        Some(_) => return Err(error::corruption("content path publication state is invalid")),
        None => {}
    }
    if entry.created_at_millis >= request.created_before.get() {
        return Ok(None);
    }
    let guarded = artifact_delete_guard_exists(&write, entry.kind, &entry.identity)?;
    if !guarded {
        put_artifact_delete_guard(&write, entry.kind, &entry.identity)?;
        store
            .faults
            .check(FaultPoint::BeforeArtifactCleanupCommit)?;
        write.commit().map_err(error::redb)?;
        store.faults.check(FaultPoint::AfterArtifactCleanupCommit)?;
    } else {
        drop(write);
    }
    let path = store.content_path(digest);
    let parent = path
        .parent()
        .ok_or_else(|| error::corruption("artifact content inventory path has no parent"))?;
    let decision = store.database().begin_read().map_err(error::redb)?;
    let references = decision
        .open_table(ARTIFACT_REFERENCES)
        .map_err(error::redb)?;
    let metadata = decision
        .open_table(ARTIFACTS_BY_DIGEST)
        .map_err(error::redb)?;
    let reservations = decision
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;
    let prefix = codec::component(&entry.identity)?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("artifact digest cleanup prefix has no end"))?;
    let owned = metadata
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .is_some()
        || references
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .is_some()
        || reservations
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .is_some();
    drop(reservations);
    drop(metadata);
    drop(references);
    drop(decision);
    let removed = if owned {
        None
    } else {
        remove_cleanup_file_if_present(store, &path, parent)?
    };
    let finalize = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&finalize)?;
    if !artifact_delete_guard_exists(&finalize, entry.kind, &entry.identity)?
        || !artifact_path_exists(&finalize, entry)?
    {
        return Err(error::corruption(
            "content cleanup guard or inventory disappeared before finalization",
        ));
    }
    if optional_publication_in_transaction(&finalize, &entry.publication)?
        .is_some_and(|record| matches!(record.state, PublicationState::Writable))
    {
        return Err(error::corruption(
            "content cleanup target became writable while delete-guarded",
        ));
    }
    remove_artifact_path(&finalize, entry)?;
    remove_artifact_delete_guard(&finalize, entry.kind, &entry.identity)?;
    remove_released_publication_if_uninventoried(&finalize, &entry.publication)?;
    store
        .faults
        .check(FaultPoint::BeforeArtifactPathFinalizeCommit)?;
    finalize.commit().map_err(error::redb)?;
    store
        .faults
        .check(FaultPoint::AfterArtifactPathFinalizeCommit)?;
    Ok(removed)
}

fn remove_released_publication_if_uninventoried(
    write: &redb::WriteTransaction,
    publication: &ArtifactPublicationId,
) -> Result<(), PersistenceError> {
    let Some(record) = optional_publication_in_transaction(write, publication)? else {
        return Ok(());
    };
    if !matches!(record.state, PublicationState::Released) {
        return Ok(());
    }
    for kind in [
        ArtifactPathKind::TempPending,
        ArtifactPathKind::TempReady,
        ArtifactPathKind::ContentIntent,
    ] {
        if artifact_path_exists(write, &artifact_path_entry(&record, kind)?)? {
            return Ok(());
        }
    }
    let mut publications = write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let removed = publications
        .remove(publication.as_str())
        .map_err(error::redb)?;
    if removed.is_none() {
        return Err(error::corruption(
            "released publication disappeared while finalizing its inventory",
        ));
    }
    drop(removed);
    drop(publications);
    remove_publication_catalog(write, &record)
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
    remove_file_if_present(store, path, parent, None)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FinalizedPathBytes {
    temporary: Option<u64>,
    content: Option<u64>,
}

fn finalize_released_publication_paths(
    store: &RedbStore,
    record: &PublicationRecord,
    fault_boundary: Option<(FaultPoint, FaultPoint)>,
) -> Result<FinalizedPathBytes, PersistenceError> {
    let prepare = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&prepare)?;
    let current = optional_publication_in_transaction(&prepare, &record.publication)?;
    match current.as_ref().map(|stored| &stored.state) {
        Some(PublicationState::Committed { .. }) if matches!(record.state, PublicationState::Committed { .. }) => {}
        Some(PublicationState::Released) | None
            if matches!(record.state, PublicationState::Writable | PublicationState::Released) => {}
        Some(_) => {
            return Err(error::corruption(
                "artifact path finalization disagrees with publication state",
            ));
        }
        None if matches!(record.state, PublicationState::Committed { .. }) => {
            return Err(error::corruption(
                "committed publication disappeared before path finalization",
            ));
        }
        None => {}
    }
    let pending = artifact_path_entry(record, ArtifactPathKind::TempPending)?;
    let ready = artifact_path_entry(record, ArtifactPathKind::TempReady)?;
    let pending_exists = artifact_path_exists(&prepare, &pending)?;
    let ready_exists = artifact_path_exists(&prepare, &ready)?;
    if pending_exists && ready_exists {
        return Err(error::corruption(
            "artifact path finalization found conflicting temp states",
        ));
    }
    let temp_entry = if pending_exists {
        Some(pending)
    } else if ready_exists {
        Some(ready)
    } else {
        None
    };
    let content = artifact_path_entry(record, ArtifactPathKind::ContentIntent)?;
    let content_exists = artifact_path_exists(&prepare, &content)?;
    let content_owned = content_exists
        && digest_has_metadata_or_references(
            &prepare,
            &record.metadata.reference().digest().to_hex(),
        )?;
    let mut guard_changed = false;
    if let Some(entry) = temp_entry.as_ref() {
        if !artifact_delete_guard_exists(&prepare, entry.kind, &entry.identity)? {
            put_artifact_delete_guard(&prepare, entry.kind, &entry.identity)?;
            guard_changed = true;
        }
    }
    if content_exists
        && !artifact_delete_guard_exists(&prepare, content.kind, &content.identity)?
    {
        put_artifact_delete_guard(&prepare, content.kind, &content.identity)?;
        guard_changed = true;
    }
    if guard_changed {
        store
            .faults
            .check(FaultPoint::BeforeArtifactPathDeleteIntentCommit)?;
        prepare.commit().map_err(error::redb)?;
        store
            .faults
            .check(FaultPoint::AfterArtifactPathDeleteIntentCommit)?;
    } else {
        drop(prepare);
    }

    let mut removed = FinalizedPathBytes::default();
    if let Some(entry) = temp_entry.as_ref() {
        removed.temporary = remove_file_if_present(
            store,
            &store.temp_root.join(&entry.identity),
            &store.temp_root,
            fault_boundary,
        )?;
    }
    if content_exists && !content_owned {
        let content_path = store.content_path(record.metadata.reference().digest());
        let parent = content_path.parent().ok_or_else(|| {
            error::corruption("artifact content path has no parent during finalization")
        })?;
        removed.content =
            remove_file_if_present(store, &content_path, parent, fault_boundary)?;
    }

    let finalize = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_catalog(&finalize)?;
    let reloaded = optional_publication_in_transaction(&finalize, &record.publication)?;
    if reloaded != current {
        return Err(error::corruption(
            "publication changed while its delete guard was held",
        ));
    }
    if let Some(entry) = temp_entry.as_ref() {
        if !artifact_path_exists(&finalize, entry)?
            || !artifact_delete_guard_exists(&finalize, entry.kind, &entry.identity)?
        {
            return Err(error::corruption(
                "temporary inventory or delete guard disappeared before finalization",
            ));
        }
        remove_artifact_path(&finalize, entry)?;
        remove_artifact_delete_guard(&finalize, entry.kind, &entry.identity)?;
    }
    if content_exists {
        if !artifact_path_exists(&finalize, &content)?
            || !artifact_delete_guard_exists(&finalize, content.kind, &content.identity)?
        {
            return Err(error::corruption(
                "content inventory or delete guard disappeared before finalization",
            ));
        }
        remove_artifact_path(&finalize, &content)?;
        remove_artifact_delete_guard(&finalize, content.kind, &content.identity)?;
    }
    remove_released_publication_if_uninventoried(&finalize, &record.publication)?;
    if temp_entry.is_none()
        && !content_exists
        && current
            .as_ref()
            .is_none_or(|record| matches!(record.state, PublicationState::Committed { .. }))
    {
        return Ok(removed);
    }
    store
        .faults
        .check(FaultPoint::BeforeArtifactPathFinalizeCommit)?;
    finalize.commit().map_err(error::redb)?;
    store
        .faults
        .check(FaultPoint::AfterArtifactPathFinalizeCommit)?;
    Ok(removed)
}

fn remove_file_if_present(
    store: &RedbStore,
    path: &Path,
    parent: &Path,
    fault_boundary: Option<(FaultPoint, FaultPoint)>,
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
    let (before, after) = fault_boundary.unwrap_or((
        FaultPoint::BeforeArtifactCleanupDelete,
        FaultPoint::AfterArtifactCleanupDelete,
    ));
    store.faults.check(before)?;
    fs::remove_file(path).map_err(error::io)?;
    sync_directory(parent)?;
    store.faults.check(after)?;
    Ok(Some(metadata.len()))
}
