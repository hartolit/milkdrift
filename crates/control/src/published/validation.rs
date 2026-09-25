//! A publication must have supported implementations for both its starting tasks and every
//! capability envelope that its agreement permits future revisions to introduce.
use super::{PublishedWorkflowService, failure, rejected};
use milkdrift_authority::CapabilityAuthorityScope;
use milkdrift_blueprint::{NodeKind, ReducerStrategy};
use milkdrift_capability::{CapabilityRequirement, OperationId, SideEffectClass};
use milkdrift_capability_host::CapabilityHost;
use milkdrift_persistence::published::PublishedMethod;
use milkdrift_runtime::ExecutorError;
use std::collections::{BTreeSet, VecDeque};

impl PublishedWorkflowService {
    pub(super) fn validate_implementations(
        &self,
        method: &PublishedMethod,
        host: &CapabilityHost,
    ) -> Result<(), ExecutorError> {
        let catalog = host
            .catalog_generations(&CapabilityAuthorityScope::allow_any(
                SideEffectClass::Unknown,
            ))
            .map_err(failure)?;
        let maximum = method
            .descriptor
            .operation(&OperationId::new("method.invoke").map_err(failure)?)
            .ok_or_else(|| rejected("public operation is absent"))?
            .side_effect();
        let basis = milkdrift_authority::AuthorityRequest {
            decision: milkdrift_authority::DecisionId::new("publication:service-validation")
                .map_err(failure)?,
            actor: method.service.actor.clone(),
            grant: method.service.grant.clone(),
            grant_revision: method.service.grant_revision,
            grant_digest: method.service.grant_digest.clone(),
            revocation_generation: method.service.revocation_generation,
            operation: milkdrift_authority::AuthorityOperation::InvokeCapability,
            resources: milkdrift_authority::RequestedResourceFacts::empty(),
            budget: milkdrift_authority::AuthorityBudget::default(),
            evaluated_at: milkdrift_authority::BoundaryTimeMillis::new(
                self.clock.now().map_err(failure)?.get(),
            ),
            provenance: milkdrift_authority::AuthorityExecutionProvenance::default(),
        };
        let mut pending = VecDeque::from([method.revision.clone()]);
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop_front() {
            if !visited.insert(id.clone()) {
                continue;
            }
            if visited.len() > 512 {
                return Err(rejected(
                    "publication revision graph exceeds the supported walk bound",
                ));
            }
            let revision = self
                .store
                .revision(&id)
                .map_err(failure)?
                .ok_or_else(|| rejected("publication references an absent definition"))?;
            for operation in [
                milkdrift_authority::AuthorityOperation::CreateRun,
                milkdrift_authority::AuthorityOperation::StartRun,
            ] {
                let mut request = basis.clone();
                request.operation = operation;
                request.resources.workflow = Some(revision.semantic().workflow().clone());
                if operation == milkdrift_authority::AuthorityOperation::CreateRun {
                    request.budget.artifact_bytes =
                        Some(method.workspace_budget.max_total_artifact_bytes());
                }
                self.current_decision(request)?;
            }
            let mut requirements = revision
                .semantic()
                .agreement()
                .map_or_else(Vec::new, |agreement| {
                    agreement.scope().requirements().to_vec()
                });
            for node in revision.semantic().nodes().values() {
                if let Some(policy) =
                    crate::ControllerPolicyDocument::from_revision(&revision, node.id())
                        .map_err(failure)?
                    && !method.allowance.fits_within(
                        &crate::ControllerLifecycleOwner::resource_budget(&policy)
                            .map_err(failure)?,
                    )
                {
                    return Err(rejected(
                        "published allowance must fit every internal controller policy without opening another account",
                    ));
                }
                match node.kind() {
                    NodeKind::Task { config } => requirements.push(config.requirement().clone()),
                    NodeKind::Reducer { config } => {
                        if let ReducerStrategy::Capability(operation) = config.strategy() {
                            requirements.push(CapabilityRequirement::new(operation.clone()));
                        }
                    }
                    NodeKind::Subworkflow { reference } => {
                        pending.push_back(reference.revision().clone())
                    }
                    NodeKind::Repeat { config } => {
                        pending.push_back(config.body().revision().clone())
                    }
                    NodeKind::Branch { .. }
                    | NodeKind::Fork { .. }
                    | NodeKind::Join { .. }
                    | NodeKind::Wait { .. }
                    | NodeKind::SignalWait { .. }
                    | NodeKind::Terminal { .. } => {}
                }
            }
            for requirement in requirements {
                if requirement.maximum_side_effect_class() > maximum {
                    return Err(rejected(
                        "public side-effect contract understates internal permitted work",
                    ));
                }
                let mut permitted = false;
                for generation in &catalog {
                    if !generation.current
                        || generation.draining
                        || !generation.descriptor.matches(&requirement).is_match()
                    {
                        continue;
                    }
                    let descriptor = &generation.descriptor;
                    let needs = &generation.authority_requirements;
                    let mut resources = milkdrift_authority::RequestedResourceFacts::empty();
                    resources.workflow = Some(revision.semantic().workflow().clone());
                    resources.capability = Some(descriptor.identity().clone());
                    // Match runtime's prospective check: unnamed implementations select one
                    // granted identity, while all other declared envelope dimensions stay fixed.
                    resources.capability_envelope = Some(
                        CapabilityAuthorityScope::requirement_envelope(
                            &requirement.clone().exact(descriptor.identity().clone()),
                        )
                        .map_err(failure)?,
                    );
                    resources.category = Some(descriptor.category().clone());
                    resources.capability_operation = Some(requirement.operation().clone());
                    resources.provider_profile = descriptor.provider_profile().cloned();
                    resources.locality = Some(descriptor.locality());
                    resources.peer = descriptor.peer().cloned();
                    resources.trust_zones = descriptor.trust_zones().clone();
                    resources.execution_trust_class = Some(descriptor.execution_trust());
                    resources.side_effect = requirement.maximum_side_effect_class();
                    resources.filesystem = needs.filesystem.clone();
                    resources.network_profiles = needs.network_profiles.clone();
                    resources.network_destinations = needs.network_destinations.clone();
                    resources.secrets = needs.secrets.clone();
                    let mut request = basis.clone();
                    request.resources = resources;
                    request.budget = needs.budget;
                    if self.current_decision(request).is_ok() {
                        permitted = true;
                        break;
                    }
                }
                if !permitted {
                    return Err(rejected(
                        "publication requires a current registered implementation within its service grant",
                    ));
                }
            }
        }
        Ok(())
    }
}
