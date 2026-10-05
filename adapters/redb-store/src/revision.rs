//! Store immutable definitions with their ancestry and content-discovery indexes.
//!
//! Insertion verifies parents and exact canonical content before one transaction publishes
//! the revision. Read paths check indexes against those documents. A successful insert
//! only makes a revision available; runtime commands own run creation and adoption.

use std::ops::Bound;

use milkdrift_blueprint::{
    BlueprintRevision, BlueprintRevisionDocument, ContentDigest, DocumentError, RevisionId,
};
use milkdrift_persistence::{
    ImmutableRevisionPut, PageSize, PersistenceError, RevisionCursor, RevisionPage,
    RevisionPageQuery, RevisionStore, RevisionSummary,
};
use redb::ReadableTable;

use crate::{
    RedbStore, codec, error,
    fault::FaultPoint,
    schema::{REVISIONS, REVISIONS_BY_DIGEST, REVISIONS_BY_WORKFLOW},
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
        let workflow_key = codec::pair(
            revision.semantic().workflow().as_str(),
            revision.id().as_str(),
        )?;

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

        {
            let mut by_workflow = write
                .open_table(REVISIONS_BY_WORKFLOW)
                .map_err(error::redb)?;
            if by_workflow
                .insert(workflow_key.as_slice(), summary_bytes.as_slice())
                .map_err(error::redb)?
                .is_some()
            {
                return Err(error::corruption(
                    "revision workflow insert replaced an existing index row",
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
        let by_workflow = read
            .open_table(REVISIONS_BY_WORKFLOW)
            .map_err(error::redb)?;
        validated_revision_by_id(&revisions, &by_digest, &by_workflow, revision)
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
        let by_workflow = read
            .open_table(REVISIONS_BY_WORKFLOW)
            .map_err(error::redb)?;
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
            let components = codec::decode_components::<2>(key.value())?;
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
            validate_workflow_index(&by_workflow, &summary)?;
            summaries.push(summary);
        }
        Ok(summaries)
    }

    fn revisions(&self, query: &RevisionPageQuery) -> Result<RevisionPage, PersistenceError> {
        if query
            .cursor
            .as_ref()
            .is_some_and(|cursor| !cursor.matches(&query.filter))
        {
            return Err(PersistenceError::InvalidCursor(
                "revision cursor belongs to another filter".to_owned(),
            ));
        }
        let read = self.database().begin_read().map_err(error::redb)?;
        let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
        let by_digest = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
        let by_workflow = read
            .open_table(REVISIONS_BY_WORKFLOW)
            .map_err(error::redb)?;
        let start = query
            .cursor
            .as_ref()
            .map(|cursor| cursor.after_revision().as_str());
        let limit = usize::try_from(query.limit.get()).map_err(|_| PersistenceError::Bounds {
            location: "revision_page_size",
            reason: "cannot be represented on this platform".to_owned(),
        })?;
        let result = if let Some(workflows) = &query.filter.workflows {
            scoped_summaries(&by_workflow, workflows, start, limit)?
                .into_iter()
                .map(|summary| {
                    if revisions
                        .get(summary.revision.as_str())
                        .map_err(error::redb)?
                        .is_none()
                    {
                        return Err(error::corruption(
                            "workflow index points to an absent revision",
                        ));
                    }
                    let revision = validated_revision_by_id(
                        &revisions,
                        &by_digest,
                        &by_workflow,
                        &summary.revision,
                    )?
                    .ok_or_else(|| {
                        error::corruption("workflow index points to an absent revision")
                    })?;
                    if RevisionSummary::from(&revision) != summary {
                        return Err(error::corruption(
                            "workflow index disagrees with its revision",
                        ));
                    }
                    Ok(summary)
                })
                .collect::<Result<Vec<_>, PersistenceError>>()?
        } else {
            let rows = match start {
                Some(after) => revisions
                    .range::<&str>((Bound::Excluded(after), Bound::Unbounded))
                    .map_err(error::redb)?,
                None => revisions.iter().map_err(error::redb)?,
            };
            rows.take(limit)
                .map(|row| {
                    let (key, _) = row.map_err(error::redb)?;
                    let id: RevisionId =
                        serde_json::from_value(serde_json::Value::String(key.value().to_owned()))
                            .map_err(|cause| {
                            error::corruption(format!("invalid revision identity: {cause}"))
                        })?;
                    let revision =
                        validated_revision_by_id(&revisions, &by_digest, &by_workflow, &id)?
                            .ok_or_else(|| {
                                error::corruption("revision disappeared during read transaction")
                            })?;
                    Ok(RevisionSummary::from(&revision))
                })
                .collect::<Result<Vec<_>, PersistenceError>>()?
        };
        let next = if result.len() == limit {
            result
                .last()
                .map(|summary| RevisionCursor::new(summary.revision.clone(), query.filter.clone()))
        } else {
            None
        };
        Ok(RevisionPage {
            revisions: result,
            next,
        })
    }
}

fn validated_revision_by_id_in_transaction(
    write: &redb::WriteTransaction,
    revision: &RevisionId,
) -> Result<Option<BlueprintRevision>, PersistenceError> {
    let revisions = write.open_table(REVISIONS).map_err(error::redb)?;
    let by_digest = write.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    let by_workflow = write
        .open_table(REVISIONS_BY_WORKFLOW)
        .map_err(error::redb)?;
    validated_revision_by_id(&revisions, &by_digest, &by_workflow, revision)
}

fn validated_revision_by_id<R, I, W>(
    revisions: &R,
    by_digest: &I,
    by_workflow: &W,
    revision: &RevisionId,
) -> Result<Option<BlueprintRevision>, PersistenceError>
where
    R: ReadableTable<&'static str, &'static [u8]>,
    I: ReadableTable<&'static [u8], &'static [u8]>,
    W: ReadableTable<&'static [u8], &'static [u8]>,
{
    let Some(bytes) = revisions.get(revision.as_str()).map_err(error::redb)? else {
        for row in by_digest.iter().map_err(error::redb)? {
            let (key, value) = row.map_err(error::redb)?;
            let summary = decode_summary(value.value())?;
            let components = codec::decode_components::<2>(key.value())?;
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
    validate_workflow_index(by_workflow, &RevisionSummary::from(&stored))?;
    Ok(Some(stored))
}

fn validate_workflow_index(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
    summary: &RevisionSummary,
) -> Result<(), PersistenceError> {
    let key = codec::pair(summary.workflow.as_str(), summary.revision.as_str())?;
    let row = table
        .get(key.as_slice())
        .map_err(error::redb)?
        .ok_or_else(|| error::corruption("revision is absent from its workflow index"))?;
    if decode_summary(row.value())? != *summary {
        return Err(error::corruption(
            "workflow index disagrees with authoritative revision",
        ));
    }
    Ok(())
}

fn scoped_summaries(
    table: &impl ReadableTable<&'static [u8], &'static [u8]>,
    workflows: &milkdrift_authority::WorkflowSet,
    after: Option<&str>,
    limit: usize,
) -> Result<Vec<RevisionSummary>, PersistenceError> {
    // Keep one index head per permitted workflow and merge by revision identity. There is no
    // scan through hidden definitions, per-caller cache, or cursor-sized copy of the result set.
    let mut readers = workflows
        .values()
        .iter()
        .map(|workflow| {
            let prefix = codec::component(workflow.as_str())?;
            let end = codec::prefix_end(prefix.clone())
                .ok_or_else(|| error::corruption("workflow index prefix has no end"))?;
            let start = match after {
                Some(after) => Bound::Excluded(codec::pair(workflow.as_str(), after)?),
                None => Bound::Included(prefix),
            };
            let rows = table
                .range::<&[u8]>((
                    start.as_ref().map(Vec::as_slice),
                    Bound::Excluded(end.as_slice()),
                ))
                .map_err(error::redb)?;
            Ok(rows.map(move |row| {
                let (key, bytes) = row.map_err(error::redb)?;
                let summary = decode_summary(bytes.value())?;
                let expected = codec::pair(workflow.as_str(), summary.revision.as_str())?;
                if &summary.workflow != workflow || key.value() != expected.as_slice() {
                    return Err(error::corruption(
                        "workflow index key disagrees with its summary",
                    ));
                }
                Ok(summary)
            }))
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    let mut heads = std::collections::BTreeMap::new();
    for (index, reader) in readers.iter_mut().enumerate() {
        if let Some(summary) = reader.next().transpose()?
            && heads
                .insert(summary.revision.clone(), (index, summary))
                .is_some()
        {
            return Err(error::corruption(
                "revision appears in multiple workflow ranges",
            ));
        }
    }
    let mut result = Vec::with_capacity(limit);
    while result.len() < limit {
        let Some((_, (index, summary))) = heads.pop_first() else {
            break;
        };
        result.push(summary);
        let reader = readers
            .get_mut(index)
            .ok_or_else(|| error::corruption("workflow merge lost its reader"))?;
        if let Some(summary) = reader.next().transpose()?
            && heads
                .insert(summary.revision.clone(), (index, summary))
                .is_some()
        {
            return Err(error::corruption(
                "revision appears in multiple workflow ranges",
            ));
        }
    }
    Ok(result)
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
