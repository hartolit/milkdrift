use milkdrift_authority::ActorRef;
use milkdrift_capability::{
    CapabilityId, InvocationEvent, InvocationRequest, InvocationRequestDocument, OperationId,
    PeerId, ResolvedCapabilitySnapshot, TerminalStatus,
};
use milkdrift_contracts::is_canonical_blake3_digest;
use serde::{Deserialize, Serialize};

use crate::{
    CatalogDigest, DelegationRef, InvocationOrigin, PeerExecutionId, PeerProtocolError,
    PeerRequestId, ServingAuthorization,
};

const INVOCATION_DIGEST_DOMAIN: &[u8] = b"milkdrift.serving.invocation.v2\0";
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
    pub(crate) fn validate(&self) -> Result<(), PeerProtocolError> {
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
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionLimits {
    /// Cumulative internal process/model entries. Absence forbids composed execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_invocations: Option<milkdrift_capability::InvocationCounts>,
    /// Maximum total input plus output artifact bytes.
    pub artifact_bytes: u64,
    /// Maximum execution duration.
    pub duration_ms: u64,
    /// Maximum observed cost in millionths, when applicable.
    pub cost_micros: u64,
    /// Exact currency for a monetary allowance; absent means billing is not allowed.
    #[serde(default)]
    pub cost_currency: Option<String>,
    /// Maximum complete prompt tokens; absent means token consumption is not allowed.
    #[serde(default)]
    pub input_units: Option<u64>,
    /// Maximum generated tokens including reasoning; absent forbids token consumption.
    #[serde(default)]
    pub output_units: Option<u64>,
    /// Maximum semantic observations retained and streamed.
    pub observations: u32,
}

impl ExecutionLimits {
    /// Requires nonzero duration and observation ceilings.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.duration_ms == 0 || self.observations == 0 || self.observations > 1_000_000 {
            return Err(PeerProtocolError::InvalidContract(
                "execution duration and observation limits must be bounded and nonzero".to_owned(),
            ));
        }
        if let Some(currency) = &self.cost_currency {
            milkdrift_capability::AdmissionMonetaryBound::new(self.cost_micros, currency)
                .map_err(|error| PeerProtocolError::InvalidContract(error.to_string()))?;
        } else if self.cost_micros != 0 {
            return Err(PeerProtocolError::InvalidContract(
                "a monetary allowance requires its exact currency".to_owned(),
            ));
        }
        Ok(())
    }

    /// True when every requested ceiling is no greater than this grant ceiling.
    #[must_use]
    pub fn contains(&self, requested: &Self) -> bool {
        requested.nested_invocations.is_none_or(|counts| {
            self.nested_invocations
                .is_some_and(|maximum| maximum.contains(counts))
        }) && requested.artifact_bytes <= self.artifact_bytes
            && requested.duration_ms <= self.duration_ms
            && requested.cost_micros <= self.cost_micros
            && (requested.cost_currency.is_none() || requested.cost_currency == self.cost_currency)
            && requested
                .input_units
                .is_none_or(|value| self.input_units.is_some_and(|maximum| value <= maximum))
            && requested
                .output_units
                .is_none_or(|value| self.output_units.is_some_and(|maximum| value <= maximum))
            && requested.observations <= self.observations
    }

    /// Allowance enforced against the prepared adapter before remote entry. An absent
    /// dimension permits only an adapter that declares the resource inapplicable.
    pub fn admission_envelope(
        &self,
    ) -> Result<milkdrift_capability::InvocationAdmissionEnvelope, PeerProtocolError> {
        use milkdrift_capability::{
            AdmissionBound, AdmissionMonetaryBound, AdmissionUnit, InvocationAdmissionEnvelope,
        };
        self.validate()?;
        let envelope = InvocationAdmissionEnvelope::new(
            AdmissionUnit::ModelTokens,
            self.input_units
                .map_or(AdmissionBound::NotApplicable, AdmissionBound::Bounded),
            self.output_units
                .map_or(AdmissionBound::NotApplicable, AdmissionBound::Bounded),
            AdmissionBound::Bounded(self.artifact_bytes),
            self.cost_currency
                .as_ref()
                .map_or(Ok(AdmissionBound::NotApplicable), |currency| {
                    AdmissionMonetaryBound::new(self.cost_micros, currency)
                        .map(AdmissionBound::Bounded)
                })
                .map_err(|error| PeerProtocolError::InvalidContract(error.to_string()))?,
        );
        Ok(self.nested_invocations.map_or_else(
            || envelope.clone(),
            |counts| envelope.clone().with_nested_invocations(counts),
        ))
    }

    /// Validate reported usage without treating missing observations as zero consumption.
    #[must_use]
    pub fn permits_usage(&self, usage: &milkdrift_capability::UsageObservation) -> bool {
        usage
            .duration_ms()
            .is_none_or(|value| value <= self.duration_ms)
            && usage.cost_micros().is_none_or(|value| {
                value <= self.cost_micros && usage.currency() == self.cost_currency.as_deref()
            })
            && usage
                .input_units()
                .is_none_or(|value| self.input_units.is_some_and(|limit| value <= limit))
            && usage
                .output_units()
                .is_none_or(|value| self.output_units.is_some_and(|limit| value <= limit))
            && usage.nested_work().is_none_or(|nested| {
                self.nested_invocations
                    .is_some_and(|maximum| maximum.contains(nested.invocations()))
                    && nested.artifact_bytes() <= self.artifact_bytes
            })
    }

    /// Refuses entry unless the prepared adapter can enforce every accepted dimension.
    pub fn permits_prepared(
        &self,
        prepared: &milkdrift_capability::InvocationAdmissionEnvelope,
        input_artifact_bytes: u64,
    ) -> bool {
        use milkdrift_capability::{AdmissionBound, AdmissionUnit};
        let permits_units = |bound: &AdmissionBound<u64>, maximum: Option<u64>| match bound {
            AdmissionBound::NotApplicable => true,
            AdmissionBound::Bounded(value) => {
                prepared.unit() == AdmissionUnit::ModelTokens
                    && maximum.is_some_and(|maximum| *value <= maximum)
            }
            AdmissionBound::Unknown => false,
        };
        let output_bytes = match prepared.artifact_bytes() {
            AdmissionBound::NotApplicable => Some(0),
            AdmissionBound::Bounded(value) => Some(*value),
            AdmissionBound::Unknown => None,
        };
        prepared.nested_invocations().is_none_or(|counts| {
            self.nested_invocations
                .is_some_and(|maximum| maximum.contains(counts))
        }) && permits_units(prepared.input_units(), self.input_units)
            && permits_units(prepared.output_units(), self.output_units)
            && output_bytes
                .and_then(|value| value.checked_add(input_artifact_bytes))
                .is_some_and(|value| value <= self.artifact_bytes)
            && match prepared.monetary_cost() {
                AdmissionBound::NotApplicable => true,
                AdmissionBound::Bounded(value) => {
                    self.cost_currency.as_deref() == Some(value.currency())
                        && value.maximum_micros() <= self.cost_micros
                }
                AdmissionBound::Unknown => false,
            }
    }
}

