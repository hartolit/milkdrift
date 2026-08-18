use milkdrift_workspace::{
    ArtifactId, ArtifactMetadata, ArtifactReference, ArtifactSensitivity, RunId, WorkspaceBudget,
    WorkspaceUsage,
};

use crate::{
    ActorRef, ArtifactPublicationId, EvidenceId, PageSize, PersistenceError, TimestampMillis,
    bounded::MAX_ARTIFACT_CHUNK_BYTES,
};

/// Request to begin one bounded, content-addressed artifact publication.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct BeginArtifactPublication {
    /// Idempotent publication-session identity.
    pub publication: ArtifactPublicationId,
    /// Workspace accounting domain.
    pub run: RunId,
    /// Complete expected digest/size/media/sensitivity/retention/provenance.
    pub metadata: ArtifactMetadata,
    /// Immutable workspace limits.
    pub budget: WorkspaceBudget,
    /// Exact durable usage before charging this logical artifact record.
    pub expected_usage: WorkspaceUsage,
    /// Exact usage after charging metadata/content once.
    pub resulting_usage: WorkspaceUsage,
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
        })
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
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ArtifactReadRequest {
    /// Exact content identity and verification facts.
    pub reference: ArtifactReference,
    /// Zero-based byte offset.
    pub offset: u64,
    /// Maximum bytes to return, from 1 through one MiB.
    pub maximum_bytes: u32,
    /// Default-deny access proof.
    pub authority: ArtifactReadAuthority,
}

impl ArtifactReadRequest {
    /// Validates a bounded read request.
    pub fn new(
        reference: ArtifactReference,
        offset: u64,
        maximum_bytes: u32,
        authority: ArtifactReadAuthority,
    ) -> Result<Self, PersistenceError> {
        if maximum_bytes == 0
            || usize::try_from(maximum_bytes).map_or(true, |value| value > MAX_ARTIFACT_CHUNK_BYTES)
        {
            return Err(PersistenceError::Bounds {
                location: "artifact.read.maximum_bytes",
                reason: format!("must be between 1 and {MAX_ARTIFACT_CHUNK_BYTES}"),
            });
        }
        Ok(Self {
            reference,
            offset,
            maximum_bytes,
            authority,
        })
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

/// Bounded request for safe orphan cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrphanCleanupRequest {
    /// Boundary-clock observation used for age/retention decisions.
    pub observed_at: TimestampMillis,
    /// Delete temporary/unreferenced content only when older than this timestamp.
    pub created_before: TimestampMillis,
    /// Maximum candidates examined/deleted in one call.
    pub limit: PageSize,
}

/// Report from safe artifact cleanup.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OrphanCleanupResult {
    /// Temporary publication streams removed.
    pub temporary_publications_removed: u32,
    /// Unreferenced content blobs removed after retention checks.
    pub unreferenced_blobs_removed: u32,
    /// Bytes reclaimed.
    pub bytes_reclaimed: u64,
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
