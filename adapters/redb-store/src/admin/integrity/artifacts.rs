use super::super::*;
use super::{ScanContext, phase};
use crate::admin::cursor::{
    make_artifact_digest_cursor, make_delete_guard_cursor, parse_artifact_digest_cursor,
    parse_delete_guard_cursor, push_failure,
};

pub(super) fn scan_committed(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let metadata = read.open_table(ARTIFACT_METADATA).map_err(error::redb)?;
    let manifest = read.open_table(ARTIFACT_MANIFEST).map_err(error::redb)?;
    let by_digest = read.open_table(ARTIFACTS_BY_DIGEST).map_err(error::redb)?;
    let references = read.open_table(ARTIFACT_REFERENCES).map_err(error::redb)?;
    let ownership = read
        .open_table(RUN_ARTIFACT_OWNERSHIP)
        .map_err(error::redb)?;
    let temporary_manifest = read
        .open_table(ARTIFACT_TEMP_MANIFEST)
        .map_err(error::redb)?;
    let temporary_owners = read.open_table(ARTIFACT_TEMP_OWNERS).map_err(error::redb)?;
    let accounting = read.open_table(ARTIFACT_ACCOUNTING).map_err(error::redb)?;

    context.string_bytes(
        phase::ARTIFACT_MANIFEST,
        &manifest,
        "artifact_indexes",
        |key, bytes| {
            let document: ArtifactMetadata = json::decode(bytes, "artifact manifest")?;
            if document.reference().artifact().as_str() != key {
                return Err(error::corruption(
                    "artifact manifest key does not match its checked document",
                ));
            }
            let primary = metadata
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact manifest has no metadata row"))?;
            let primary: ArtifactMetadata = json::decode(primary.value(), "artifact metadata")?;
            if primary != document {
                return Err(error::corruption(
                    "artifact manifest disagrees with its metadata row",
                ));
            }
            let digest = document.reference().digest().to_hex();
            let digest_key = codec::pair(&digest, key)?;
            let indexed = by_digest
                .get(digest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact manifest has no digest index row"))?;
            let indexed: ArtifactMetadata = json::decode(indexed.value(), "artifact metadata")?;
            if indexed != document {
                return Err(error::corruption(
                    "artifact digest index disagrees with its manifest",
                ));
            }
            Ok(())
        },
    )?;
    scan_digest_index(context, &by_digest, &metadata, &manifest, &accounting)?;
    context.binary_bytes(
        phase::ARTIFACT_REFERENCES,
        &references,
        "artifact_indexes",
        |key, bytes| {
            let (digest, artifact, run) = artifact_occurrence_key(key)?;
            let reference: ArtifactReference = json::decode(bytes, "artifact reference")?;
            if digest != reference.digest().to_hex() || artifact != reference.artifact().as_str() {
                return Err(error::corruption(
                    "artifact-reference key does not match its checked document",
                ));
            }
            let ownership_key = codec::components(&[&run, &digest, &artifact])?;
            let owned = ownership
                .get(ownership_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact occurrence has no ownership row"))?;
            let owned: ArtifactReference = json::decode(owned.value(), "run artifact ownership")?;
            if owned != reference {
                return Err(error::corruption(
                    "artifact occurrence disagrees with its ownership row",
                ));
            }
            Ok(())
        },
    )?;
    context.binary_bytes(
        phase::ARTIFACT_OWNERSHIP,
        &ownership,
        "artifact_indexes",
        |key, bytes| {
            let components = codec::decode_components(key, 3)?;
            let reference: ArtifactReference = json::decode(bytes, "run artifact ownership")?;
            let digest = reference.digest().to_hex();
            if components[1] != digest || components[2] != reference.artifact().as_str() {
                return Err(error::corruption(
                    "run artifact-ownership key does not match its checked document",
                ));
            }
            let prefix = codec::components(&[&digest, components[2], components[0]])?;
            let end = codec::prefix_end(prefix.clone()).ok_or_else(|| {
                error::corruption("artifact-reference ownership prefix has no end")
            })?;
            let occurrence = references
                .range::<&[u8]>(prefix.as_slice()..end.as_slice())
                .map_err(error::redb)?
                .next()
                .transpose()
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact ownership has no occurrence row"))?;
            let occurrence: ArtifactReference =
                json::decode(occurrence.1.value(), "artifact reference")?;
            if occurrence != reference {
                return Err(error::corruption(
                    "artifact ownership disagrees with its occurrence row",
                ));
            }
            Ok(())
        },
    )?;
    context.string_bytes(
        phase::ARTIFACT_TEMP_MANIFEST,
        &temporary_manifest,
        "artifact_indexes",
        |key, bytes| {
            let publication: ArtifactPublicationId =
                json::decode(bytes, "artifact temporary manifest")?;
            match temporary_owners.get(key).map_err(error::redb)? {
                Some(owner) if owner.value() == publication.as_str() => Ok(()),
                _ => Err(error::corruption(
                    "artifact temporary manifest has no matching owner row",
                )),
            }
        },
    )?;
    context.string_bytes(
        phase::ARTIFACT_ACCOUNTING,
        &accounting,
        "artifact_indexes",
        |key, bytes| {
            if key != crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY {
                return Err(error::corruption(
                    "artifact accounting contains an unknown record",
                ));
            }
            let record: crate::artifact::ArtifactAccountingRecord =
                json::decode(bytes, "artifact accounting")?;
            if record.schema_version != crate::artifact::ARTIFACT_ACCOUNTING_SCHEMA_VERSION {
                return Err(PersistenceError::UnsupportedVersion {
                    document: "artifact_accounting",
                    found: record.schema_version,
                    supported: crate::artifact::ARTIFACT_ACCOUNTING_SCHEMA_VERSION,
                });
            }
            Ok(())
        },
    )?;
    context.string_string(
        phase::ARTIFACT_TEMP_OWNERS,
        &temporary_owners,
        "artifact_indexes",
        |key, owner| {
            let publication = ArtifactPublicationId::new(owner).map_err(|cause| {
                error::corruption(format!("invalid artifact temporary owner: {cause}"))
            })?;
            let manifested = temporary_manifest
                .get(key)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("temporary owner has no manifest row"))?;
            let manifested: ArtifactPublicationId =
                json::decode(manifested.value(), "artifact temporary manifest")?;
            if manifested != publication {
                return Err(error::corruption(
                    "artifact temporary owner disagrees with its manifest",
                ));
            }
            Ok(())
        },
    )
}

pub(super) fn scan_publications(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let publications = read
        .open_table(ARTIFACT_PUBLICATIONS)
        .map_err(error::redb)?;
    let publications_by_age = read
        .open_table(ARTIFACT_PUBLICATIONS_BY_AGE)
        .map_err(error::redb)?;
    let reservations = read
        .open_table(ARTIFACT_RESERVATIONS)
        .map_err(error::redb)?;
    let paths = read.open_table(ARTIFACT_PATHS).map_err(error::redb)?;
    let delete_guards = read
        .open_table(ARTIFACT_DELETE_GUARDS)
        .map_err(error::redb)?;
    let digest_reservations = read
        .open_table(ARTIFACT_DIGEST_RESERVATIONS)
        .map_err(error::redb)?;

    context.string_bytes(
        phase::ARTIFACT_PUBLICATIONS,
        &publications,
        "artifact_publication_indexes",
        |key, bytes| crate::artifact::validate_publication_scrub(read, key, bytes),
    )?;
    context.binary_string(
        phase::ARTIFACT_PUBLICATION_AGE,
        &publications_by_age,
        "artifact_publication_indexes",
        |key, publication| crate::artifact::validate_publication_age_scrub(read, key, publication),
    )?;
    context.string_string(
        phase::ARTIFACT_RESERVATIONS,
        &reservations,
        "artifact_publication_indexes",
        |run, publication| {
            crate::artifact::validate_publication_reservation_scrub(read, run, publication)
        },
    )?;
    context.binary_bytes(
        phase::ARTIFACT_PATHS,
        &paths,
        "artifact_path_indexes",
        |key, value| crate::artifact::validate_path_scrub(read, key, value),
    )?;
    scan_delete_guards(context, &delete_guards, &paths)?;
    context.binary_u8(
        phase::ARTIFACT_DIGEST_RESERVATIONS,
        &digest_reservations,
        "artifact_publication_indexes",
        |key, marker| crate::artifact::validate_digest_reservation_scrub(read, key, marker),
    )
}

fn artifact_occurrence_key(key: &[u8]) -> Result<(String, String, String), PersistenceError> {
    let components = match codec::decode_components(key, 4) {
        Ok(components) => components,
        Err(_) => {
            let components = codec::decode_components(key, 5)?;
            if components[3] != "publication" {
                return Err(error::corruption(
                    "five-part artifact occurrence key has an unknown owner kind",
                ));
            }
            components
        }
    };
    Ok((
        components[0].to_owned(),
        components[1].to_owned(),
        components[2].to_owned(),
    ))
}

fn scan_digest_index(
    context: &mut ScanContext<'_, '_>,
    by_digest: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
    metadata: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    manifest: &impl redb::ReadableTable<&'static str, &'static [u8]>,
    accounting: &impl redb::ReadableTable<&'static str, &'static [u8]>,
) -> Result<(), PersistenceError> {
    let phase = phase::ARTIFACT_DIGESTS;
    if *context.more_remaining || phase < context.start_phase {
        return Ok(());
    }
    let (mut total, mut current_digest, mut current_size, after_key) =
        if phase == context.start_phase {
            let state = context.start_key.as_deref().ok_or_else(|| {
                PersistenceError::InvalidCursor(
                    "artifact digest integrity cursor has no state".to_owned(),
                )
            })?;
            parse_artifact_digest_cursor(state)?
        } else {
            (0, None, 0, None)
        };
    let lower = after_key.map_or(Bound::Unbounded, Bound::Excluded);
    for item in by_digest
        .range::<&[u8]>((lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        if context.result.documents_checked == context.maximum {
            *context.more_remaining = true;
            break;
        }
        let (key, value) = item.map_err(error::redb)?;
        context.result.documents_checked += 1;
        let checked = (|| {
            let components = codec::decode_components(key.value(), 2)?;
            let document: ArtifactMetadata = json::decode(value.value(), "artifact metadata")?;
            let digest = document.reference().digest().to_hex();
            let artifact = document.reference().artifact().as_str();
            if components[0] != digest || components[1] != artifact {
                return Err(error::corruption(
                    "artifact digest key does not match its checked metadata",
                ));
            }
            let primary = metadata
                .get(artifact)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact digest index has no metadata row"))?;
            let primary: ArtifactMetadata = json::decode(primary.value(), "artifact metadata")?;
            let manifested = manifest
                .get(artifact)
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("artifact digest index has no manifest row"))?;
            let manifested: ArtifactMetadata =
                json::decode(manifested.value(), "artifact manifest")?;
            if primary != document || manifested != document {
                return Err(error::corruption(
                    "artifact digest index disagrees with metadata or manifest",
                ));
            }
            Ok((digest, document.reference().size_bytes()))
        })();
        match checked {
            Ok((digest, size)) => {
                if current_digest.as_deref() == Some(digest.as_str()) {
                    if current_size != size {
                        push_failure(
                            context.result,
                            "artifact_indexes",
                            "artifact metadata disagrees on size for one content digest",
                        )?;
                    }
                } else {
                    match total.checked_add(size) {
                        Some(next) => total = next,
                        None => push_failure(
                            context.result,
                            "artifact_indexes",
                            "derived artifact content-byte total overflows",
                        )?,
                    }
                    current_digest = Some(digest);
                    current_size = size;
                }
            }
            Err(cause) => push_failure(context.result, "artifact_indexes", &cause.to_string())?,
        }
        *context.last_cursor = Some(make_artifact_digest_cursor(
            phase,
            key.value(),
            total,
            current_digest.as_deref(),
            current_size,
            context.verify_artifact_content,
            context.last_cursor.as_ref(),
        )?);
    }
    if !*context.more_remaining {
        let stored = accounting
            .get(crate::artifact::GLOBAL_ARTIFACT_BYTES_KEY)
            .map_err(error::redb)?
            .map(|bytes| {
                json::decode::<crate::artifact::ArtifactAccountingRecord>(
                    bytes.value(),
                    "artifact accounting",
                )
            })
            .transpose();
        match stored {
            Ok(None) if total == 0 => {}
            Ok(Some(record))
                if record.schema_version == crate::artifact::ARTIFACT_ACCOUNTING_SCHEMA_VERSION
                    && record.committed_content_bytes == total => {}
            Ok(Some(record))
                if record.schema_version != crate::artifact::ARTIFACT_ACCOUNTING_SCHEMA_VERSION =>
            {
                push_failure(
                    context.result,
                    "artifact_indexes",
                    "artifact accounting has an unsupported schema version",
                )?
            }
            Ok(_) => push_failure(
                context.result,
                "artifact_indexes",
                "artifact accounting does not equal the derived unique-digest byte total",
            )?,
            Err(cause) => {
                push_failure(context.result, "artifact_indexes", &cause.to_string())?;
            }
        }
    }
    Ok(())
}

fn scan_delete_guards(
    context: &mut ScanContext<'_, '_>,
    guards: &impl redb::ReadableTable<&'static [u8], u8>,
    paths: &impl redb::ReadableTable<&'static [u8], &'static [u8]>,
) -> Result<(), PersistenceError> {
    let phase = phase::ARTIFACT_DELETE_GUARDS;
    if *context.more_remaining || phase < context.start_phase {
        return Ok(());
    }
    let resumed = if phase == context.start_phase {
        context
            .start_key
            .as_deref()
            .map(parse_delete_guard_cursor)
            .transpose()?
    } else {
        None
    };
    let guard_lower = match resumed {
        Some((guard, true, _)) => Bound::Included(guard),
        Some((guard, false, _)) => Bound::Excluded(guard),
        None => Bound::Unbounded,
    };
    for row in guards
        .range::<&[u8]>((guard_lower, Bound::Unbounded))
        .map_err(error::redb)?
    {
        let (guard, marker) = row.map_err(error::redb)?;
        let guard_key = guard.value();
        let resume_path = resumed
            .filter(|(resumed_guard, in_progress, _)| *in_progress && *resumed_guard == guard_key)
            .and_then(|(_, _, path)| path);
        let is_resumed_guard =
            resumed.is_some_and(|(key, in_progress, _)| in_progress && key == guard_key);
        if !is_resumed_guard {
            if context.result.documents_checked == context.maximum {
                *context.more_remaining = true;
                break;
            }
            context.result.documents_checked += 1;
            if let Err(cause) =
                crate::artifact::validate_delete_guard_scrub(guard_key, marker.value())
            {
                push_failure(context.result, "artifact_path_indexes", &cause.to_string())?;
                *context.last_cursor = Some(make_delete_guard_cursor(
                    guard_key,
                    false,
                    None,
                    context.verify_artifact_content,
                    context.last_cursor.as_ref(),
                )?);
                continue;
            }
        }
        let path_lower = resume_path.map_or(Bound::Unbounded, Bound::Excluded);
        let mut found = false;
        let mut last_path = resume_path.map(<[u8]>::to_vec);
        for path in paths
            .range::<&[u8]>((path_lower, Bound::Unbounded))
            .map_err(error::redb)?
        {
            if context.result.documents_checked == context.maximum {
                *context.last_cursor = Some(make_delete_guard_cursor(
                    guard_key,
                    true,
                    last_path.as_deref(),
                    context.verify_artifact_content,
                    context.last_cursor.as_ref(),
                )?);
                *context.more_remaining = true;
                return Ok(());
            }
            let (path_key, path_value) = path.map_err(error::redb)?;
            context.result.documents_checked += 1;
            last_path = Some(path_key.value().to_vec());
            match crate::artifact::artifact_path_guard_key(path_key.value(), path_value.value()) {
                Ok(candidate) if candidate.as_slice() == guard_key => {
                    found = true;
                    break;
                }
                Ok(_) => {}
                Err(cause) => {
                    push_failure(context.result, "artifact_path_indexes", &cause.to_string())?
                }
            }
        }
        if !found {
            push_failure(
                context.result,
                "artifact_path_indexes",
                "artifact delete guard has no matching path inventory row",
            )?;
        }
        *context.last_cursor = Some(make_delete_guard_cursor(
            guard_key,
            false,
            None,
            context.verify_artifact_content,
            context.last_cursor.as_ref(),
        )?);
    }
    Ok(())
}
