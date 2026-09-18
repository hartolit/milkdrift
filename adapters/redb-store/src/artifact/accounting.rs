use super::{
    ARTIFACT_ACCOUNTING, ARTIFACT_ACCOUNTING_SCHEMA_VERSION, ARTIFACT_DIGEST_RESERVATIONS,
    ARTIFACT_MANIFEST, ARTIFACT_METADATA, ARTIFACT_OWNERSHIP, ARTIFACT_PUBLICATIONS,
    ARTIFACT_REFERENCES, ARTIFACT_RESERVATIONS, ARTIFACT_TEMP_OWNERS, ARTIFACTS_BY_DIGEST,
    ArtifactAccountingRecord, ArtifactMetadata, ArtifactReference, BTreeSet, CausalReference,
    GLOBAL_ARTIFACT_BYTES_KEY, PersistenceError, PublicationRecord, PublicationState, ROOT_SCOPES,
    RUN_EVENTS, ReadableTable, ReadableTableMetadata, RedbStore, SCOPES, StorageFailureClass,
    VALUES, WORKSPACE_USAGE, WorkspaceUsage, WorkspaceValueEntry, codec, error, json, verify_blob,
};
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

pub(crate) fn validated_owner_artifact_reference_in_transaction(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let owner = owner.into();
    let owner_key = super::owner::domain_key(&owner)?;
    validate_artifact_state(write)?;
    let usage = crate::journal::workspace_domain_in_transaction(write, &owner)?
        .map_or(WorkspaceUsage::EMPTY, |(_budget, usage)| usage);
    let ownership = write.open_table(ARTIFACT_OWNERSHIP).map_err(error::redb)?;
    validate_artifact_ownership(&ownership, &owner, usage)?;
    drop(ownership);
    let indexed = indexed_owner_artifact_reference(write, &owner, reference)?;
    let authoritative = manifested_owner_artifact_reference(write, &owner, reference)?;
    if indexed != authoritative {
        return Err(error::corruption(format!(
            "artifact-reference index disagrees with authoritative ownership for owner {owner_key} and artifact {}",
            reference.artifact()
        )));
    }
    Ok(authoritative)
}

pub(crate) fn validate_artifact_ownership<T>(
    ownership: &T,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    usage: WorkspaceUsage,
) -> Result<(), PersistenceError>
where
    T: redb::ReadableTable<&'static [u8], &'static [u8]>,
{
    let owner = owner.into();
    let owner_key = super::owner::domain_key(&owner)?;
    let prefix = codec::components(&[owner_key.as_str()])?;
    let end = codec::prefix_end(prefix.clone())
        .ok_or_else(|| error::corruption("artifact-ownership prefix has no range end"))?;
    let mut artifacts = BTreeSet::new();
    let mut bytes = 0_u64;
    for item in ownership
        .range::<&[u8]>(prefix.as_slice()..end.as_slice())
        .map_err(error::redb)?
    {
        let (key, value) = item.map_err(error::redb)?;
        let components = codec::decode_components(key.value(), 3)?;
        let reference: ArtifactReference = json::decode(value.value(), "artifact ownership")?;
        if components[0] != owner_key.as_str()
            || components[1] != reference.digest().to_hex()
            || components[2] != reference.artifact().as_str()
            || !artifacts.insert(reference.artifact().clone())
        {
            return Err(error::corruption(
                "artifact-ownership key, document, or unique identity is inconsistent",
            ));
        }
        bytes = bytes
            .checked_add(reference.size_bytes())
            .ok_or_else(|| error::corruption("artifact-ownership bytes overflow"))?;
        let count = u64::try_from(artifacts.len())
            .map_err(|_| error::corruption("artifact-ownership count exceeds u64"))?;
        if count > usage.artifacts() || bytes > usage.artifact_bytes() {
            return Err(error::corruption(
                "artifact ownership exceeds authoritative workspace usage",
            ));
        }
    }
    let count = u64::try_from(artifacts.len())
        .map_err(|_| error::corruption("artifact-ownership count exceeds u64"))?;
    if count != usage.artifacts() || bytes != usage.artifact_bytes() {
        return Err(error::corruption(
            "artifact ownership does not equal authoritative workspace usage",
        ));
    }
    Ok(())
}

