use crate::{
    CatalogDigest, ClientInvocationAuthorization, ExecutionLimits, PeerProtocolError,
    PeerRequestId, ServingInvocationRequest,
};
use milkdrift_capability::{
    CapabilityId, InputReference, InvocationRequest, OperationId, PeerId,
    ResolvedCapabilitySnapshot,
};
use serde::{Deserialize, Serialize};

/// An authenticated client's current serving installation and exact capability catalog.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectDiscovery {
    /// Stable installation identity; submissions must target this exact host.
    pub host: PeerId,
    /// Host-wide ceiling applied to each client's per-call request.
    pub limits: ExecutionLimits,
    /// Current authorized capability generations.
    pub catalog: crate::CatalogSnapshot,
}

/// Choices for preparing an independent call without accepting or executing it.
///
/// The host supplies the current catalog, exact selection, profile, required idempotency key
/// and deadline. Persist the returned [`DirectInvocationRequest`] before submission and replay
/// that exact document after a lost reply; preparing again does not recover an accepted call.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectInvocationDraft {
    /// Installation selected by the caller; another host must refuse preparation.
    pub host: PeerId,
    /// Caller-owned identity for the eventual submission and adapter invocation.
    pub request_id: PeerRequestId,
    /// Explicit capability from authorized discovery.
    pub capability: CapabilityId,
    /// Explicit operation offered by that capability.
    pub operation: OperationId,
    /// Named inline values or committed artifact references; no workflow context.
    pub inputs: Vec<InputReference>,
    /// Explicit per-call ceilings, or the discovered host ceilings when absent.
    /// Preparation refuses unavailable allowances rather than silently clamping them.
    pub limits: Option<ExecutionLimits>,
}

/// Bounded projection of one accepted invocation, independent of persistence layout.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServingInvocationRead {
    /// Authenticated submitting principal and serving host.
    pub caller: crate::ServingCaller,
    /// Whether the caller supplied a workflow origin or explicit independent inputs.
    pub origin: crate::InvocationOrigin,
    /// Durable acceptance, current status, and available history.
    pub acceptance: crate::InvocationLookup,
    /// Exact accepted capability identity.
    pub capability: milkdrift_capability::CapabilityId,
    /// Exact accepted descriptor revision.
    pub descriptor_revision: u64,
    /// Accepted capability operation.
    pub operation: milkdrift_capability::OperationId,
    /// Accounted input and output artifact bytes.
    pub artifact_bytes: u64,
    /// Reported terminal duration; absent means no durable report.
    pub duration_ms: Option<u64>,
    /// Reported terminal cost; absent means no durable report.
    pub cost_micros: Option<u64>,
    /// Latest durable cancellation acknowledgement, distinct from execution outcome.
    pub cancellation: Option<crate::PeerCancellationAcknowledgement>,
}

/// Independent client submission. Authentication supplies authority; this document cannot claim it.
/// The exact host, catalog, invocation, limits and deadline are immutable across retries.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectInvocationRequest {
    /// Explicit serving installation selected by the client.
    pub host: PeerId,
    /// Caller-scoped replay key.
    pub request_id: PeerRequestId,
    /// Exact discovery generation.
    pub catalog_generation: u64,
    /// Canonical discovered catalog digest.
    pub catalog_digest: CatalogDigest,
    /// Exact descriptor and operation selected from discovery.
    pub selection: ResolvedCapabilitySnapshot,
    /// Explicit inputs with no workflow context or workspace references.
    pub request: InvocationRequest,
    /// Requested enforceable per-call ceilings.
    pub limits: ExecutionLimits,
    /// Absolute admission and execution deadline; retry does not renew it.
    pub deadline_unix_ms: u64,
}

impl DirectInvocationRequest {
    /// Freezes the authenticated server basis into the one canonical serving request.
    ///
    /// # Errors
    /// Refuses authority for another host and propagates the request consistency,
    /// bounds and encoding failures from [`ServingInvocationRequest::new`].
    pub fn bind(
        &self,
        authority: ClientInvocationAuthorization,
    ) -> Result<ServingInvocationRequest, PeerProtocolError> {
        if self.host != authority.host {
            return Err(PeerProtocolError::InvalidContract(
                "direct request targets another host".to_owned(),
            ));
        }
        ServingInvocationRequest::new(
            self.request_id.clone(),
            self.catalog_generation,
            self.catalog_digest.clone(),
            self.selection.clone(),
            self.request.clone(),
            self.limits.clone(),
            self.deadline_unix_ms,
            authority,
        )
    }
}

/// One bounded range of a caller's declared capability output, including its exact metadata.
/// This does not grant access to arbitrary artifacts or the implementation's internal workspace.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationOutputChunk {
    /// Exact public execution whose retained output list authorizes this disclosure.
    pub execution: crate::PeerExecutionId,
    /// Immutable output metadata; sensitivity is preserved.
    pub metadata: milkdrift_workspace::ArtifactMetadata,
    /// First returned byte.
    pub offset: u64,
    /// At most 65,536 bytes. JSON framing remains below the control document ceiling.
    pub bytes: Vec<u8>,
    /// True exactly when this range reaches the immutable size.
    pub complete: bool,
}
