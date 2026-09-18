//! Client authority comes from configuration, while resources come from immutable owner facts.
use super::{PeerService, ServingError, maximum_budget, relationship_generation};
use milkdrift_authority::{
    ActorRef, AuthorityBudget, AuthorityDecisionSnapshot, AuthorityEvaluator,
    AuthorityExecutionProvenance, AuthorityGrant, AuthorityOperation, AuthorityRequest,
    BoundaryTimeMillis, DecisionId, RequestedResourceFacts,
};
use milkdrift_peer_protocol::{
    ServingAuthorization, ServingCaller, ServingInvocationRequest, ServingPrincipal,
};

impl PeerService {
    pub(super) fn client_grant(&self, actor: &ActorRef) -> Result<&AuthorityGrant, ServingError> {
        self.clients.get(actor).ok_or(ServingError::Unauthenticated)
    }

    pub(super) fn evaluate_client(
        &self,
        actor: &ActorRef,
        operation: AuthorityOperation,
        resources: RequestedResourceFacts,
        budget: AuthorityBudget,
        provenance: AuthorityExecutionProvenance,
    ) -> Result<AuthorityDecisionSnapshot, ServingError> {
        let grant = self.client_grant(actor)?;
        let now = self.now()?;
        let bytes = serde_json::to_vec(&(actor, operation, &resources, &budget, now))
            .map_err(|error| ServingError::Protocol(error.to_string()))?;
        let decision = self
            .authority
            .evaluate(&AuthorityRequest {
                decision: DecisionId::new(format!("decision:{}", blake3::hash(&bytes)))
                    .map_err(|error| ServingError::Configuration(error.to_string()))?,
                actor: actor.clone(),
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
            })
            .map_err(|error| ServingError::Configuration(error.to_string()))?;
        if !decision.is_allowed() {
            return Err(ServingError::Unauthorized(
                "client authority does not grant this operation and resource scope".to_owned(),
            ));
        }
        Ok(decision)
    }

    pub(super) fn authorize_serving_request(
        &self,
        caller: &ServingCaller,
        request: &ServingInvocationRequest,
    ) -> Result<(AuthorityDecisionSnapshot, u64), ServingError> {
        if caller.host != self.config.local_peer || *caller != request.authorization.caller() {
            return Err(ServingError::Unauthorized(
                "serving request belongs to another authenticated caller or host".to_owned(),
            ));
        }
        match &caller.principal {
            ServingPrincipal::Peer { peer } => {
                let relationship = self.relationship(peer)?;
                let generation = self.exact_generation(&relationship, request)?;
                for reference in request
                    .request
                    .inputs()
                    .iter()
                    .filter_map(|input| input.value().artifact())
                    .chain(request.request.context_manifest())
                {
                    let metadata = self.artifacts.metadata(reference)?;
                    let mut resources = RequestedResourceFacts::empty();
                    resources.artifact = Some(metadata.reference().artifact().clone());
                    resources.artifact_sensitivity = Some(metadata.sensitivity());
                    for operation in [
                        AuthorityOperation::ReadArtifactMetadata,
                        AuthorityOperation::ReadArtifactContent,
                    ] {
                        self.require_operation(
                            &relationship,
                            operation,
                            resources.clone(),
                            AuthorityBudget {
                                artifact_bytes: Some(metadata.reference().size_bytes()),
                                ..AuthorityBudget::default()
                            },
                        )?;
                    }
                }
                let decision = self.authorize_invocation(
                    &relationship,
                    request,
                    &generation.descriptor,
                    &generation.authority_requirements,
                    self.now()?,
                )?;
                Ok((decision, relationship_generation(&relationship)))
            }
            ServingPrincipal::Client { actor } => self.authorize_client_invocation(actor, request),
        }
    }

    fn authorize_client_invocation(
        &self,
        actor: &ActorRef,
        request: &ServingInvocationRequest,
    ) -> Result<(AuthorityDecisionSnapshot, u64), ServingError> {
        let grant = self.client_grant(actor)?;
        let policy = self
            .client_policy
            .as_ref()
            .ok_or(ServingError::Unauthenticated)?;
        let ServingAuthorization::Client(basis) = &request.authorization else {
            return Err(ServingError::Unauthorized(
                "client authentication cannot claim a workflow delegation".to_owned(),
            ));
        };
        if basis.grant != *grant.identity()
            || basis.grant_revision != grant.revision()
            || basis.grant_digest
                != grant
                    .digest()
                    .map_err(|error| ServingError::Configuration(error.to_string()))?
            || basis.revocation_generation != grant.revocation_generation()
            || !policy.execution_limits.contains(&request.limits)
        {
            return Err(ServingError::Unauthorized(
                "accepted client authority or per-call limits are no longer available".to_owned(),
            ));
        }
        let _selection =
            crate::DirectInputSelection::new(&request.request, request.limits.artifact_bytes)
                .map_err(|error| ServingError::Protocol(error.to_string()))?;
        let generation = self.exact_generation_in_scope(&grant.resources().capability, request)?;
        if !generation.accepts_direct_inputs {
            return Err(ServingError::Unauthorized(
                "selected adapter requires a workflow origin".to_owned(),
            ));
        }
        let descriptor = &generation.descriptor;
        let requirements = &generation.authority_requirements;
        if requirements
            .budget
            .duration_ms
            .is_some_and(|duration| duration > request.limits.duration_ms)
        {
            return Err(ServingError::Unauthorized(
                "configured adapter duration exceeds the accepted per-call allowance".to_owned(),
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
        resources.side_effect = request.selection.operation_contract().side_effect();
        resources.filesystem = requirements.filesystem.clone();
        resources.network_profiles = requirements.network_profiles.clone();
        resources.network_destinations = requirements.network_destinations.clone();
        resources.secrets = requirements.secrets.clone();
        let decision = self.evaluate_client(
            actor,
            AuthorityOperation::InvokeCapability,
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
            AuthorityExecutionProvenance {
                descriptor_revision: Some(request.selection.descriptor_revision()),
                idempotency: Some(request.selection.operation_contract().idempotency()),
                ..AuthorityExecutionProvenance::default()
            },
        )?;
        for reference in request
            .request
            .inputs()
            .iter()
            .filter_map(|input| input.value().artifact())
        {
            self.authorize_client_artifact(actor, reference)?;
        }
        Ok((
            decision,
            grant
                .revision()
                .saturating_add(grant.revocation_generation()),
        ))
    }

    pub(super) fn authorize_client_artifact(
        &self,
        actor: &ActorRef,
        reference: &milkdrift_capability::ArtifactReference,
    ) -> Result<milkdrift_workspace::ArtifactMetadata, ServingError> {
        let metadata = self
            .artifacts
            .metadata(reference)
            .map_err(|_| ServingError::Unauthorized("artifact input is unavailable".to_owned()))?;
        let mut resources = RequestedResourceFacts::empty();
        resources.artifact = Some(metadata.reference().artifact().clone());
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        for operation in [
            AuthorityOperation::ReadArtifactMetadata,
            AuthorityOperation::ReadArtifactContent,
        ] {
            self.evaluate_client(
                actor,
                operation,
                resources.clone(),
                AuthorityBudget {
                    artifact_bytes: Some(metadata.reference().size_bytes()),
                    ..AuthorityBudget::default()
                },
                AuthorityExecutionProvenance::default(),
            )?;
        }
        Ok(metadata)
    }
}
