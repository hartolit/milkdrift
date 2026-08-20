use milkdrift_blueprint::{
    BlueprintRevision, BlueprintRevisionDocument, ContentDigest, DocumentError, RevisionId,
};
use milkdrift_persistence::{
    ImmutableRevisionPut, PageSize, PersistenceError, RevisionStore, RevisionSummary,
};
use redb::ReadableTable;

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    schema::{REVISIONS, REVISIONS_BY_DIGEST},
};

impl RevisionStore for RedbStore {
    #[tracing::instrument(
        name = "milkdrift.redb_store.put_revision",
        skip_all,
        fields(
            revision = %revision.id(),
            workflow = %revision.semantic().workflow(),
            lineage_sequence = revision.sequence()
        )
    )]
    fn put_revision(
        &self,
        revision: &BlueprintRevision,
    ) -> Result<ImmutableRevisionPut, PersistenceError> {
        let document = BlueprintRevisionDocument::new(revision)
            .to_canonical_json()
            .map_err(invalid_blueprint)?;
        let summary = RevisionSummary::from(revision);
        let summary_bytes = crate::json::encode(&summary_wire(&summary), "revision summary")?;
        let digest_key = codec::pair(revision.content_digest().as_str(), revision.id().as_str())?;

        let write = self.database().begin_write().map_err(error::redb)?;
        crate::trie::validate_roots_in_transaction(&write)?;
        {
            let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
            let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
            validate_revision_cardinality(&revisions, &by_digest)?;
        }
        if let Some(existing) = validated_revision_by_id_in_transaction(&write, revision.id())? {
            if existing != *revision {
                return Err(PersistenceError::ImmutableConflict {
                    entity: "blueprint_revision",
                    identity: revision.id().to_string(),
                });
            }
            return Ok(ImmutableRevisionPut::AlreadyPresent);
        }

        for parent_id in revision.parents() {
            let parent =
                validated_revision_by_id_in_transaction(&write, parent_id)?.ok_or_else(|| {
                    PersistenceError::NotFound {
                        entity: "parent_revision",
                        identity: parent_id.to_string(),
                    }
                })?;
            if parent.semantic().workflow() != revision.semantic().workflow()
                || parent.sequence() >= revision.sequence()
            {
                return Err(PersistenceError::InvalidDocument(format!(
                    "parent revision {parent_id} must share workflow {} and precede lineage sequence {}",
                    revision.semantic().workflow(),
                    revision.sequence()
                )));
            }
        }
        {
            let mut revisions = write.open_table(REVISIONS).map_err(error::redb)?;
            if revisions
                .insert(revision.id().as_str(), document.as_slice())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "revision insert replaced an existing document",
                ));
            }
        }
        {
            let mut by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
            if by_digest
                .insert(digest_key.as_slice(), summary_bytes.as_slice())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "revision digest insert replaced an existing index row",
                ));
            }
        }
        insert_revision_catalog(&write, revision, &document, &digest_key, &summary_bytes)?;
        self.faults.check(FaultPoint::BeforeRevisionCommit)?;
        write.commit().map_err(error::redb)?;
        self.faults.check(FaultPoint::AfterRevisionCommit)?;
        Ok(ImmutableRevisionPut::Inserted)
    }

    fn revision(
        &self,
        revision: &RevisionId,
    ) -> Result<Option<BlueprintRevision>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let by_digest = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        validate_revision_cardinality(&revisions, &by_digest)?;
        let stored = validated_revision_by_id(&revisions, &by_digest, revision)?;
        validate_revision_catalog(&read, revision, stored.as_ref())?;
        Ok(stored)
    }

    fn revision_summary(
        &self,
        revision: &RevisionId,
    ) -> Result<Option<RevisionSummary>, PersistenceError> {
        let revision = self.revision(revision)?;
        Ok(revision.as_ref().map(RevisionSummary::from))
    }

    fn revisions_by_content(
        &self,
        digest: &ContentDigest,
        limit: PageSize,
    ) -> Result<Vec<RevisionSummary>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        crate::trie::validate_roots(&read)?;
        let table = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        validate_revision_cardinality(&revisions, &table)?;
        let family = crate::trie::CatalogFamily::RevisionContent;
        let group = revision_content_group(digest);
        let mut first = [0_u8; 32];
        first[..16].copy_from_slice(&group);
        let after = predecessor_path(first);
        let page = crate::trie::page(
            &read,
            family,
            None,
            after,
            usize::try_from(limit.get()).map_err(|_| PersistenceError::Bounds {
                location: "revision_page_size",
                reason: "cannot be represented on this platform".to_owned(),
            })?,
        )?;
        let mut summaries = Vec::with_capacity(page.leaves.len());
        for leaf in page.leaves {
            if leaf.path[..16] != group {
                break;
            }
            let components = codec::decode_components(&leaf.logical_key, 2)?;
            if components[0] != digest.as_str() {
                return Err(error::corruption(
                    "revision content catalog ordering prefix collides across digests",
                ));
            }
            let value = table
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision content catalog is dangling"))?;
            if leaf.payload_digest != crate::trie::digest_payload(family, value.value()) {
                return Err(error::corruption(
                    "revision digest summary disagrees with its authenticated catalog",
                ));
            }
            let summary = decode_summary(value.value())?;
            if &summary.content_digest != digest {
                return Err(error::corruption(
                    "revision digest index contains a mismatched summary",
                ));
            }
            let revision_bytes = revisions
                .get(summary.revision.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision digest index is dangling"))?;
            let revision = decode_revision(revision_bytes.value())?;
            if RevisionSummary::from(&revision) != summary {
                return Err(error::corruption(
                    "revision digest index disagrees with authoritative revision bytes",
                ));
            }
            let expected_key = codec::pair(digest.as_str(), summary.revision.as_str())?;
            if leaf.logical_key != expected_key {
                return Err(error::corruption(
                    "revision digest-index key disagrees with its summary",
                ));
            }
            validate_revision_catalog(&read, &summary.revision, Some(&revision))?;
            summaries.push(summary);
        }
        Ok(summaries)
    }
}

