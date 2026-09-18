use super::ServingError;
use milkdrift_authority::{
    FilesystemScope, NetworkProfileRef, NetworkScope, SecretRef, SensitiveSecret,
};
use milkdrift_capability::{CapabilityId, OperationId, PeerId, SideEffectClass, TrustZone};
use milkdrift_peer_protocol::{
    DelegationRef, ExecutionLimits, HardLimits, HeartbeatLease, PeerAuthority,
    ProtocolVersionRange, SessionId,
};
use milkdrift_workspace::ArtifactSensitivity;
use std::{collections::BTreeSet, fmt, sync::Arc, time::Duration};
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
    pub fn validate(&self) -> Result<(), ServingError> {
        validate_current_protocol_range(self.versions)?;
        self.execution_limits
            .validate()
            .map_err(|error| ServingError::Configuration(error.to_string()))?;
        NetworkScope::new(
            self.execution_network_profiles.clone(),
            self.execution_network_destinations.clone(),
        )
        .map_err(|error| ServingError::Configuration(error.to_string()))?;
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
            return Err(ServingError::Configuration(
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

/// Independent client grants and bounds supplied by the daemon's authentication owner.
/// These are configuration facts; invocation payloads cannot create or replace them.
#[derive(Clone, Debug)]
pub struct ServingClientPolicy {
    /// Server-owned grants for independently authenticated actors.
    pub grants: Vec<milkdrift_authority::AuthorityGrant>,
    /// Current configured revocation generations.
    pub revocations: std::collections::BTreeMap<milkdrift_authority::GrantId, u64>,
    /// Maximum per-call resource ceilings for independent work.
    pub execution_limits: ExecutionLimits,
    /// Maximum active accepted calls per actor.
    pub maximum_concurrent: u32,
    /// Maximum authenticated calls per actor per minute.
    pub maximum_requests_per_minute: u32,
    /// How long an exact direct catalog may be selected for new acceptance.
    pub catalog_ttl_ms: u64,
}

impl ServingClientPolicy {
    pub(super) fn validate(&self) -> Result<(), ServingError> {
        self.execution_limits
            .validate()
            .map_err(|error| ServingError::Configuration(error.to_string()))?;
        if self.grants.len() > 256
            || self.maximum_concurrent == 0
            || self.maximum_requests_per_minute == 0
            || self.maximum_requests_per_minute > 100_000
            || self.catalog_ttl_ms == 0
            || self.catalog_ttl_ms > 300_000
        {
            return Err(ServingError::Configuration(
                "independent client policy exceeds serving bounds".to_owned(),
            ));
        }
        let mut actors = BTreeSet::new();
        for grant in &self.grants {
            if !actors.insert(grant.actor())
                || grant
                    .revision()
                    .checked_add(grant.revocation_generation())
                    .is_none()
            {
                return Err(ServingError::Configuration(
                    "client actors must be unique and authority generations must fit".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl PeerServerConfig {
    /// Validates all bounds and rejects duplicate remote identities.
    pub fn validate(&self) -> Result<(), ServingError> {
        validate_current_protocol_range(self.versions)?;
        self.limits
            .validate()
            .map_err(|error| ServingError::Configuration(error.to_string()))?;
        self.lease
            .validate()
            .map_err(|error| ServingError::Configuration(error.to_string()))?;
        self.workers.validate()?;
        if self.relationships.len() > 256 {
            return Err(ServingError::Configuration(
                "at most 256 peer relationships are supported".to_owned(),
            ));
        }
        let mut peers = BTreeSet::new();
        for relationship in &self.relationships {
            relationship.validate()?;
            if relationship.remote_peer == self.local_peer
                || !peers.insert(relationship.remote_peer.clone())
            {
                return Err(ServingError::Configuration(
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
    fn validate(self) -> Result<(), ServingError> {
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
            return Err(ServingError::Configuration(
                "peer worker, queue, recovery, or retention bounds are invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

fn validate_current_protocol_range(versions: ProtocolVersionRange) -> Result<(), ServingError> {
    if versions != ProtocolVersionRange::default() {
        return Err(ServingError::Configuration(
            "peer protocol configuration must select exactly v1.4".to_owned(),
        ));
    }
    Ok(())
}
