use serde::{Deserialize, Serialize};

use crate::{
    CapabilityDescriptor, CapabilityId, ContractError, OperationContract, OperationId,
    ProviderProfileRef, SCHEMA_VERSION_V1, canonical_json_bytes,
};

const DIGEST_DOMAIN: &[u8] = b"milkdrift.resolved-capability-snapshot.v1\0";

/// Immutable exact capability resolution supplied to an executor before dispatch.
///
/// The digest covers every selection fact using a versioned canonical payload and
/// a domain-separated BLAKE3 hash. It does not represent live availability.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCapabilitySnapshot {
    capability: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    operation: OperationId,
    operation_contract: OperationContract,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedCapabilitySnapshotWire {
    capability: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    operation: OperationId,
    operation_contract: OperationContract,
    digest: String,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotDigestPayload<'a> {
    schema_version: u32,
    capability: &'a CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<&'a ProviderProfileRef>,
    operation: &'a OperationId,
    operation_contract: &'a OperationContract,
}

impl<'de> Deserialize<'de> for ResolvedCapabilitySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ResolvedCapabilitySnapshotWire::deserialize(deserializer)?;
        let snapshot = Self {
            capability: wire.capability,
            descriptor_revision: wire.descriptor_revision,
            provider_profile: wire.provider_profile,
            operation: wire.operation,
            operation_contract: wire.operation_contract,
            digest: wire.digest,
        };
        snapshot.validate().map_err(serde::de::Error::custom)?;
        Ok(snapshot)
    }
}

impl ResolvedCapabilitySnapshot {
    /// Resolves and clones one exact operation from an immutable descriptor.
    pub fn from_descriptor(
        descriptor: &CapabilityDescriptor,
        operation: &OperationId,
    ) -> Result<Self, ContractError> {
        let operation_contract = descriptor.operation(operation).ok_or_else(|| {
            ContractError::InvalidContract(format!(
                "descriptor '{}' does not advertise operation '{}'",
                descriptor.identity(),
                operation
            ))
        })?;
        let digest = Self::compute_digest(
            descriptor.identity(),
            descriptor.descriptor_revision(),
            descriptor.provider_profile(),
            operation,
            operation_contract,
        )?;
        Ok(Self {
            capability: descriptor.identity().clone(),
            descriptor_revision: descriptor.descriptor_revision(),
            provider_profile: descriptor.provider_profile().cloned(),
            operation: operation.clone(),
            operation_contract: operation_contract.clone(),
            digest,
        })
    }

    /// Returns the exact capability identity.
    #[must_use]
    pub const fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// Returns the exact immutable descriptor revision.
    #[must_use]
    pub const fn descriptor_revision(&self) -> u64 {
        self.descriptor_revision
    }

    /// Returns the exact provider-profile reference, when selected.
    #[must_use]
    pub const fn provider_profile(&self) -> Option<&ProviderProfileRef> {
        self.provider_profile.as_ref()
    }

    /// Returns the exact selected operation identity.
    #[must_use]
    pub const fn operation(&self) -> &OperationId {
        &self.operation
    }

    /// Returns the exact immutable operation contract supplied to the executor.
    #[must_use]
    pub const fn operation_contract(&self) -> &OperationContract {
        &self.operation_contract
    }

    /// Returns the canonical lowercase BLAKE3 digest of all selection facts.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Verifies that this snapshot is the exact resolution of `descriptor`.
    pub fn validate_against(&self, descriptor: &CapabilityDescriptor) -> Result<(), ContractError> {
        self.validate()?;
        let descriptor_operation = descriptor.operation(&self.operation).ok_or_else(|| {
            ContractError::InvalidContract(format!(
                "descriptor '{}' does not advertise snapshot operation '{}'",
                descriptor.identity(),
                self.operation
            ))
        })?;
        if self.capability != *descriptor.identity()
            || self.descriptor_revision != descriptor.descriptor_revision()
            || self.provider_profile.as_ref() != descriptor.provider_profile()
            || self.operation_contract != *descriptor_operation
        {
            return Err(ContractError::InvalidContract(
                "resolved capability snapshot does not match the exact descriptor revision"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.descriptor_revision == 0 {
            return Err(ContractError::InvalidContract(
                "resolved capability snapshot descriptor revision must be nonzero".to_owned(),
            ));
        }
        let expected = Self::compute_digest(
            &self.capability,
            self.descriptor_revision,
            self.provider_profile.as_ref(),
            &self.operation,
            &self.operation_contract,
        )?;
        if self.digest != expected {
            return Err(ContractError::InvalidContract(
                "resolved capability snapshot digest does not match its exact facts".to_owned(),
            ));
        }
        Ok(())
    }

    fn compute_digest(
        capability: &CapabilityId,
        descriptor_revision: u64,
        provider_profile: Option<&ProviderProfileRef>,
        operation: &OperationId,
        operation_contract: &OperationContract,
    ) -> Result<String, ContractError> {
        let payload = SnapshotDigestPayload {
            schema_version: SCHEMA_VERSION_V1,
            capability,
            descriptor_revision,
            provider_profile,
            operation,
            operation_contract,
        };
        let canonical_payload = canonical_json_bytes(&payload)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(DIGEST_DOMAIN);
        hasher.update(&canonical_payload);
        Ok(hasher.finalize().to_hex().to_string())
    }
}
