use super::super::{
    BlueprintRevisionDocument, PersistenceError, REVISIONS, REVISIONS_BY_DIGEST, RevisionSummary,
    codec, error,
};
use super::{ScanContext, phase};

pub(super) fn scan(context: &mut ScanContext<'_, '_>) -> Result<(), PersistenceError> {
    let read = context.read;
    let revision_digests = read.open_table(REVISIONS_BY_DIGEST).map_err(error::redb)?;
    let revisions = read.open_table(REVISIONS).map_err(error::redb)?;
    context.string_bytes(
        phase::REVISION_PRIMARY,
        &revisions,
        "revision_indexes",
        |key, bytes| {
            let (_document, revision) =
                BlueprintRevisionDocument::from_json(bytes).map_err(|cause| {
                    error::corruption(format!("stored revision failed verification: {cause}"))
                })?;
            if revision.id().as_str() != key {
                return Err(error::corruption(
                    "revision primary key does not match its checked document",
                ));
            }
            let summary = RevisionSummary::from(&revision);
            let digest_key =
                codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
            let indexed = revision_digests
                .get(digest_key.as_slice())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision has no digest index row"))?;
            if crate::revision::decode_summary(indexed.value())? != summary {
                return Err(error::corruption(
                    "revision digest summary disagrees with its primary document",
                ));
            }
            Ok(())
        },
    )?;
    context.binary_bytes(
        phase::REVISION_DIGESTS,
        &revision_digests,
        "revision_indexes",
        |key, bytes| {
            let summary = crate::revision::decode_summary(bytes)?;
            let expected = codec::pair(summary.content_digest.as_str(), summary.revision.as_str())?;
            if key != expected.as_slice() {
                return Err(error::corruption(
                    "revision digest key does not match its checked summary",
                ));
            }
            let document = revisions
                .get(summary.revision.as_str())
                .map_err(error::redb)?
                .ok_or_else(|| error::corruption("revision digest index pointer is dangling"))?;
            let (_document, revision) = BlueprintRevisionDocument::from_json(document.value())
                .map_err(|cause| {
                    error::corruption(format!("stored revision failed verification: {cause}"))
                })?;
            if RevisionSummary::from(&revision) != summary {
                return Err(error::corruption(
                    "revision digest summary disagrees with its authoritative revision",
                ));
            }
            Ok(())
        },
    )
}
