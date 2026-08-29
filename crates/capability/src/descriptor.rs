use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    BoundedJson, CapabilityId, ContractError, ExtensionKey, FeatureId, OperationId, PeerId,
    ProviderProfileRef, SchemaId, TrustZone, bounded::validate_extensions,
};

const MAX_OPERATIONS: usize = 256;
const MAX_FEATURES: usize = 256;
const MAX_LABELS: usize = 64;

/// Broad stable category used for discovery without closing the operation namespace.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(
    rename_all = "snake_case",
    tag = "type",
    content = "identity",
    deny_unknown_fields
)]
pub enum CapabilityCategory {
    /// Generative or analytical model operation.
    Model,
    /// General tool operation.
    Tool,
    /// Long-lived or one-shot external process.
    Process,
    /// Human-mediated capability.
    Human,
    /// Capability reached through a peer machine.
    Peer,
    /// Open category identified with a namespace.
    Custom(FeatureId),
}

/// Streaming shapes an operation explicitly supports.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingMode {
    /// Only one terminal response is produced.
    None,
    /// Bounded progress events may be emitted before the terminal event.
    Progress,
    /// Bounded output fragments may be emitted before the terminal event.
    OutputFragments,
}

/// Executor cancellation behavior advertised for an operation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancellationBehavior {
    /// Cancellation is not supported.
    Unsupported,
    /// Cancellation is attempted but a terminal result can remain uncertain.
    BestEffort,
    /// The executor promises an acknowledged terminal cancellation boundary.
    Acknowledged,
}

/// How an executor interprets idempotency keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdempotencyBehavior {
    /// Idempotency keys are not accepted.
    Unsupported,
    /// Duplicate keys are scoped only to this capability identity.
    CapabilityScoped,
    /// Duplicate keys are scoped to the named provider profile.
    ProviderProfileScoped,
}

/// Potential side effects of an invocation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    /// No externally visible mutation is expected.
    None,
    /// Reads external or protected state without intending to mutate it.
    ReadOnly,
    /// Writes are expected and are designed to be idempotent with a key.
    IdempotentWrite,
    /// Writes may be externally visible and non-idempotent.
    NonIdempotentWrite,
    /// The executor cannot determine whether externally visible effects occurred.
    Unknown,
}

/// Where execution is expected to occur.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// Same machine as the owning daemon.
    Local,
    /// A separately operated peer machine.
    Peer,
    /// A remote provider or service.
    Remote,
    /// Location is intentionally not claimed.
    Unspecified,
}

/// Exact execution-isolation/trust fact advertised by a capability generation.
///
/// This is intentionally separate from operator-defined trust-zone labels. A
/// trusted host process is not interchangeable with an enforced sandbox even
/// when both are local or share the same policy zone.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTrustClass {
    /// The capability does not execute an operating-system process, or makes no
    /// process-isolation claim.
    #[default]
    Unspecified,
    /// A process executes with the daemon account's host authority while
    /// Milkdrift mediates declared arguments, environment, inputs, and outputs.
    TrustedHostProcess,
    /// A separate adapter enforces and advertises a complete container,
    /// namespace, or virtual-machine isolation contract.
    SandboxedProcess,
}

const fn execution_trust_unspecified(value: &ExecutionTrustClass) -> bool {
    matches!(value, ExecutionTrustClass::Unspecified)
}

/// A named schema and its independently versioned contract body.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaContract {
    id: SchemaId,
    version: u32,
    schema: BoundedJson,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaContractWire {
    id: SchemaId,
    version: u32,
    schema: BoundedJson,
}

impl<'de> Deserialize<'de> for SchemaContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SchemaContractWire::deserialize(deserializer)?;
        Self::new(wire.id, wire.version, wire.schema).map_err(serde::de::Error::custom)
    }
}

