use std::ops::Bound;

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
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let by_digest = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        validated_revision_by_id(&revisions, &by_digest, revision)
    }

    fn revision_summary(
        &self,
        revision: &RevisionId,
    ) -> Result<Option<RevisionSummary>, PersistenceError> {
        Ok(self.revision(revision)?.as_ref().map(RevisionSummary::from))
    }

    fn revisions_by_content(
        &self,
        digest: &ContentDigest,
        limit: PageSize,
    ) -> Result<Vec<RevisionSummary>, PersistenceError> {
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let prefix = codec::component(digest.as_str())?;
        let end = codec::prefix_end(prefix.clone()).ok_or_else(|| PersistenceError::Bounds {
            location: "revision_content_prefix",
            reason: "prefix has no finite upper bound".to_owned(),
        })?;
        let limit = usize::try_from(limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "revision_page_size",
            reason: "cannot be represented on this platform".to_owned(),
        })?;
        let mut summaries = Vec::with_capacity(limit);
        let rows = table
            .range::<&[u8]>((
                Bound::Included(prefix.as_slice()),
                Bound::Excluded(end.as_slice()),
            ))
            .map_err(error::redb)?;
        for row in rows.take(limit) {
            let (key, value) = row.map_err(error::redb)?;
            let components = codec::decode_components(key.value(), 2)?;
            if components[0] != digest.as_str() {
                return Err(error::corruption(
                    "revision digest index key has the wrong content digest",
                ));
            }
            let summary = decode_summary(value.value())?;
            if summary.content_digest != *digest || components[1] != summary.revision.as_str() {
                return Err(error::corruption(
                    "revision digest index key disagrees with its summary",
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
            summaries.push(summary);
        }
        Ok(summaries)
    }
}

fn validated_revision_by_id_in_transaction(
    write: &redb::WriteTransaction,
    revision: &RevisionId,
) -> Result<Option<BlueprintRevision>, PersistenceError> {
    let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
    let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    validated_revision_by_id(&revisions, &by_digest, revision)
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
        for row in by_digest.iter().map_err(error::redb)? {
            let (key, value) = row.map_err(error::redb)?;
            let summary = decode_summary(value.value())?;
            let components = codec::decode_components(key.value(), 2)?;
            if components[0] != summary.content_digest.as_str()
                || components[1] != summary.revision.as_str()
            {
                return Err(error::corruption(
                    "revision digest index key disagrees with its summary",
                ));
            }
            if &summary.revision == revision {
                return Err(error::corruption(
                    "revision digest index points to a missing primary document",
                ));
            }
        }
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
