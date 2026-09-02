use milkdrift_authority::ActorRef;
use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, ArtifactSensitivity, RunId, WorkspaceBudget,
    WorkspaceUsage,
};

use crate::{
    ArtifactPublicationId, ControllerArtifactOwner, ControllerReservationId, EvidenceId, PageSize,
    PersistenceError, TimestampMillis, bounded::MAX_ARTIFACT_CHUNK_BYTES,
};

/// Maximum opaque key bytes retained by one resumable orphan-cleanup cursor.
pub const MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES: usize = 512;

/// Request to begin one bounded, content-addressed artifact publication.
///
/// Checked request facts cannot be rewritten after construction:
///
/// ```compile_fail
/// use milkdrift_persistence::BeginArtifactPublication;
/// let mut request: BeginArtifactPublication = todo!();
/// request.resulting_usage = request.expected_usage;
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BeginArtifactPublication {
    /// Idempotent publication-session identity.
    publication: ArtifactPublicationId,
    /// Workspace accounting domain.
    run: RunId,
    /// Complete expected digest/size/media/sensitivity/retention/provenance.
    metadata: ArtifactMetadata,
    /// Immutable workspace limits.
    budget: WorkspaceBudget,
    /// Exact durable usage before charging this logical artifact record.
    expected_usage: WorkspaceUsage,
    /// Exact usage after charging metadata/content once.
    resulting_usage: WorkspaceUsage,
    /// Explicit controller-account source used at first logical commit.
    controller_owner: ControllerArtifactOwner,
}

impl BeginArtifactPublication {
    /// Constructs a request and proves its exact accounting transition.
    pub fn new(
        publication: ArtifactPublicationId,
        run: RunId,
        metadata: ArtifactMetadata,
        budget: WorkspaceBudget,
        expected_usage: WorkspaceUsage,
    ) -> Result<Self, PersistenceError> {
        let resulting_usage = budget
            .admit_artifact(&expected_usage, &metadata)
            .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
        Ok(Self {
            publication,
            run,
            metadata,
            budget,
            expected_usage,
            resulting_usage,
            controller_owner: ControllerArtifactOwner::RunBinding,
        })
    }

    /// Constructs an invocation publication owned by its committed controller reservation.
    pub fn for_invocation(
        publication: ArtifactPublicationId,
        run: RunId,
        metadata: ArtifactMetadata,
        budget: WorkspaceBudget,
        expected_usage: WorkspaceUsage,
        reservation: ControllerReservationId,
    ) -> Result<Self, PersistenceError> {
        let mut request = Self::new(publication, run, metadata, budget, expected_usage)?;
        request.controller_owner = ControllerArtifactOwner::InvocationReservation(reservation);
        Ok(request)
    }

    /// Returns the idempotent publication-session identity.
    #[must_use]
    pub const fn publication(&self) -> &ArtifactPublicationId {
        &self.publication
    }

    /// Returns the owning workspace accounting domain.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Returns the complete immutable artifact metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ArtifactMetadata {
        &self.metadata
    }

    /// Returns the immutable workspace budget.
    #[must_use]
    pub const fn budget(&self) -> &WorkspaceBudget {
        &self.budget
    }

    /// Returns durable usage expected before publication.
    #[must_use]
    pub const fn expected_usage(&self) -> WorkspaceUsage {
        self.expected_usage
    }

    /// Returns the exact usage resulting from publication.
    #[must_use]
    pub const fn resulting_usage(&self) -> WorkspaceUsage {
        self.resulting_usage
    }

    /// Returns the explicit controller-account source for this logical publication.
    #[must_use]
    pub const fn controller_owner(&self) -> &ControllerArtifactOwner {
        &self.controller_owner
    }

    /// Revalidates the request's derived accounting transition at a trust boundary.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        self.budget
            .validate_usage(&self.expected_usage)
            .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
        let resulting_usage = self
            .budget
            .admit_artifact(&self.expected_usage, &self.metadata)
            .map_err(|error| PersistenceError::InvalidDocument(error.to_string()))?;
        if resulting_usage != self.resulting_usage {
            return Err(PersistenceError::InvalidDocument(
                "artifact resulting usage does not match its budget charge".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Result of beginning a publication session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)] // Typed immutable metadata avoids a second lossy result shape.
pub enum BeginArtifactOutcome {
    /// A temporary bounded stream is ready for sequential chunks.
    Writable,
    /// The same publication identity was redelivered and remains writable.
    Resumed {
        /// Exact next byte offset.
        next_offset: u64,
    },
    /// Exact metadata/content was already durably committed.
    AlreadyCommitted(ArtifactMetadata),
}

impl BeginArtifactOutcome {
    /// Returns the next accepted byte offset when an interrupted publication resumed.
    #[must_use]
    pub const fn next_offset(&self) -> Option<u64> {
        match self {
            Self::Resumed { next_offset } => Some(*next_offset),
            Self::Writable | Self::AlreadyCommitted(_) => None,
        }
    }

    /// Returns the immutable metadata when the publication had already committed.
    #[must_use]
    pub const fn committed_metadata(&self) -> Option<&ArtifactMetadata> {
        match self {
            Self::AlreadyCommitted(metadata) => Some(metadata),
            Self::Writable | Self::Resumed { .. } => None,
        }
    }
}

/// Progress after one sequential bounded chunk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactWriteProgress {
    /// Exact number of temporary bytes durably accepted so far.
    pub bytes_received: u64,
    /// Whether the expected exact size has been reached.
    pub complete_size: bool,
}

