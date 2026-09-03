use std::{collections::BTreeSet, fmt, sync::Arc, time::Duration};

use milkdrift_authority::{
    FilesystemScope, NetworkProfileRef, NetworkScope, SecretRef, SensitiveSecret,
};
use milkdrift_capability::{CapabilityId, OperationId, PeerId, SideEffectClass, TrustZone};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PeerAuthority,
    ProtocolVersionRange, SessionId,
};
use milkdrift_workspace::ArtifactSensitivity;
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
    /// Explicit host filesystem roots that allowed remote capabilities may require.
    pub execution_filesystem: Vec<FilesystemScope>,
    /// Explicit credential-free network profiles that allowed remote capabilities may require.
    pub execution_network_profiles: BTreeSet<NetworkProfileRef>,
    /// Explicit network destinations that allowed remote capabilities may require.
    pub execution_network_destinations: BTreeSet<String>,
    /// Explicit secret references that allowed remote capabilities may require.
    pub execution_secrets: BTreeSet<SecretRef>,
    /// Relationship-level execution quotas.
    pub execution_limits: ExecutionLimits,
    /// Maximum concurrent accepted executions.
    pub maximum_concurrent: u16,
    /// Maximum authenticated requests in any fixed one-minute window, per action/operation.
    pub maximum_requests_per_minute: u32,
    /// Maximum sum of artifact bytes accepted per execution.
    pub maximum_artifact_bytes: u64,
    /// Explicit artifact sensitivity classes transferable over this relationship.
    pub artifact_sensitivities: BTreeSet<ArtifactSensitivity>,
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
            .field("execution_filesystem", &self.execution_filesystem)
            .field(
                "execution_network_profiles",
                &self.execution_network_profiles,
            )
            .field(
                "execution_network_destinations",
                &self.execution_network_destinations,
            )
            .field("execution_secrets", &self.execution_secrets)
            .field("execution_limits", &self.execution_limits)
            .field("maximum_concurrent", &self.maximum_concurrent)
            .field(
                "maximum_requests_per_minute",
                &self.maximum_requests_per_minute,
            )
            .field("maximum_artifact_bytes", &self.maximum_artifact_bytes)
            .field("artifact_sensitivities", &self.artifact_sensitivities)
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
        validate_current_protocol_range(self.versions)?;
        self.execution_limits
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        NetworkScope::new(
            self.execution_network_profiles.clone(),
            self.execution_network_destinations.clone(),
        )
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        if self.bearer_credential.is_empty()
            || self.bearer_credential.len() > 8_192
            || self.maximum_concurrent == 0
            || self.maximum_requests_per_minute == 0
            || self.maximum_requests_per_minute > 100_000
            || self.catalog_ttl_ms == 0
            || self.catalog_ttl_ms > 300_000
            || self.expires_at_unix_ms == 0
            || self.artifact_sensitivities.len() > 3
        {
            return Err(PeerHttpError::Configuration(
                "peer credential, concurrency, TTL, or expiry is invalid".to_owned(),
            ));
        }
        Ok(())
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
    /// Fixed serving-worker and durable admission bounds.
    pub workers: PeerWorkerConfig,
}

impl PeerServerConfig {
    /// Validates all bounds and rejects duplicate remote identities.
    pub fn validate(&self) -> Result<(), PeerHttpError> {
        validate_current_protocol_range(self.versions)?;
        self.limits
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        self.lease
            .validate()
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        self.workers.validate()?;
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

/// Fixed bounded serving-peer worker, queue, recovery and retention policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerWorkerConfig {
    /// Fixed worker thread count.
    pub threads: u16,
    /// Global accepted nonterminal ceiling across relationships.
    pub maximum_global_active: u32,
    /// Durable accepted pre-entry queue ceiling.
    pub maximum_dispatch_queue: u32,
    /// Maximum complete terminal/uncertain records retaining detailed hot observations.
    pub maximum_hot_terminal_records: u64,
    /// Maximum oldest eligible records compacted by one transaction.
    pub archive_batch_size: u32,
    /// Minimum terminal age before detailed observation rows are compacted.
    pub observation_hot_retention: Duration,
    /// Maximum claims recovered in one transaction/page.
    pub recovery_page: u16,
    /// Idle durable-queue poll interval.
    pub poll_interval: Duration,
}

impl Default for PeerWorkerConfig {
    fn default() -> Self {
        Self {
            threads: 4,
            maximum_global_active: 256,
            maximum_dispatch_queue: 256,
            maximum_hot_terminal_records: 10_000,
            archive_batch_size: 256,
            observation_hot_retention: Duration::from_secs(24 * 60 * 60),
            recovery_page: 128,
            poll_interval: Duration::from_millis(100),
        }
    }
}

impl PeerWorkerConfig {
    fn validate(self) -> Result<(), PeerHttpError> {
        if self.threads == 0
            || self.threads > 256
            || self.maximum_global_active == 0
            || self.maximum_dispatch_queue == 0
            || self.maximum_dispatch_queue > self.maximum_global_active
            || self.maximum_hot_terminal_records < u64::from(self.maximum_global_active)
            || self.maximum_hot_terminal_records > 1_000_000
            || self.archive_batch_size == 0
            || u64::from(self.archive_batch_size) > self.maximum_hot_terminal_records
            || self.observation_hot_retention.is_zero()
            || self.observation_hot_retention > Duration::from_secs(365 * 24 * 60 * 60)
            || self.recovery_page == 0
            || self.poll_interval.is_zero()
            || self.poll_interval > Duration::from_secs(60)
        {
            return Err(PeerHttpError::Configuration(
                "peer worker, queue, recovery, or retention bounds are invalid".to_owned(),
            ));
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
        validate_current_protocol_range(self.versions)?;
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

fn validate_current_protocol_range(versions: ProtocolVersionRange) -> Result<(), PeerHttpError> {
    if versions != ProtocolVersionRange::default() {
        return Err(PeerHttpError::Configuration(
            "peer protocol configuration must select exactly v1.2".to_owned(),
        ));
    }
    Ok(())
}

fn endpoint_is_loopback(endpoint: &Url) -> bool {
    match endpoint.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use milkdrift_peer_protocol::{ProtocolVersion, ProtocolVersionRange};

    use super::validate_current_protocol_range;

    #[test]
    fn configured_protocol_ranges_must_name_only_v1_2() -> Result<(), Box<dyn std::error::Error>> {
        assert!(validate_current_protocol_range(ProtocolVersionRange::default()).is_ok());
        for (minimum, maximum) in [(1_u16, 1_u16), (1, 2), (2, 3), (3, 3)] {
            let range = ProtocolVersionRange::new(
                ProtocolVersion {
                    major: 1,
                    minor: minimum,
                },
                ProtocolVersion {
                    major: 1,
                    minor: maximum,
                },
            )?;
            assert!(validate_current_protocol_range(range).is_err());
        }
        Ok(())
    }
}
