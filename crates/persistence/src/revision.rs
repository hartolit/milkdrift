use milkdrift_blueprint::{BlueprintRevision, ContentDigest, RevisionId, WorkflowId};

use crate::{PageSize, PersistenceError};

/// Outcome of inserting immutable revision bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImmutableRevisionPut {
    /// The exact revision was inserted after ancestry validation.
    Inserted,
    /// Byte-identical verified revision content already existed.
    AlreadyPresent,
}

/// Small immutable revision lookup/index record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionSummary {
    /// Exact revision identity.
    pub revision: RevisionId,
    /// Workflow lineage owning the revision.
    pub workflow: WorkflowId,
    /// User-facing lineage sequence from the blueprint document.
    pub lineage_sequence: u64,
    /// Semantic content digest.
    pub content_digest: ContentDigest,
    /// Exact parent identities.
    pub parents: Vec<RevisionId>,
}

impl From<&BlueprintRevision> for RevisionSummary {
    fn from(revision: &BlueprintRevision) -> Self {
        Self {
            revision: revision.id().clone(),
            workflow: revision.semantic().workflow().clone(),
            lineage_sequence: revision.sequence(),
            content_digest: revision.content_digest().clone(),
            parents: revision.parents().to_vec(),
        }
    }
}

/// Narrow immutable workflow-revision store.
pub trait RevisionStore: Send + Sync {
    /// Stores one verified immutable revision.
    ///
    /// This operation is atomic. Unless an exact identity is already present with
    /// byte-identical canonical content, every parent must already exist, belong to
    /// the same workflow, and have a strictly lower lineage sequence. A reused
    /// identity with different bytes is [`PersistenceError::ImmutableConflict`].
    fn put_revision(
        &self,
        revision: &BlueprintRevision,
    ) -> Result<ImmutableRevisionPut, PersistenceError>;

    /// Reads and integrity-verifies one revision, returning absence distinctly.
    fn revision(
        &self,
        revision: &RevisionId,
    ) -> Result<Option<BlueprintRevision>, PersistenceError>;

    /// Reads the small immutable summary used for pin/ancestry validation.
    fn revision_summary(
        &self,
        revision: &RevisionId,
    ) -> Result<Option<RevisionSummary>, PersistenceError>;

    /// Finds all revisions sharing exact semantic content, bounded by the caller.
    fn revisions_by_content(
        &self,
        digest: &ContentDigest,
        limit: PageSize,
    ) -> Result<Vec<RevisionSummary>, PersistenceError>;
}