/// Opaque server-stored delegation reference plus immutable narrowing facts.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAuthorization {
    /// Exact enclosing publications from the originating runtime; prevents cycles across hosts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub publication_ancestry: Vec<milkdrift_capability::PublicationAncestor>,
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
    /// Validated direct or workflow origin, independently of peer transport.
    pub origin: InvocationOrigin,
    /// Origin-owned reservation committed before submission. The serving host retains
    /// its linkage and enforces `limits`; it does not create a second controller charge.
    pub controller_reservation: Option<String>,
}

impl DelegatedAuthorization {
    /// Validates non-secret bounded delegation facts.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        self.limits.validate()?;
        if let InvocationOrigin::Workflow { provenance } = &self.origin {
            provenance.validate()?;
        }
        if self.publication_ancestry.len()
            > usize::from(milkdrift_capability::MAX_PUBLICATION_DEPTH)
            || self.publication_ancestry.iter().any(|ancestor| {
                self.publication_ancestry.len() > usize::from(ancestor.maximum_depth())
            })
            || (!self.publication_ancestry.is_empty() && self.origin.workflow().is_none())
        {
            return Err(PeerProtocolError::InvalidContract(
                "publication ancestry requires bounded workflow provenance".to_owned(),
            ));
        }
        if self
            .controller_reservation
            .as_ref()
            .is_some_and(|reservation| {
                !safe_reference(reservation) || self.origin.workflow().is_none()
            })
        {
            return Err(PeerProtocolError::InvalidContract(
                "controller allowance requires exact workflow origin and reservation identity"
                    .to_owned(),
            ));
        }
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

/// One exact submission whose identity can recover a lost acceptance reply.
///
/// [`Self::new`] digests the selection, inputs, catalog, limits, deadline, and delegation together.
/// Retry the same facts under the same request ID. Changing even the deadline is a conflicting
/// request, not a renewal of the original acceptance. The serving host owns authorization and
/// durable acceptance; this type checks the portable request's internal consistency.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServingInvocationRequest {
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
    pub authorization: ServingAuthorization,
    /// Canonical digest used for same-key/different-request rejection.
    pub request_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServingInvocationRequestWire {
    request_id: PeerRequestId,
    catalog_generation: u64,
    catalog_digest: CatalogDigest,
    selection: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
    limits: ExecutionLimits,
    deadline_unix_ms: u64,
    authorization: ServingAuthorization,
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
    limits: &'a ExecutionLimits,
    deadline_unix_ms: u64,
    authorization: &'a ServingAuthorization,
}

impl<'de> Deserialize<'de> for ServingInvocationRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ServingInvocationRequestWire::deserialize(deserializer)?;
        let request = Self {
            request_id: wire.request_id,
            catalog_generation: wire.catalog_generation,
            catalog_digest: wire.catalog_digest,
            selection: wire.selection,
            request: wire.request,
            limits: wire.limits,
            deadline_unix_ms: wire.deadline_unix_ms,
            authorization: wire.authorization,
            request_digest: wire.request_digest,
        };
        request.validate().map_err(serde::de::Error::custom)?;
        Ok(request)
    }
}

