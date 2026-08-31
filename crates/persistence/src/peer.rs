use milkdrift_authority::{
    ActorRef, AuthorityDecisionSnapshot, DecisionId, GrantDigest, GrantId, PeerId, PolicyId,
};
use milkdrift_capability::{
    ArtifactReference, CapabilityId, IdempotencyBehavior, OperationId, SideEffectClass,
};
use milkdrift_peer_protocol::{
    DelegationRef, PeerCancellationAcknowledgement, PeerCancellationRequest, PeerExecutionId,
    PeerExecutionProvenance, PeerInvocationRequest, PeerObservation, PeerRequestId,
};
use serde::{Deserialize, Serialize};

use crate::{PageSize, PersistenceError, TimestampMillis, WorkerId};

/// Current checksummed hot peer-execution primary-record schema.
pub const PEER_EXECUTION_RECORD_SCHEMA_VERSION_V2: u32 = 2;
/// Current compact immutable archived peer-execution tombstone schema.
pub const PEER_EXECUTION_TOMBSTONE_SCHEMA_VERSION_V1: u32 = 1;

/// Durable relationship facts consulted inside the acceptance transaction.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerRelationshipState {
    /// Authenticated remote identity.
    pub peer: PeerId,
    /// Exact revocation/configuration generation.
    pub generation: u64,
    /// Whether this generation may accept new work.
    pub enabled: bool,
    /// Hard relationship expiry.
    pub expires_at_unix_ms: u64,
    /// Per-peer accepted nonterminal ceiling.
    pub maximum_active: u32,
}

/// Latest exact catalog generation eligible for new acceptance.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerCatalogState {
    /// Relationship owner.
    pub peer: PeerId,
    /// Relationship generation that authorized publication.
    pub relationship_generation: u64,
    /// Exact catalog generation.
    pub generation: u64,
    /// Canonical catalog digest.
    pub digest: String,
    /// Hard catalog expiry.
    pub expires_at_unix_ms: u64,
}

/// One atomic dispatch ownership lease.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerDispatchClaim {
    /// Fixed worker identity in the current daemon boot.
    pub worker: WorkerId,
    /// Monotonic claim generation for this execution.
    pub generation: u64,
    /// Durable claim boundary.
    pub claimed_at_unix_ms: u64,
    /// Claim lease expiry.
    pub lease_expires_at_unix_ms: u64,
}

/// Evidence that dispatch crossed the local adapter-entry boundary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerEntryEvidence {
    /// Worker owning the claim at entry.
    pub worker: WorkerId,
    /// Exact claim generation used for the CAS.
    pub claim_generation: u64,
    /// Durable boundary immediately before local adapter invocation.
    pub entered_at_unix_ms: u64,
    /// Fresh authority decision made against the exact adapter requirements at entry.
    pub authority: AuthorityDecisionSnapshot,
}

/// Latest durable cancellation request and its separately durable acknowledgement.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerCancellationRecord {
    /// Exact idempotent cancellation request.
    pub request: PeerCancellationRequest,
    /// Request persistence boundary.
    pub requested_at_unix_ms: u64,
    /// Acknowledgement, when the adapter/service supplied one.
    pub acknowledgement: Option<PeerCancellationAcknowledgement>,
    /// Acknowledgement persistence boundary.
    pub acknowledged_at_unix_ms: Option<u64>,
}

