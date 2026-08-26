use std::collections::BTreeSet;

use milkdrift_authority::PeerId;
use serde::{Deserialize, Serialize};

use crate::{PeerProtocolError, SessionId};

/// Current incompatible-change protocol major.
pub const PROTOCOL_MAJOR_V1: u16 = 1;
/// Current backward-compatible protocol minor.
pub const PROTOCOL_MINOR_V0: u16 = 0;

/// One selected peer protocol version.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersion {
    /// Incompatible-change line.
    pub major: u16,
    /// Backward-compatible feature line.
    pub minor: u16,
}

impl ProtocolVersion {
    /// Version implemented by this package.
    pub const V1_0: Self = Self {
        major: PROTOCOL_MAJOR_V1,
        minor: PROTOCOL_MINOR_V0,
    };
}

/// Inclusive versions accepted by one configured relationship.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolVersionRange {
    /// Lowest accepted version.
    pub minimum: ProtocolVersion,
    /// Highest accepted version.
    pub maximum: ProtocolVersion,
}

impl ProtocolVersionRange {
    /// Constructs an ordered range within a single major line.
    pub fn new(
        minimum: ProtocolVersion,
        maximum: ProtocolVersion,
    ) -> Result<Self, PeerProtocolError> {
        if minimum.major == 0 || minimum.major != maximum.major || minimum > maximum {
            return Err(PeerProtocolError::InvalidContract(
                "a protocol range must be ordered within one nonzero major version".to_owned(),
            ));
        }
        Ok(Self { minimum, maximum })
    }

    /// Selects the highest mutually supported version. Unknown majors fail closed.
    pub fn negotiate(self, remote: Self) -> Result<ProtocolVersion, PeerProtocolError> {
        if self.minimum.major != remote.minimum.major || self.minimum.major != PROTOCOL_MAJOR_V1 {
            return Err(PeerProtocolError::IncompatibleVersion);
        }
        let minimum = self.minimum.max(remote.minimum);
        let maximum = self.maximum.min(remote.maximum);
        (minimum <= maximum)
            .then_some(maximum)
            .ok_or(PeerProtocolError::IncompatibleVersion)
    }
}

impl Default for ProtocolVersionRange {
    fn default() -> Self {
        Self {
            minimum: ProtocolVersion::V1_0,
            maximum: ProtocolVersion::V1_0,
        }
    }
}

/// Optional feature flags understood at v1 minor boundaries.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureSet {
    /// Resumable sequenced observation polling/streaming.
    pub resumable_observations: bool,
    /// Resumable content-addressed artifact chunks.
    pub resumable_artifacts: bool,
    /// Bounded incremental catalog update support.
    pub incremental_catalog: bool,
}

/// Hard peer limits negotiated down to the lower value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HardLimits {
    /// Maximum complete request document bytes.
    pub document_bytes: u32,
    /// Maximum observation entries per page.
    pub observation_items: u16,
    /// Maximum artifact chunk bytes.
    pub artifact_chunk_bytes: u32,
    /// Maximum simultaneous connections admitted for the peer.
    pub connections: u16,
    /// Maximum simultaneous accepted executions.
    pub executions: u16,
}

impl HardLimits {
    /// Enforces nonzero package ceilings.
    pub fn validate(self) -> Result<Self, PeerProtocolError> {
        if self.document_bytes == 0
            || usize::try_from(self.document_bytes).unwrap_or(usize::MAX)
                > crate::MAX_PEER_DOCUMENT_BYTES
            || self.observation_items == 0
            || usize::from(self.observation_items) > crate::document::MAX_CONTAINER_ITEMS
            || self.artifact_chunk_bytes == 0
            || self.artifact_chunk_bytes > crate::artifact::MAX_ARTIFACT_CHUNK_BYTES
            || self.connections == 0
            || self.executions == 0
        {
            return Err(PeerProtocolError::Bounds {
                location: "session.hard_limits",
                reason: "limits must be nonzero and no larger than protocol ceilings".to_owned(),
            });
        }
        Ok(self)
    }

