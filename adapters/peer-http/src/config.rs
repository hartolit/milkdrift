use std::{collections::BTreeSet, fmt, sync::Arc, time::Duration};

use milkdrift_authority::{PeerId, SensitiveSecret};
use milkdrift_capability::{CapabilityId, OperationId, SideEffectClass, TrustZone};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PeerAuthority,
    ProtocolVersionRange, SessionId,
};
use url::{Host, Url};

use crate::PeerHttpError;

/// Explicitly named development-only plaintext policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InsecureLoopbackMode {
    /// Plaintext is rejected.
    #[default]
    Disabled,
    /// Plaintext is allowed only for literal loopback or `localhost` endpoints.
    AllowInsecureLoopbackDevelopment,
}

/// One authenticated, expiring, default-deny peer relationship.
#[derive(Clone)]
pub struct PeerRelationship {
    /// Authenticated remote peer identity.
    pub remote_peer: PeerId,
    /// Bearer credential value resolved from a daemon-owned secret reference.
    pub bearer_credential: Arc<SensitiveSecret>,
    /// Allowed peer protocol range.
    pub versions: ProtocolVersionRange,
    /// Exact action authority. Empty denies all operations.
    pub authority: PeerAuthority,
    /// Capabilities explicitly allowed. Empty advertises and invokes nothing.
    pub capability_allow: BTreeSet<CapabilityId>,
    /// Capabilities explicitly denied after allow matching.
    pub capability_deny: BTreeSet<CapabilityId>,
    /// Operations explicitly allowed. Empty advertises and invokes nothing.
    pub operation_allow: BTreeSet<OperationId>,
    /// Maximum remote side-effect classification.
    pub maximum_side_effect: SideEffectClass,
    /// Relationship-level execution quotas.
    pub execution_limits: ExecutionLimits,
    /// Maximum concurrent accepted executions.
    pub maximum_concurrent: u16,
    /// Maximum authenticated requests in any fixed one-minute window, per action/operation.
    pub maximum_requests_per_minute: u32,
    /// Maximum sum of artifact bytes accepted per execution.
    pub maximum_artifact_bytes: u64,
    /// Catalog TTL.
    pub catalog_ttl_ms: u64,
    /// Trust/locality zone added only to the local remote-adapter descriptor.
    pub trust_zone: TrustZone,
    /// Opaque server-stored delegation record reference.
    pub delegation: DelegationRef,
    /// Revocation generation bound into runtime relationship state.
    pub revocation_generation: u64,
    /// Hard relationship expiration boundary.
    pub expires_at_unix_ms: u64,
    /// False revokes transport authentication and every protocol action.
    pub enabled: bool,
}

impl fmt::Debug for PeerRelationship {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PeerRelationship")
            .field("remote_peer", &self.remote_peer)
            .field("bearer_credential", &"[redacted]")
            .field("versions", &self.versions)
            .field("authority", &self.authority)
            .field("capability_allow", &self.capability_allow)
            .field("capability_deny", &self.capability_deny)
            .field("operation_allow", &self.operation_allow)
            .field("maximum_side_effect", &self.maximum_side_effect)
            .field("execution_limits", &self.execution_limits)
            .field("maximum_concurrent", &self.maximum_concurrent)
            .field(
                "maximum_requests_per_minute",
                &self.maximum_requests_per_minute,
            )
            .field("maximum_artifact_bytes", &self.maximum_artifact_bytes)
            .field("catalog_ttl_ms", &self.catalog_ttl_ms)
            .field("trust_zone", &self.trust_zone)
            .field("delegation", &"[opaque]")
            .field("revocation_generation", &self.revocation_generation)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .field("enabled", &self.enabled)
            .finish()
    }
}