/// Dispatch/entry/outcome state; durable acceptance is represented by existence of the record.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum PeerExecutionPhase {
    /// Accepted work is indexed in the durable dispatch queue.
    DispatchAvailable {
        /// Time this generation became available.
        available_at_unix_ms: u64,
    },
    /// A fixed worker owns a pre-entry lease.
    DispatchClaimed {
        /// Exact claim.
        claim: PeerDispatchClaim,
    },
    /// Adapter entry is durable; automatic replacement invocation is forbidden.
    Entered {
        /// Claim retained until terminal/uncertain release.
        claim: PeerDispatchClaim,
        /// Exact entry evidence.
        evidence: PeerEntryEvidence,
    },
    /// A durable cancellation request exists; connection closure is not an acknowledgement.
    CancellationRequested {
        /// Prior claim when dispatch had already acquired ownership.
        claim: Option<PeerDispatchClaim>,
        /// Entry evidence when cancellation arrived after adapter entry.
        evidence: Option<PeerEntryEvidence>,
    },
    /// One terminal observation is durable.
    Terminal {
        /// Terminal observation sequence.
        sequence: u64,
        /// Terminal persistence boundary.
        terminal_at_unix_ms: u64,
    },
    /// Entry is known but terminal evidence was lost across a failure boundary.
    Uncertain {
        /// Uncertainty persistence boundary.
        uncertain_at_unix_ms: u64,
        /// Bounded redacted reason.
        reason: String,
    },
}

impl PeerExecutionPhase {
    /// Whether this execution still owns durable admission capacity.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self,
            Self::DispatchAvailable { .. }
                | Self::DispatchClaimed { .. }
                | Self::Entered { .. }
                | Self::CancellationRequested { .. }
        )
    }

    /// Returns the current claim when one exists.
    #[must_use]
    pub const fn claim(&self) -> Option<&PeerDispatchClaim> {
        match self {
            Self::DispatchClaimed { claim } | Self::Entered { claim, .. } => Some(claim),
            Self::CancellationRequested { claim, .. } => claim.as_ref(),
            Self::DispatchAvailable { .. } | Self::Terminal { .. } | Self::Uncertain { .. } => None,
        }
    }

    /// Returns durable entry evidence when adapter entry is known.
    #[must_use]
    pub const fn entry_evidence(&self) -> Option<&PeerEntryEvidence> {
        match self {
            Self::Entered { evidence, .. } => Some(evidence),
            Self::CancellationRequested { evidence, .. } => evidence.as_ref(),
            Self::DispatchAvailable { .. }
            | Self::DispatchClaimed { .. }
            | Self::Terminal { .. }
            | Self::Uncertain { .. } => None,
        }
    }
}

/// Fixed-size durable accounting retained on the primary record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionAccounting {
    /// Number of append-only observation rows.
    pub observations: u32,
    /// Sum of exact output artifact sizes observed so far.
    pub artifact_bytes: u64,
    /// Terminal duration when reported.
    pub duration_ms: Option<u64>,
    /// Terminal cost when reported.
    pub cost_micros: Option<u64>,
}

/// Serving peer's durable accepted-execution primary record.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionRecord {
    /// Record schema.
    pub schema_version: u32,
    /// Authenticated submitting peer.
    pub owner_peer: PeerId,
    /// Relationship generation used by the atomic admission decision.
    pub relationship_generation: u64,
    /// Exact canonical accepted request and capability generation.
    pub request: PeerInvocationRequest,
    /// Exact allowed authority decision.
    pub authority: AuthorityDecisionSnapshot,
    /// Stable remote execution identity.
    pub execution: PeerExecutionId,
    /// Monotonic global acceptance sequence.
    pub acceptance_sequence: u64,
    /// Durable acceptance boundary.
    pub accepted_at_unix_ms: u64,
    /// Current phase.
    pub phase: PeerExecutionPhase,
    /// Latest cancellation facts, orthogonal to connection state.
    pub cancellation: Option<PeerCancellationRecord>,
    /// Highest contiguous observation sequence.
    pub last_observation_sequence: u64,
    /// Bounded usage totals.
    pub accounting: PeerExecutionAccounting,
    /// Domain-separated rolling digest of every contiguous observation document.
    pub observation_digest: String,
    /// Optimistic primary-record revision.
    pub revision: u64,
}

/// Immutable authority identity retained after the complete accepted decision is compacted.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerAcceptedAuthoritySummary {
    /// Stable decision identity.
    pub decision: DecisionId,
    /// Authenticated initiating actor retained from the accepted request.
    pub actor: ActorRef,
    /// Exact immutable grant lineage.
    pub grant: GrantId,
    /// Exact grant revision.
    pub grant_revision: u64,
    /// Exact grant digest.
    pub grant_digest: GrantDigest,
    /// Revocation generation observed at acceptance.
    pub revocation_generation: u64,
    /// Exact policy identity.
    pub policy: PolicyId,
    /// Exact policy revision.
    pub policy_version: u32,
    /// Canonical accepted decision digest.
    pub decision_digest: String,
}