    /// Computes the strict intersection of two hard-limit offers.
    #[must_use]
    pub fn intersect(self, remote: Self) -> Self {
        Self {
            document_bytes: self.document_bytes.min(remote.document_bytes),
            observation_items: self.observation_items.min(remote.observation_items),
            artifact_chunk_bytes: self.artifact_chunk_bytes.min(remote.artifact_chunk_bytes),
            connections: self.connections.min(remote.connections),
            executions: self.executions.min(remote.executions),
        }
    }
}

impl Default for HardLimits {
    fn default() -> Self {
        Self {
            document_bytes: u32::try_from(crate::MAX_PEER_DOCUMENT_BYTES).unwrap_or(u32::MAX),
            observation_items: 256,
            artifact_chunk_bytes: crate::artifact::MAX_ARTIFACT_CHUNK_BYTES,
            connections: 8,
            executions: 32,
        }
    }
}

/// Heartbeat and accepted-execution lease timing.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatLease {
    /// Transport-independent heartbeat interval.
    pub heartbeat_ms: u64,
    /// Idle timeout after missing heartbeats.
    pub idle_timeout_ms: u64,
    /// Accepted execution lease duration.
    pub execution_lease_ms: u64,
}

impl HeartbeatLease {
    /// Validates bounded nonzero timing and an idle timeout of at least two heartbeats.
    pub fn validate(self) -> Result<Self, PeerProtocolError> {
        if self.heartbeat_ms == 0
            || self.heartbeat_ms > 60_000
            || self.idle_timeout_ms < self.heartbeat_ms.saturating_mul(2)
            || self.idle_timeout_ms > 600_000
            || self.execution_lease_ms == 0
            || self.execution_lease_ms > 86_400_000
        {
            return Err(PeerProtocolError::InvalidContract(
                "invalid heartbeat, idle timeout, or execution lease".to_owned(),
            ));
        }
        Ok(self)
    }
}

/// Server lifecycle advertised independently from connection state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DrainState {
    /// New operations may be accepted.
    Ready,
    /// Existing work may finish but new work is rejected.
    Draining,
    /// The daemon is shutting down.
    ShuttingDown,
}

/// Exact actions granted to an authenticated peer relationship.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerAction {
    /// Read a filtered capability catalog.
    ReadCatalog,
    /// Submit one exact invocation.
    Invoke,
    /// Request cancellation of an exact owned execution.
    Cancel,
    /// Upload verified artifact bytes.
    ArtifactUpload,
    /// Download authorized verified artifact bytes.
    ArtifactDownload,
    /// Read diagnostics or request a reconnect/drain action.
    Administer,
}

/// Default-deny action grant for one authenticated peer.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PeerAuthority {
    /// Explicit allowed actions; empty denies every family.
    pub actions: BTreeSet<PeerAction>,
}

impl PeerAuthority {
    /// Returns whether the exact protocol action is granted.
    #[must_use]
    pub fn permits(&self, action: PeerAction) -> bool {
        self.actions.contains(&action)
    }
}

/// Authenticated session identity facts. Claimed peer fields are cross-checks only.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    /// Authenticated local endpoint identity.
    pub local_peer: PeerId,
    /// Authenticated remote endpoint identity.
    pub remote_peer: PeerId,
    /// Local daemon instance/session identity.
    pub local_session: SessionId,
    /// Remote daemon instance/session identity.
    pub remote_session: SessionId,
}

/// Initial version and identity offer sent after transport authentication.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeRequest {
    /// Caller identity expected from transport authentication.
    pub claimed_peer: PeerId,
    /// Fresh daemon boot/session identity.
    pub session: SessionId,
    /// Configured protocol range.
    pub versions: ProtocolVersionRange,
    /// Optional understood features.
    pub features: FeatureSet,
    /// Caller hard limits.
    pub limits: HardLimits,
}

/// Successful handshake response containing no secrets or internal configuration.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandshakeResponse {
    /// Authenticated server peer identity.
    pub peer: PeerId,
    /// Current server boot/session identity.
    pub session: SessionId,
    /// Negotiated version.
    pub selected_version: ProtocolVersion,
    /// Mutually useful features.
    pub features: FeatureSet,
    /// Negotiated hard limits.
    pub limits: HardLimits,
    /// Heartbeat and accepted execution lease policy.
    pub lease: HeartbeatLease,
    /// Current graceful lifecycle.
    pub drain: DrainState,
}
