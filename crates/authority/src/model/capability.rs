use std::collections::BTreeSet;

use milkdrift_capability::{
    CapabilityCategory, CapabilityId, CapabilityRequirement, ExecutionTrustClass, Locality,
    OperationId, PeerId, ProviderProfileRef, SideEffectClass, TrustZone,
};
use serde::{Deserialize, Serialize};

use crate::{AuthorityError, Selection};

/// Capability-selection constraints in a grant.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct CapabilityAuthorityScope(CapabilityAuthorityScopeKind);

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", deny_unknown_fields)]
enum CapabilityAuthorityScopeKind {
    DenyAll,
    Allow(Box<CapabilityAuthorityAllowScope>),
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CapabilityAuthorityAllowScope {
    identities: Selection<CapabilityId>,
    categories: Selection<CapabilityCategory>,
    operations: Selection<OperationId>,
    provider_profiles: Selection<ProviderProfileRef>,
    trust_zones: Selection<TrustZone>,
    execution_trust_classes: Selection<ExecutionTrustClass>,
    localities: Selection<Locality>,
    peers: Selection<PeerId>,
    maximum_side_effect: SideEffectClass,
}

impl CapabilityAuthorityScope {
    /// Explicitly denies every capability identity, descriptor, profile, and operation.
    #[must_use]
    pub const fn deny_all() -> Self {
        Self(CapabilityAuthorityScopeKind::DenyAll)
    }

    /// Explicitly permits every selector value subject to the supplied side-effect ceiling.
    #[must_use]
    pub fn allow_any(maximum_side_effect: SideEffectClass) -> Self {
        CapabilityAuthorityScopeBuilder::new(maximum_side_effect).build()
    }

    /// Constructs the exact semantic envelope requested by one workflow requirement.
    ///
    /// Unspecified requirement dimensions become explicit `Any` selectors. Exact identity,
    /// operation, provider profile, category, trust-zone, and execution-trust facts become
    /// nonempty `Only` selectors.
    pub fn requirement_envelope(
        requirement: &CapabilityRequirement,
    ) -> Result<Self, AuthorityError> {
        let mut builder =
            CapabilityAuthorityScopeBuilder::new(requirement.maximum_side_effect_class())
                .only_operations(BTreeSet::from([requirement.operation().clone()]))?;
        if let Some(identity) = requirement.exact_capability() {
            builder = builder.only_capabilities(BTreeSet::from([identity.clone()]))?;
        }
        if !requirement.categories().is_empty() {
            builder = builder.only_categories(requirement.categories().clone())?;
        }
        if let Some(profile) = requirement.provider_profile_ref() {
            builder = builder.only_provider_profiles(BTreeSet::from([profile.clone()]))?;
        }
        if !requirement.trust_zones().is_empty() {
            builder = builder.only_trust_zones(requirement.trust_zones().clone())?;
        }
        if let Some(trust_class) = requirement.execution_trust_class() {
            builder = builder.only_execution_trust_classes(BTreeSet::from([trust_class]))?;
        }
        Ok(builder.build())
    }

    /// Whether this scope is the explicit default-deny capability scope.
    #[must_use]
    pub const fn denies_all(&self) -> bool {
        matches!(self.0, CapabilityAuthorityScopeKind::DenyAll)
    }

    /// Capability identity selector for an allow scope.
    #[must_use]
    pub const fn identity_selection(&self) -> Option<&Selection<CapabilityId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.identities),
        }
    }
    /// Capability category selector for an allow scope.
    #[must_use]
    pub const fn category_selection(&self) -> Option<&Selection<CapabilityCategory>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.categories),
        }
    }
    /// Capability operation selector for an allow scope.
    #[must_use]
    pub const fn operation_selection(&self) -> Option<&Selection<OperationId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.operations),
        }
    }
    /// Provider profile selector for an allow scope.
    #[must_use]
    pub const fn provider_profile_selection(&self) -> Option<&Selection<ProviderProfileRef>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.provider_profiles),
        }
    }
    /// Trust-zone selector for an allow scope.
    #[must_use]
    pub const fn trust_zone_selection(&self) -> Option<&Selection<TrustZone>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.trust_zones),
        }
    }
    /// Execution trust-class selector for an allow scope.
    #[must_use]
    pub const fn execution_trust_class_selection(&self) -> Option<&Selection<ExecutionTrustClass>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.execution_trust_classes),
        }
    }
    /// Locality selector for an allow scope.
    #[must_use]
    pub const fn locality_selection(&self) -> Option<&Selection<Locality>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.localities),
        }
    }
    /// Authenticated peer selector for an allow scope.
    #[must_use]
    pub const fn peer_selection(&self) -> Option<&Selection<PeerId>> {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => None,
            CapabilityAuthorityScopeKind::Allow(scope) => Some(&scope.peers),
        }
    }
    /// Maximum permitted side-effect class.
    #[must_use]
    pub const fn maximum_side_effect(&self) -> SideEffectClass {
        match self.0 {
            CapabilityAuthorityScopeKind::DenyAll => SideEffectClass::None,
            CapabilityAuthorityScopeKind::Allow(ref scope) => scope.maximum_side_effect,
        }
    }

    /// Whether this allow scope contains an explicit wildcard in any dimension.
    #[must_use]
    pub fn has_any_selector(&self) -> bool {
        match &self.0 {
            CapabilityAuthorityScopeKind::DenyAll => false,
            CapabilityAuthorityScopeKind::Allow(scope) => [
                scope.identities.is_any(),
                scope.categories.is_any(),
                scope.operations.is_any(),
                scope.provider_profiles.is_any(),
                scope.trust_zones.is_any(),
                scope.execution_trust_classes.is_any(),
                scope.localities.is_any(),
                scope.peers.is_any(),
            ]
            .into_iter()
            .any(|value| value),
        }
    }

    /// Tests exact containment using selector algebra and the side-effect ceiling.
    #[must_use]
    pub fn is_subset_of(&self, allowed: &Self) -> bool {
        match (&self.0, &allowed.0) {
            (CapabilityAuthorityScopeKind::DenyAll, _) => true,
            (_, CapabilityAuthorityScopeKind::DenyAll) => false,
            (
                CapabilityAuthorityScopeKind::Allow(requested),
                CapabilityAuthorityScopeKind::Allow(allowed),
            ) => {
                requested.identities.is_subset_of(&allowed.identities)
                    && requested.categories.is_subset_of(&allowed.categories)
                    && requested.operations.is_subset_of(&allowed.operations)
                    && requested
                        .provider_profiles
                        .is_subset_of(&allowed.provider_profiles)
                    && requested.trust_zones.is_subset_of(&allowed.trust_zones)
                    && requested
                        .execution_trust_classes
                        .is_subset_of(&allowed.execution_trust_classes)
                    && requested.localities.is_subset_of(&allowed.localities)
                    && requested.peers.is_subset_of(&allowed.peers)
                    && requested.maximum_side_effect <= allowed.maximum_side_effect
            }
        }
    }
}