/// Outcome after flush/fsync, digest verification, atomic content publication, and
/// metadata/accounting commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitArtifactOutcome {
    /// This call committed the logical metadata record; bytes may have deduplicated.
    Published {
        /// Exact committed metadata.
        metadata: ArtifactMetadata,
        /// Whether content bytes already existed by digest.
        content_deduplicated: bool,
        /// Resulting workspace usage.
        usage: WorkspaceUsage,
    },
    /// An exact publication identity was redelivered after successful commit.
    Replayed {
        /// Original exact metadata.
        metadata: ArtifactMetadata,
        /// Original resulting workspace usage.
        usage: WorkspaceUsage,
    },
}

impl CommitArtifactOutcome {
    /// Returns the exact immutable metadata for either a new commit or a replay.
    #[must_use]
    pub const fn metadata(&self) -> &ArtifactMetadata {
        match self {
            Self::Published { metadata, .. } | Self::Replayed { metadata, .. } => metadata,
        }
    }

    /// Returns the exact resulting workspace usage for either outcome.
    #[must_use]
    pub const fn usage(&self) -> &WorkspaceUsage {
        match self {
            Self::Published { usage, .. } | Self::Replayed { usage, .. } => usage,
        }
    }

    /// Reports whether this call performed the first logical metadata commit.
    #[must_use]
    pub const fn was_published(&self) -> bool {
        matches!(self, Self::Published { .. })
    }

    /// Reports whether a newly published logical artifact reused existing content bytes.
    ///
    /// A replay returns `None` because it performs no content-publication decision.
    #[must_use]
    pub const fn content_deduplicated(&self) -> Option<bool> {
        match self {
            Self::Published {
                content_deduplicated,
                ..
            } => Some(*content_deduplicated),
            Self::Replayed { .. } => None,
        }
    }
}

/// Explicit access proof shape for artifact reads/exports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactReadAuthority {
    /// Only artifacts classified public may be returned.
    PublicOnly,
    /// Runtime authority explicitly approved access to sensitive content.
    Authorized {
        /// Actor receiving the content.
        actor: ActorRef,
        /// Durable authority/audit evidence reference.
        evidence: EvidenceId,
    },
}

/// One bounded random-access read request.
///
/// The per-read bound remains private after validation:
///
/// ```compile_fail
/// use milkdrift_persistence::ArtifactReadRequest;
/// let mut request: ArtifactReadRequest = todo!();
/// request.maximum_bytes = u32::MAX;
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ArtifactReadRequest {
    /// Exact content identity and verification facts.
    reference: ArtifactReference,
    /// Zero-based byte offset.
    offset: u64,
    /// Maximum bytes to return, from 1 through one MiB.
    maximum_bytes: u32,
    /// Default-deny access proof.
    authority: ArtifactReadAuthority,
}

