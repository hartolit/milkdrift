use std::collections::BTreeSet;

use milkdrift_capability::{
    CapabilityDescriptor, CapabilityDescriptorDocument, CapabilityObservation, OperationId,
};
use milkdrift_contracts::is_canonical_blake3_digest;
use serde::{Deserialize, Serialize};

use crate::{CatalogDigest, PeerProtocolError};

const MAX_CATALOG_ENTRIES: usize = 256;
const MAX_CATALOG_UPDATES: usize = 256;
const CATALOG_DIGEST_DOMAIN: &[u8] = b"milkdrift.peer.catalog.v1\0";

/// One exact descriptor generation and filtered operation authority advertisement.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    /// Exact descriptor facts from the hosting adapter.
    pub descriptor: CapabilityDescriptor,
    /// Operations that this authenticated peer may actually invoke.
    pub invocable_operations: BTreeSet<OperationId>,
    /// Current filtered health observation.
    pub observation: CapabilityObservation,
    /// Whether new invocation is closed while accepted work drains.
    pub draining: bool,
}

impl CatalogEntry {
    /// Checks identity and operation consistency without inventing adapter features.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.observation.capability() != self.descriptor.identity()
            || self.invocable_operations.is_empty()
            || self.invocable_operations.len() > 256
            || !self
                .invocable_operations
                .iter()
                .all(|operation| self.descriptor.operation(operation).is_some())
        {
            return Err(PeerProtocolError::InvalidContract(
                "catalog entry identity, operation, or observation mismatch".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Complete expiring catalog observation for one authenticated relationship.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSnapshot {
    /// Monotonic relationship-local generation.
    pub generation: u64,
    /// Boundary time at which this observation was issued.
    pub issued_at_unix_ms: u64,
    /// Hard expiry; consumers must stop new resolution after it.
    pub expires_at_unix_ms: u64,
    /// Stable sorted entries, never durable workflow truth.
    pub entries: Vec<CatalogEntry>,
    /// Canonical digest over generation, expiry, and exact entries.
    pub digest: CatalogDigest,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogSnapshotWire {
    generation: u64,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    entries: Vec<CatalogEntry>,
    digest: CatalogDigest,
}

#[derive(Serialize)]
struct CatalogDigestPayload<'a> {
    schema_version: u32,
    generation: u64,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    entries: &'a [CatalogEntry],
}

impl<'de> Deserialize<'de> for CatalogSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CatalogSnapshotWire::deserialize(deserializer)?;
        let snapshot = Self {
            generation: wire.generation,
            issued_at_unix_ms: wire.issued_at_unix_ms,
            expires_at_unix_ms: wire.expires_at_unix_ms,
            entries: wire.entries,
            digest: wire.digest,
        };
        snapshot.validate().map_err(serde::de::Error::custom)?;
        Ok(snapshot)
    }
}

impl CatalogSnapshot {
    /// Constructs, sorts, validates, and digests a complete snapshot.
    pub fn new(
        generation: u64,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
        mut entries: Vec<CatalogEntry>,
    ) -> Result<Self, PeerProtocolError> {
        entries.sort_by(|left, right| {
            left.descriptor
                .identity()
                .cmp(right.descriptor.identity())
                .then_with(|| {
                    left.descriptor
                        .descriptor_revision()
                        .cmp(&right.descriptor.descriptor_revision())
                })
        });
        let digest = compute_digest(generation, issued_at_unix_ms, expires_at_unix_ms, &entries)?;
        let snapshot = Self {
            generation,
            issued_at_unix_ms,
            expires_at_unix_ms,
            entries,
            digest,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Revalidates defensive bounds and the canonical digest.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.generation == 0
            || self.issued_at_unix_ms >= self.expires_at_unix_ms
            || self.entries.len() > MAX_CATALOG_ENTRIES
        {
            return Err(PeerProtocolError::Bounds {
                location: "catalog",
                reason: "generation, expiry, or entry count is invalid".to_owned(),
            });
        }
        let mut keys = BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            let key = (
                entry.descriptor.identity().clone(),
                entry.descriptor.descriptor_revision(),
            );
            if !keys.insert(key) {
                return Err(PeerProtocolError::InvalidContract(
                    "catalog contains a duplicate descriptor generation".to_owned(),
                ));
            }
        }
        if !is_canonical_blake3_digest(self.digest.as_str())
            || self.digest
                != compute_digest(
                    self.generation,
                    self.issued_at_unix_ms,
                    self.expires_at_unix_ms,
                    &self.entries,
                )?
        {
            return Err(PeerProtocolError::DigestMismatch("catalog"));
        }
        Ok(())
    }

    /// True only while the exact advertised TTL remains live.
    #[must_use]
    pub const fn is_live_at(&self, observed_at_unix_ms: u64) -> bool {
        observed_at_unix_ms <= self.expires_at_unix_ms
    }
}

fn compute_digest(
    generation: u64,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    entries: &[CatalogEntry],
) -> Result<CatalogDigest, PeerProtocolError> {
    for entry in entries {
        let _ = CapabilityDescriptorDocument::new(entry.descriptor.clone())
            .to_canonical_json()
            .map_err(|error| PeerProtocolError::InvalidContract(error.to_string()))?;
    }
    let payload = CatalogDigestPayload {
        schema_version: 1,
        generation,
        issued_at_unix_ms,
        expires_at_unix_ms,
        entries,
    };
    let bytes = milkdrift_contracts::canonical_json_bytes(
        &payload,
        milkdrift_contracts::JsonLimits {
            maximum_depth: 32,
            maximum_string_bytes: 262_144,
            maximum_key_bytes: 192,
            maximum_container_items: 512,
        },
    )
    .map_err(|error| PeerProtocolError::Json(format!("{error:?}")))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(CATALOG_DIGEST_DOMAIN);
    hasher.update(&bytes);
    CatalogDigest::new(format!("b3_{}", hasher.finalize().to_hex()))
}

/// Incremental catalog mutation relative to one exact prior generation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogUpdate {
    /// Required preceding complete or update generation.
    pub prior_generation: u64,
    /// New catalog generation.
    pub generation: u64,
    /// New complete-catalog digest after applying every update.
    pub digest: CatalogDigest,
    /// New hard expiry.
    pub expires_at_unix_ms: u64,
    /// Bounded ordered mutations.
    pub updates: Vec<CatalogUpdateKind>,
}

impl CatalogUpdate {
    /// Validates monotonicity and update bounds.
    pub fn validate(&self) -> Result<(), PeerProtocolError> {
        if self.prior_generation == 0
            || self.generation <= self.prior_generation
            || self.updates.is_empty()
            || self.updates.len() > MAX_CATALOG_UPDATES
            || !is_canonical_blake3_digest(self.digest.as_str())
        {
            return Err(PeerProtocolError::InvalidContract(
                "invalid incremental catalog update".to_owned(),
            ));
        }
        for update in &self.updates {
            if let CatalogUpdateKind::Upsert { entry } = update {
                entry.validate()?;
            }
        }
        Ok(())
    }
}

/// One bounded incremental catalog mutation.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
pub enum CatalogUpdateKind {
    /// Add or replace one exact generation.
    Upsert {
        /// Complete replacement entry.
        entry: Box<CatalogEntry>,
    },
    /// Stop resolving and drain one exact generation.
    Drain {
        /// Exact capability identity.
        capability: milkdrift_capability::CapabilityId,
        /// Exact descriptor generation.
        descriptor_revision: u64,
    },
    /// Remove or revoke one exact generation.
    Remove {
        /// Exact capability identity.
        capability: milkdrift_capability::CapabilityId,
        /// Exact descriptor generation.
        descriptor_revision: u64,
    },
}
