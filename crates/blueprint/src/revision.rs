use serde::Serialize;

use crate::{
    AuthorRef, ContentDigest, MutationBatch, MutationError, RevisionId, SemanticBlueprint,
    WorkflowId, mutation::apply_batch,
};

const MAX_REASON_BYTES: usize = 2_048;

/// Immutable validated workflow revision.
///
/// Fields are private and no mutable accessor exists:
///
/// ```compile_fail
/// # use milkdrift_blueprint::BlueprintRevision;
/// fn corrupt(revision: &mut BlueprintRevision) {
///     revision.sequence = 99;
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct BlueprintRevision {
    id: RevisionId,
    sequence: u64,
    content_digest: ContentDigest,
    parents: Vec<RevisionId>,
    author: AuthorRef,
    reason: String,
    semantic: SemanticBlueprint,
}

impl BlueprintRevision {
    /// Atomically creates the first immutable revision through a validated mutation batch.
    pub fn genesis(
        workflow: WorkflowId,
        batch: MutationBatch,
        author: AuthorRef,
        reason: impl Into<String>,
    ) -> Result<Self, MutationError> {
        let empty = SemanticBlueprint::empty(workflow)
            .map_err(|error| MutationError::InvalidRevision(error.to_string()))?;
        let candidate = apply_batch(&empty, &batch)?;
        if candidate.merge_parents.is_some() {
            return Err(MutationError::InvalidRevision(
                "a genesis revision cannot declare merge parents".to_owned(),
            ));
        }
        Self::publish(candidate.semantic, 1, Vec::new(), author, reason.into())
    }

    /// Applies a batch to exactly the expected base and publishes a new immutable revision.
    pub fn revise(
        &self,
        expected_base: &RevisionId,
        batch: MutationBatch,
        author: AuthorRef,
        reason: impl Into<String>,
    ) -> Result<Self, MutationError> {
        if expected_base != &self.id {
            return Err(MutationError::BaseRevisionConflict {
                expected: expected_base.clone(),
                actual: self.id.clone(),
            });
        }
        let candidate = apply_batch(&self.semantic, &batch)?;
        let mut parents = candidate
            .merge_parents
            .unwrap_or_else(|| vec![self.id.clone()]);
        if !parents.contains(&self.id) {
            return Err(MutationError::InvalidRevision(
                "explicit merge parents must contain the exact base revision".to_owned(),
            ));
        }
        parents.sort();
        parents.dedup();
        Self::publish(
            candidate.semantic,
            self.sequence
                .checked_add(1)
                .ok_or_else(|| MutationError::InvalidRevision("sequence overflow".to_owned()))?,
            parents,
            author,
            reason.into(),
        )
    }

    fn publish(
        semantic: SemanticBlueprint,
        sequence: u64,
        parents: Vec<RevisionId>,
        author: AuthorRef,
        reason: String,
    ) -> Result<Self, MutationError> {
        validate_revision_metadata(sequence, &parents, &reason)?;
        let content_digest = calculate_content_digest(&semantic)?;
        let id = calculate_revision_id(sequence, &content_digest, &parents, &author, &reason)?;
        Ok(Self {
            id,
            sequence,
            content_digest,
            parents,
            author,
            reason,
            semantic,
        })
    }

    pub(crate) fn from_verified_parts(
        id: RevisionId,
        sequence: u64,
        content_digest: ContentDigest,
        parents: Vec<RevisionId>,
        author: AuthorRef,
        reason: String,
        semantic: SemanticBlueprint,
    ) -> Result<Self, MutationError> {
        validate_revision_metadata(sequence, &parents, &reason)?;
        crate::validation::validate_semantic(&semantic)?;
        let actual_content = calculate_content_digest(&semantic)?;
        if actual_content != content_digest {
            return Err(MutationError::InvalidRevision(
                "semantic content digest does not match the document".to_owned(),
            ));
        }
        let actual_id =
            calculate_revision_id(sequence, &content_digest, &parents, &author, &reason)?;
        if actual_id != id {
            return Err(MutationError::InvalidRevision(
                "revision identity does not match its derived fields".to_owned(),
            ));
        }
        Ok(Self {
            id,
            sequence,
            content_digest,
            parents,
            author,
            reason,
            semantic,
        })
    }

    /// Exact integrity identity for this revision and its lineage metadata.
    #[must_use]
    pub const fn id(&self) -> &RevisionId {
        &self.id
    }

    /// User-facing monotonic sequence along the selected base lineage.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Semantic content identity, independent of author, reason, lineage, time, and layout.
    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    /// Exact immutable parent revision references.
    #[must_use]
    pub fn parents(&self) -> &[RevisionId] {
        &self.parents
    }

    /// Bounded author provenance reference; this is not an authority grant.
    #[must_use]
    pub const fn author(&self) -> &AuthorRef {
        &self.author
    }

    /// Bounded reason supplied for the atomic change.
    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Immutable validated semantic content.
    #[must_use]
    pub const fn semantic(&self) -> &SemanticBlueprint {
        &self.semantic
    }
}

fn validate_revision_metadata(
    sequence: u64,
    parents: &[RevisionId],
    reason: &str,
) -> Result<(), MutationError> {
    if sequence == 0 {
        return Err(MutationError::InvalidRevision(
            "revision sequence must be nonzero".to_owned(),
        ));
    }
    if reason.is_empty() || reason.len() > MAX_REASON_BYTES {
        return Err(MutationError::InvalidRevision(format!(
            "reason must contain 1..={MAX_REASON_BYTES} bytes"
        )));
    }
    if sequence == 1 && !parents.is_empty() {
        return Err(MutationError::InvalidRevision(
            "genesis revision cannot have parents".to_owned(),
        ));
    }
    if sequence > 1 && parents.is_empty() {
        return Err(MutationError::InvalidRevision(
            "non-genesis revision requires at least one parent".to_owned(),
        ));
    }
    if !parents.windows(2).all(|window| window[0] < window[1]) {
        return Err(MutationError::InvalidRevision(
            "parent identities must be distinct and canonically sorted".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn calculate_content_digest(
    semantic: &SemanticBlueprint,
) -> Result<ContentDigest, MutationError> {
    let bytes = crate::document::canonical_value_bytes(semantic)
        .map_err(|error| MutationError::Serialization(error.to_string()))?;
    Ok(ContentDigest::from_hash(blake3::hash(&bytes)))
}

#[derive(Serialize)]
struct RevisionIdentityInput<'a> {
    sequence: u64,
    content_digest: &'a ContentDigest,
    parents: &'a [RevisionId],
    author: &'a AuthorRef,
    reason: &'a str,
}

pub(crate) fn calculate_revision_id(
    sequence: u64,
    content_digest: &ContentDigest,
    parents: &[RevisionId],
    author: &AuthorRef,
    reason: &str,
) -> Result<RevisionId, MutationError> {
    let input = RevisionIdentityInput {
        sequence,
        content_digest,
        parents,
        author,
        reason,
    };
    let bytes = crate::document::canonical_value_bytes(&input)
        .map_err(|error| MutationError::Serialization(error.to_string()))?;
    Ok(RevisionId::from_hash(blake3::hash(&bytes)))
}