/// Final immutable disposition retained by an archived execution.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum PeerArchivedDisposition {
    /// The final terminal observation is retained as the compact terminal summary.
    Terminal {
        /// Exact final terminal observation; artifact references contain no artifact bytes.
        observation: Box<PeerObservation>,
    },
    /// Adapter entry is known but no terminal observation can be proved.
    Uncertain {
        /// Durable uncertainty boundary.
        uncertain_at_unix_ms: u64,
        /// Bounded redacted reason.
        reason: String,
    },
}

impl PeerArchivedDisposition {
    /// Stable public lifecycle status represented by the compact summary.
    #[must_use]
    pub const fn is_uncertain(&self) -> bool {
        matches!(self, Self::Uncertain { .. })
    }

    /// Retained final terminal observation when one exists.
    #[must_use]
    pub const fn terminal_observation(&self) -> Option<&PeerObservation> {
        match self {
            Self::Terminal { observation } => Some(observation),
            Self::Uncertain { .. } => None,
        }
    }
}

/// Compact immutable authority for an archived accepted peer request.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionTombstone {
    /// Tombstone document schema.
    pub schema_version: u32,
    /// Authenticated submitting peer.
    pub owner_peer: PeerId,
    /// Serving peer targeted by the accepted delegation.
    pub target_peer: PeerId,
    /// Opaque non-secret delegation provenance identity.
    pub delegation_ref: DelegationRef,
    /// Relationship generation used by admission.
    pub relationship_generation: u64,
    /// Exact accepted request identity.
    pub request_id: PeerRequestId,
    /// Canonical accepted request digest used for permanent conflict detection.
    pub request_digest: String,
    /// Stable serving execution identity.
    pub execution: PeerExecutionId,
    /// Monotonic global acceptance sequence.
    pub acceptance_sequence: u64,
    /// Durable acceptance boundary.
    pub accepted_at_unix_ms: u64,
    /// Accepted catalog generation.
    pub catalog_generation: u64,
    /// Accepted catalog digest.
    pub catalog_digest: String,
    /// Generation-pinned capability identity.
    pub capability: CapabilityId,
    /// Immutable capability descriptor revision.
    pub capability_generation: u64,
    /// Canonical selected capability snapshot digest.
    pub capability_digest: String,
    /// Exact selected operation.
    pub operation: OperationId,
    /// Accepted side-effect classification.
    pub side_effect: SideEffectClass,
    /// Accepted capability idempotency classification.
    pub idempotency: IdempotencyBehavior,
    /// Compact immutable accepted authority identity.
    pub authority: PeerAcceptedAuthoritySummary,
    /// Originating daemon workflow/run provenance.
    pub provenance: PeerExecutionProvenance,
    /// Final terminal or uncertain disposition.
    pub disposition: PeerArchivedDisposition,
    /// Latest cancellation request/acknowledgement facts, when present.
    pub cancellation: Option<PeerCancellationRecord>,
    /// Highest contiguous observation sequence before compaction.
    pub last_observation_sequence: u64,
    /// Domain-separated rolling digest of the compacted observation history.
    pub observation_digest: String,
    /// Fixed-size final accounting summary.
    pub accounting: PeerExecutionAccounting,
    /// All detailed observation rows through this sequence were compacted.
    pub compacted_through_sequence: u64,
    /// Atomic archive/compaction boundary.
    pub archived_at_unix_ms: u64,
}

/// Singular authoritative placement returned by idempotency and execution lookup.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerExecutionSnapshot {
    /// Active or hot-terminal complete record with detailed observations.
    Hot(Box<PeerExecutionRecord>),
    /// Compact immutable archived identity and outcome summary.
    Archived(Box<PeerExecutionTombstone>),
}