impl PeerRelationship {
    /// Validates quotas, credential presence, TTL, and the default-deny relationship shape.
    pub fn validate(&self) -> Result<(), PeerHttpError> {
        self.versions
            .negotiate(self.versions)
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        self.execution_limits
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        if self.bearer_credential.is_empty()
            || self.bearer_credential.len() > 8_192
            || self.maximum_concurrent == 0
            || self.maximum_requests_per_minute == 0
            || self.maximum_requests_per_minute > 100_000
            || self.catalog_ttl_ms == 0
            || self.catalog_ttl_ms > 300_000
            || self.expires_at_unix_ms == 0
        {
            return Err(PeerHttpError::Configuration(
                "peer credential, concurrency, TTL, or expiry is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    /// True only for capabilities and operations explicitly allowed by this relationship.
    #[must_use]
    pub fn permits_capability(&self, capability: &CapabilityId, operation: &OperationId) -> bool {
        self.capability_allow.contains(capability)
            && !self.capability_deny.contains(capability)
            && self.operation_allow.contains(operation)
    }
}

/// Server-side peer route configuration with a distinct authentication realm.
#[derive(Clone, Debug)]
pub struct PeerServerConfig {
    /// Authenticated identity of this daemon.
    pub local_peer: PeerId,
    /// Fresh boot/session identity.
    pub session: SessionId,
    /// Server-supported versions.
    pub versions: ProtocolVersionRange,
    /// Server package hard limits.
    pub limits: HardLimits,
    /// Heartbeat, timeout, and execution lease policy.
    pub lease: HeartbeatLease,
    /// Configured inbound relationships.
    pub relationships: Vec<PeerRelationship>,
}

impl PeerServerConfig {
    /// Validates all bounds and rejects duplicate remote identities.
    pub fn validate(&self) -> Result<(), PeerHttpError> {
        self.limits
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        self.lease
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        if self.relationships.len() > 256 {
            return Err(PeerHttpError::Configuration(
                "at most 256 peer relationships are supported".to_owned(),
            ));
        }
        let mut peers = BTreeSet::new();
        for relationship in &self.relationships {
            relationship.validate()?;
            if relationship.remote_peer == self.local_peer
                || !peers.insert(relationship.remote_peer.clone())
            {
                return Err(PeerHttpError::Configuration(
                    "peer relationships must have unique non-local identities".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

/// Operator-configured outbound peer endpoint. Workflow/model input cannot replace it.
#[derive(Clone, Debug)]
pub struct PeerClientConfig {
    /// Operator-configured base URL.
    pub endpoint: Url,
    /// Authenticated local identity claimed only as a transport cross-check.
    pub local_peer: PeerId,
    /// Exact remote identity required in handshake responses.
    pub expected_remote_peer: PeerId,
    /// Current local boot/session identity.
    pub session: SessionId,
    /// Allowed versions for this relationship.
    pub versions: ProtocolVersionRange,
    /// Resolved bearer credential, redacted and never serialized.
    pub bearer_credential: Arc<SensitiveSecret>,
    /// Explicit development-only exception.
    pub insecure_loopback: InsecureLoopbackMode,
    /// Complete request deadline.
    pub request_timeout: Duration,
    /// Idle wait between resumable observation polls.
    pub observation_poll_interval: Duration,
}

impl PeerClientConfig {
    /// Refuses credentials in URLs, fragments, non-HTTP schemes, and plaintext non-loopback.
    pub fn validate(&self) -> Result<(), PeerHttpError> {
        if !self.endpoint.username().is_empty()
            || self.endpoint.password().is_some()
            || self.endpoint.fragment().is_some()
            || self.bearer_credential.is_empty()
            || self.request_timeout.is_zero()
            || self.observation_poll_interval.is_zero()
        {
            return Err(PeerHttpError::Configuration(
                "peer endpoint, credential, or deadline is invalid".to_owned(),
            ));
        }
        match self.endpoint.scheme() {
            "https" => Ok(()),
            "http"
                if self.insecure_loopback
                    == InsecureLoopbackMode::AllowInsecureLoopbackDevelopment
                    && endpoint_is_loopback(&self.endpoint) =>
            {
                Ok(())
            }
            "http" => Err(PeerHttpError::Configuration(
                "plaintext peer HTTP requires explicitly enabled loopback development mode"
                    .to_owned(),
            )),
            _ => Err(PeerHttpError::Configuration(
                "peer endpoint must use HTTPS".to_owned(),
            )),
        }
    }
}

fn endpoint_is_loopback(endpoint: &Url) -> bool {
    match endpoint.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}
