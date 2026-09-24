//! Peer relationship admission and exact capability authority evaluation.

use std::collections::BTreeSet;

use crate::{AdapterExecutionContext, CatalogGenerationView};
use milkdrift_authority::{
    ActorRef, ArtifactAuthorityScope, AuthorityBudget, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityGrantBuilder, AuthorityOperation,
    AuthorityRequest, BoundaryTimeMillis, CapabilityAuthorityScope,
    CapabilityAuthorityScopeBuilder, CapabilityExecutionRequirements, DaemonAuthorityScope,
    DecisionId, GrantId, LayoutAuthorityScope, NetworkScope, PeerAuthorityScope,
    RequestedResourceFacts, ResourceScope, Selection, WorkflowRunScope, WorkspaceAuthorityScope,
};
use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{CapabilityDescriptor, PeerId};
use milkdrift_peer_protocol::{PeerAction, ServingInvocationRequest};
use milkdrift_persistence::{AttemptId, NodeExecutionId};
use milkdrift_workspace::RunId;

use super::{PeerService, RateWindow, maximum_budget};
use super::{ServingError, config::PeerRelationship};

impl PeerService {
    pub(super) fn authorize_peer_artifact_metadata(
        &self,
        relationship: &PeerRelationship,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<(), ServingError> {
        let metadata = self.artifacts.metadata(reference).map_err(|_| {
            ServingError::Unauthorized("artifact metadata is unavailable".to_owned())
        })?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(metadata.reference().artifact().clone());
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        self.require_operation(
            relationship,
            AuthorityOperation::ReadArtifactMetadata,
            resources,
            AuthorityBudget::default(),
        )
    }

    pub(super) fn require_execution_operation(
        &self,
        relationship: &PeerRelationship,
        execution: &milkdrift_persistence::PeerExecutionSnapshot,
        operation: AuthorityOperation,
    ) -> Result<(), ServingError> {
        let mut resources = RequestedResourceFacts::empty();
        match execution {
            milkdrift_persistence::PeerExecutionSnapshot::Hot(record) => {
                resources.capability = Some(record.request.selection.capability().clone());
                resources.capability_operation = Some(record.request.selection.operation().clone());
                resources.side_effect = record.request.selection.operation_contract().side_effect();
            }
            milkdrift_persistence::PeerExecutionSnapshot::Archived(record) => {
                if let Some(observation) = record.disposition.terminal_observation()
                    && let Some(terminal) = observation.event.kind().terminal()
                {
                    for reference in terminal.outputs() {
                        self.authorize_peer_artifact_metadata(relationship, reference)?;
                    }
                }
                resources.capability = Some(record.capability.clone());
                resources.capability_operation = Some(record.operation.clone());
                resources.side_effect = record.side_effect;
            }
        }
        self.require_operation(
            relationship,
            operation,
            resources,
            AuthorityBudget::default(),
        )
    }

    pub(super) fn relationship(&self, peer: &PeerId) -> Result<PeerRelationship, ServingError> {
        let relationship = self
            .relationships
            .get(peer)
            .cloned()
            .ok_or(ServingError::Unauthenticated)?;
        if !relationship.enabled
            || self.now()? > relationship.expires_at_unix_ms
            || self
                .revoked_peers
                .lock()
                .map_or(true, |revoked| revoked.contains(peer))
        {
            return Err(ServingError::Unauthenticated);
        }
        Ok(relationship)
    }

    pub(super) fn require_operation(
        &self,
        relationship: &PeerRelationship,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
    ) -> Result<(), ServingError> {
        let decision = self.evaluate_operation(relationship, operation, resources, budget)?;
        if decision.is_allowed() {
            Ok(())
        } else {
            Err(ServingError::Unauthorized(format!(
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
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, ServingError> {
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
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, ServingError> {
        let grant = self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| ServingError::Unauthorized("peer grant is absent".to_owned()))?;
        resources.peer = Some(relationship.remote_peer.clone());
        let now = self.now()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"milkdrift.peer-authority.v1\0");
        hasher.update(relationship.remote_peer.as_str().as_bytes());
        hasher.update(format!("{operation:?}{resources:?}{budget:?}{now}").as_bytes());
        let request = AuthorityRequest {
            decision: DecisionId::new(format!("decision:{}", hasher.finalize()))
                .map_err(|error| ServingError::Configuration(error.to_string()))?,
            actor: grant.actor().clone(),
            grant: grant.identity().clone(),
            grant_revision: grant.revision(),
            grant_digest: grant
                .digest()
                .map_err(|error| ServingError::Configuration(error.to_string()))?,
            revocation_generation: grant.revocation_generation(),
            operation,
            resources,
            budget,
            evaluated_at: BoundaryTimeMillis::new(now),
            provenance,
        };
        self.authority
            .evaluate(&request)
            .map_err(|error| ServingError::Configuration(error.to_string()))
    }

    pub(super) fn check_rate(
        &self,
        relationship: &PeerRelationship,
        bucket: &'static str,
    ) -> Result<(), ServingError> {
        self.check_caller_rate(
            self.peer_caller(&relationship.remote_peer),
            relationship.maximum_requests_per_minute,
            bucket,
        )
    }

    pub(super) fn check_client_rate(
        &self,
        actor: &ActorRef,
        bucket: &'static str,
    ) -> Result<(), ServingError> {
        let _grant = self.client_grant(actor)?;
        let policy = self
            .client_policy
            .as_ref()
            .ok_or(ServingError::Unauthenticated)?;
        self.check_caller_rate(
            self.client_caller(actor),
            policy.maximum_requests_per_minute,
            bucket,
        )
    }

    fn check_caller_rate(
        &self,
        caller: milkdrift_peer_protocol::ServingCaller,
        maximum: u32,
        bucket: &'static str,
    ) -> Result<(), ServingError> {
        let now = self.now()?;
        let key = (caller, bucket);
        let mut windows = self
            .rate_windows
            .lock()
            .map_err(|_| ServingError::Unavailable("peer rate state unavailable".to_owned()))?;
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
        if window.requests >= maximum {
            return Err(ServingError::Overloaded(
                "authenticated peer request-rate quota reached".to_owned(),
            ));
        }
        window.requests = window.requests.saturating_add(1);
        Ok(())
    }

    pub(super) fn authorize_invocation(
        &self,
        relationship: &PeerRelationship,
        request: &ServingInvocationRequest,
        descriptor: &CapabilityDescriptor,
        requirements: &CapabilityExecutionRequirements,
        now: u64,
    ) -> Result<milkdrift_authority::AuthorityDecisionSnapshot, ServingError> {
        let _validated_context = adapter_execution_context(request)?;
        if !relationship.execution_limits.contains(&request.limits)
            || request.limits.artifact_bytes > relationship.maximum_artifact_bytes
            || requirements
                .budget
                .duration_ms
                .is_some_and(|duration| duration > request.limits.duration_ms)
        {
            return Err(ServingError::Unauthorized(
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
        let origin = request.authorization.origin();
        let delegated = origin.workflow();
        let provenance = AuthorityExecutionProvenance {
            revision: delegated
                .map(|value| parse_revision(&value.revision))
                .transpose()?,
            node: delegated
                .map(|value| NodeId::new(value.node.clone()))
                .transpose()
                .map_err(|error| ServingError::Protocol(error.to_string()))?,
            execution: delegated.map(|value| value.execution.clone()),
            attempt: delegated.map(|value| value.attempt.clone()),
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
            return Err(ServingError::Unauthorized(
                "peer capability invocation authority is not granted".to_owned(),
            ));
        }
        let expected_actor = milkdrift_authority::ActorRef::new(format!(
            "peer:{}",
            relationship.remote_peer.as_str()
        ))
        .map_err(|error| ServingError::Configuration(error.to_string()))?;
        let delegation = request.authorization.delegation().ok_or_else(|| {
            ServingError::Unauthorized(
                "peer transport requires targeted peer delegation".to_owned(),
            )
        })?;
        if delegation.reference != relationship.delegation
            || delegation.issuer_peer != relationship.remote_peer
            || delegation.target_peer != self.config.local_peer
            || delegation.actor != expected_actor
            || delegation.expires_at_unix_ms < now
            || delegation.expires_at_unix_ms > relationship.expires_at_unix_ms
        {
            return Err(ServingError::Unauthorized(
                "delegation record is absent, expired, or does not match authenticated facts"
                    .to_owned(),
            ));
        }
        Ok(decision)
    }

    pub(super) fn exact_generation(
        &self,
        relationship: &PeerRelationship,
        request: &ServingInvocationRequest,
    ) -> Result<CatalogGenerationView, ServingError> {
        let scope = &self
            .grants
            .get(&relationship.remote_peer)
            .ok_or_else(|| ServingError::Unauthorized("peer grant is absent".to_owned()))?
            .resources()
            .capability;
        self.exact_generation_in_scope(scope, request)
    }

    pub(super) fn exact_generation_in_scope(
        &self,
        scope: &CapabilityAuthorityScope,
        request: &ServingInvocationRequest,
    ) -> Result<CatalogGenerationView, ServingError> {
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
                ServingError::Unauthorized(
                    "selected local capability generation is no longer registered".to_owned(),
                )
            })
    }
}

pub(crate) fn adapter_execution_context(
    request: &ServingInvocationRequest,
) -> Result<AdapterExecutionContext, ServingError> {
    let origin = request.authorization.origin();
    let Some(provenance) = origin.workflow() else {
        let selection =
            crate::DirectInputSelection::new(&request.request, request.limits.artifact_bytes)
                .map_err(|error| ServingError::Protocol(error.to_string()))?;
        return Ok(AdapterExecutionContext::direct(selection));
    };
    Ok(AdapterExecutionContext::new(
        RunId::new(provenance.run.clone())
            .map_err(|error| ServingError::Protocol(error.to_string()))?,
        parse_revision(&provenance.revision)?,
        NodeId::new(provenance.node.clone())
            .map_err(|error| ServingError::Protocol(error.to_string()))?,
        NodeExecutionId::new(provenance.execution.clone())
            .map_err(|error| ServingError::Protocol(error.to_string()))?,
        AttemptId::new(provenance.attempt.clone())
            .map_err(|error| ServingError::Protocol(error.to_string()))?,
    ))
}

fn parse_revision(value: &str) -> Result<RevisionId, ServingError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| ServingError::Protocol(error.to_string()))
}

pub(super) fn peer_authority_grant(
    relationship: &PeerRelationship,
) -> Result<AuthorityGrant, ServingError> {
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
        operations.extend([
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ]);
    }
    if actions.contains(&PeerAction::ArtifactDownload) {
        operations.insert(AuthorityOperation::PeerArtifactDownload);
        operations.insert(AuthorityOperation::ReadArtifactMetadata);
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
        .map_err(|error| ServingError::Configuration(error.to_string()))?,
        secrets: relationship.execution_secrets.clone(),
        artifacts: if (actions.contains(&PeerAction::ArtifactUpload)
            || actions.contains(&PeerAction::ArtifactDownload))
            && !relationship.artifact_sensitivities.is_empty()
        {
            ArtifactAuthorityScope::new(
                Selection::any(),
                relationship.artifact_sensitivities.clone(),
            )
            .map_err(|error| ServingError::Configuration(error.to_string()))?
        } else {
            ArtifactAuthorityScope::none()
        },
        layouts: LayoutAuthorityScope::none(),
        peers: PeerAuthorityScope::new(BTreeSet::from([relationship.remote_peer.clone()]), false)
            .map_err(|error| ServingError::Configuration(error.to_string()))?,
        daemon: DaemonAuthorityScope::default(),
        workspace: WorkspaceAuthorityScope::none(),
    };
    let peer_hash = blake3::hash(relationship.remote_peer.as_str().as_bytes());
    AuthorityGrantBuilder::new(
        GrantId::new(format!("grant:peer-{}", &peer_hash.to_hex().as_str()[..24]))
            .map_err(|error| ServingError::Configuration(error.to_string()))?,
        relationship.revocation_generation.saturating_add(1).max(1),
        ActorRef::new(format!("peer:{}", relationship.remote_peer.as_str()))
            .map_err(|error| ServingError::Configuration(error.to_string()))?,
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
        // Units cover both logical prompt and generated work; a composed method reserves both.
        units: relationship
            .execution_limits
            .input_units
            .into_iter()
            .chain(relationship.execution_limits.output_units)
            .reduce(u64::saturating_add),
        concurrency: Some(u32::from(relationship.maximum_concurrent)),
    })
    .validity(
        BoundaryTimeMillis::new(0),
        BoundaryTimeMillis::new(relationship.expires_at_unix_ms),
    )
    .revocation_generation(relationship.revocation_generation)
    .build()
    .map_err(|error| ServingError::Configuration(error.to_string()))
}

pub(super) fn peer_capability_authority(
    identities: BTreeSet<milkdrift_capability::CapabilityId>,
    operations: BTreeSet<milkdrift_capability::OperationId>,
    maximum_side_effect: milkdrift_capability::SideEffectClass,
) -> Result<CapabilityAuthorityScope, ServingError> {
    if identities.is_empty() || operations.is_empty() {
        Ok(CapabilityAuthorityScope::deny_all())
    } else {
        Ok(CapabilityAuthorityScopeBuilder::new(maximum_side_effect)
            .only_capabilities(identities)
            .and_then(|builder| builder.only_operations(operations))
            .map_err(|error| ServingError::Configuration(error.to_string()))?
            .build())
    }
}
