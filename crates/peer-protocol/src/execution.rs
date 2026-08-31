use milkdrift_authority::{ActorRef, PeerId};
use milkdrift_capability::{
    CapabilityId, InvocationEvent, InvocationRequest, InvocationRequestDocument, OperationId,
    ResolvedCapabilitySnapshot, TerminalStatus,
};
use serde::{Deserialize, Serialize};

use crate::{
    CatalogDigest, DelegationRef, PeerExecutionId, PeerProtocolError, PeerRequestId,
    identity::validate_blake3_digest,
};

const INVOCATION_DIGEST_DOMAIN: &[u8] = b"milkdrift.peer.invocation.v1\0";
const MAX_OBSERVATIONS_PER_PAGE: usize = 256;

/// Exact originating workflow coordinates carried across the peer execution boundary.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerExecutionProvenance {
    /// Originating durable run identity.
    pub run: String,
    /// Originating immutable revision identity.
    pub revision: String,
    /// Originating semantic node identity.
    pub node: String,
    /// Originating logical node-execution identity.
    pub execution: String,
    /// Originating immutable attempt identity.
    pub attempt: String,
}

impl PeerExecutionProvenance {
    fn validate(&self) -> Result<(), PeerProtocolError> {
        if [
            self.run.as_str(),
            self.revision.as_str(),
            self.node.as_str(),
            self.execution.as_str(),
            self.attempt.as_str(),
        ]
        .into_iter()
        .any(|value| !safe_reference(value))
        {
            return Err(PeerProtocolError::InvalidContract(
                "peer execution provenance contains an invalid identity".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Per-request resource ceilings checked before durable remote acceptance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    /// Maximum total input plus output artifact bytes.
    pub artifact_bytes: u64,
    /// Maximum execution duration.
    pub duration_ms: u64,
    /// Maximum observed cost in millionths, when applicable.
    pub cost_micros: u64,
    /// Maximum semantic observations retained and streamed.
    pub observations: u32,
}

impl ExecutionLimits {
    /// Requires nonzero duration and observation ceilings.
    pub fn validate(self) -> Result<Self, PeerProtocolError> {
        if self.duration_ms == 0 || self.observations == 0 || self.observations > 1_000_000 {
            return Err(PeerProtocolError::InvalidContract(
                "execution duration and observation limits must be bounded and nonzero".to_owned(),
            ));
        }
        Ok(self)
    }

    /// True when every requested ceiling is no greater than this grant ceiling.
    #[must_use]
    pub const fn contains(self, requested: Self) -> bool {
        requested.artifact_bytes <= self.artifact_bytes
            && requested.duration_ms <= self.duration_ms
            && requested.cost_micros <= self.cost_micros
            && requested.observations <= self.observations
    }
}

/// Opaque server-stored delegation reference plus immutable narrowing facts.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAuthorization {
    /// Opaque relationship-local record identity; it is not a bearer credential.
    pub reference: DelegationRef,
    /// Daemon issuing the delegation.
    pub issuer_peer: PeerId,
    /// Authorized local actor retained for audit.
    pub actor: ActorRef,
    /// Only peer allowed to consume the delegation.
    pub target_peer: PeerId,
    /// Only capability allowed.
    pub capability: CapabilityId,
    /// Only operation allowed.
    pub operation: OperationId,
    /// Only immutable request allowed.
    pub request: PeerRequestId,
    /// Narrow resource ceilings.
    pub limits: ExecutionLimits,
    /// Hard expiration boundary.
    pub expires_at_unix_ms: u64,
    /// Non-reusable nonce bound to the server record.
    pub nonce: String,
    /// Exact originating workflow coordinates used by materializing adapters.
    pub provenance: PeerExecutionProvenance,
}

impl DelegatedAuthorization {
    /// Validates non-secret bounded delegation facts.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        self.limits.validate()?;
        self.provenance.validate()?;
        if self.expires_at_unix_ms == 0
            || self.nonce.is_empty()
            || self.nonce.len() > 192
            || !self.nonce.is_ascii()
        {
            return Err(PeerProtocolError::InvalidContract(
                "delegation expiry or nonce is invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

fn safe_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 192
        && value.is_ascii()
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

/// Exact immutable invocation submitted to one selected peer/catalog/generation.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerInvocationRequest {
    /// Locally generated immutable idempotency key.
    pub request_id: PeerRequestId,
    /// Exact selected catalog generation.
    pub catalog_generation: u64,
    /// Exact selected catalog digest.
    pub catalog_digest: CatalogDigest,
    /// Exact remote descriptor/operation snapshot pinned on acceptance.
    pub selection: ResolvedCapabilitySnapshot,
    /// Provider-neutral bounded request using safe references.
    pub request: InvocationRequest,
    /// Enforced resource ceilings.
    pub limits: ExecutionLimits,
    /// Absolute remote admission/execution deadline.
    pub deadline_unix_ms: u64,
    /// Constrained authority reference; never an operator credential.
    pub delegation: DelegatedAuthorization,
    /// Canonical digest used for same-key/different-request rejection.
    pub request_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PeerInvocationRequestWire {
    request_id: PeerRequestId,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    selection: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
    limits: ExecutionLimits,
    deadline_unix_ms: u64,
    delegation: DelegatedAuthorization,
    request_digest: String,
}

#[derive(Serialize)]
struct InvocationDigestPayload<'a> {
    schema_version: u32,
    request_id: &'a PeerRequestId,
    catalog_generation: u64,
    catalog_digest: &'a CatalogDigest,
    selection: &'a ResolvedCapabilitySnapshot,
    request: &'a InvocationRequest,
    limits: ExecutionLimits,
    deadline_unix_ms: u64,
    delegation: &'a DelegatedAuthorization,
}

impl<'de> Deserialize<'de> for PeerInvocationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PeerInvocationRequestWire::deserialize(deserializer)?;
        let request = Self {
            request_id: wire.request_id,
            catalog_generation: wire.catalog_generation,
            catalog_digest: wire.catalog_digest,
            selection: wire.selection,
            request: wire.request,
            limits: wire.limits,
            deadline_unix_ms: wire.deadline_unix_ms,
            delegation: wire.delegation,
            request_digest: wire.request_digest,
        };
        request.validate().map_err(serde::de::Error::custom)?;
        Ok(request)
    }
}

impl PeerInvocationRequest {
    /// Constructs and canonically digests one exact peer request.
    #[allow(clippy::too_many_arguments)] // One validated peer execution contract keeps its exact provenance facts explicit.
    pub fn new(
        request_id: PeerRequestId,
        catalog_generation: u64,
        catalog_digest: CatalogDigest,
        selection: ResolvedCapabilitySnapshot,
        request: InvocationRequest,
        limits: ExecutionLimits,
        deadline_unix_ms: u64,
        delegation: DelegatedAuthorization,
    ) -> Result<Self, PeerProtocolError> {
        let request_digest = compute_request_digest(
            &request_id,
            catalog_generation,
            &catalog_digest,
            &selection,
            &request,
            limits,
            deadline_unix_ms,
            &delegation,
        )?;
        let value = Self {
            request_id,
            catalog_generation,
            catalog_digest,
            selection,
            request,
            limits,
            deadline_unix_ms,
            delegation,
            request_digest,
        };
        value.validate()?;
        Ok(value)
    }

    /// Revalidates exact selection, delegation, bounds, and canonical digest.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        self.limits.validate()?;
        self.delegation.validate()?;
        let _ = InvocationRequestDocument::new(self.request.clone())
            .to_canonical_json()
            .map_err(|error| PeerProtocolError::InvalidContract(error.to_string()))?;
        if self.catalog_generation == 0
            || self.deadline_unix_ms == 0
            || self.request.capability() != self.selection.capability()
            || self.request.operation() != self.selection.operation()
            || self.request.provider_profile() != self.selection.provider_profile()
            || self.delegation.request != self.request_id
            || self.delegation.capability != *self.selection.capability()
            || self.delegation.operation != *self.selection.operation()
            || !self.delegation.limits.contains(self.limits)
            || !validate_blake3_digest(self.catalog_digest.as_str())
        {
            return Err(PeerProtocolError::InvalidContract(
                "peer invocation selection, catalog, request, or delegation mismatch".to_owned(),
            ));
        }
        let expected = compute_request_digest(
            &self.request_id,
            self.catalog_generation,
            &self.catalog_digest,
            &self.selection,
            &self.request,
            self.limits,
            self.deadline_unix_ms,
            &self.delegation,
        )?;
        if self.request_digest != expected {
            return Err(PeerProtocolError::DigestMismatch("invocation"));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // One validated peer execution contract keeps its exact provenance facts explicit.
fn compute_request_digest(
    request_id: &PeerRequestId,
    catalog_generation: u64,
    catalog_digest: &CatalogDigest,
    selection: &ResolvedCapabilitySnapshot,
    request: &InvocationRequest,
    limits: ExecutionLimits,
    deadline_unix_ms: u64,
    delegation: &DelegatedAuthorization,
) -> Result<String, PeerProtocolError> {
    let bytes = milkdrift_contracts::canonical_json_bytes(
        &InvocationDigestPayload {
            schema_version: 1,
            request_id,
            catalog_generation,
            catalog_digest,
            selection,
            request,
            limits,
            deadline_unix_ms,
            delegation,
        },
        milkdrift_contracts::JsonLimits {
            maximum_depth: 32,
            maximum_string_bytes: 262_144,
            maximum_key_bytes: 192,
            maximum_container_items: 512,
        },
    )
    .map_err(|error| PeerProtocolError::Json(format!("{error:?}")))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(INVOCATION_DIGEST_DOMAIN);
    hasher.update(&bytes);
    Ok(format!("b3_{}", hasher.finalize().to_hex()))
}

/// Durable submission outcome. Accepted identities never change across replay.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationAcceptance {
    /// Acceptance was durably recorded before this response.
    Accepted {
        /// Exact idempotency key.
        request_id: PeerRequestId,
        /// Stable remote execution identity.
        execution: PeerExecutionId,
        /// Canonical request digest stored with acceptance.
        request_digest: String,
        /// Durable acceptance boundary time.
        accepted_at_unix_ms: u64,
        /// Accepted execution lease expiry.
        lease_expires_at_unix_ms: u64,
        /// True when this is an idempotent replay response.
        replayed: bool,
    },
    /// Exact replay resolved a compact immutable archived execution without reinvocation.
    Archived {
        /// Exact idempotency key.
        request_id: PeerRequestId,
        /// Original stable remote execution identity.
        execution: PeerExecutionId,
        /// Canonical request digest stored with acceptance.
        request_digest: String,
        /// Original durable acceptance boundary.
        accepted_at_unix_ms: u64,
        /// Compact terminal/uncertain and history summary.
        summary: Box<ArchivedExecutionSummary>,
    },
    /// Rejected before a new execution was accepted.
    Rejected {
        /// Stable request identity.
        request_id: PeerRequestId,
        /// Stable rejection code.
        code: String,
        /// Bounded redacted detail.
        detail: String,
        /// Whether retrying the same request may be useful.
        retryable: bool,
        /// Existing execution when lookup proved one but request bytes conflicted.
        known_execution: Option<PeerExecutionId>,
    },
}

/// Current durable knowledge returned by idempotency lookup.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationLookup {
    /// The server proves it has no durable record for this key.
    NotAccepted,
    /// One exact accepted execution and its current status are known.
    Known {
        /// Stable execution identity.
        execution: PeerExecutionId,
        /// Canonical accepted request digest.
        request_digest: String,
        /// Original durable acceptance boundary.
        accepted_at_unix_ms: u64,
        /// Current durable execution status.
        status: RemoteExecutionStatus,
        /// Highest durably appended semantic observation sequence.
        last_sequence: u64,
        /// Explicit hot or archived history availability.
        history: ObservationHistory,
    },
    /// The backing record is irrecoverably unavailable; no false conclusion is made.
    Unknown {
        /// Bounded diagnostic reason.
        reason: String,
    },
}

/// Durable execution lifecycle independent of a live connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteExecutionStatus {
    /// Accepted durably but adapter entry is not yet recorded.
    Accepted,
    /// Adapter entry or later semantic evidence is durable.
    Running,
    /// One terminal observation is durable.
    Terminal,
    /// Acceptance is known but outcome evidence is irrecoverable.
    OutcomeUnknown,
}

/// Compact immutable archived outcome and observation-history summary.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedExecutionSummary {
    /// Terminal or truthful outcome-unknown disposition.
    pub status: RemoteExecutionStatus,
    /// Highest observation sequence before archival.
    pub last_sequence: u64,
    /// Domain-separated digest of every compacted observation row.
    pub observation_digest: String,
    /// Atomic archive/compaction boundary.
    pub archived_at_unix_ms: u64,
    /// Retained final terminal summary, absent for outcome uncertainty.
    pub final_observation: Option<PeerObservation>,
    /// Bounded redacted uncertainty reason, present only for outcome uncertainty.
    pub uncertainty_reason: Option<String>,
}

impl ArchivedExecutionSummary {
    /// Validates terminal/uncertain summary consistency for one execution.
    pub fn validate(&self, execution: &PeerExecutionId) -> Result<(), PeerProtocolError> {
        if self.archived_at_unix_ms == 0
            || !valid_blake3_digest(&self.observation_digest)
            || self
                .uncertainty_reason
                .as_ref()
                .is_some_and(|reason| reason.is_empty() || reason.len() > 2_048)
        {
            return Err(PeerProtocolError::InvalidContract(
                "archived execution summary has invalid bounds or digest".to_owned(),
            ));
        }
        match (
            self.status,
            &self.final_observation,
            &self.uncertainty_reason,
        ) {
            (RemoteExecutionStatus::Terminal, Some(observation), None)
                if observation.execution == *execution
                    && observation.sequence == self.last_sequence
                    && observation.event.kind().terminal().is_some() =>
            {
                observation.validate()
            }
            (RemoteExecutionStatus::OutcomeUnknown, None, Some(_)) => Ok(()),
            _ => Err(PeerProtocolError::InvalidContract(
                "archived execution disposition is inconsistent".to_owned(),
            )),
        }
    }
}

/// Whether detailed observation rows remain hot or were explicitly compacted.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum ObservationHistory {
    /// Complete contiguous rows through the durable head remain queryable.
    Hot,
    /// Detailed rows were compacted; only the immutable outcome/history summary remains.
    Archived {
        /// Compact archived summary.
        summary: Box<ArchivedExecutionSummary>,
    },
}

/// Provider-neutral category for one semantic observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCategory {
    /// Bounded progress.
    Progress,
    /// Stream fragment represented by a bounded artifact/value observation.
    Stream,
    /// Output artifact reference.
    Artifact,
    /// Final success, failure, rejection, or cancellation.
    Terminal,
    /// Explicit uncertain terminal evidence.
    Uncertainty,
}