fn validate_revision_cardinality<R, I>(revisions: &R, by_digest: &I) -> Result<(), PersistenceError>
where
    R: ReadableTable<&'static str, &'static [u8]>,
    I: ReadableTable<&'static [u8], &'static [u8]>,
{
    if revisions.len().map_err(error::redb)? != by_digest.len().map_err(error::redb)? {
        return Err(error::corruption(
            "revision primary table and digest index have different cardinality",
        ));
    }
    Ok(())
}

fn revision_identity_path(revision: &RevisionId) -> [u8; 32] {
    let family = crate::trie::CatalogFamily::RevisionIdentity;
    crate::trie::hashed_path(family, revision.as_str().as_bytes())
}

fn revision_content_group(digest: &ContentDigest) -> [u8; 16] {
    let family = crate::trie::CatalogFamily::RevisionContent;
    let hash = crate::trie::hashed_path(family, digest.as_str().as_bytes());
    let mut group = [0_u8; 16];
    group.copy_from_slice(&hash[..16]);
    group
}

fn revision_content_path(
    digest: &ContentDigest,
    logical_key: &[u8],
) -> Result<[u8; 32], PersistenceError> {
    crate::trie::ordered_path(
        crate::trie::CatalogFamily::RevisionContent,
        &revision_content_group(digest),
        logical_key,
    )
}

fn predecessor_path(mut path: [u8; 32]) -> Option<[u8; 32]> {
    for index in (0..path.len()).rev() {
        if path[index] != 0 {
            path[index] -= 1;
            path[index + 1..].fill(u8::MAX);
            return Some(path);
        }
    }
    None
}

fn insert_revision_catalog(
    write: &redb::WriteTransaction,
    revision: &BlueprintRevision,
    document: &[u8],
    digest_key: &[u8],
    summary: &[u8],
) -> Result<(), PersistenceError> {
    let identity_family = crate::trie::CatalogFamily::RevisionIdentity;
    if crate::trie::put(
        write,
        identity_family,
        revision_identity_path(revision.id()),
        revision.id().as_str().as_bytes(),
        crate::trie::digest_payload(identity_family, document),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "revision identity catalog unexpectedly replaced a leaf",
        ));
    }
    let content_family = crate::trie::CatalogFamily::RevisionContent;
    if crate::trie::put(
        write,
        content_family,
        revision_content_path(revision.content_digest(), digest_key)?,
        digest_key,
        crate::trie::digest_payload(content_family, summary),
    )?
    .is_some()
    {
        return Err(error::corruption(
            "revision content catalog unexpectedly replaced a leaf",
        ));
    }
    Ok(())
}

