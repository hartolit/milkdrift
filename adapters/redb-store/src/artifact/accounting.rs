use super::*;
use super::{
    cleanup::{remove_publication_age_index, remove_temporary_manifest},
    path::{ArtifactPathKind, artifact_path_entry, publication_temp_name, remove_artifact_path},
    publication::{
        validated_artifact_digest_in_transaction, validated_artifact_metadata_in_transaction,
    },
};

pub(crate) fn validate_artifact_state(
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
    Ok(stored)
}

pub(crate) fn validated_run_artifact_reference_in_transaction(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    validate_artifact_state(write)?;
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

pub(crate) fn indexed_run_artifact_reference(
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
    let Some((key, bytes)) = item else {
        return Ok(false);
    };
    let components = codec::decode_components(key.value(), 4)
        .or_else(|_| codec::decode_components(key.value(), 5))?;
    if components[0] != digest
        || components[1] != reference.artifact().as_str()
        || components[2] != run.as_str()
    {
        return Err(error::corruption(
            "artifact-reference index key contradicts its lookup prefix",
        ));
    }
    let stored: ArtifactReference = json::decode(bytes.value(), "artifact reference")?;
    if stored != *reference {
        return Err(error::corruption(
            "artifact-reference index prefix contradicts its stored document",
        ));
    }
    Ok(true)
}

pub(crate) fn manifested_run_artifact_reference(
    write: &redb::WriteTransaction,
    run: &RunId,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let digest = reference.digest().to_hex();
    let key = codec::components(&[run.as_str(), &digest, reference.artifact().as_str()])?;
    let ownership = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    let Some(bytes) = ownership.get(key.as_slice()).map_err(error::redb)? else {
        return Ok(false);
    };
    let stored: ArtifactReference = json::decode(bytes.value(), "run artifact ownership")?;
    if stored != *reference {
        return Err(error::corruption(
            "run artifact-ownership key contradicts its stored document",
        ));
    }
    Ok(true)
}

pub(crate) fn persist_artifact_reference_occurrence(
    write: &redb::WriteTransaction,
    key: &[u8],
    reference: &ArtifactReference,
) -> Result<(), PersistenceError> {
    let bytes = json::encode(reference, "artifact reference")?;
    if write
        .open_table(ARTIFACT_REFERENCES)
        .map_err(error::redb)?
        .insert(key, bytes.as_slice())
        .map_err(error::redb)?
        .is_some()
    {
        return Err(error::corruption(
            "artifact reference occurrence already exists before its authoritative append",
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
    let mut table = write
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    if let Some(previous) = table.get(key.as_slice()).map_err(error::redb)? {
        let previous: ArtifactReference = json::decode(previous.value(), "run artifact ownership")?;
        if previous != *reference {
            return Err(error::corruption(
                "existing run artifact ownership disagrees with its identity",
            ));
        }
        return Ok(());
    }
    table
        .insert(key.as_slice(), bytes.as_slice())
        .map_err(error::redb)?;
    Ok(())
}

pub(crate) const fn usage_covers(current: WorkspaceUsage, historical: WorkspaceUsage) -> bool {
    current.value_versions() >= historical.value_versions()
        && current.inline_bytes() >= historical.inline_bytes()
        && current.artifacts() >= historical.artifacts()
        && current.artifact_bytes() >= historical.artifact_bytes()
}

pub(crate) fn commit_artifact_metadata(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    record: &mut PublicationRecord,
    content_deduplicated: bool,
) -> Result<(), PersistenceError> {
    let mut artifact_accounting = validate_artifact_state(write)?;
    let previous_artifact =
        validated_artifact_metadata_in_transaction(write, record.metadata.reference().artifact())?;
    if previous_artifact
        .as_ref()
        .is_some_and(|metadata| metadata != &record.metadata)
    {
        return Err(PersistenceError::ImmutableConflict {
            entity: "artifact",
            identity: record.metadata.reference().artifact().to_string(),
        });
    }

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
    let digest_was_known = validated_artifact_digest_in_transaction(
        write,
        record.metadata.reference().digest(),
        record.metadata.reference().size_bytes(),
    )?;
    {
        let mut by_digest = write.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
        if let Some(previous) = by_digest
            .insert(digest_key.as_slice(), metadata_bytes.as_slice())
            .map_err(error::redb)?
            && previous.value() != metadata_bytes.as_slice()
        {
            return Err(error::corruption(
                "artifact digest index replaced different metadata",
            ));
        }
    }
    {
        let manifest_bytes = json::encode(&record.metadata, "artifact manifest")?;
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
                    manifest_bytes.as_slice(),
                )
                .map_err(error::redb)?;
        }
    }
    let resulting_content_bytes = if digest_was_known {
        artifact_accounting.committed_content_bytes
    } else {
        artifact_accounting
            .committed_content_bytes
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
    artifact_accounting.committed_content_bytes = resulting_content_bytes;
    let accounting_bytes = json::encode(&artifact_accounting, "artifact accounting")?;
    write
        .open_table(ARTIFACT_ACCOUNTING)
        .map_err(error::redb)?
        .insert(GLOBAL_ARTIFACT_BYTES_KEY, accounting_bytes.as_slice())
        .map_err(error::redb)?;

    let usage_bytes = json::encode(&record.resulting_usage, "workspace usage")?;
    write
        .open_table(WORKSPACE_USAGE)
        .map_err(error::redb)?
        .insert(record.run.as_str(), usage_bytes.as_slice())
        .map_err(error::redb)?;
    crate::journal::persist_workspace_value_usage_accounting_in_transaction(
        write,
        &record.run,
        record.resulting_usage,
    )?;

    let reference_key = codec::components(&[
        &digest,
        record.metadata.reference().artifact().as_str(),
        record.run.as_str(),
        "publication",
        record.publication.as_str(),
    ])?;
    persist_artifact_reference_occurrence(write, &reference_key, record.metadata.reference())?;
    persist_run_artifact_ownership(write, &record.run, record.metadata.reference())?;
    remove_publication_age_index(write, record)?;

    record.state = PublicationState::Committed {
        content_deduplicated,
    };
    let record_bytes = json::encode(record, "artifact publication")?;
    write
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?
        .insert(record.publication.as_str(), record_bytes.as_slice())
        .map_err(error::redb)?;

    write
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?
        .remove(record.run.as_str())
        .map_err(error::redb)?;
    let temp_name = publication_temp_name(&record.publication);
    write
        .open_table(ARTIFACT_TEMP_OWNERS)
        .map_err(error::redb)?
        .remove(temp_name.as_str())
        .map_err(error::redb)?;
    remove_temporary_manifest(write, &temp_name, &record.publication)?;
    let reservation_key = codec::pair(&digest, record.publication.as_str())?;
    write
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?
        .remove(reservation_key.as_slice())
        .map_err(error::redb)?;
    remove_artifact_path(
        write,
        &artifact_path_entry(record, ArtifactPathKind::ContentIntent)?,
    )?;
    validate_artifact_state(write)?;
    Ok(())
}