impl ServingInvocationRequest {
    /// Constructs and canonically digests one exact peer request.
    #[allow(clippy::too_many_arguments)] // Peer admission binds catalog selection, invocation, deadline, resource limits, and authorization in one digest.
    pub fn new(
        request_id: PeerRequestId,
        catalog_generation: u64,
        catalog_digest: CatalogDigest,
        selection: ResolvedCapabilitySnapshot,
        request: InvocationRequest,
        limits: ExecutionLimits,
        deadline_unix_ms: u64,
        authorization: impl Into<ServingAuthorization>,
    ) -> Result<Self, PeerProtocolError> {
        let authorization = authorization.into();
        let request_digest = compute_request_digest(
            &request_id,
            catalog_generation,
            &catalog_digest,
            &selection,
            &request,
            &limits,
            deadline_unix_ms,
            &authorization,
        )?;
        let value = Self {
            request_id,
            catalog_generation,
            catalog_digest,
            selection,
            request,
            limits,
            deadline_unix_ms,
            authorization,
            request_digest,
        };
        value.validate()?;
        Ok(value)
    }

    /// Revalidates exact selection, authorization, bounds, and canonical digest.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        self.limits.validate()?;
        self.authorization.validate()?;
        if matches!(self.authorization.origin(), InvocationOrigin::Direct)
            && (self.request.context_manifest().is_some()
                || self.request.inputs().iter().any(|input| {
                    input.name() == milkdrift_capability::CONTEXT_MANIFEST_INPUT_NAME
                        || input
                            .name()
                            .starts_with(milkdrift_capability::CONTEXT_ITEM_INPUT_PREFIX)
                        || matches!(
                            input.value(),
                            milkdrift_capability::InvocationValueReference::WorkspaceValue { .. }
                        )
                }))
        {
            return Err(PeerProtocolError::InvalidContract(
                "direct origin cannot carry workflow-selected inputs".to_owned(),
            ));
        }
        let _ = InvocationRequestDocument::new(self.request.clone())
            .to_canonical_json()
            .map_err(|error| PeerProtocolError::InvalidContract(error.to_string()))?;
        if self.catalog_generation == 0
            || self.deadline_unix_ms == 0
            || self.request.capability() != self.selection.capability()
            || self.request.operation() != self.selection.operation()
            || self.request.provider_profile() != self.selection.provider_profile()
            || self.authorization.delegation().is_some_and(|delegation| {
                delegation.request != self.request_id
                    || delegation.capability != *self.selection.capability()
                    || delegation.operation != *self.selection.operation()
                    || !delegation.limits.contains(&self.limits)
            })
            || !is_canonical_blake3_digest(self.catalog_digest.as_str())
        {
            return Err(PeerProtocolError::InvalidContract(
                "peer invocation selection, catalog, request, or authorization mismatch".to_owned(),
            ));
        }
        let expected = compute_request_digest(
            &self.request_id,
            self.catalog_generation,
            &self.catalog_digest,
            &self.selection,
            &self.request,
            &self.limits,
            self.deadline_unix_ms,
            &self.authorization,
        )?;
        if self.request_digest != expected {
            return Err(PeerProtocolError::DigestMismatch("invocation"));
        }
        if self.input_artifact_bytes()? > self.limits.artifact_bytes {
            return Err(PeerProtocolError::Bounds {
                location: "peer_invocation.input_artifact_bytes",
                reason: "exact input artifact bytes exceed the accepted total quota".to_owned(),
            });
        }
        Ok(())
    }

    /// Returns the exact total size of every artifact materialized as input.
    pub fn input_artifact_bytes(&self) -> Result<u64, PeerProtocolError> {
        let mut references = self
            .request
            .inputs()
            .iter()
            .filter_map(|input| input.value().artifact())
            .chain(self.request.context_manifest());
        references.try_fold(0_u64, |total, reference| {
            let size = reference.size_bytes().ok_or_else(|| {
                PeerProtocolError::InvalidContract(
                    "peer input artifact references require exact byte sizes".to_owned(),
                )
            })?;
            total
                .checked_add(size)
                .ok_or_else(|| PeerProtocolError::Bounds {
                    location: "peer_invocation.input_artifact_bytes",
                    reason: "input artifact byte accounting overflowed".to_owned(),
                })
        })
    }
}

