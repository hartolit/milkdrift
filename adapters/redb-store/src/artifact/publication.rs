use super::*;
use super::{
    accounting::{
        commit_artifact_metadata, usage_covers, validate_artifact_catalog,
        validated_run_artifact_reference_in_transaction,
    },
    cleanup::{
        cleanup_content_files, cleanup_temporary_files, expire_writable_publications,
        finalize_released_publication_paths, release_writable_publication,
        validate_writable_publication_indexes,
    },
    path::{
        ArtifactPathKind, artifact_delete_guard_exists, artifact_path_entry, content_intent_state,
        ensure_temp_inventory_ready, open_regular_for_append, open_regular_for_read,
        publication_age_key, publication_length_or_published, publication_temp_name,
        put_artifact_path, require_content_intent, require_temp_inventory_ready, sync_directory,
        verify_blob,
    },
};
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

        let existing_publication =
            optional_publication_in_transaction(&write, &request.publication)?;
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
                && owner.value() != request.publication.as_str()
            {
                return Err(PersistenceError::Storage {
                    class: StorageFailureClass::OwnerBusy,
                    message: format!(
                        "run {} already has an active artifact publication",
                        request.run
                    ),
                });
            }
        }

        let created_at_millis = self.artifact_clock.now()?.get();
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
        if artifact_delete_guard_exists(&write, ArtifactPathKind::ContentIntent, &content_identity)?
        {
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
            .ok_or_else(|| crate::error::internal("artifact path has no parent"))?;
        let parent_existed = final_parent.exists();
        crate::store::prepare_owned_directory(final_parent, "artifact digest shard")?;
        if !parent_existed {
            sync_directory(&self.artifact_root)?;
        }

        let digest_was_cataloged = {
            let read = self.database().begin_write().map_err(error::redb)?;
            let cataloged = validated_artifact_digest_in_transaction(
                &read,
                record.metadata.reference().digest(),
                record.metadata.reference().size_bytes(),
            )?;
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
        if (matches!(
            record.state,
            PublicationState::Writable | PublicationState::Released
        ) && actual != expected_usage)
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

pub(crate) fn validate_publication_request(
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

pub(crate) fn decode_publication(bytes: &[u8]) -> Result<PublicationRecord, PersistenceError> {
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

pub(crate) fn publication_in_transaction(
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

pub(crate) fn optional_publication_in_transaction(
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

pub(crate) fn metadata_in_transaction(
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

pub(crate) fn artifact_catalog_payload(
    metadata: &ArtifactMetadata,
) -> Result<[u8; 32], PersistenceError> {
    let bytes = json::encode(metadata, "artifact metadata")?;
    Ok(trie::digest_payload(CatalogFamily::Artifact, &bytes))
}

fn artifact_digest_catalog_logical_key(digest: ContentDigest) -> Vec<u8> {
    let mut key = b"\0content-digest\0".to_vec();
    key.extend_from_slice(digest.to_hex().as_bytes());
    key
}

pub(crate) fn validate_artifact_catalog_leaf(
    read: &redb::ReadTransaction,
    leaf: &trie::TrieLeaf,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::Artifact;
    if leaf.path != trie::hashed_path(family, &leaf.logical_key) {
        return Err(error::corruption(
            "artifact catalog path disagrees with its logical identity",
        ));
    }
    if let Some(digest_hex) = leaf.logical_key.strip_prefix(b"\0content-digest\0") {
        let digest_hex = std::str::from_utf8(digest_hex)
            .map_err(|_| error::corruption("artifact content catalog digest is not UTF-8"))?;
        let digest = ContentDigest::from_hex(digest_hex).map_err(|cause| {
            error::corruption(format!(
                "artifact content catalog digest is invalid: {cause}"
            ))
        })?;
        let prefix = codec::component(digest_hex)?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("artifact digest prefix has no range end"))?;
        let by_digest = read.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
        let indexed = by_digest
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .next()
            .transpose()
            .map_err(error::redb)?
            .ok_or_else(|| {
                error::corruption("authenticated artifact content has no digest-index row")
            })?;
        let metadata: ArtifactMetadata = json::decode(indexed.1.value(), "artifact metadata")?;
        if metadata.reference().digest() != digest
            || leaf.payload_digest
                != artifact_digest_catalog_payload(digest, metadata.reference().size_bytes())?
        {
            return Err(error::corruption(
                "authenticated artifact content disagrees with its digest index",
            ));
        }
        return Ok(());
    }

    let artifact_text = std::str::from_utf8(&leaf.logical_key)
        .map_err(|_| error::corruption("artifact catalog identity is not UTF-8"))?;
    let artifact = ArtifactId::new(artifact_text).map_err(|cause| {
        error::corruption(format!("artifact catalog identity is invalid: {cause}"))
    })?;
    let metadata = read
        .open_table(ARTIFACT_METADATA)
        .map_err(error::redb)?
        .get(artifact.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode::<ArtifactMetadata>(bytes.value(), "artifact metadata"))
        .transpose()?
        .ok_or_else(|| error::corruption("authenticated artifact has no metadata row"))?;
    let manifest = read
        .open_table(ARTIFACT_MANIFEST)
        .map_err(error::redb)?
        .get(artifact.as_str())
        .map_err(error::redb)?
        .map(|bytes| json::decode::<ArtifactMetadata>(bytes.value(), "artifact manifest"))
        .transpose()?
        .ok_or_else(|| error::corruption("authenticated artifact has no manifest row"))?;
    let digest_key = codec::pair(&metadata.reference().digest().to_hex(), artifact.as_str())?;
    let indexed = read
        .open_table(ARTIFACTS_BY_DIGEST)
        .map_err(error::redb)?
        .get(digest_key.as_slice())
        .map_err(error::redb)?
        .map(|bytes| json::decode::<ArtifactMetadata>(bytes.value(), "artifact metadata"))
        .transpose()?
        .ok_or_else(|| error::corruption("authenticated artifact has no digest-index row"))?;
    if metadata != manifest
        || metadata != indexed
        || metadata.reference().artifact() != &artifact
        || leaf.payload_digest != artifact_catalog_payload(&metadata)?
    {
        return Err(error::corruption(
            "authenticated artifact catalog rows disagree",
        ));
    }
    Ok(())
}

fn artifact_digest_catalog_payload(
    digest: ContentDigest,
    size_bytes: u64,
) -> Result<[u8; 32], PersistenceError> {
    let size = size_bytes.to_string();
    let bytes = codec::components(&[&digest.to_hex(), &size])?;
    Ok(trie::digest_payload(CatalogFamily::Artifact, &bytes))
}

pub(crate) fn validated_artifact_digest_in_transaction(
    write: &redb::WriteTransaction,
    digest: ContentDigest,
    size_bytes: u64,
) -> Result<bool, PersistenceError> {
    let family = CatalogFamily::Artifact;
    let logical_key = artifact_digest_catalog_logical_key(digest);
    let witness = trie::verify_member_in_transaction(
        write,
        family,
        trie::hashed_path(family, &logical_key),
        &logical_key,
    )?;
    let digest_hex = digest.to_hex();
    let prefix = codec::component(&digest_hex)?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("artifact digest prefix has no range end"))?;
    let by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let indexed = by_digest
        .range(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
        .next()
        .transpose()
        .map_err(error::redb)?
        .map(|(_, bytes)| json::decode::<ArtifactMetadata>(bytes.value(), "artifact metadata"))
        .transpose()?;
    let expected = artifact_digest_catalog_payload(digest, size_bytes)?;
    match (indexed, witness) {
        (None, None) => Ok(false),
        (Some(metadata), Some(authenticated))
            if metadata.reference().digest() == digest
                && metadata.reference().size_bytes() == size_bytes
                && authenticated == expected =>
        {
            Ok(true)
        }
        (Some(_), Some(_)) => Err(error::corruption(
            "artifact digest index disagrees with its authenticated content membership",
        )),
        (None, Some(_)) => Err(error::corruption(
            "authenticated artifact content is missing from its digest index",
        )),
        (Some(_), None) => Err(error::corruption(
            "artifact digest index is absent from its authenticated content catalog",
        )),
    }
}

pub(crate) fn persist_artifact_digest_catalog(
    write: &redb::WriteTransaction,
    digest: ContentDigest,
    size_bytes: u64,
    previously_known: bool,
) -> Result<(), PersistenceError> {
    let family = CatalogFamily::Artifact;
    let logical_key = artifact_digest_catalog_logical_key(digest);
    let expected = artifact_digest_catalog_payload(digest, size_bytes)?;
    let replaced = trie::put(
        write,
        family,
        trie::hashed_path(family, &logical_key),
        &logical_key,
        expected,
    )?;
    match (previously_known, replaced) {
        (false, None) => Ok(()),
        (true, Some(previous)) if previous == expected => Ok(()),
        _ => Err(error::corruption(
            "artifact content membership changed outside its authoritative transaction",
        )),
    }
}

pub(crate) fn publication_catalog_path(publication: &ArtifactPublicationId) -> [u8; 32] {
    let family = CatalogFamily::ArtifactPublication;
    trie::hashed_path(family, publication.as_str().as_bytes())
}

pub(crate) fn persist_publication_catalog(
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

pub(crate) fn remove_publication_catalog(
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
