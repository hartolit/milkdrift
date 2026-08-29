use std::collections::BTreeMap;

use crate::{
    AuthorityDecisionSnapshot, AuthorityError, AuthorityGrant, AuthorityRequest,
    DecisionReasonCode, FilesystemScope, GrantId, PolicyId,
};

/// Object-safe pure authority policy boundary injected into runtime and host services.
pub trait AuthorityEvaluator: Send + Sync {
    /// Evaluates only the supplied immutable facts and returns an exact snapshot.
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError>;
}

/// Deterministic immutable grant set and revocation-generation policy.
pub struct GrantSetEvaluator {
    policy: PolicyId,
    policy_version: u32,
    grants: BTreeMap<(GrantId, u64), AuthorityGrant>,
    revocations: BTreeMap<GrantId, u64>,
}

impl GrantSetEvaluator {
    /// Constructs an evaluator from exact immutable revisions.
    pub fn new(
        policy: PolicyId,
        policy_version: u32,
        grants: impl IntoIterator<Item = AuthorityGrant>,
        revocations: BTreeMap<GrantId, u64>,
    ) -> Result<Self, AuthorityError> {
        if policy_version == 0 {
            return Err(AuthorityError::InvalidContract(
                "policy version must be nonzero".to_owned(),
            ));
        }
        let mut indexed = BTreeMap::new();
        for grant in grants {
            let key = (grant.identity().clone(), grant.revision());
            if let Some(existing) = indexed.insert(key, grant.clone())
                && existing != grant
            {
                return Err(AuthorityError::InvalidContract(
                    "conflicting reuse of one grant identity and revision".to_owned(),
                ));
            }
        }
        Ok(Self {
            policy,
            policy_version,
            grants: indexed,
            revocations,
        })
    }
}

impl AuthorityEvaluator for GrantSetEvaluator {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        request.validate()?;
        let exact = self
            .grants
            .get(&(request.grant.clone(), request.grant_revision));
        let mut reasons = Vec::new();
        let grant = match exact {
            Some(grant) => grant,
            None => {
                if self
                    .grants
                    .keys()
                    .any(|(identity, _revision)| identity == &request.grant)
                {
                    reasons.push(DecisionReasonCode::GrantRevisionMismatch);
                } else {
                    reasons.push(DecisionReasonCode::GrantNotFound);
                }
                return AuthorityDecisionSnapshot::from_evaluation(
                    self.policy.clone(),
                    self.policy_version,
                    request.clone(),
                    reasons,
                    Default::default(),
                    request.resources.side_effect,
                );
            }
        };

        if grant.digest()? != request.grant_digest {
            reasons.push(DecisionReasonCode::GrantDigestMismatch);
        }
        if grant.actor() != &request.actor {
            reasons.push(DecisionReasonCode::WrongActor);
        }
        let current_revocation = self
            .revocations
            .get(grant.identity())
            .copied()
            .unwrap_or(grant.revocation_generation());
        if request.revocation_generation != grant.revocation_generation()
            || current_revocation != grant.revocation_generation()
        {
            reasons.push(DecisionReasonCode::Revoked);
        }
        if request.evaluated_at < grant.valid_from() {
            reasons.push(DecisionReasonCode::NotYetValid);
        }
        if request.evaluated_at > grant.valid_until() {
            reasons.push(DecisionReasonCode::Expired);
        }
        if !grant.operations().contains(&request.operation) {
            reasons.push(DecisionReasonCode::OperationMismatch);
        }
        if !grant.resources().workflow_run.matches(&request.resources) {
            reasons.push(DecisionReasonCode::WorkflowRunMismatch);
        }