/// One monotonically sequenced durable remote execution observation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerObservation {
    /// Stable remote execution.
    pub execution: PeerExecutionId,
    /// Sequence beginning at one and contiguous within the execution log.
    pub sequence: u64,
    /// Provider-neutral category.
    pub category: ObservationCategory,
    /// Existing bounded capability event mapped without provider leakage.
    pub event: InvocationEvent,
    /// Remote append boundary time.
    pub observed_at_unix_ms: u64,
}

impl PeerObservation {
    /// Validates sequence and exact category/event mapping.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        let category_matches = match self.event.kind() {
            milkdrift_capability::InvocationEventKind::Progress { .. } => {
                matches!(
                    self.category,
                    ObservationCategory::Progress | ObservationCategory::Stream
                )
            }
            milkdrift_capability::InvocationEventKind::Output { .. } => {
                self.category == ObservationCategory::Artifact
            }
            milkdrift_capability::InvocationEventKind::Terminal { terminal } => {
                if terminal.status() == TerminalStatus::Uncertain {
                    self.category == ObservationCategory::Uncertainty
                } else {
                    self.category == ObservationCategory::Terminal
                }
            }
        };
        if self.sequence == 0
            || self.event.sequence() != self.sequence
            || self.observed_at_unix_ms == 0
            || !category_matches
        {
            return Err(PeerProtocolError::InvalidContract(
                "observation sequence or category does not match its event".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Bounded resumable observation page. Transport heartbeats are not entries.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPage {
    /// Exact remote execution.
    pub execution: PeerExecutionId,
    /// Exclusive resume cursor requested by the caller.
    pub after_sequence: u64,
    /// Contiguous semantic observations after the cursor.
    pub observations: Vec<PeerObservation>,
    /// Highest returned sequence, or the input cursor for an empty page.
    pub next_sequence: u64,
    /// True only after terminal evidence is included or already precedes the cursor.
    pub terminal: bool,
    /// True when no later semantic observation can be appended.
    pub closed: bool,
    /// Explicit detailed-history availability.
    pub history: ObservationHistory,
}

impl ObservationPage {
    /// Validates page cardinality, execution ownership, and contiguous cursors.
    pub fn validate(&self, maximum_items: usize) -> Result<(), PeerProtocolError> {
        let limit = maximum_items.min(MAX_OBSERVATIONS_PER_PAGE);
        if self.observations.len() > limit {
            return Err(PeerProtocolError::Bounds {
                location: "observations",
                reason: "observation page exceeds bounds".to_owned(),
            });
        }
        match &self.history {
            ObservationHistory::Hot if self.closed && !self.terminal => {
                return Err(PeerProtocolError::InvalidContract(
                    "hot observation history closes only with terminal evidence".to_owned(),
                ));
            }
            ObservationHistory::Archived { summary } => {
                summary.validate(&self.execution)?;
                if !self.observations.is_empty()
                    || !self.closed
                    || self.after_sequence > summary.last_sequence
                    || self.terminal != (summary.status == RemoteExecutionStatus::Terminal)
                {
                    return Err(PeerProtocolError::InvalidContract(
                        "archived observation page does not match its compacted summary".to_owned(),
                    ));
                }
            }
            ObservationHistory::Hot => {}
        }
        let mut expected = self.after_sequence.saturating_add(1);
        for observation in &self.observations {
            observation.validate()?;
            if observation.execution != self.execution || observation.sequence != expected {
                return Err(PeerProtocolError::InvalidContract(
                    "observation page is not contiguous for one execution".to_owned(),
                ));
            }
            expected = expected.saturating_add(1);
        }
        let expected_next = self
            .observations
            .last()
            .map_or(self.after_sequence, |item| item.sequence);
        if self.next_sequence != expected_next {
            return Err(PeerProtocolError::InvalidContract(
                "observation resume cursor does not match page contents".to_owned(),
            ));
        }
        Ok(())
    }
}

fn valid_blake3_digest(value: &str) -> bool {
    value.len() == 67
        && value.starts_with("b3_")
        && value[3..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

/// Exact cancellation request; socket closure is deliberately unrelated.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerCancellationRequest {
    /// Idempotent cancellation request identity.
    pub request_id: PeerRequestId,
    /// Exact accepted remote execution.
    pub execution: PeerExecutionId,
    /// Monotonic cancellation sequence for that execution.
    pub sequence: u64,
    /// Bounded redacted reason.
    pub reason: String,
}

/// Stable outcome of a cancellation attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationDisposition {
    /// Cancellation was accepted for processing.
    Accepted,
    /// Relationship authority rejected the request.
    Rejected,
    /// The underlying operation does not support cancellation.
    Unsupported,
    /// Terminal evidence already made cancellation too late.
    TooLate,
    /// Disconnect or missing durable evidence prevents confirmation.
    Unknown,
}

/// Cancellation acknowledgement separate from transport connection state.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerCancellationAcknowledgement {
    /// Cancellation request being acknowledged.
    pub request_id: PeerRequestId,
    /// Exact target execution.
    pub execution: PeerExecutionId,
    /// Stable disposition.
    pub disposition: CancellationDisposition,
    /// True only when no later external effect can occur.
    pub terminal_boundary: bool,
    /// Existing terminal evidence when already known.
    pub terminal_evidence: Option<PeerObservation>,
    /// Bounded diagnostic detail.
    pub detail: Option<String>,
}

impl PeerCancellationAcknowledgement {
    /// Enforces truthful terminal-boundary and evidence semantics.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.detail.as_ref().is_some_and(|value| value.len() > 512)
            || (self.terminal_boundary
                && matches!(
                    self.disposition,
                    CancellationDisposition::Rejected
                        | CancellationDisposition::Unsupported
                        | CancellationDisposition::Unknown
                ))
            || self
                .terminal_evidence
                .as_ref()
                .is_some_and(|item| item.execution != self.execution || item.validate().is_err())
        {
            return Err(PeerProtocolError::InvalidContract(
                "invalid cancellation acknowledgement semantics".to_owned(),
            ));
        }
        Ok(())
    }
}