pub(crate) fn indexed_owner_artifact_reference(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let owner = owner.into();
    let owner_key = super::owner::domain_key(&owner)?;
    let digest = reference.digest().to_hex();
    let prefix = codec::components(&[&digest, reference.artifact().as_str(), owner_key.as_str()])?;
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
        || components[2] != owner_key.as_str()
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

pub(crate) fn manifested_owner_artifact_reference(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    reference: &ArtifactReference,
) -> Result<bool, PersistenceError> {
    let owner = owner.into();
    let owner_key = super::owner::domain_key(&owner)?;
    let digest = reference.digest().to_hex();
    let key = codec::components(&[owner_key.as_str(), &digest, reference.artifact().as_str()])?;
    let ownership = write.open_table(ARTIFACT_OWNERSHIP).map_err(error::redb)?;
    let Some(bytes) = ownership.get(key.as_slice()).map_err(error::redb)? else {
        return Ok(false);
    };
    let stored: ArtifactReference = json::decode(bytes.value(), "artifact ownership")?;
    if stored != *reference {
        return Err(error::corruption(
            "artifact-ownership key contradicts its stored document",
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

pub(crate) fn persist_artifact_ownership(
    write: &redb::WriteTransaction,
    owner: impl Into<milkdrift_workspace::ArtifactOwner>,
    reference: &ArtifactReference,
) -> Result<(), PersistenceError> {
    let owner = owner.into();
    let owner_key = super::owner::domain_key(&owner)?;
    let digest = reference.digest().to_hex();
    let key = codec::components(&[owner_key.as_str(), &digest, reference.artifact().as_str()])?;
    let bytes = json::encode(reference, "artifact ownership")?;
    let mut table = write.open_table(ARTIFACT_OWNERSHIP).map_err(error::redb)?;
    if let Some(previous) = table.get(key.as_slice()).map_err(error::redb)? {
        let previous: ArtifactReference = json::decode(previous.value(), "artifact ownership")?;
        if previous != *reference {
            return Err(error::corruption(
                "existing artifact ownership disagrees with its identity",
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
    validate_artifact_provenance(store, write, record)?;
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
        &record.owner,
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
        .insert(
            super::owner::domain_key(&record.owner)?.as_str(),
            usage_bytes.as_slice(),
        )
        .map_err(error::redb)?;
    crate::journal::persist_workspace_value_usage_accounting_in_transaction(
        write,
        &record.owner,
        record.resulting_usage,
    )?;

    let reference_key = codec::components(&[
        &digest,
        record.metadata.reference().artifact().as_str(),
        super::owner::domain_key(&record.owner)?.as_str(),
        "publication",
        record.publication.as_str(),
    ])?;
    persist_artifact_reference_occurrence(write, &reference_key, record.metadata.reference())?;
    persist_artifact_ownership(write, &record.owner, record.metadata.reference())?;
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
        .remove(super::owner::domain_key(&record.owner)?.as_str())
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

fn validate_artifact_provenance(
    store: &RedbStore,
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    for causal in std::iter::once(record.metadata.provenance().producer())
        .chain(record.metadata.provenance().causes())
    {
        match causal {
            CausalReference::External { .. } | CausalReference::PeerClaim { .. } => {}
            CausalReference::Artifact { reference } => {
                let Some(metadata) =
                    validated_artifact_metadata_in_transaction(write, reference.artifact())?
                else {
                    return Err(PersistenceError::NotFound {
                        entity: "artifact_provenance",
                        identity: reference.artifact().to_string(),
                    });
                };
                if metadata.reference() != reference {
                    return Err(PersistenceError::ImmutableConflict {
                        entity: "artifact_provenance",
                        identity: reference.artifact().to_string(),
                    });
                }
                verify_blob(
                    &store.content_path(reference.digest()),
                    reference,
                    store.max_artifact_bytes,
                )?;
            }
            CausalReference::WorkspaceValue { reference } => {
                let values = write.open_table(VALUES).map_err(error::redb)?;
                let scopes = write.open_table(SCOPES).map_err(error::redb)?;
                let roots = write.open_table(ROOT_SCOPES).map_err(error::redb)?;
                crate::journal::validate_scope_lineage_in_transaction(
                    &scopes,
                    &roots,
                    reference.scope(),
                )?;
                let key = crate::journal::workspace_value_key(reference)?;
                let bytes = values
                    .get(key.as_slice())
                    .map_err(error::redb)?
                    .ok_or_else(|| PersistenceError::NotFound {
                        entity: "artifact_provenance_workspace_value",
                        identity: format!(
                            "{}/{}/{}/{}",
                            reference.scope().run(),
                            reference.scope().scope(),
                            reference.key(),
                            reference.version()
                        ),
                    })?;
                let entry: WorkspaceValueEntry = json::decode(bytes.value(), "workspace value")?;
                if entry.reference() != reference {
                    return Err(error::corruption(
                        "artifact provenance workspace-value key disagrees with its document",
                    ));
                }
                crate::journal::validate_workspace_value_provenance(
                    &values, &scopes, &roots, &entry, false,
                )?;
                crate::journal::validate_workspace_value_storage_provenance_in_transaction(
                    write, &values, &scopes, &roots, &entry, false,
                )?;
            }
            CausalReference::RunInput { run, key } => {
                if record.owner.run() != Some(run) {
                    return Err(PersistenceError::InvalidDocument(
                        "artifact provenance run input belongs to another run".to_owned(),
                    ));
                }
                let head = crate::journal::validated_run_head_in_transaction(write, run)?;
                if crate::journal::validate_run_history_membership_in_transaction(write, run, head)?
                    .is_none()
                {
                    return Err(PersistenceError::NotFound {
                        entity: "artifact_provenance_run",
                        identity: run.to_string(),
                    });
                }
                let event_key =
                    codec::run_sequence(run.as_str(), milkdrift_persistence::RunSequence::FIRST)?;
                let events = write.open_table(RUN_EVENTS).map_err(error::redb)?;
                let bytes = events
                    .get(event_key.as_slice())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("run input provenance has no run-created event")
                    })?;
                let event = milkdrift_persistence::RunEventEnvelope::from_json(bytes.value())?;
                let known = matches!(
                    event.kind(),
                    milkdrift_persistence::RunEventKind::RunCreated { inputs, .. }
                        if inputs.iter().any(|input| input.key() == key)
                );
                if !known {
                    return Err(PersistenceError::NotFound {
                        entity: "artifact_provenance_run_input",
                        identity: format!("{run}/{key}"),
                    });
                }
            }
            CausalReference::Invocation { invocation } => {
                let run = record.owner.run().ok_or_else(|| {
                    PersistenceError::InvalidDocument(
                        "local invocation provenance requires a workflow owner".to_owned(),
                    )
                })?;
                crate::journal::validate_invocation_fact_in_transaction(write, run, invocation)?;
            }
            CausalReference::HostInvocation { host, invocation } => {
                if record.metadata.provenance().producer() == causal
                    && record.owner
                        != (milkdrift_workspace::ArtifactOwner::HostInvocation {
                            host: host.clone(),
                            invocation: invocation.clone(),
                        })
                {
                    return Err(PersistenceError::InvalidDocument(
                        "host invocation producer does not match publication owner".to_owned(),
                    ));
                }
            }
            CausalReference::ClientUpload { host, client, .. } => {
                if record.metadata.provenance().producer() == causal
                    && record.owner
                        != (milkdrift_workspace::ArtifactOwner::ClientInput {
                            host: host.clone(),
                            client: client.clone(),
                        })
                {
                    return Err(PersistenceError::InvalidDocument(
                        "client upload producer does not match publication owner".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}
