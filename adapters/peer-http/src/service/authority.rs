//! Peer relationship admission and exact capability authority evaluation.

use std::collections::BTreeSet;

use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    CapabilityAuthorityScopeBuilder, CapabilityExecutionRequirements, DaemonAuthorityScope,
    DecisionId, GrantId, LayoutAuthorityScope, NetworkScope, PeerAuthorityScope, PeerId,
    RequestedResourceFacts, ResourceScope, Selection, WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::CapabilityDescriptor;
use milkdrift_capability_host::{AdapterExecutionContext, CatalogGenerationView};
use milkdrift_peer_protocol::{PeerAction, PeerInvocationRequest};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;

use super::{PeerService, RateWindow, maximum_budget};
use crate::{PeerHttpError, config::PeerRelationship};

impl PeerService {
    pub(super) fn relationship(&self, peer: &PeerId) -> Result<PeerRelationship, PeerHttpError> {
        let relationship = self
            .relationships
            .get(peer)
            .cloned()
            .ok_or(PeerHttpError::Unauthenticated)?;
        if !relationship.enabled
            || self.clock.now_unix_ms() > relationship.expires_at_unix_ms
            || self
                .revoked_peers
                .lock()
                .map_or(true, |revoked| revoked.contains(peer))
        {
            return Err(PeerHttpError::Unauthenticated);
        }
        Ok(relationship)
    }