#[allow(clippy::too_many_arguments)] // The digest covers every independent admission field; omitting one would permit conflicting request replay.
fn compute_request_digest(
    request_id: &PeerRequestId,
    catalog_generation: u64,
    catalog_digest: &CatalogDigest,
    selection: &ResolvedCapabilitySnapshot,
    request: &InvocationRequest,
    limits: &ExecutionLimits,
    deadline_unix_ms: u64,
    authorization: &ServingAuthorization,
) -> Result<String, PeerProtocolError> {
    let bytes = milkdrift_contracts::canonical_json_bytes(
        &InvocationDigestPayload {
            schema_version: 2,
            request_id,
            catalog_generation,
            catalog_digest,
            selection,
            request,
            limits,
            deadline_unix_ms,
            authorization,
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

/// Whether the serving host durably accepted this exact submission.
///
/// Acceptance precedes adapter entry and is not execution success. Use [`Self::validate_for`] to
/// bind a decoded reply to the submitted request, then follow the accepted execution's observations.
/// Archived replay carries its retained outcome without asking the capability to run again.
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

impl InvocationAcceptance {
    /// Validates semantic bounds and binds the response to the exact submitted request.
    pub fn validate_for(
        &self,
        request: &ServingInvocationRequest,
    ) -> Result<(), PeerProtocolError> {
        let valid = match self {
            Self::Accepted {
                request_id,
                request_digest,
                accepted_at_unix_ms,
                lease_expires_at_unix_ms,
                ..
            } => {
                request_id == &request.request_id
                    && request_digest == &request.request_digest
                    && *accepted_at_unix_ms > 0
                    && *lease_expires_at_unix_ms >= *accepted_at_unix_ms
            }
            Self::Archived {
                request_id,
                execution,
                request_digest,
                accepted_at_unix_ms,
                summary,
            } => {
                request_id == &request.request_id
                    && request_digest == &request.request_digest
                    && *accepted_at_unix_ms > 0
                    && summary.validate(execution).is_ok()
            }
            Self::Rejected {
                request_id,
                code,
                detail,
                ..
            } => {
                request_id == &request.request_id
                    && !code.is_empty()
                    && code.len() <= 192
                    && code.is_ascii()
                    && detail.len() <= 2_048
            }
        };
        if valid {
            Ok(())
        } else {
            Err(PeerProtocolError::InvalidContract(
                "invocation acceptance is not bound to the submitted request".to_owned(),
            ))
        }
    }
}

/// Recover acceptance knowledge by request ID after a missing or ambiguous reply.
///
/// `NotAccepted` is a statement from the durable owner; a transport error cannot stand in for it.
/// `Known` may contain hot or archived history, while `Unknown` preserves missing evidence.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum InvocationLookup {
    /// The server proves it has no durable record for this key.
    NotAccepted {
        /// Exact request identity queried.
        request_id: PeerRequestId,
    },
    /// One exact accepted execution and its current status are known.
    Known {
        /// Exact request identity queried.
        request_id: PeerRequestId,
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
        /// Exact request identity queried.
        request_id: PeerRequestId,
        /// Bounded diagnostic reason.
        reason: String,
    },
}

impl InvocationLookup {
    /// Validates the internal semantics and binds the result to the exact queried identity.
    pub fn validate_for(&self, request: &PeerRequestId) -> Result<(), PeerProtocolError> {
        let valid = match self {
            Self::NotAccepted { request_id } => request_id == request,
            Self::Known {
                request_id,
                execution,
                request_digest,
                accepted_at_unix_ms,
                status,
                last_sequence,
                history,
            } => {
                request_id == request
                    && *accepted_at_unix_ms > 0
                    && is_canonical_blake3_digest(request_digest)
                    && match history {
                        ObservationHistory::Hot => true,
                        ObservationHistory::Archived { summary } => {
                            summary.validate(execution).is_ok()
                                && summary.status == *status
                                && summary.last_sequence == *last_sequence
                        }
                    }
            }
            Self::Unknown { request_id, reason } => {
                request_id == request && !reason.is_empty() && reason.len() <= 2_048
            }
        };
        if valid {
            Ok(())
        } else {
            Err(PeerProtocolError::InvalidContract(
                "invocation lookup is invalid or not bound to the queried request".to_owned(),
            ))
        }
    }
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
    /// At most 256 named output facts, retaining original remote sequence and identity.
    pub output_observations: Vec<PeerObservation>,
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
            || !is_canonical_blake3_digest(&self.observation_digest)
            || self
                .uncertainty_reason
                .as_ref()
                .is_some_and(|reason| reason.is_empty() || reason.len() > 2_048)
        {
            return Err(PeerProtocolError::InvalidContract(
                "archived execution summary has invalid bounds or digest".to_owned(),
            ));
        }
        if self.output_observations.len() > 256 {
            return Err(PeerProtocolError::InvalidContract(
                "archived output count exceeds 256".to_owned(),
            ));
        }
        let mut prior = 0;
        for output in &self.output_observations {
            output.validate()?;
            if output.execution != *execution
                || output.sequence <= prior
                || output.sequence > self.last_sequence
                || output.event.kind().output().is_none()
            {
                return Err(PeerProtocolError::InvalidContract(
                    "archived output identity or order is invalid".to_owned(),
                ));
            }
            prior = output.sequence;
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

/// Continue observing an execution after an exclusive sequence cursor.
///
/// Hot rows are contiguous and bounded. Archived history instead supplies a retained summary,
/// so an empty page does not mean no work occurred. `closed` stops further observation; inspect
/// terminal/history facts to distinguish completion from an unknown outcome. Transport keepalives
/// consume no semantic sequence numbers.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationPage {
    /// Current durable outcome, including uncertainty without a terminal event.
    pub status: RemoteExecutionStatus,
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
            ObservationHistory::Hot
                if self.closed
                    && !self.terminal
                    && self.status != RemoteExecutionStatus::OutcomeUnknown =>
            {
                return Err(PeerProtocolError::InvalidContract(
                    "hot observation history closes only with terminal evidence or durable uncertainty".to_owned(),
                ));
            }
            ObservationHistory::Archived { summary } => {
                summary.validate(&self.execution)?;
                if !self.observations.is_empty()
                    || self.status != summary.status
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
        if self.terminal && (!self.closed || self.status != RemoteExecutionStatus::Terminal) {
            return Err(PeerProtocolError::InvalidContract(
                "terminal observation page contradicts its durable status".to_owned(),
            ));
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

    /// Validates semantics and binds the acknowledgement to the exact cancellation request.
    pub fn validate_for(&self, request: &PeerCancellationRequest) -> Result<(), PeerProtocolError> {
        self.validate()?;
        if self.request_id != request.request_id || self.execution != request.execution {
            return Err(PeerProtocolError::InvalidContract(
                "cancellation acknowledgement targets a different request or execution".to_owned(),
            ));
        }
        Ok(())
    }
}