impl PeerExecutionSnapshot {
    /// Authenticated owner.
    #[must_use]
    pub const fn owner_peer(&self) -> &PeerId {
        match self {
            Self::Hot(record) => &record.owner_peer,
            Self::Archived(tombstone) => &tombstone.owner_peer,
        }
    }

    /// Accepted request identity.
    #[must_use]
    pub const fn request_id(&self) -> &PeerRequestId {
        match self {
            Self::Hot(record) => &record.request.request_id,
            Self::Archived(tombstone) => &tombstone.request_id,
        }
    }

    /// Canonical request digest.
    #[must_use]
    pub fn request_digest(&self) -> &str {
        match self {
            Self::Hot(record) => &record.request.request_digest,
            Self::Archived(tombstone) => &tombstone.request_digest,
        }
    }

    /// Stable serving execution identity.
    #[must_use]
    pub const fn execution(&self) -> &PeerExecutionId {
        match self {
            Self::Hot(record) => &record.execution,
            Self::Archived(tombstone) => &tombstone.execution,
        }
    }

    /// Highest durable semantic observation sequence.
    #[must_use]
    pub const fn last_observation_sequence(&self) -> u64 {
        match self {
            Self::Hot(record) => record.last_observation_sequence,
            Self::Archived(tombstone) => tombstone.last_observation_sequence,
        }
    }
}

/// Complete facts needed by one atomic acceptance transaction.
pub struct PeerAdmission<'a> {
    /// Authenticated owner.
    pub owner_peer: &'a PeerId,
    /// Exact durable request.
    pub request: &'a PeerInvocationRequest,
    /// Exact authority decision.
    pub authority: &'a AuthorityDecisionSnapshot,
    /// Stable deterministic execution identity.
    pub execution: &'a PeerExecutionId,
    /// Expected relationship generation.
    pub relationship_generation: u64,
    /// Acceptance boundary time.
    pub accepted_at_unix_ms: u64,
    /// Global active ceiling.
    pub maximum_global_active: u32,
    /// Durable available/claimed queue ceiling.
    pub maximum_dispatch_queue: u32,
    /// Maximum complete terminal/uncertain records retained with hot observations.
    pub maximum_hot_terminal_records: u64,
    /// Maximum oldest eligible hot records compacted inside this acceptance transaction.
    pub archive_batch_size: u32,
    /// Oldest terminal boundary eligible for automatic compaction.
    pub archive_terminal_before_or_at_unix_ms: u64,
}

/// Stable pre-acceptance capacity/authorization rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerAdmissionRejection {
    /// Startup recovery or lifecycle draining has closed durable admission.
    AdmissionClosed,
    /// Relationship is absent, disabled, stale, or expired.
    RelationshipUnavailable,
    /// Catalog generation/digest is absent, stale, or expired.
    CatalogUnavailable,
    /// Per-peer active ceiling is full.
    PeerCapacity,
    /// Global active ceiling is full.
    GlobalCapacity,
    /// Durable dispatch queue is full.
    DispatchCapacity,
    /// Hot history plus reserved active completions is full and no eligible page was reclaimable.
    RetentionCapacity,
}

/// Atomic adapter-entry outcome after lifecycle and relationship revalidation.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerEntryOutcome {
    /// The exact claim crossed the durable adapter-entry boundary.
    Entered(Box<PeerExecutionRecord>),
    /// Durable lifecycle admission closed before entry; the claim remains pre-entry.
    AdmissionClosed,
    /// The authorizing relationship generation is no longer eligible; the claim remains pre-entry.
    RelationshipUnavailable,
}

/// Complete facts needed by one atomic adapter-entry transaction.
pub struct PeerEntryRequest<'a> {
    /// Authenticated execution owner.
    pub owner: &'a PeerId,
    /// Stable remote execution identity.
    pub execution: &'a PeerExecutionId,
    /// Worker holding the exact durable claim.
    pub worker: &'a WorkerId,
    /// Exact claim generation.
    pub claim_generation: u64,
    /// Relationship generation authorized immediately before entry.
    pub relationship_generation: u64,
    /// Nonzero adapter-entry boundary time.
    pub entered_at_unix_ms: u64,
    /// Fresh exact authority decision for the complete adapter requirements.
    pub authority: &'a AuthorityDecisionSnapshot,
}