    pub(super) fn require_operation(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<(), PeerHttpError> {
        let decision = self.evaluate_operation(relationship, operation, resources, budget)?;
        if decision.is_allowed() {
            Ok(())
        } else {
            Err(PeerHttpError::Unauthorized(format!(
                "peer authority denied the operation ({})",
                decision
                    .reason_codes()
                    .iter()
                    .map(|reason| format!("{reason:?}").to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            )))
        }
    }

    fn evaluate_operation(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        self.evaluate_operation_with_provenance(
            relationship,
            operation,
            resources,
            budget,
            AuthorityExecutionProvenance::default(),
        )
    }

    fn evaluate_operation_with_provenance(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        mut resources: RequestedResourceFacts,
        budget: AuthorityBudget,
        provenance: AuthorityExecutionProvenance,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        let grant = self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| PeerHttpError::Unauthorized("peer grant is absent".to_owned()))?;
        resources.peer = Some(relationship.remote_peer.clone());
        let now = self.clock.now_unix_ms();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.peer-authority.v1\0");
        hasher.update(relationship.remote_peer.as_str().as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{budget:?}{now}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
            actor: grant.actor().clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant
                .digest()
                .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
            revocation_generation: grant.revocation_generation(),
            operation,
            resources,
            budget,
            evaluated_at: BoundaryTimeMillis::new(now),
            provenance,
        };
        self.authority
            .evaluate(&request)
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))
    }

    pub(super) fn check_rate(
        &self,
        relationship: &PeerRelationship,
        bucket: &str,
    ) -> Result<(), PeerHttpError> {
        let now = self.clock.now_unix_ms();
        let key = (relationship.remote_peer.clone(), bucket.to_owned());
        let mut windows = self
            .rate_windows
            .lock()
            .map_err(|_| PeerHttpError::Unavailable("peer rate state unavailable".to_owned()))?;
        let window = windows.entry(key).or_insert(RateWindow {
            started_at_unix_ms: now,
            requests: 0,
        });
        if now >= window.started_at_unix_ms.saturating_add(60_000) {
            *window = RateWindow {
                started_at_unix_ms: now,
                requests: 0,
            };
        }
        if window.requests >= relationship.maximum_requests_per_minute {
            return Err(PeerHttpError::Overloaded(
                "authenticated peer request-rate quota reached".to_owned(),
            ));
        }
        window.requests = window.requests.saturating_add(1);
        Ok(())
    }

    pub(super) fn authorize_invocation(
        &self,
        relationship: &PeerRelationship,
        request: &PeerInvocationRequest,
        descriptor: &CapabilityDescriptor,
        requirements: &CapabilityExecutionRequirements,
        now: u64,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, PeerHttpError> {
        let _validated_context = adapter_execution_context(request)?;
        if !relationship.execution_limits.contains(request.limits)
            || request.limits.artifact_bytes > relationship.maximum_artifact_bytes
        {
            return Err(PeerHttpError::Unauthorized(
                "capability, operation, side effect, or quota is not granted".to_owned(),
            ));
        }
        let mut resources = RequestedResourceFacts::empty();
        resources.capability = Some(descriptor.identity().clone());
        resources.category = Some(descriptor.category().clone());
        resources.capability_operation = Some(request.selection.operation().clone());
        resources.provider_profile = descriptor.provider_profile().cloned();
        resources.trust_zones = descriptor.trust_zones().clone();
        resources.execution_trust_class = Some(descriptor.execution_trust());
        resources.locality = Some(descriptor.locality());
        resources.peer = descriptor.peer().cloned();
        resources.side_effect = request.selection.operation_contract().side_effect();
        resources.filesystem = requirements.filesystem.clone();
        resources.network_profiles = requirements.network_profiles.clone();
        resources.network_destinations = requirements.network_destinations.clone();
        resources.secrets = requirements.secrets.clone();
        let delegated = &request.delegation.provenance;
        let provenance = AuthorityExecutionProvenance {
            revision: Some(parse_revision(&delegated.revision)?),
            node: Some(
                NodeId::new(delegated.node.clone())
                    .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            ),
            execution: Some(delegated.execution.clone()),
            attempt: Some(delegated.attempt.clone()),
            descriptor_revision: Some(request.selection.descriptor_revision()),
            peer: Some(relationship.remote_peer.clone()),
            idempotency: Some(request.selection.operation_contract().idempotency()),
        };
        let decision = self.evaluate_operation_with_provenance(
            relationship,
            AuthorityOperation::InvokePeerCapability,
            resources,
            AuthorityBudget {
                cost_minor: maximum_budget(
                    Some(request.limits.cost_micros.saturating_add(9_999) / 10_000),
                    requirements.budget.cost_minor,
                ),
                duration_ms: maximum_budget(
                    Some(request.limits.duration_ms),
                    requirements.budget.duration_ms,
                ),
                invocations: maximum_budget(Some(1), requirements.budget.invocations),
                artifact_bytes: maximum_budget(
                    Some(request.limits.artifact_bytes),
                    requirements.budget.artifact_bytes,
                ),
                units: requirements.budget.units,
                concurrency: Some(requirements.budget.concurrency.unwrap_or(1).max(1)),
            },
            provenance,
        )?;
        if !decision.is_allowed() {
            return Err(PeerHttpError::Unauthorized(
                "peer capability invocation authority is not granted".to_owned(),
            ));
        }
        let expected_actor = milkdrift_authority::ActorRef::new(format!(
            "peer:{}",
            relationship.remote_peer.as_str()
        ))
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?;
        let delegation = &request.delegation;
        if delegation.reference != relationship.delegation
            || delegation.issuer_peer != relationship.remote_peer
            || delegation.target_peer != self.config.local_peer
            || delegation.actor != expected_actor
            || delegation.expires_at_unix_ms < now
            || delegation.expires_at_unix_ms > relationship.expires_at_unix_ms
        {
            return Err(PeerHttpError::Unauthorized(
                "delegation record is absent, expired, or does not match authenticated facts"
                    .to_owned(),
            ));
        }
        Ok(decision)
    }

    pub(super) fn exact_generation(
        &self,
        relationship: &PeerRelationship,
        request: &PeerInvocationRequest,
    ) -> Result<CatalogGenerationView, PeerHttpError> {
        let scope = &self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| PeerHttpError::Unauthorized("peer grant is absent".to_owned()))?
            .resources()
            .capability;
        self.capability_host
            .catalog_generations(scope)?
            .into_iter()
            .find(|generation| {
                generation.descriptor.identity() == request.selection.capability()
                    && generation.descriptor.descriptor_revision()
                        == request.selection.descriptor_revision()
                    && scope
                        .operation_selection()
                        .is_some_and(|selection| selection.matches(request.selection.operation()))
                    && generation
                        .descriptor
                        .operation(request.selection.operation())
                        .is_some()
            })
            .ok_or_else(|| {
                PeerHttpError::Unauthorized(
                    "selected local capability generation is no longer registered".to_owned(),
                )
            })
    }
}

pub(super) fn adapter_execution_context(
    request: &PeerInvocationRequest,
) -> Result<AdapterExecutionContext, PeerHttpError> {
    let provenance = &request.delegation.provenance;
    Ok(AdapterExecutionContext::new(
        RunId::new(provenance.run.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        parse_revision(&provenance.revision)?,
        NodeId::new(provenance.node.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        NodeExecutionId::new(provenance.execution.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
        AttemptId::new(provenance.attempt.clone())
            .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
    ))
}

fn parse_revision(value: &str) -> Result<RevisionId, PeerHttpError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))
}