        let capability = &grant.resources().capability;
        if request
            .resources
            .capability
            .as_ref()
            .is_some_and(|identity| {
                !capability.identities().is_empty() && !capability.identities().contains(identity)
            })
            || request.resources.category.as_ref().is_some_and(|category| {
                !capability.categories().is_empty() && !capability.categories().contains(category)
            })
            || request
                .resources
                .capability_operation
                .as_ref()
                .is_some_and(|operation| {
                    !capability.operations().is_empty()
                        && !capability.operations().contains(operation)
                })
            || request
                .resources
                .provider_profile
                .as_ref()
                .is_some_and(|profile| {
                    !capability.provider_profiles().is_empty()
                        && !capability.provider_profiles().contains(profile)
                })
        {
            reasons.push(DecisionReasonCode::CapabilityMismatch);
        }
        if request.resources.trust_zone.as_ref().is_some_and(|zone| {
            !capability.trust_zones().is_empty() && !capability.trust_zones().contains(zone)
        }) || (!capability.trust_zones().is_empty()
            && !request
                .resources
                .trust_zones
                .is_subset(capability.trust_zones()))
            || request.resources.locality.is_some_and(|locality| {
                !capability.localities().is_empty() && !capability.localities().contains(&locality)
            })
            || request.resources.peer.as_ref().is_some_and(|peer| {
                !capability.peers().is_empty() && !capability.peers().contains(peer)
            })
        {
            reasons.push(DecisionReasonCode::PlacementMismatch);
        }
        if request
            .resources
            .capability_envelope
            .as_ref()
            .is_some_and(|requested| !scope_within(requested, capability))
        {
            reasons.push(DecisionReasonCode::CapabilityMismatch);
        }
        if request.resources.side_effect > capability.maximum_side_effect() {
            reasons.push(DecisionReasonCode::SideEffectExcess);
        }
        if !request.resources.filesystem.iter().all(|requested| {
            grant
                .resources()
                .filesystem
                .iter()
                .any(|allowed| filesystem_contains(allowed, requested))
        }) {
            reasons.push(DecisionReasonCode::FilesystemMismatch);
        }
        if !request
            .resources
            .network_profiles
            .is_subset(grant.resources().network.profiles())
            || !request
                .resources
                .network_destinations
                .is_subset(grant.resources().network.destinations())
        {
            reasons.push(DecisionReasonCode::NetworkMismatch);
        }
        if !request
            .resources
            .secrets
            .is_subset(&grant.resources().secrets)
        {
            reasons.push(DecisionReasonCode::SecretScopeMismatch);
        }
        if !request.budget.fits_within(grant.budget()) {
            reasons.push(DecisionReasonCode::BudgetExcess);
        }
        if reasons.is_empty() {
            reasons.push(DecisionReasonCode::Allowed);
        }
        AuthorityDecisionSnapshot::from_evaluation(
            self.policy.clone(),
            self.policy_version,
            request.clone(),
            reasons,
            grant.budget(),
            capability.maximum_side_effect(),
        )
    }
}

fn scope_within(
    requested: &crate::CapabilityAuthorityScope,
    allowed: &crate::CapabilityAuthorityScope,
) -> bool {
    set_within(requested.identities(), allowed.identities())
        && set_within(requested.categories(), allowed.categories())
        && set_within(requested.operations(), allowed.operations())
        && set_within(requested.provider_profiles(), allowed.provider_profiles())
        && set_within(requested.trust_zones(), allowed.trust_zones())
        && set_within(requested.localities(), allowed.localities())
        && set_within(requested.peers(), allowed.peers())
        && requested.maximum_side_effect() <= allowed.maximum_side_effect()
}

fn set_within<T: Ord>(
    requested: &std::collections::BTreeSet<T>,
    allowed: &std::collections::BTreeSet<T>,
) -> bool {
    requested.is_empty() || allowed.is_empty() || requested.is_subset(allowed)
}

fn filesystem_contains(allowed: &FilesystemScope, requested: &FilesystemScope) -> bool {
    let path_matches = requested.root() == allowed.root()
        || (requested.root().starts_with(allowed.root())
            && (allowed.root() == "/"
                || requested
                    .root()
                    .as_bytes()
                    .get(allowed.root().len())
                    .is_some_and(|byte| *byte == b'/')));
    path_matches && requested.access().is_subset(allowed.access())
}
