use super::*;
use super::{
    accounting::validate_artifact_state,
    path::{
        ArtifactPathEntry, ArtifactPathKind, TempInventoryState, artifact_delete_guard_exists,
        artifact_path_entry, artifact_path_exists, decode_artifact_path_entry, publication_age_key,
        publication_temp_name, put_artifact_delete_guard, remove_artifact_delete_guard,
        remove_artifact_path, require_content_intent, sync_directory, temp_inventory_state,
    },
    publication::{
        decode_publication, optional_publication_in_transaction, publication_in_transaction,
    },
};
pub(crate) fn expire_writable_publications(
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
            result.unreferenced_blobs_removed = result.unreferenced_blobs_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(has_more)
}

pub(crate) fn validate_writable_publication_indexes(
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

pub(crate) fn validate_temporary_manifest(
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

pub(crate) fn remove_temporary_manifest(
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

pub(crate) fn release_writable_publication(
    write: &redb::WriteTransaction,
    record: &PublicationRecord,
) -> Result<(), PersistenceError> {
    validate_artifact_state(write)?;
    validate_writable_publication_indexes(write, record)?;
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
    validate_artifact_state(write)?;
    Ok(())
}

pub(crate) fn remove_publication_age_index(
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

pub(crate) fn cleanup_temporary_files(
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
    let paths = read.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let temporary_start = [ArtifactPathKind::TempPending.ordered_tag()];
    let temporary_end = [ArtifactPathKind::ContentIntent.ordered_tag()];
    let lower = after.as_deref().map_or(
        std::ops::Bound::Included(temporary_start.as_slice()),
        std::ops::Bound::Excluded,
    );
    let mut entries = Vec::new();
    let mut has_more = false;
    let rows = paths
        .range::<&[u8]>((lower, std::ops::Bound::Excluded(temporary_end.as_slice())))
        .map_err(error::redb)?;
    for row in rows {
        let (key, value) = row.map_err(error::redb)?;
        let entry = decode_artifact_path_entry(key.value(), value.value())?;
        if entries.len() == remaining {
            has_more = true;
            break;
        }
        entries.push(entry);
    }
    drop(paths);
    drop(read);
    for entry in entries {
        *examined += 1;
        *last_cursor = Some(OrphanCleanupCursor::new(
            OrphanCleanupFamily::TemporaryFiles,
            entry.storage_key.clone(),
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

pub(crate) fn temporary_manifest_publication(
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

pub(crate) fn cleanup_content_files(
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
    let paths = read.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let content_start = [ArtifactPathKind::ContentIntent.ordered_tag()];
    let lower = after.as_deref().map_or(
        std::ops::Bound::Included(content_start.as_slice()),
        std::ops::Bound::Excluded,
    );
    let mut entries = Vec::new();
    let mut has_more = false;
    let rows = paths
        .range::<&[u8]>((lower, std::ops::Bound::Unbounded))
        .map_err(error::redb)?;
    for row in rows {
        let (key, value) = row.map_err(error::redb)?;
        let entry = decode_artifact_path_entry(key.value(), value.value())?;
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
    drop(paths);
    drop(read);
    for entry in entries {
        *examined += 1;
        *last_cursor = Some(OrphanCleanupCursor::new(
            OrphanCleanupFamily::ContentFiles,
            entry.storage_key.clone(),
            request.created_before,
        )?);
        if let Some(size) = cleanup_content_inventory_entry(store, &entry, request)? {
            result.unreferenced_blobs_removed = result.unreferenced_blobs_removed.saturating_add(1);
            result.bytes_reclaimed = result.bytes_reclaimed.saturating_add(size);
        }
    }
    Ok(has_more)
}

pub(crate) fn decode_cleanup_path_cursor(
    after: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, PersistenceError> {
    after
        .map(|bytes| {
            if bytes.is_empty() || bytes[0] > ArtifactPathKind::ContentIntent.ordered_tag() {
                return Err(PersistenceError::InvalidCursor(
                    "artifact cleanup cursor does not contain a valid path-inventory key"
                        .to_owned(),
                ));
            }
            Ok(bytes.to_vec())
        })
        .transpose()
}

pub(crate) fn cleanup_temporary_inventory_entry(
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
    validate_artifact_state(&write)?;
    if !artifact_path_exists(&write, entry)? {
        return Err(error::corruption(
            "artifact cleanup path disappeared after bounded enumeration",
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
        Some(_) => {
            return Err(error::corruption(
                "temporary path inventory state is invalid",
            ));
        }
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
    validate_artifact_state(&finalize)?;
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

pub(crate) fn cleanup_content_inventory_entry(
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
        error::corruption(format!(
            "artifact content inventory has invalid digest: {cause}"
        ))
    })?;
    let write = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_state(&write)?;
    if !artifact_path_exists(&write, entry)? {
        return Err(error::corruption(
            "artifact content path disappeared after bounded enumeration",
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
        Some(_) => {
            return Err(error::corruption(
                "content path publication state is invalid",
            ));
        }
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
    validate_artifact_state(&finalize)?;
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

pub(crate) fn remove_released_publication_if_uninventoried(
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
    Ok(())
}

pub(crate) fn digest_has_metadata_or_references(
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

pub(crate) fn remove_cleanup_file_if_present(
    store: &RedbStore,
    path: &Path,
    parent: &Path,
) -> Result<Option<u64>, PersistenceError> {
    remove_file_if_present(store, path, parent, None)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FinalizedPathBytes {
    pub(crate) temporary: Option<u64>,
    pub(crate) content: Option<u64>,
}

pub(crate) fn finalize_released_publication_paths(
    store: &RedbStore,
    record: &PublicationRecord,
    fault_boundary: Option<(FaultPoint, FaultPoint)>,
) -> Result<FinalizedPathBytes, PersistenceError> {
    let prepare = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_state(&prepare)?;
    let current = optional_publication_in_transaction(&prepare, &record.publication)?;
    match current.as_ref().map(|stored| &stored.state) {
        Some(PublicationState::Committed { .. })
            if matches!(record.state, PublicationState::Committed { .. }) => {}
        Some(PublicationState::Released) | None
            if matches!(
                record.state,
                PublicationState::Writable | PublicationState::Released
            ) => {}
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
    if let Some(entry) = temp_entry.as_ref()
        && !artifact_delete_guard_exists(&prepare, entry.kind, &entry.identity)?
    {
        put_artifact_delete_guard(&prepare, entry.kind, &entry.identity)?;
        guard_changed = true;
    }
    if content_exists && !artifact_delete_guard_exists(&prepare, content.kind, &content.identity)? {
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
        removed.content = remove_file_if_present(store, &content_path, parent, fault_boundary)?;
    }

    let finalize = store.database().begin_write().map_err(error::redb)?;
    validate_artifact_state(&finalize)?;
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

pub(crate) fn remove_file_if_present(
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
