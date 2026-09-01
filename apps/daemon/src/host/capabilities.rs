//! Local capability registration, health checks, and authorized read-model ownership.

use super::{
    ActorSession, AdapterConfig, Arc, AuthorityOperation, CapabilityHost, CapabilityId,
    CapabilityRead, ConfiguredSecretResolver, ControlService, EndpointProfile, ErrorCode,
    InvocationDataAccess, LocalProcessAdapter, ModelEndpointAdapter, Owner, ProcessProfileDocument,
    PublicFailure, RequestedResourceFacts, ResultSink, WorkflowControlAdapter, bounded,
    descriptor_for_profile, fs, snake_debug, unix_millis, workflow_control_descriptor,
};

impl Owner {
    pub(super) fn capabilities(
        &self,
        session: &ActorSession,
    ) -> Result<Vec<CapabilityRead>, PublicFailure> {
        self.authorize(
            session,
            AuthorityOperation::ListCapabilities,
            RequestedResourceFacts::empty(),
            "read:capabilities",
        )?;
        self.authorize(
            session,
            AuthorityOperation::InspectCapabilityHealth,
            RequestedResourceFacts::empty(),
            "read:capability-health",
        )?;
        self.authorize(
            session,
            AuthorityOperation::InspectProviderProfile,
            RequestedResourceFacts::empty(),
            "read:provider-profile",
        )?;
        let scope = &session.grant.resources().capability;
        self.capability_host
            .generations(scope, unix_millis())
            .map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })
            .map(|views| {
                views
                    .into_iter()
                    .map(|view| CapabilityRead {
                        capability_id: view.capability.as_str().to_owned(),
                        generation: view.descriptor_revision,
                        descriptor_digest: view.descriptor_digest,
                        category: snake_debug(&view.category),
                        operations: view
                            .operations
                            .iter()
                            .map(|operation| operation.as_str().to_owned())
                            .collect(),
                        provider_profile: view
                            .provider_profile
                            .map(|profile| profile.as_str().to_owned()),
                        locality: snake_debug(&view.locality),
                        peer_id: view.peer.map(|peer| peer.as_str().to_owned()),
                        trust_zones: view
                            .trust_zones
                            .iter()
                            .map(|zone| zone.as_str().to_owned())
                            .collect(),
                        execution_trust: snake_debug(&view.execution_trust),
                        current: view.current,
                        draining: view.draining,
                        health: snake_debug(&view.health),
                        available: view.available,
                        active_permits: view.active_permits,
                        permit_limit: view.permit_limit,
                    })
                    .collect()
            })
    }
}

pub(super) fn register_control(
    host: &CapabilityHost,
    control: Arc<ControlService>,
    data: Arc<dyn InvocationDataAccess>,
) -> Result<(), String> {
    let adapter = Arc::new(WorkflowControlAdapter::new(
        control,
        Arc::new(ResultSink { data }),
    ));
    let descriptor = workflow_control_descriptor().map_err(|error| error.to_string())?;
    let capability = descriptor.identity().clone();
    let revision = descriptor.descriptor_revision();
    host.register(descriptor, adapter, None)
        .map_err(|error| error.to_string())?;
    host.refresh_health(&capability, revision, unix_millis())
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn register_configured(
    config: &AdapterConfig,
    host: &CapabilityHost,
    data: Arc<dyn InvocationDataAccess>,
    secrets: Arc<ConfiguredSecretResolver>,
) -> Result<(), String> {
    for path in &config.process_profiles {
        let bytes = fs::read(path)
            .map_err(|error| format!("process profile read failed: {:?}", error.kind()))?;
        let profile = ProcessProfileDocument::from_json(&bytes)
            .map_err(|error| error.to_string())?
            .into_profile();
        let adapter = Arc::new(
            LocalProcessAdapter::new(profile, data.clone(), secrets.clone())
                .map_err(|error| error.to_string())?,
        );
        let descriptor = adapter.descriptor().clone();
        let capability = descriptor.identity().clone();
        let revision = descriptor.descriptor_revision();
        host.register(descriptor, adapter, None)
            .map_err(|error| error.to_string())?;
        host.refresh_health(&capability, revision, unix_millis())
            .map_err(|error| error.to_string())?;
    }
    for configured in &config.model_profiles {
        let bytes = fs::read(&configured.profile)
            .map_err(|error| format!("model profile read failed: {:?}", error.kind()))?;
        let profile = EndpointProfile::from_json(&bytes).map_err(|error| error.to_string())?;
        let capability = CapabilityId::new(configured.capability_id.clone())
            .map_err(|error| error.to_string())?;
        let descriptor = descriptor_for_profile(capability.clone(), &profile)
            .map_err(|error| error.to_string())?;
        let adapter = Arc::new(
            ModelEndpointAdapter::new(capability, profile, secrets.clone(), data.clone())
                .map_err(|error| error.to_string())?,
        );
        let capability = descriptor.identity().clone();
        let revision = descriptor.descriptor_revision();
        host.register(descriptor, adapter, None)
            .map_err(|error| error.to_string())?;
        host.refresh_health(&capability, revision, unix_millis())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