pub(super) fn peer_authority_grant(
    relationship: &PeerRelationship,
) -> Result<AuthorityGrant, PeerHttpError> {
    let actions = relationship.authority.actions();
    let mut operations = BTreeSet::new();
    if actions.is_empty() {
        // Authority grants require a nonempty closed vocabulary. This operation is never used by
        // the peer transport and therefore preserves the relationship's default deny behavior.
        operations.insert(AuthorityOperation::Inspect);
    } else {
        operations.insert(AuthorityOperation::NegotiatePeerSession);
    }
    if actions.contains(&PeerAction::ReadCatalog) {
        operations.extend([
            AuthorityOperation::InspectPeer,
            AuthorityOperation::ListCapabilities,
            AuthorityOperation::InspectCapabilityHealth,
            AuthorityOperation::InspectProviderProfile,
        ]);
    }
    if actions.contains(&PeerAction::Invoke) {
        operations.extend([
            AuthorityOperation::InvokePeerCapability,
            AuthorityOperation::InspectPeerExecution,
        ]);
    }
    if actions.contains(&PeerAction::Cancel) {
        operations.insert(AuthorityOperation::CancelPeerCapability);
    }
    if actions.contains(&PeerAction::ArtifactUpload) {
        operations.insert(AuthorityOperation::PeerArtifactUpload);
    }
    if actions.contains(&PeerAction::ArtifactDownload) {
        operations.insert(AuthorityOperation::PeerArtifactDownload);
    }
    if actions.contains(&PeerAction::Administer) {
        operations.insert(AuthorityOperation::AdministerPeer);
    }

    let identities: BTreeSet<_> = relationship
        .capability_allow
        .difference(&relationship.capability_deny)
        .cloned()
        .collect();
    let capability = peer_capability_authority(
        identities,
        relationship.operation_allow.clone(),
        relationship.maximum_side_effect,
    )?;
    let resource_scope = ResourceScope {
        workflow_run: WorkflowRunScope::Any,
        capability,
        filesystem: relationship.execution_filesystem.clone(),
        network: NetworkScope::new(
            relationship.execution_network_profiles.clone(),
            relationship.execution_network_destinations.clone(),
        )
        .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        secrets: relationship.execution_secrets.clone(),
        artifacts: if (actions.contains(&PeerAction::ArtifactUpload)
            || actions.contains(&PeerAction::ArtifactDownload))
            && !relationship.artifact_sensitivities.is_empty()
        {
            ArtifactAuthorityScope::new(
                Selection::any(),
                relationship.artifact_sensitivities.clone(),
            )
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?
        } else {
            ArtifactAuthorityScope::none()
        },
        layouts: LayoutAuthorityScope::none(),
        peers: PeerAuthorityScope::new(BTreeSet::from([relationship.remote_peer.clone()]), false)
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        daemon: DaemonAuthorityScope::default(),
        workspace: WorkspaceAuthorityScope::none(),
    };
    let peer_hash = blake3::hash(relationship.remote_peer.as_str().as_bytes());
    AuthorityGrantBuilder::new(
        GrantId::new(format!("grant:peer-{}", &peer_hash.to_hex().as_str()[..24]))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
        relationship.revocation_generation.saturating_add(1).max(1),
        ActorRef::new(format!("peer:{}", relationship.remote_peer.as_str()))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?,
    )
    .operations(operations)
    .resources(resource_scope)
    .budget(AuthorityBudget {
        cost_minor: Some(
            relationship
                .execution_limits
                .cost_micros
                .saturating_add(9_999)
                / 10_000,
        ),
        duration_ms: Some(relationship.execution_limits.duration_ms),
        invocations: Some(1),
        artifact_bytes: Some(relationship.maximum_artifact_bytes),
        concurrency: Some(u32::from(relationship.maximum_concurrent)),
        ..AuthorityBudget::default()
    })
    .validity(
        BoundaryTimeMillis::new(0),
        BoundaryTimeMillis::new(relationship.expires_at_unix_ms),
    )
    .revocation_generation(relationship.revocation_generation)
    .build()
    .map_err(|error| PeerHttpError::Configuration(error.to_string()))
}

pub(super) fn peer_capability_authority(
    identities: BTreeSet<milkdrift_capability::CapabilityId>,
    operations: BTreeSet<milkdrift_capability::OperationId>,
    maximum_side_effect: milkdrift_capability::SideEffectClass,
) -> Result<CapabilityAuthorityScope, PeerHttpError> {
    if identities.is_empty() || operations.is_empty() {
        Ok(CapabilityAuthorityScope::deny_all())
    } else {
        Ok(CapabilityAuthorityScopeBuilder::new(maximum_side_effect)
            .only_capabilities(identities)
            .and_then(|builder| builder.only_operations(operations))
            .map_err(|error| PeerHttpError::Configuration(error.to_string()))?
            .build())
    }
}
