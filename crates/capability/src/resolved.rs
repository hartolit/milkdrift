use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    BoundedJson, CapabilityCategory, CapabilityDescriptor, CapabilityId, ContractError,
    ExecutionTrustClass, ExtensionKey, IdempotencyBehavior, InvocationRequest, OperationContract,
    OperationId, ProviderProfileRef, SCHEMA_VERSION_V1, SideEffectClass,
    document::canonical_json_bytes,
};

const DIGEST_DOMAIN_V1: &[u8] = b"milkdrift.resolved-capability-snapshot.v1\0";
const DIGEST_DOMAIN_V2: &[u8] = b"milkdrift.resolved-capability-snapshot.v2\0";
const RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2: u32 = 2;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<CapabilityCategory>,
    operation: OperationId,
    operation_contract: OperationContract,
    #[serde(default, skip_serializing_if = "execution_trust_unspecified")]
    execution_trust: ExecutionTrustClass,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    descriptor_extensions: BTreeMap<ExtensionKey, BoundedJson>,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolvedCapabilitySnapshotWire {
    capability: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    #[serde(default)]
    category: Option<CapabilityCategory>,
    operation: OperationId,
    operation_contract: OperationContract,
    #[serde(default)]
    execution_trust: ExecutionTrustClass,
    #[serde(default)]
    descriptor_extensions: BTreeMap<ExtensionKey, BoundedJson>,
    digest: String,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotDigestPayload<'a> {
    schema_version: u32,
    capability: &'a CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<&'a ProviderProfileRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<&'a CapabilityCategory>,
    operation: &'a OperationId,
    operation_contract: &'a OperationContract,
    #[serde(skip_serializing_if = "execution_trust_unspecified")]
    execution_trust: ExecutionTrustClass,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    descriptor_extensions: &'a BTreeMap<ExtensionKey, BoundedJson>,
}

const fn execution_trust_unspecified(value: &ExecutionTrustClass) -> bool {
    matches!(value, ExecutionTrustClass::Unspecified)
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
            category: wire.category,
            operation: wire.operation,
            operation_contract: wire.operation_contract,
            execution_trust: wire.execution_trust,
            descriptor_extensions: wire.descriptor_extensions,
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
        let digest = Self::compute_digest(&SnapshotDigestPayload {
            schema_version: RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2,
            capability: descriptor.identity(),
            descriptor_revision: descriptor.descriptor_revision(),
            provider_profile: descriptor.provider_profile(),
            category: Some(descriptor.category()),
            operation,
            operation_contract,
            execution_trust: descriptor.execution_trust(),
            descriptor_extensions: descriptor.extensions(),
        })?;
        Ok(Self {
            capability: descriptor.identity().clone(),
            descriptor_revision: descriptor.descriptor_revision(),
            provider_profile: descriptor.provider_profile().cloned(),
            category: Some(descriptor.category().clone()),
            operation: operation.clone(),
            operation_contract: operation_contract.clone(),
            execution_trust: descriptor.execution_trust(),
            descriptor_extensions: descriptor.extensions().clone(),
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

    /// Exact stable category frozen from the descriptor revision.
    #[must_use]
    pub const fn category(&self) -> Option<&CapabilityCategory> {
        self.category.as_ref()
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

    /// Exact execution-isolation/trust class frozen from the descriptor.
    #[must_use]
    pub const fn execution_trust(&self) -> ExecutionTrustClass {
        self.execution_trust
    }

    /// Bounded descriptor extension facts frozen for attempt provenance.
    #[must_use]
    pub const fn descriptor_extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.descriptor_extensions
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
            || self
                .category
                .as_ref()
                .is_some_and(|category| category != descriptor.category())
            || self.operation_contract != *descriptor_operation
            || self.execution_trust != descriptor.execution_trust()
            || self.descriptor_extensions != *descriptor.extensions()
        {
            return Err(ContractError::InvalidContract(
                "resolved capability snapshot does not match the exact descriptor revision"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// Verifies that an invocation request names this exact resolved selection and obeys its
    /// idempotency contract.
    pub fn validate_request(&self, request: &InvocationRequest) -> Result<(), ContractError> {
        self.validate()?;
        if request.capability() != self.capability()
            || request.operation() != self.operation()
            || request.provider_profile() != self.provider_profile()
        {
            return Err(ContractError::InvalidContract(
                "invocation selection does not equal the resolved capability snapshot".to_owned(),
            ));
        }
        if self.operation_contract.idempotency() == IdempotencyBehavior::Unsupported
            && request.idempotency_key().is_some()
        {
            return Err(ContractError::InvalidContract(
                "an operation advertising unsupported idempotency cannot receive a key".to_owned(),
            ));
        }
        if self.operation_contract.side_effect() == SideEffectClass::IdempotentWrite
            && (self.operation_contract.idempotency() == IdempotencyBehavior::Unsupported
                || request.idempotency_key().is_none())
        {
            return Err(ContractError::InvalidContract(
                "an idempotent write requires advertised idempotency and a stable key".to_owned(),
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
        let schema_version = if self.category.is_some() {
            RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2
        } else {
            SCHEMA_VERSION_V1
        };
        let expected = Self::compute_digest(&SnapshotDigestPayload {
            schema_version,
            capability: &self.capability,
            descriptor_revision: self.descriptor_revision,
            provider_profile: self.provider_profile.as_ref(),
            category: self.category.as_ref(),
            operation: &self.operation,
            operation_contract: &self.operation_contract,
            execution_trust: self.execution_trust,
            descriptor_extensions: &self.descriptor_extensions,
        })?;
        if self.digest != expected {
            return Err(ContractError::InvalidContract(
                "resolved capability snapshot digest does not match its exact facts".to_owned(),
            ));
        }
        Ok(())
    }

    fn compute_digest(payload: &SnapshotDigestPayload<'_>) -> Result<String, ContractError> {
        let canonical_payload = canonical_json_bytes(payload)?;
        let mut hasher = blake3::Hasher::new();
        let domain = match payload.schema_version {
            SCHEMA_VERSION_V1 => DIGEST_DOMAIN_V1,
            RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2 => DIGEST_DOMAIN_V2,
            _ => {
                return Err(ContractError::UnsupportedVersion {
                    document: "resolved capability snapshot digest",
                    found: payload.schema_version,
                    supported: RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V2,
                });
            }
        };
        hasher.update(domain);
        hasher.update(&canonical_payload);
        Ok(hasher.finalize().to_hex().to_string())
    }
}