/// Validating builder for an explicit conjunctive capability allow scope.
#[derive(Clone, Debug)]
pub struct CapabilityAuthorityScopeBuilder {
    identities: Selection<CapabilityId>,
    categories: Selection<CapabilityCategory>,
    operations: Selection<OperationId>,
    provider_profiles: Selection<ProviderProfileRef>,
    trust_zones: Selection<TrustZone>,
    execution_trust_classes: Selection<ExecutionTrustClass>,
    localities: Selection<Locality>,
    peers: Selection<PeerId>,
    maximum_side_effect: SideEffectClass,
}

impl CapabilityAuthorityScopeBuilder {
    /// Starts an explicit allow scope with `Any` in every dimension.
    #[must_use]
    pub const fn new(maximum_side_effect: SideEffectClass) -> Self {
        Self {
            identities: Selection::any(),
            categories: Selection::any(),
            operations: Selection::any(),
            provider_profiles: Selection::any(),
            trust_zones: Selection::any(),
            execution_trust_classes: Selection::any(),
            localities: Selection::any(),
            peers: Selection::any(),
            maximum_side_effect,
        }
    }

    /// Narrows capability identities to a nonempty exact allowlist.
    pub fn only_capabilities(
        mut self,
        values: BTreeSet<CapabilityId>,
    ) -> Result<Self, AuthorityError> {
        self.identities = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows categories to a nonempty exact allowlist.
    pub fn only_categories(
        mut self,
        values: BTreeSet<CapabilityCategory>,
    ) -> Result<Self, AuthorityError> {
        self.categories = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows operations to a nonempty exact allowlist.
    pub fn only_operations(
        mut self,
        values: BTreeSet<OperationId>,
    ) -> Result<Self, AuthorityError> {
        self.operations = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows provider profiles to a nonempty exact allowlist.
    pub fn only_provider_profiles(
        mut self,
        values: BTreeSet<ProviderProfileRef>,
    ) -> Result<Self, AuthorityError> {
        self.provider_profiles = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows trust zones to a nonempty exact allowlist.
    pub fn only_trust_zones(mut self, values: BTreeSet<TrustZone>) -> Result<Self, AuthorityError> {
        self.trust_zones = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows execution trust classes to a nonempty exact allowlist.
    pub fn only_execution_trust_classes(
        mut self,
        values: BTreeSet<ExecutionTrustClass>,
    ) -> Result<Self, AuthorityError> {
        self.execution_trust_classes = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows localities to a nonempty exact allowlist.
    pub fn only_localities(mut self, values: BTreeSet<Locality>) -> Result<Self, AuthorityError> {
        self.localities = Selection::only(values)?;
        Ok(self)
    }

    /// Narrows authenticated peers to a nonempty exact allowlist.
    pub fn only_peers(mut self, values: BTreeSet<PeerId>) -> Result<Self, AuthorityError> {
        self.peers = Selection::only(values)?;
        Ok(self)
    }

    /// Publishes the explicit allow scope.
    #[must_use]
    pub fn build(self) -> CapabilityAuthorityScope {
        CapabilityAuthorityScope(CapabilityAuthorityScopeKind::Allow(Box::new(
            CapabilityAuthorityAllowScope {
                identities: self.identities,
                categories: self.categories,
                operations: self.operations,
                provider_profiles: self.provider_profiles,
                trust_zones: self.trust_zones,
                execution_trust_classes: self.execution_trust_classes,
                localities: self.localities,
                peers: self.peers,
                maximum_side_effect: self.maximum_side_effect,
            },
        )))
    }
}