/// Atomic idempotent admission outcome.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerAdmissionOutcome {
    /// This transaction accepted a new identity and made it dispatchable.
    Accepted(Box<PeerExecutionRecord>),
    /// Exact idempotent replay.
    Replayed(PeerExecutionSnapshot),
    /// Same key, different canonical request.
    Conflict(PeerExecutionSnapshot),
    /// Rejected without creating an execution.
    Rejected(PeerAdmissionRejection),
}

/// Request for one atomic oldest-available claim.
pub struct PeerDispatchClaimRequest<'a> {
    /// Worker requesting ownership.
    pub worker: &'a WorkerId,
    /// Claim boundary.
    pub claimed_at_unix_ms: u64,
    /// Claim lease expiry.
    pub lease_expires_at_unix_ms: u64,
}

/// Atomic claim result.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerClaimOutcome {
    /// No durable work is available.
    Empty,
    /// Exact execution and claim were acquired.
    Claimed(PeerExecutionRecord),
    /// A pre-entry cancellation was claimed for durable terminalization, never adapter entry.
    CancellationRequested(PeerExecutionRecord),
}

/// Idempotent append result.
#[derive(Clone, Debug, PartialEq)]
pub enum PeerObservationAppend {
    /// A new append-only row committed.
    Appended(PeerExecutionRecord),
    /// The exact already-committed sequence was replayed.
    Replayed(PeerExecutionRecord),
}

/// Bounded stable contiguous observation page.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerObservationPage {
    /// Observed authoritative hot record or archived tombstone.
    pub execution: PeerExecutionSnapshot,
    /// Rows strictly after the requested sequence.
    pub observations: Vec<PeerObservation>,
}

/// Redacted peer execution lifecycle/accounting health.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PeerExecutionStatus {
    /// Accepted nonterminal records.
    pub active: u32,
    /// Active records still consuming the pre-entry dispatch queue.
    pub dispatch_queued: u32,
    /// Complete terminal/uncertain records with hot detailed observations.
    pub hot_terminal: u64,
    /// Compact immutable archived identities.
    pub tombstones: u64,
    /// Monotonic successful archival generation.
    pub archive_generation: u64,
    /// Most recent successful archival boundary.
    pub last_archived_at_unix_ms: Option<u64>,
}

/// Bounded startup recovery report.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PeerRecoveryResult {
    /// Pre-entry claims returned to dispatch availability.
    pub requeued: u32,
    /// Entered work converted to outcome uncertainty.
    pub uncertain: u32,
    /// Whether another bounded page remains.
    pub more: bool,
}

/// Explicit terminal archival request.
pub struct PeerRetentionRequest {
    /// Archive only terminal/uncertain records at or before this boundary.
    pub terminal_before_or_at: TimestampMillis,
    /// Boundary recorded on newly archived rows.
    pub archived_at: TimestampMillis,
    /// Bounded number of records.
    pub limit: PageSize,
}

/// Bounded archival result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PeerRetentionPage {
    /// Records newly marked archived.
    pub archived: u32,
    /// Whether another eligible page remains.
    pub more: bool,
}

/// Narrow serving-peer execution persistence port.
///
/// Implementations own atomic admission/accounting, dispatch indexes, append-only
/// observations, recovery and explicit retention. They expose no database transaction types.
pub trait PeerExecutionStore: Send + Sync {
    /// Opens or closes the durable admission/entry gate through one serialized transaction.
    fn set_peer_admission_open(&self, open: bool) -> Result<(), PersistenceError>;

    /// Records or replaces a relationship only at a strictly newer generation; exact replay is safe.
    fn configure_peer_relationship(
        &self,
        relationship: &PeerRelationshipState,
    ) -> Result<(), PersistenceError>;