pub(crate) fn migrate_revision_catalog(
    write: &redb::WriteTransaction,
    revision: &BlueprintRevision,
    document: &[u8],
    digest_key: &[u8],
    summary: &[u8],
) -> Result<(), PersistenceError> {
    insert_revision_catalog(write, revision, document, digest_key, summary)
}

pub(crate) fn validate_catalog_leaf(
    read: &redb::ReadTransaction,
    family: crate::trie::CatalogFamily,
    leaf: &crate::trie::TrieLeaf,
) -> Result<(), PersistenceError> {
    match family {
        crate::trie::CatalogFamily::RevisionIdentity => {
            let revision_text = std::str::from_utf8(&leaf.logical_key)
                .map_err(|_| error::corruption("revision catalog identity is not UTF-8"))?;
            let revision: RevisionId = serde_json::from_value(serde_json::Value::String(
                revision_text.to_owned(),
            ))
            .map_err(|cause| {
                error::corruption(format!("invalid revision catalog identity: {cause}"))
            })?;
            let bytes = read
                .open_table(REVISIONS)
                .map_err(error::redb)?
                .get(revision.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision identity catalog is dangling"))?
                .value()
                .to_vec();
            let stored = decode_revision(&bytes)?;
            if stored.id() != &revision
                || leaf.path != revision_identity_path(&revision)
                || leaf.payload_digest
                    != crate::trie::digest_payload(family, bytes.as_slice())
            {
                return Err(error::corruption(
                    "revision identity leaf disagrees with its checked document",
                ));
            }
            validate_revision_catalog(read, &revision, Some(&stored))
        }
        crate::trie::CatalogFamily::RevisionContent => {
            let bytes = read
                .open_table(REVISIONS_BY_DIGEST)
                .map_err(error::redb)?
                .get(leaf.logical_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision content catalog is dangling"))?
                .value()
                .to_vec();
            let summary = decode_summary(&bytes)?;
            let key = codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
            if key != leaf.logical_key
                || leaf.path != revision_content_path(&summary.content_digest, &key)?
                || leaf.payload_digest
                    != crate::trie::digest_payload(family, bytes.as_slice())
            {
                return Err(error::corruption(
                    "revision content leaf disagrees with its checked summary",
                ));
            }
            let document = read
                .open_table(REVISIONS)
                .map_err(error::redb)?
                .get(summary.revision.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision content catalog is dangling"))?;
            let revision = decode_revision(document.value())?;
            if RevisionSummary::from(&revision) != summary {
                return Err(error::corruption(
                    "revision content leaf disagrees with its authoritative revision",
                ));
            }
            validate_revision_catalog(read, revision.id(), Some(&revision))
        }
        _ => Err(error::corruption(
            "revision catalog validator received another family's leaf",
        )),
    }
}

fn validate_revision_catalog_in_transaction(
    write: &redb::WriteTransaction,
    revision: &RevisionId,
    stored: Option<&BlueprintRevision>,
) -> Result<(), PersistenceError> {
    let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
    let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    validate_revision_catalog_tables(
        |family, path, key| crate::trie::verify_member_in_transaction(write, family, path, key),
        &revisions,
        &by_digest,
        revision,
        stored,
    )
}

fn validated_revision_by_id_in_transaction(
    write: &redb::WriteTransaction,
    revision: &RevisionId,
) -> Result<Option<BlueprintRevision>, PersistenceError> {
    let stored = {
        let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
        let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        validated_revision_by_id(&revisions, &by_digest, revision)?
    };
    validate_revision_catalog_in_transaction(write, revision, stored.as_ref())?;
    Ok(stored)
}

fn validate_revision_catalog(
    read: &redb::ReadTransaction,
    revision: &RevisionId,
    stored: Option<&BlueprintRevision>,
) -> Result<(), PersistenceError> {
    let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
    let by_digest = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    validate_revision_catalog_tables(
        |family, path, key| crate::trie::verify_member(read, family, path, key),
        &revisions,
        &by_digest,
        revision,
        stored,
    )
}

fn validate_revision_catalog_tables<R, I, V>(
    mut verify: V,
    revisions: &R,
    by_digest: &I,
    revision: &RevisionId,
    stored: Option<&BlueprintRevision>,
) -> Result<(), PersistenceError>
where
    R: ReadableTable<&'static str, &'static [u8]>,
    I: ReadableTable<&'static [u8], &'static [u8]>,
    V: FnMut(
        crate::trie::CatalogFamily,
        [u8; 32],
        &[u8],
    ) -> Result<Option<[u8; 32]>, PersistenceError>,
{
    let identity_family = crate::trie::CatalogFamily::RevisionIdentity;
    let identity = verify(
        identity_family,
        revision_identity_path(revision),
        revision.as_str().as_bytes(),
    )?;
    let Some(stored) = stored else {
        return if identity.is_none() {
            Ok(())
        } else {
            Err(error::corruption(
                "revision catalog names a missing primary document",
            ))
        };
    };
    let document = revisions
        .get(revision.as_str())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("revision primary document disappeared"))?;
    if identity
        != Some(crate::trie::digest_payload(
            identity_family,
            document.value(),
        ))
    {
        return Err(error::corruption(
            "revision primary document disagrees with its authenticated catalog",
        ));
    }
    let digest_key = codec::pair(stored.content_digest().as_str(), stored.id().as_str())?;
    let summary = by_digest
        .get(digest_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("revision digest summary disappeared"))?;
    let content_family = crate::trie::CatalogFamily::RevisionContent;
    let content = verify(
        content_family,
        revision_content_path(stored.content_digest(), &digest_key)?,
        &digest_key,
    )?;
    if content != Some(crate::trie::digest_payload(content_family, summary.value())) {
        return Err(error::corruption(
            "revision digest summary disagrees with its authenticated catalog",
        ));
    }
    Ok(())
}

fn validated_revision_by_id<R, I>(
    revisions: &R,
    by_digest: &I,
    revision: &RevisionId,
) -> Result<Option<BlueprintRevision>, PersistenceError>
where
    R: ReadableTable<&'static str, &'static [u8]>,
    I: ReadableTable<&'static [u8], &'static [u8]>,
{
    let Some(bytes) = revisions.get(revision.as_str()).map_err(error::redb)? else {
        return Ok(None);
    };
    let stored = decode_revision(bytes.value())?;
    if stored.id() != revision {
        return Err(error::corruption(
            "revision key does not match its verified document",
        ));
    }
    let digest_key = codec::pair(stored.content_digest().as_str(), stored.id().as_str())?;
    let indexed = by_digest
        .get(digest_key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("revision is absent from its digest index"))?;
    if decode_summary(indexed.value())? != RevisionSummary::from(&stored) {
        return Err(error::corruption(
            "revision digest index disagrees with authoritative revision bytes",
        ));
    }
    Ok(Some(stored))
}

#[derive(serde::Serialize)]
struct RevisionSummaryWire<'a> {
    revision: &'a RevisionId,
    workflow: &'a milkdrift_blueprint::WorkflowId,
    lineage_sequence: u64,
    content_digest: &'a ContentDigest,
    parents: &'a [RevisionId],
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedRevisionSummaryWire {
    revision: RevisionId,
    workflow: milkdrift_blueprint::WorkflowId,
    lineage_sequence: u64,
    content_digest: ContentDigest,
    parents: Vec<RevisionId>,
}

fn summary_wire(summary: &RevisionSummary) -> RevisionSummaryWire<'_> {
    RevisionSummaryWire {
        revision: &summary.revision,
        workflow: &summary.workflow,
        lineage_sequence: summary.lineage_sequence,
        content_digest: &summary.content_digest,
        parents: &summary.parents,
    }
}

pub(crate) fn decode_summary(bytes: &[u8]) -> Result<RevisionSummary, PersistenceError> {
    let wire: OwnedRevisionSummaryWire = crate::json::decode(bytes, "revision summary")?;
    Ok(RevisionSummary {
        revision: wire.revision,
        workflow: wire.workflow,
        lineage_sequence: wire.lineage_sequence,
        content_digest: wire.content_digest,
        parents: wire.parents,
    })
}

pub(crate) fn decode_revision(bytes: &[u8]) -> Result<BlueprintRevision, PersistenceError> {
    BlueprintRevisionDocument::from_json(bytes)
        .map(|(_document, revision)| revision)
        .map_err(stored_blueprint)
}

fn invalid_blueprint(error: DocumentError) -> PersistenceError {
    PersistenceError::InvalidDocument(error.to_string())
}

fn stored_blueprint(error: DocumentError) -> PersistenceError {
    match error {
        DocumentError::UnsupportedVersion { found, supported } => {
            PersistenceError::UnsupportedVersion {
                document: "blueprint_revision",
                found,
                supported,
            }
        }
        other => PersistenceError::Corruption(format!(
            "stored blueprint revision failed verification: {other}"
        )),
    }
}