impl ArtifactReadRequest {
    /// Validates a bounded read request.
    pub fn new(
        reference: ArtifactReference,
        offset: u64,
        maximum_bytes: u32,
        authority: ArtifactReadAuthority,
    ) -> Result<Self, PersistenceError> {
        let request = Self {
            reference,
            offset,
            maximum_bytes,
            authority,
        };
        request.validate()?;
        Ok(request)
    }

    /// Returns the exact content identity and verification facts.
    #[must_use]
    pub const fn reference(&self) -> &ArtifactReference {
        &self.reference
    }

    /// Returns the zero-based byte offset.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the maximum number of bytes to return.
    #[must_use]
    pub const fn maximum_bytes(&self) -> u32 {
        self.maximum_bytes
    }

    /// Returns the default-deny read authority.
    #[must_use]
    pub const fn authority(&self) -> &ArtifactReadAuthority {
        &self.authority
    }

    /// Revalidates all bounded read facts at an adapter trust boundary.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if self.maximum_bytes == 0
            || usize::try_from(self.maximum_bytes)
                .map_or(true, |value| value > MAX_ARTIFACT_CHUNK_BYTES)
        {
            return Err(PersistenceError::Bounds {
                location: "artifact.read.maximum_bytes",
                reason: format!("must be between 1 and {MAX_ARTIFACT_CHUNK_BYTES}"),
            });
        }
        if self.offset > self.reference.size_bytes() {
            return Err(PersistenceError::Bounds {
                location: "artifact.read.offset",
                reason: "offset is beyond exact artifact size".to_owned(),
            });
        }
        Ok(())
    }
}

/// One bounded verified artifact chunk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReadChunk {
    /// Offset represented by the first returned byte.
    pub offset: u64,
    /// Bounded bytes.
    pub bytes: Vec<u8>,
    /// True exactly when this chunk reaches the committed exact size.
    pub end_of_artifact: bool,
}

/// Closed filesystem candidate family traversed by orphan cleanup.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OrphanCleanupFamily {
    /// Writable publication sessions ordered by their durable age-index key.
    WritablePublications,
    /// Unowned temporary publication files.
    TemporaryFiles,
    /// Content-addressed blobs lacking every durable owner/reference.
    ContentFiles,
}

/// Stable exclusive resume point for one bounded orphan-cleanup cycle.
///
/// The key is adapter-defined and opaque to callers. The cursor is bound to the
/// exact age threshold so advancing that threshold cannot silently skip a file
/// that was too young on an earlier page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanCleanupCursor {
    family: OrphanCleanupFamily,
    after_key: Vec<u8>,
    created_before: TimestampMillis,
}

impl OrphanCleanupCursor {
    /// Constructs a validated exclusive cleanup resume point.
    pub fn new(
        family: OrphanCleanupFamily,
        after_key: Vec<u8>,
        created_before: TimestampMillis,
    ) -> Result<Self, PersistenceError> {
        if after_key.is_empty() || after_key.len() > MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES {
            return Err(PersistenceError::InvalidCursor(format!(
                "orphan-cleanup cursor key must contain 1..={MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES} bytes"
            )));
        }
        Ok(Self {
            family,
            after_key,
            created_before,
        })
    }

    /// Filesystem candidate family containing the exclusive resume key.
    #[must_use]
    pub const fn family(&self) -> OrphanCleanupFamily {
        self.family
    }

    /// Opaque adapter-defined exclusive resume key.
    #[must_use]
    pub fn after_key(&self) -> &[u8] {
        &self.after_key
    }

    /// Exact age threshold to which this cleanup cycle is bound.
    #[must_use]
    pub const fn created_before(&self) -> TimestampMillis {
        self.created_before
    }
}

/// Bounded request for safe orphan cleanup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanCleanupRequest {
    /// Boundary-clock observation used for age/retention decisions.
    pub observed_at: TimestampMillis,
    /// Delete temporary/unreferenced content only when older than this timestamp.
    pub created_before: TimestampMillis,
    /// Maximum candidates examined/deleted in one call.
    pub limit: PageSize,
    /// Exclusive cursor from the prior page of this exact age-threshold cycle.
    pub cursor: Option<OrphanCleanupCursor>,
}