    /// Records the exact currently eligible catalog generation.
    fn publish_peer_catalog(&self, catalog: &PeerCatalogState) -> Result<(), PersistenceError>;

    /// Reads the latest durable catalog generation for restart-safe monotonic publication.
    fn peer_catalog(&self, peer: &PeerId) -> Result<Option<PeerCatalogState>, PersistenceError>;

    /// Atomically checks idempotency, relationship/catalog generations and every capacity counter.
    fn admit_peer_execution(
        &self,
        admission: &PeerAdmission<'_>,
    ) -> Result<PeerAdmissionOutcome, PersistenceError>;

    /// Indexed lookup by authenticated owner and idempotency key.
    fn peer_execution_by_request(
        &self,
        owner: &PeerId,
        request: &PeerRequestId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError>;

    /// Indexed lookup by execution identity with owner cross-check.
    fn peer_execution(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
    ) -> Result<Option<PeerExecutionSnapshot>, PersistenceError>;

    /// Returns bounded redacted active/hot/tombstone accounting.
    fn peer_execution_status(&self) -> Result<PeerExecutionStatus, PersistenceError>;

    /// Verifies counters and every peer ownership/index family without retaining a record catalog.
    fn verify_peer_execution_integrity(&self) -> Result<(), PersistenceError>;

    /// Reads a bounded contiguous page without materializing retained history.
    fn peer_observations(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        after_sequence: u64,
        limit: PageSize,
    ) -> Result<PeerObservationPage, PersistenceError>;

    /// Claims the oldest durable available dispatch through one transaction.
    fn claim_peer_dispatch(
        &self,
        request: &PeerDispatchClaimRequest<'_>,
    ) -> Result<PeerClaimOutcome, PersistenceError>;

    /// Distinct CAS immediately before adapter invocation.
    fn mark_peer_entered(
        &self,
        request: &PeerEntryRequest<'_>,
    ) -> Result<PeerEntryOutcome, PersistenceError>;

    /// Releases only an exact pre-entry claim back to the durable queue.
    fn release_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        available_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError>;

    /// Extends only the exact current claim lease.
    fn extend_peer_claim(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        lease_expires_at_unix_ms: u64,
    ) -> Result<(), PersistenceError>;

    /// Persists uncertainty after known entry without fabricating terminal evidence.
    fn mark_peer_uncertain(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        worker: &WorkerId,
        claim_generation: u64,
        uncertain_at_unix_ms: u64,
        reason: &str,
    ) -> Result<PeerExecutionRecord, PersistenceError>;

    /// Appends one next semantic observation, with exact replay idempotency.
    fn append_peer_observation(
        &self,
        owner: &PeerId,
        execution: &PeerExecutionId,
        observation: &PeerObservation,
    ) -> Result<PeerObservationAppend, PersistenceError>;

    /// Persists a cancellation request before adapter interaction.
    fn request_peer_cancellation(
        &self,
        owner: &PeerId,
        request: &PeerCancellationRequest,
        requested_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError>;

    /// Persists a separate acknowledgement through exact request matching.
    fn acknowledge_peer_cancellation(
        &self,
        owner: &PeerId,
        acknowledgement: &PeerCancellationAcknowledgement,
        acknowledged_at_unix_ms: u64,
    ) -> Result<PeerExecutionRecord, PersistenceError>;

    /// Recovers one bounded page of claims from a previous daemon owner.
    fn recover_peer_claims(
        &self,
        recovered_at_unix_ms: u64,
        limit: PageSize,
    ) -> Result<PeerRecoveryResult, PersistenceError>;

    /// Marks one bounded terminal page archived without deleting idempotency/provenance facts.
    fn archive_peer_executions(
        &self,
        request: &PeerRetentionRequest,
    ) -> Result<PeerRetentionPage, PersistenceError>;

    /// Returns an artifact reference durably indexed from a specific observation, when present.
    fn peer_observation_artifact(
        &self,
        execution: &PeerExecutionId,
        sequence: u64,
    ) -> Result<Option<ArtifactReference>, PersistenceError>;
}