impl SchemaContract {
    /// Constructs a nonzero schema version with a bounded schema value.
    pub fn new(id: SchemaId, version: u32, schema: BoundedJson) -> Result<Self, ContractError> {
        if version == 0 {
            return Err(ContractError::InvalidContract(
                "schema version must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            id,
            version,
            schema,
        })
    }

    /// Returns the schema identity.
    #[must_use]
    pub fn id(&self) -> &SchemaId {
        &self.id
    }

    /// Returns the schema version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the bounded schema document.
    #[must_use]
    pub const fn schema(&self) -> &BoundedJson {
        &self.schema
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.version == 0 {
            return Err(ContractError::InvalidContract(
                "schema version must be nonzero".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Explicit feature advertisement, optionally with a settings schema.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureContract {
    identity: FeatureId,
    settings_schema: Option<SchemaContract>,
}

impl FeatureContract {
    /// Creates an advertised feature.
    #[must_use]
    pub const fn new(identity: FeatureId, settings_schema: Option<SchemaContract>) -> Self {
        Self {
            identity,
            settings_schema,
        }
    }

    /// Returns the namespaced feature identity.
    #[must_use]
    pub const fn identity(&self) -> &FeatureId {
        &self.identity
    }

    /// Returns the optional schema governing this feature's settings.
    #[must_use]
    pub const fn settings_schema(&self) -> Option<&SchemaContract> {
        self.settings_schema.as_ref()
    }

    fn validate(&self) -> Result<(), ContractError> {
        if let Some(schema) = &self.settings_schema {
            schema.validate()?;
        }
        Ok(())
    }
}

/// Provider-neutral contract for one namespaced operation.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationContract {
    input: SchemaContract,
    output: SchemaContract,
    streaming: BTreeSet<StreamingMode>,
    cancellation: CancellationBehavior,
    idempotency: IdempotencyBehavior,
    side_effect: SideEffectClass,
    features: BTreeMap<FeatureId, FeatureContract>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationContractWire {
    input: SchemaContract,
    output: SchemaContract,
    streaming: BTreeSet<StreamingMode>,
    cancellation: CancellationBehavior,
    idempotency: IdempotencyBehavior,
    side_effect: SideEffectClass,
    features: BTreeMap<FeatureId, FeatureContract>,
}

impl<'de> Deserialize<'de> for OperationContract {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = OperationContractWire::deserialize(deserializer)?;
        Self::new(
            wire.input,
            wire.output,
            wire.streaming,
            wire.cancellation,
            wire.idempotency,
            wire.side_effect,
            wire.features,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl OperationContract {
    /// Constructs and validates an operation contract.
    pub fn new(
        input: SchemaContract,
        output: SchemaContract,
        streaming: BTreeSet<StreamingMode>,
        cancellation: CancellationBehavior,
        idempotency: IdempotencyBehavior,
        side_effect: SideEffectClass,
        features: BTreeMap<FeatureId, FeatureContract>,
    ) -> Result<Self, ContractError> {
        input.validate()?;
        output.validate()?;
        if streaming.is_empty() {
            return Err(ContractError::InvalidContract(
                "an operation must advertise at least one streaming mode".to_owned(),
            ));
        }
        if features.len() > MAX_FEATURES {
            return Err(ContractError::Bounds {
                location: "operation.features".to_owned(),
                reason: format!("at most {MAX_FEATURES} features are allowed"),
            });
        }
        for (identity, feature) in &features {
            if identity != feature.identity() {
                return Err(ContractError::InvalidContract(
                    "feature map keys must equal feature identities".to_owned(),
                ));
            }
            feature.validate()?;
        }
        validate_side_effect_idempotency(side_effect, idempotency)?;
        Ok(Self {
            input,
            output,
            streaming,
            cancellation,
            idempotency,
            side_effect,
            features,
        })
    }

    /// Returns the operation input schema.
    #[must_use]
    pub const fn input(&self) -> &SchemaContract {
        &self.input
    }

    /// Returns the operation output schema.
    #[must_use]
    pub const fn output(&self) -> &SchemaContract {
        &self.output
    }

    /// Returns every supported streaming shape.
    #[must_use]
    pub const fn streaming(&self) -> &BTreeSet<StreamingMode> {
        &self.streaming
    }

    /// Returns the advertised cancellation behavior.
    #[must_use]
    pub const fn cancellation(&self) -> CancellationBehavior {
        self.cancellation
    }

    /// Returns the advertised idempotency behavior.
    #[must_use]
    pub const fn idempotency(&self) -> IdempotencyBehavior {
        self.idempotency
    }

    /// Returns the operation's maximum expected side-effect class.
    #[must_use]
    pub const fn side_effect(&self) -> SideEffectClass {
        self.side_effect
    }

    /// Returns the advertised features.
    #[must_use]
    pub const fn features(&self) -> &BTreeMap<FeatureId, FeatureContract> {
        &self.features
    }

    fn validate(&self) -> Result<(), ContractError> {
        self.input.validate()?;
        self.output.validate()?;
        if self.streaming.is_empty() || self.features.len() > MAX_FEATURES {
            return Err(ContractError::InvalidContract(
                "operation needs a streaming mode and bounded features".to_owned(),
            ));
        }
        for (identity, feature) in &self.features {
            if identity != feature.identity() {
                return Err(ContractError::InvalidContract(
                    "feature map keys must equal feature identities".to_owned(),
                ));
            }
            feature.validate()?;
        }
        validate_side_effect_idempotency(self.side_effect, self.idempotency)?;
        Ok(())
    }
}

fn validate_side_effect_idempotency(
    side_effect: SideEffectClass,
    idempotency: IdempotencyBehavior,
) -> Result<(), ContractError> {
    if side_effect == SideEffectClass::IdempotentWrite
        && idempotency == IdempotencyBehavior::Unsupported
    {
        return Err(ContractError::InvalidContract(
            "an idempotent-write operation must advertise an idempotency-key scope".to_owned(),
        ));
    }
    Ok(())
}

/// Immutable concurrency and admission limits.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConstraints {
    max_concurrent: u32,
    max_queued: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionConstraintsWire {
    max_concurrent: u32,
    max_queued: u32,
}

impl<'de> Deserialize<'de> for AdmissionConstraints {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AdmissionConstraintsWire::deserialize(deserializer)?;
        Self::new(wire.max_concurrent, wire.max_queued).map_err(serde::de::Error::custom)
    }
}

impl AdmissionConstraints {
    /// Constructs limits; concurrency must be nonzero.
    pub fn new(max_concurrent: u32, max_queued: u32) -> Result<Self, ContractError> {
        if max_concurrent == 0 {
            return Err(ContractError::InvalidContract(
                "max_concurrent must be nonzero".to_owned(),
            ));
        }
        Ok(Self {
            max_concurrent,
            max_queued,
        })
    }

    /// Maximum simultaneously admitted invocations.
    #[must_use]
    pub const fn max_concurrent(&self) -> u32 {
        self.max_concurrent
    }

    /// Maximum invocations allowed to wait for admission.
    #[must_use]
    pub const fn max_queued(&self) -> u32 {
        self.max_queued
    }
}

/// Optional estimates supplied by an adapter without implying enforcement.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceObservations {
    /// Adapter-supplied expected cost in millionths of a currency unit.
    estimated_cost_micros: Option<u64>,
    /// Adapter-supplied expected duration in milliseconds.
    estimated_duration_ms: Option<u64>,
    /// ISO 4217 currency code when a cost is supplied.
    currency: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceObservationsWire {
    estimated_cost_micros: Option<u64>,
    estimated_duration_ms: Option<u64>,
    currency: Option<String>,
}

impl<'de> Deserialize<'de> for ResourceObservations {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = ResourceObservationsWire::deserialize(deserializer)?;
        Self::new(
            wire.estimated_cost_micros,
            wire.estimated_duration_ms,
            wire.currency,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl ResourceObservations {
    /// Constructs validated optional resource estimates.
    pub fn new(
        estimated_cost_micros: Option<u64>,
        estimated_duration_ms: Option<u64>,
        currency: Option<String>,
    ) -> Result<Self, ContractError> {
        let observations = Self {
            estimated_cost_micros,
            estimated_duration_ms,
            currency,
        };
        observations.validate()?;
        Ok(observations)
    }

    /// Returns the expected cost in millionths, when observed.
    #[must_use]
    pub const fn estimated_cost_micros(&self) -> Option<u64> {
        self.estimated_cost_micros
    }

    /// Returns the expected duration in milliseconds, when observed.
    #[must_use]
    pub const fn estimated_duration_ms(&self) -> Option<u64> {
        self.estimated_duration_ms
    }

    /// Returns the currency associated with the expected cost.
    #[must_use]
    pub fn currency(&self) -> Option<&str> {
        self.currency.as_deref()
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.estimated_cost_micros.is_some() != self.currency.is_some() {
            return Err(ContractError::InvalidContract(
                "estimated cost and currency must be supplied together".to_owned(),
            ));
        }
        if let Some(currency) = &self.currency
            && (currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_uppercase()))
        {
            return Err(ContractError::InvalidContract(
                "currency must be a three-letter uppercase ISO code".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Immutable capability description. Live state belongs in [`CapabilityObservation`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapabilityDescriptor {
    identity: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    category: CapabilityCategory,
    operations: BTreeMap<OperationId, OperationContract>,
    admission: AdmissionConstraints,
    locality: Locality,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer: Option<PeerId>,
    trust_zones: BTreeSet<TrustZone>,
    #[serde(default, skip_serializing_if = "execution_trust_unspecified")]
    execution_trust: ExecutionTrustClass,
    resource_observations: Option<ResourceObservations>,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorWire {
    identity: CapabilityId,
    descriptor_revision: u64,
    provider_profile: Option<ProviderProfileRef>,
    category: CapabilityCategory,
    operations: BTreeMap<OperationId, OperationContract>,
    admission: AdmissionConstraints,
    locality: Locality,
    #[serde(default)]
    peer: Option<PeerId>,
    trust_zones: BTreeSet<TrustZone>,
    #[serde(default)]
    execution_trust: ExecutionTrustClass,
    resource_observations: Option<ResourceObservations>,
    labels: BTreeSet<String>,
    extensions: BTreeMap<ExtensionKey, BoundedJson>,
}

impl<'de> Deserialize<'de> for CapabilityDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DescriptorWire::deserialize(deserializer)?;
        DescriptorBuilder::new(
            wire.identity,
            wire.descriptor_revision,
            wire.category,
            wire.admission,
            wire.locality,
        )
        .provider_profile(wire.provider_profile)
        .operations(wire.operations)
        .peer(wire.peer)
        .trust_zones(wire.trust_zones)
        .execution_trust(wire.execution_trust)
        .resource_observations(wire.resource_observations)
        .labels(wire.labels)
        .extensions(wire.extensions)
        .build()
        .map_err(serde::de::Error::custom)
    }
}

impl CapabilityDescriptor {
    /// Stable capability identity.
    #[must_use]
    pub const fn identity(&self) -> &CapabilityId {
        &self.identity
    }

    /// Immutable descriptor revision.
    #[must_use]
    pub const fn descriptor_revision(&self) -> u64 {
        self.descriptor_revision
    }

    /// Opaque provider-profile reference selected by this descriptor.
    #[must_use]
    pub const fn provider_profile(&self) -> Option<&ProviderProfileRef> {
        self.provider_profile.as_ref()
    }

    /// Stable discovery category.
    #[must_use]
    pub const fn category(&self) -> &CapabilityCategory {
        &self.category
    }

    /// Advertised operations keyed by their exact identities.
    #[must_use]
    pub const fn operations(&self) -> &BTreeMap<OperationId, OperationContract> {
        &self.operations
    }

    /// Looks up one exact advertised operation.
    #[must_use]
    pub fn operation(&self, identity: &OperationId) -> Option<&OperationContract> {
        self.operations.get(identity)
    }

    /// Immutable admission limits advertised by the capability.
    #[must_use]
    pub const fn admission(&self) -> &AdmissionConstraints {
        &self.admission
    }

    /// Advertised execution locality.
    #[must_use]
    pub const fn locality(&self) -> Locality {
        self.locality
    }

    /// Authenticated peer owning this generation, present only for peer capabilities.
    #[must_use]
    pub const fn peer(&self) -> Option<&PeerId> {
        self.peer.as_ref()
    }

    /// Trust zones in which this capability may execute.
    #[must_use]
    pub const fn trust_zones(&self) -> &BTreeSet<TrustZone> {
        &self.trust_zones
    }

    /// Exact execution-isolation/trust class for this generation.
    #[must_use]
    pub const fn execution_trust(&self) -> ExecutionTrustClass {
        self.execution_trust
    }

    /// Optional adapter-supplied resource estimates.
    #[must_use]
    pub const fn resource_observations(&self) -> Option<&ResourceObservations> {
        self.resource_observations.as_ref()
    }

    /// Human-readable discovery labels.
    #[must_use]
    pub const fn labels(&self) -> &BTreeSet<String> {
        &self.labels
    }

    /// Bounded namespaced descriptor extensions.
    #[must_use]
    pub const fn extensions(&self) -> &BTreeMap<ExtensionKey, BoundedJson> {
        &self.extensions
    }

    /// Returns whether this descriptor satisfies a selection requirement.
    #[must_use]
    pub fn matches(&self, requirement: &CapabilityRequirement) -> RequirementMatch {
        let mut reasons = Vec::new();
        if let Some(exact) = &requirement.exact_capability
            && exact != &self.identity
        {
            reasons.push("capability_identity".to_owned());
        }
        if let Some(profile) = &requirement.provider_profile
            && self.provider_profile.as_ref() != Some(profile)
        {
            reasons.push("provider_profile".to_owned());
        }
        if !requirement.categories.is_empty() && !requirement.categories.contains(&self.category) {
            reasons.push("category".to_owned());
        }
        match self.operations.get(&requirement.operation) {
            None => reasons.push("operation".to_owned()),
            Some(operation) => {
                if !requirement
                    .required_features
                    .iter()
                    .all(|feature| operation.features.contains_key(feature))
                {
                    reasons.push("features".to_owned());
                }
                if let Some(streaming) = requirement.streaming
                    && !operation.streaming.contains(&streaming)
                {
                    reasons.push("streaming".to_owned());
                }
                if requirement.cancellation_required
                    && operation.cancellation == CancellationBehavior::Unsupported
                {
                    reasons.push("cancellation".to_owned());
                }
                if operation.side_effect > requirement.maximum_side_effect {
                    reasons.push("side_effect".to_owned());
                }
            }
        }
        if !requirement
            .trust_zones
            .iter()
            .all(|zone| self.trust_zones.contains(zone))
        {
            reasons.push("trust_zone".to_owned());
        }
        if requirement
            .execution_trust
            .is_some_and(|required| required != self.execution_trust)
        {
            reasons.push("execution_trust".to_owned());
        }
        RequirementMatch { reasons }
    }
}

/// Builder that publishes a descriptor only after complete validation.
pub struct DescriptorBuilder {
    descriptor: CapabilityDescriptor,
}

impl DescriptorBuilder {
    /// Starts a descriptor with required immutable facts.
    #[must_use]
    pub fn new(
        identity: CapabilityId,
        descriptor_revision: u64,
        category: CapabilityCategory,
        admission: AdmissionConstraints,
        locality: Locality,
    ) -> Self {
        Self {
            descriptor: CapabilityDescriptor {
                identity,
                descriptor_revision,
                provider_profile: None,
                category,
                operations: BTreeMap::new(),
                admission,
                locality,
                peer: None,
                trust_zones: BTreeSet::new(),
                execution_trust: ExecutionTrustClass::Unspecified,
                resource_observations: None,
                labels: BTreeSet::new(),
                extensions: BTreeMap::new(),
            },
        }
    }

    /// Sets the opaque provider profile reference.
    #[must_use]
    pub fn provider_profile(mut self, value: Option<ProviderProfileRef>) -> Self {
        self.descriptor.provider_profile = value;
        self
    }

    /// Replaces advertised operations.
    #[must_use]
    pub fn operations(mut self, value: BTreeMap<OperationId, OperationContract>) -> Self {
        self.descriptor.operations = value;
        self
    }

    /// Binds this descriptor to one authenticated peer identity.
    #[must_use]
    pub fn peer(mut self, value: Option<PeerId>) -> Self {
        self.descriptor.peer = value;
        self
    }

    /// Replaces trust-zone labels.
    #[must_use]
    pub fn trust_zones(mut self, value: BTreeSet<TrustZone>) -> Self {
        self.descriptor.trust_zones = value;
        self
    }

    /// Sets the exact execution-isolation/trust class.
    #[must_use]
    pub const fn execution_trust(mut self, value: ExecutionTrustClass) -> Self {
        self.descriptor.execution_trust = value;
        self
    }

    /// Sets optional resource estimates.
    #[must_use]
    pub fn resource_observations(mut self, value: Option<ResourceObservations>) -> Self {
        self.descriptor.resource_observations = value;
        self
    }

    /// Replaces human-readable labels.
    #[must_use]
    pub fn labels(mut self, value: BTreeSet<String>) -> Self {
        self.descriptor.labels = value;
        self
    }

    /// Replaces bounded namespaced extensions.
    #[must_use]
    pub fn extensions(mut self, value: BTreeMap<ExtensionKey, BoundedJson>) -> Self {
        self.descriptor.extensions = value;
        self
    }

    /// Validates all facts and publishes the immutable descriptor.
    pub fn build(self) -> Result<CapabilityDescriptor, ContractError> {
        let descriptor = self.descriptor;
        if descriptor.descriptor_revision == 0 {
            return Err(ContractError::InvalidContract(
                "descriptor revision must be nonzero".to_owned(),
            ));
        }
        if descriptor.operations.is_empty() || descriptor.operations.len() > MAX_OPERATIONS {
            return Err(ContractError::Bounds {
                location: "descriptor.operations".to_owned(),
                reason: format!("operation count must be between 1 and {MAX_OPERATIONS}"),
            });
        }
        for operation in descriptor.operations.values() {
            operation.validate()?;
        }
        if descriptor.admission.max_concurrent == 0 {
            return Err(ContractError::InvalidContract(
                "max_concurrent must be nonzero".to_owned(),
            ));
        }
        if descriptor.peer.is_some() != (descriptor.locality == Locality::Peer) {
            return Err(ContractError::InvalidContract(
                "peer identity must be present exactly for peer-locality descriptors".to_owned(),
            ));
        }
        if descriptor.labels.len() > MAX_LABELS
            || descriptor
                .labels
                .iter()
                .any(|label| label.is_empty() || label.len() > 96)
        {
            return Err(ContractError::Bounds {
                location: "descriptor.labels".to_owned(),
                reason: format!("at most {MAX_LABELS} nonempty labels of 96 bytes are allowed"),
            });
        }
        if descriptor.trust_zones.len() > MAX_LABELS {
            return Err(ContractError::Bounds {
                location: "descriptor.trust_zones".to_owned(),
                reason: format!("at most {MAX_LABELS} trust zones are allowed"),
            });
        }
        if let Some(resources) = &descriptor.resource_observations {
            resources.validate()?;
        }
        validate_extensions(&descriptor.extensions)?;
        Ok(descriptor)
    }
}

/// Selection expression carried by a blueprint task.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    exact_capability: Option<CapabilityId>,
    provider_profile: Option<ProviderProfileRef>,
    categories: BTreeSet<CapabilityCategory>,
    operation: OperationId,
    required_features: BTreeSet<FeatureId>,
    streaming: Option<StreamingMode>,
    cancellation_required: bool,
    maximum_side_effect: SideEffectClass,
    trust_zones: BTreeSet<TrustZone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_trust: Option<ExecutionTrustClass>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityRequirementWire {
    exact_capability: Option<CapabilityId>,
    provider_profile: Option<ProviderProfileRef>,
    categories: BTreeSet<CapabilityCategory>,
    operation: OperationId,
    required_features: BTreeSet<FeatureId>,
    streaming: Option<StreamingMode>,
    cancellation_required: bool,
    maximum_side_effect: SideEffectClass,
    trust_zones: BTreeSet<TrustZone>,
    #[serde(default)]
    execution_trust: Option<ExecutionTrustClass>,
}

impl<'de> Deserialize<'de> for CapabilityRequirement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CapabilityRequirementWire::deserialize(deserializer)?;
        let requirement = Self {
            exact_capability: wire.exact_capability,
            provider_profile: wire.provider_profile,
            categories: wire.categories,
            operation: wire.operation,
            required_features: wire.required_features,
            streaming: wire.streaming,
            cancellation_required: wire.cancellation_required,
            maximum_side_effect: wire.maximum_side_effect,
            trust_zones: wire.trust_zones,
            execution_trust: wire.execution_trust,
        };
        requirement.validate().map_err(serde::de::Error::custom)?;
        Ok(requirement)
    }
}

impl CapabilityRequirement {
    /// Constructs a constraint expression around one exact operation.
    pub fn new(operation: OperationId) -> Self {
        Self {
            exact_capability: None,
            provider_profile: None,
            categories: BTreeSet::new(),
            operation,
            required_features: BTreeSet::new(),
            streaming: None,
            cancellation_required: false,
            maximum_side_effect: SideEffectClass::Unknown,
            trust_zones: BTreeSet::new(),
            execution_trust: None,
        }
    }

    /// Pins selection to one capability identity.
    #[must_use]
    pub fn exact(mut self, identity: CapabilityId) -> Self {
        self.exact_capability = Some(identity);
        self
    }

    /// Pins selection to an opaque provider profile.
    #[must_use]
    pub fn provider_profile(mut self, profile: ProviderProfileRef) -> Self {
        self.provider_profile = Some(profile);
        self
    }

    /// Adds an accepted stable category.
    #[must_use]
    pub fn category(mut self, category: CapabilityCategory) -> Self {
        self.categories.insert(category);
        self
    }

    /// Requires one explicitly advertised feature.
    #[must_use]
    pub fn feature(mut self, feature: FeatureId) -> Self {
        self.required_features.insert(feature);
        self
    }

    /// Requires one streaming mode.
    #[must_use]
    pub fn streaming(mut self, streaming: StreamingMode) -> Self {
        self.streaming = Some(streaming);
        self
    }

    /// Requires some advertised cancellation behavior.
    #[must_use]
    pub const fn cancellation(mut self, required: bool) -> Self {
        self.cancellation_required = required;
        self
    }

    /// Sets the highest acceptable side-effect class.
    #[must_use]
    pub const fn maximum_side_effect(mut self, maximum: SideEffectClass) -> Self {
        self.maximum_side_effect = maximum;
        self
    }

    /// Requires execution within a trust zone.
    #[must_use]
    pub fn trust_zone(mut self, zone: TrustZone) -> Self {
        self.trust_zones.insert(zone);
        self
    }

    /// Requires one exact execution-isolation/trust class.
    #[must_use]
    pub const fn execution_trust(mut self, trust: ExecutionTrustClass) -> Self {
        self.execution_trust = Some(trust);
        self
    }

    /// Exact namespaced operation required by this expression.
    #[must_use]
    pub const fn operation(&self) -> &OperationId {
        &self.operation
    }

    /// Exact capability identity required by this expression, when pinned.
    #[must_use]
    pub const fn exact_capability(&self) -> Option<&CapabilityId> {
        self.exact_capability.as_ref()
    }

    /// Provider-profile reference required by this expression, when pinned.
    #[must_use]
    pub const fn provider_profile_ref(&self) -> Option<&ProviderProfileRef> {
        self.provider_profile.as_ref()
    }

    /// Accepted capability categories; an empty set accepts any category.
    #[must_use]
    pub const fn categories(&self) -> &BTreeSet<CapabilityCategory> {
        &self.categories
    }

    /// Feature identities that must all be advertised by the operation.
    #[must_use]
    pub const fn required_features(&self) -> &BTreeSet<FeatureId> {
        &self.required_features
    }

    /// Required streaming shape, when constrained.
    #[must_use]
    pub const fn streaming_mode(&self) -> Option<StreamingMode> {
        self.streaming
    }

    /// Whether the operation must advertise cancellation support.
    #[must_use]
    pub const fn cancellation_required(&self) -> bool {
        self.cancellation_required
    }

    /// Highest accepted side-effect class.
    #[must_use]
    pub const fn maximum_side_effect_class(&self) -> SideEffectClass {
        self.maximum_side_effect
    }

    /// Trust zones that must all be advertised by a matching descriptor.
    #[must_use]
    pub const fn trust_zones(&self) -> &BTreeSet<TrustZone> {
        &self.trust_zones
    }

    /// Exact required execution-isolation/trust class, when constrained.
    #[must_use]
    pub const fn execution_trust_class(&self) -> Option<ExecutionTrustClass> {
        self.execution_trust
    }

    /// Validates collection bounds after deserialization or composition.
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.categories.len() > 32
            || self.required_features.len() > MAX_FEATURES
            || self.trust_zones.len() > MAX_LABELS
        {
            return Err(ContractError::Bounds {
                location: "capability_requirement".to_owned(),
                reason: "requirement category, feature, or trust-zone count exceeded".to_owned(),
            });
        }
        Ok(())
    }
}

/// Result of matching a descriptor against a requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementMatch {
    reasons: Vec<String>,
}

impl RequirementMatch {
    /// Whether every constraint matched.
    #[must_use]
    pub fn is_match(&self) -> bool {
        self.reasons.is_empty()
    }

    /// Stable field names that did not match.
    #[must_use]
    pub fn mismatch_reasons(&self) -> &[String] {
        &self.reasons
    }
}

/// Mutable registry observation kept outside the immutable descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityObservation {
    capability: CapabilityId,
    observed_at_unix_ms: u64,
    available: bool,
    current_load: u32,
    health_summary: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CapabilityObservationWire {
    capability: CapabilityId,
    observed_at_unix_ms: u64,
    available: bool,
    current_load: u32,
    health_summary: String,
}

impl<'de> Deserialize<'de> for CapabilityObservation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CapabilityObservationWire::deserialize(deserializer)?;
        Self::new(
            wire.capability,
            wire.observed_at_unix_ms,
            wire.available,
            wire.current_load,
            wire.health_summary,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl CapabilityObservation {
    /// Creates a bounded live observation.
    pub fn new(
        capability: CapabilityId,
        observed_at_unix_ms: u64,
        available: bool,
        current_load: u32,
        health_summary: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let health_summary = health_summary.into();
        if health_summary.len() > 512 {
            return Err(ContractError::Bounds {
                location: "observation.health_summary".to_owned(),
                reason: "must not exceed 512 bytes".to_owned(),
            });
        }
        Ok(Self {
            capability,
            observed_at_unix_ms,
            available,
            current_load,
            health_summary,
        })
    }

    /// Capability identity observed by the registry boundary.
    #[must_use]
    pub const fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// Boundary-supplied observation time in Unix milliseconds.
    #[must_use]
    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }

    /// Whether the capability was available at the observation boundary.
    #[must_use]
    pub const fn available(&self) -> bool {
        self.available
    }

    /// Adapter-observed current load.
    #[must_use]
    pub const fn current_load(&self) -> u32 {
        self.current_load
    }

    /// Bounded health summary.
    #[must_use]
    pub fn health_summary(&self) -> &str {
        &self.health_summary
    }
}
