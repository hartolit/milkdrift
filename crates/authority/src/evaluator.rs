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
        if operation_uses_workflow_scope(request.operation)
            && !grant.resources().workflow_run.matches(&request.resources)
        {
            reasons.push(DecisionReasonCode::WorkflowRunMismatch);
        }

        let capability = &grant.resources().capability;
        if (capability.denies_all() && capability_facts_are_requested(&request.resources))
            || request
                .resources
                .capability
                .as_ref()
                .is_some_and(|identity| {
                    !selection_matches(capability.identity_selection(), identity)
                })
            || request.resources.category.as_ref().is_some_and(|category| {
                !selection_matches(capability.category_selection(), category)
            })
            || request
                .resources
                .capability_operation
                .as_ref()
                .is_some_and(|operation| {
                    !selection_matches(capability.operation_selection(), operation)
                })
            || request
                .resources
                .provider_profile
                .as_ref()
                .is_some_and(|profile| {
                    !selection_matches(capability.provider_profile_selection(), profile)
                })
        {
            reasons.push(DecisionReasonCode::CapabilityMismatch);
        }
        if request
            .resources
            .trust_zone
            .as_ref()
            .is_some_and(|zone| !selection_matches(capability.trust_zone_selection(), zone))
            || (!request.resources.trust_zones.is_empty()
                && !request
                    .resources
                    .trust_zones
                    .iter()
                    .all(|zone| selection_matches(capability.trust_zone_selection(), zone)))
            || request
                .resources
                .execution_trust_class
                .is_some_and(|trust| {
                    !selection_matches(capability.execution_trust_class_selection(), &trust)
                })
            || request.resources.locality.is_some_and(|locality| {
                !selection_matches(capability.locality_selection(), &locality)
            })
            || request
                .resources
                .peer
                .as_ref()
                .is_some_and(|peer| !selection_matches(capability.peer_selection(), peer))
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
        if let (Some(artifact), Some(sensitivity)) = (
            request.resources.artifact.as_ref(),
            request.resources.artifact_sensitivity,
        ) && !grant.resources().artifacts.matches(artifact, sensitivity)
        {
            reasons.push(DecisionReasonCode::ArtifactScopeMismatch);
        }
        if let (Some(revision), Some(owner)) = (
            request.resources.revision.as_ref(),
            request.resources.layout_owner.as_ref(),
        ) && !grant.resources().layouts.matches(revision, owner)
        {
            reasons.push(DecisionReasonCode::LayoutScopeMismatch);
        }
        if matches!(
            request.operation,
            crate::AuthorityOperation::InspectPeer
                | crate::AuthorityOperation::NegotiatePeerSession
                | crate::AuthorityOperation::InspectPeerExecution
                | crate::AuthorityOperation::InvokePeerCapability
                | crate::AuthorityOperation::CancelPeerCapability
                | crate::AuthorityOperation::PeerArtifactUpload
                | crate::AuthorityOperation::PeerArtifactDownload
                | crate::AuthorityOperation::AdministerPeer
        ) && request
            .resources
            .peer
            .as_ref()
            .is_none_or(|peer| !grant.resources().peers.matches(peer))
        {
            reasons.push(DecisionReasonCode::PeerScopeMismatch);
        }
        let daemon = grant.resources().daemon;
        if (request.resources.daemon_readiness && !daemon.readiness)
            || (request.resources.daemon_detailed_health && !daemon.detailed_health)
            || (request.resources.daemon_own_authority && !daemon.own_authority)
            || (request.resources.daemon_configuration && !daemon.configuration)
            || (request.resources.daemon_audit && !daemon.audit)
        {
            reasons.push(DecisionReasonCode::DaemonScopeMismatch);
        }
        if request
            .resources
            .workspace_scope
            .as_ref()
            .is_some_and(|scope| !grant.resources().workspace.matches(scope))
        {
            reasons.push(DecisionReasonCode::WorkspaceScopeMismatch);
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

fn operation_uses_workflow_scope(operation: crate::AuthorityOperation) -> bool {
    !matches!(
        operation,
        crate::AuthorityOperation::ListCapabilities
            | crate::AuthorityOperation::InspectCapabilityHealth
            | crate::AuthorityOperation::InspectProviderProfile
            | crate::AuthorityOperation::AdministerCapabilities
            | crate::AuthorityOperation::ReadArtifactMetadata
            | crate::AuthorityOperation::ReadArtifactContent
            | crate::AuthorityOperation::PublishArtifact
            | crate::AuthorityOperation::ExportArtifact
            | crate::AuthorityOperation::DeleteArtifact
            | crate::AuthorityOperation::AdministerArtifactRetention
            | crate::AuthorityOperation::InspectPeer
            | crate::AuthorityOperation::NegotiatePeerSession
            | crate::AuthorityOperation::InspectPeerExecution
            | crate::AuthorityOperation::InvokePeerCapability
            | crate::AuthorityOperation::CancelPeerCapability
            | crate::AuthorityOperation::PeerArtifactUpload
            | crate::AuthorityOperation::PeerArtifactDownload
            | crate::AuthorityOperation::AdministerPeer
            | crate::AuthorityOperation::NegotiateControlProtocol
            | crate::AuthorityOperation::ReadReadiness
            | crate::AuthorityOperation::InspectDaemonHealth
            | crate::AuthorityOperation::InspectOwnAuthority
            | crate::AuthorityOperation::InspectAudit
            | crate::AuthorityOperation::InspectConfiguration
            | crate::AuthorityOperation::ReloadConfiguration
            | crate::AuthorityOperation::DrainDaemon
            | crate::AuthorityOperation::ShutdownDaemon
    )
}

fn scope_within(
    requested: &crate::CapabilityAuthorityScope,
    allowed: &crate::CapabilityAuthorityScope,
) -> bool {
    requested.is_subset_of(allowed)
}

fn selection_matches<T: Ord>(selection: Option<&crate::Selection<T>>, value: &T) -> bool {
    selection.is_some_and(|selection| selection.matches(value))
}

fn capability_facts_are_requested(resources: &crate::RequestedResourceFacts) -> bool {
    resources.capability.is_some()
        || resources.category.is_some()
        || resources.capability_operation.is_some()
        || resources.provider_profile.is_some()
        || resources.trust_zone.is_some()
        || !resources.trust_zones.is_empty()
        || resources.execution_trust_class.is_some()
        || resources.locality.is_some()
        || resources.peer.is_some()
        || resources.capability_envelope.is_some()
}

fn filesystem_contains(allowed: &FilesystemScope, requested: &FilesystemScope) -> bool {
    allowed.contains(requested)
}