/// Report from safe artifact cleanup.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OrphanCleanupResult {
    /// Temporary publication streams removed.
    pub temporary_publications_removed: u32,
    /// Unreferenced content blobs removed after retention checks.
    pub unreferenced_blobs_removed: u32,
    /// Bytes reclaimed.
    pub bytes_reclaimed: u64,
    /// Exclusive resume point for the next bounded page; absent when exhausted.
    pub next_cursor: Option<OrphanCleanupCursor>,
}

/// Narrow synchronous, streaming, content-addressed artifact port.
pub trait ArtifactStore: Send + Sync {
    /// Begins an idempotent sequential temporary publication and validates its intended
    /// budget transition. Usage is checked again and committed only at publication.
    fn begin_publication(
        &self,
        request: &BeginArtifactPublication,
    ) -> Result<BeginArtifactOutcome, PersistenceError>;

    /// Appends exactly at `offset`; chunks are bounded and publication cannot exceed size.
    fn write_chunk(
        &self,
        publication: &ArtifactPublicationId,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ArtifactWriteProgress, PersistenceError>;

    /// Verifies exact size/digest, durably flushes content, publishes atomically, and then
    /// commits immutable metadata/accounting. A crash before metadata commit can leave
    /// only an unreferenced blob; a committed reference can never point at missing bytes.
    fn commit_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<CommitArtifactOutcome, PersistenceError>;

    /// Aborts an uncommitted temporary stream. A committed artifact is immutable.
    fn abort_publication(
        &self,
        publication: &ArtifactPublicationId,
    ) -> Result<(), PersistenceError>;

    /// Reads immutable metadata without exposing content.
    fn metadata(&self, artifact: &ArtifactId)
    -> Result<Option<ArtifactMetadata>, PersistenceError>;

    /// Proves both immutable metadata and verified content are durably committed.
    /// Journal append uses this exact predicate for every required reference.
    fn is_committed(&self, reference: &ArtifactReference) -> Result<bool, PersistenceError>;

    /// Returns whether this run's accounting domain has already admitted the exact
    /// artifact, whether through publication or an earlier journal reference.
    fn is_referenced_by_run(
        &self,
        run: &RunId,
        reference: &ArtifactReference,
    ) -> Result<bool, PersistenceError>;

    /// Returns a bounded verified chunk. Non-public sensitivity requires `Authorized`.
    fn read_chunk(
        &self,
        request: &ArtifactReadRequest,
    ) -> Result<ArtifactReadChunk, PersistenceError>;

    /// Deletes only abandoned temporary streams and blobs that have no metadata/event/
    /// workspace references and whose retention policy permits removal.
    fn cleanup_orphans(
        &self,
        request: OrphanCleanupRequest,
    ) -> Result<OrphanCleanupResult, PersistenceError>;
}

/// Applies the default-deny sensitivity rule for an artifact read request.
pub fn authorize_artifact_read(
    sensitivity: ArtifactSensitivity,
    authority: &ArtifactReadAuthority,
) -> Result<(), PersistenceError> {
    if sensitivity.permits_unauthorized_export()
        || matches!(authority, ArtifactReadAuthority::Authorized { .. })
    {
        Ok(())
    } else {
        Err(PersistenceError::ArtifactAccessDenied(
            "explicit authority is required to read non-public artifact content".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orphan_cleanup_cursor_is_bounded_and_threshold_bound() -> Result<(), PersistenceError> {
        let threshold = TimestampMillis::new(500);
        assert!(
            OrphanCleanupCursor::new(OrphanCleanupFamily::TemporaryFiles, Vec::new(), threshold,)
                .is_err()
        );
        assert!(
            OrphanCleanupCursor::new(
                OrphanCleanupFamily::ContentFiles,
                vec![0; MAX_ORPHAN_CLEANUP_CURSOR_KEY_BYTES + 1],
                threshold,
            )
            .is_err()
        );

        let cursor = OrphanCleanupCursor::new(
            OrphanCleanupFamily::ContentFiles,
            vec![0, 0xff, b'/'],
            threshold,
        )?;
        assert_eq!(cursor.family(), OrphanCleanupFamily::ContentFiles);
        assert_eq!(cursor.after_key(), &[0, 0xff, b'/']);
        assert_eq!(cursor.created_before(), threshold);
        Ok(())
    }
}
