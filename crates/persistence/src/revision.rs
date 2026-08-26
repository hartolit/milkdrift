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

/// Stable revision-list filter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionFilter {
    /// Optional exact workflow lineage.
    pub workflow: Option<WorkflowId>,
}

/// Exclusive physical resume point bound to one exact revision filter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionCursor {
    after_revision: RevisionId,
    filter: RevisionFilter,
}

impl RevisionCursor {
    /// Constructs an exclusive continuation for one exact filter.
    #[must_use]
    pub const fn new(after_revision: RevisionId, filter: RevisionFilter) -> Self {
        Self {
            after_revision,
            filter,
        }
    }

    /// Last physically scanned revision identity.
    #[must_use]
    pub const fn after_revision(&self) -> &RevisionId {
        &self.after_revision
    }

    /// Whether this continuation belongs to the supplied filter.
    #[must_use]
    pub fn matches(&self, filter: &RevisionFilter) -> bool {
        &self.filter == filter
    }
}

/// Bounded revision-list query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionPageQuery {
    /// Exact stable filter.
    pub filter: RevisionFilter,
    /// Optional exclusive continuation.
    pub cursor: Option<RevisionCursor>,
    /// Maximum physical rows scanned and returned.
    pub limit: PageSize,
}

/// Bounded stable revision page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionPage {
    /// Matching revision summaries.
    pub revisions: Vec<RevisionSummary>,
    /// Advancing continuation, absent when fewer than the scan limit existed.
    pub next: Option<RevisionCursor>,
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

    /// Lists a bounded stable identity-ordered page without scanning complete lineage history.
    fn revisions(&self, query: &RevisionPageQuery) -> Result<RevisionPage, PersistenceError>;
}
