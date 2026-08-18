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
        let summary_bytes = serde_json::to_vec(&summary_wire(&summary)).map_err(invalid_json)?;
        let digest_key = codec::pair(revision.content_digest().as_str(), revision.id().as_str())?;

        let write = self.database().begin_write().map_err(error::redb)?;
        {
            let mut revisions = write.open_table(REVISIONS).map_err(error::redb)?;
            if let Some(existing) = revisions.get(revision.id().as_str()).map_err(error::redb)? {
                let existing = existing.value();
                if existing != document.as_slice() {
                    return Err(PersistenceError::ImmutableConflict {
                        entity: "blueprint_revision",
                        identity: revision.id().to_string(),
                    });
                }
                decode_revision(existing)?;
                let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
                let indexed = by_digest
                    .get(digest_key.as_slice())
                    .map_err(error::redb)?
                    .ok_or_else(|| {
                        error::corruption("existing revision is absent from its digest index")
                    })?;
                if decode_summary(indexed.value())? != summary {
                    return Err(error::corruption(
                        "existing revision digest index summary is inconsistent",
                    ));
                }
                return Ok(ImmutableRevisionPut::AlreadyPresent);
            }

            for parent_id in revision.parents() {
                let parent_bytes = revisions
                    .get(parent_id.as_str())
                    .map_err(error::redb)?
                    .ok_or_else(|| PersistenceError::NotFound {
                        entity: "parent_revision",
                        identity: parent_id.to_string(),
                    })?;
                let parent = decode_revision(parent_bytes.value())?;
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
            revisions
                .insert(revision.id().as_str(), document.as_slice())
                .map_err(error::redb)?;
        }
        {
            let mut by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
            by_digest
                .insert(digest_key.as_slice(), summary_bytes.as_slice())
                .map_err(error::redb)?;
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
        let table = read.open_table(REVISIONS).map_err(error::redb)?;
        table
            .get(revision.as_str())
            .map_err(error::redb)?
            .map(|bytes| {
                let stored = decode_revision(bytes.value())?;
                if stored.id() != revision {
                    return Err(error::corruption(
                        "revision key does not match its verified document",
                    ));
                }
                Ok(stored)
            })
            .transpose()
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
        let prefix = codec::component(digest.as_str())?;
        let end = codec::prefix_end(prefix.clone())
            .ok_or_else(|| error::corruption("revision digest prefix has no range end"))?;
        let read = self.database().begin_read().map_err(error::redb)?;
        let table = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let mut summaries = Vec::with_capacity(limit.get() as usize);
        for item in table
            .range(prefix.as_slice()..end.as_slice())
            .map_err(error::redb)?
            .take(limit.get() as usize)
        {
            let (_key, value) = item.map_err(error::redb)?;
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
            summaries.push(summary);
        }
        Ok(summaries)
    }
}

#[derive(serde::Serialize)]
struct RevisionSummaryWire<'a> {
    revision: &'a RevisionId,
    workflow: &'a milkdrift_blueprint::WorkflowId,
    lineage_sequence: u64,
    content_digest: &'a ContentDigest,
    parents: &'a [RevisionId],
}

#[derive(serde::Deserialize)]
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

fn decode_summary(bytes: &[u8]) -> Result<RevisionSummary, PersistenceError> {
    let wire: OwnedRevisionSummaryWire = serde_json::from_slice(bytes)
        .map_err(|cause| error::corruption(format!("invalid revision summary: {cause}")))?;
    Ok(RevisionSummary {
        revision: wire.revision,
        workflow: wire.workflow,
        lineage_sequence: wire.lineage_sequence,
        content_digest: wire.content_digest,
        parents: wire.parents,
    })
}

fn decode_revision(bytes: &[u8]) -> Result<BlueprintRevision, PersistenceError> {
    BlueprintRevisionDocument::from_json(bytes)
        .map(|(_document, revision)| revision)
        .map_err(stored_blueprint)
}

fn invalid_blueprint(error: DocumentError) -> PersistenceError {
    PersistenceError::InvalidDocument(error.to_string())
}

fn invalid_json(error: serde_json::Error) -> PersistenceError {
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
