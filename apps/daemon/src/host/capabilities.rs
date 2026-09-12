//! Local capability registration, health checks, and authorized read-model ownership.

use super::{Owner, PublicFailure, read_model::bounded, read_model::snake_debug};
use crate::{auth::ActorSession, config::AdapterConfig};
use milkdrift_authority::{AuthorityOperation, CapabilityAuthorityScope, RequestedResourceFacts};
use milkdrift_capability::{CapabilityId, SideEffectClass};
use milkdrift_capability_host::{CapabilityHost, InvocationDataAccess};
use milkdrift_control::{ControlService, WorkflowControlAdapter, workflow_control_descriptor};
use milkdrift_control_protocol::{CapabilityRead, ErrorCode};
use milkdrift_local_process::{LocalProcessAdapter, ProcessProfileDocument};
use milkdrift_local_secret::LocalSecretResolver;
use milkdrift_model_provider::{EndpointProfile, ModelEndpointAdapter, descriptor_for_profile};
use std::{fs, sync::Arc};

pub(super) const CAPABILITY_OBSERVATION_STALE_AFTER_MS: u64 = 60_000;

impl Owner {
    pub(super) fn refresh_capability_health(&self) -> Result<(), PublicFailure> {
        let now = self.now()?;
        let unavailable = |error: milkdrift_capability_host::HostError| {
            PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
        };
        // The host bounds this inventory. Refresh current generations before their observations
        // expire; one broken adapter must not prevent the others from being observed.
        let views = self
            .capability_host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                now,
            )
            .map_err(unavailable)?;
        let mut failure = None;
        for view in views {
            if view.current
                && !view.draining
                && view.observed_at_unix_ms.is_none_or(|observed| {
                    now.saturating_sub(observed) >= CAPABILITY_OBSERVATION_STALE_AFTER_MS / 2
                })
                && let Err(error) = self.capability_host.refresh_health(
                    &view.capability,
                    view.descriptor_revision,
                    now,
                )
            {
                failure.get_or_insert_with(|| unavailable(error));
            }
        }
        failure.map_or(Ok(()), Err)
    }

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
        let now = self.now()?;
        self.capability_host
            .generations(scope, now)
            .map_err(|error| {
                PublicFailure::new(ErrorCode::Unavailable, bounded(&error.to_string()), true)
            })
            .map(|views| {
                views
                    .into_iter()
                    .map(|view| {
                        let operation_contracts = view
                            .operation_contracts
                            .iter()
                            .map(|(operation, contract)| {
                                super::read_model::public_operation_contract(operation, contract)
                            })
                            .collect::<Vec<_>>();
                        CapabilityRead {
                            capability_id: view.capability.as_str().to_owned(),
                            generation: view.descriptor_revision,
                            descriptor_digest: view.descriptor_digest,
                            category: snake_debug(&view.category),
                            operations: operation_contracts
                                .iter()
                                .map(|contract| contract.operation.clone())
                                .collect(),
                            operation_contracts,
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
                        }
                    })
                    .collect()
            })
    }
}

pub(super) fn register_control(
    host: &CapabilityHost,
    control: Arc<ControlService>,
    data: Arc<dyn InvocationDataAccess>,
    store: Arc<dyn milkdrift_persistence::ArtifactStore>,
    workspace: Arc<dyn milkdrift_persistence::WorkspaceStore>,
    authority: Arc<dyn milkdrift_authority::AuthorityEvaluator>,
    observed_at_unix_ms: u64,
) -> Result<(), String> {
    let adapter = Arc::new(WorkflowControlAdapter::new(
        control,
        Arc::new(ResultSink {
            data,
            store,
            workspace,
            authority,
        }),
    ));
    let descriptor = workflow_control_descriptor().map_err(|error| error.to_string())?;
    let capability = descriptor.identity().clone();
    let revision = descriptor.descriptor_revision();
    host.register(descriptor, adapter, None)
        .map_err(|error| error.to_string())?;
    host.refresh_health(&capability, revision, observed_at_unix_ms)
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn register_configured(
    config: &AdapterConfig,
    host: &CapabilityHost,
    data: Arc<dyn InvocationDataAccess>,
    secrets: Arc<LocalSecretResolver>,
    observed_at_unix_ms: u64,
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
        host.refresh_health(&capability, revision, observed_at_unix_ms)
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
        host.refresh_health(&capability, revision, observed_at_unix_ms)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

use milkdrift_capability_host::{AdapterInvocation, MaterializationLimits};
use milkdrift_control::{ControlArtifactAccess, ControlError, MAX_CONTROL_RESULT_BYTES};

struct ResultSink {
    data: Arc<dyn InvocationDataAccess>,
    store: Arc<dyn milkdrift_persistence::ArtifactStore>,
    workspace: Arc<dyn milkdrift_persistence::WorkspaceStore>,
    authority: Arc<dyn milkdrift_authority::AuthorityEvaluator>,
}

impl ControlArtifactAccess for ResultSink {
    fn read(
        &self,
        invocation: &AdapterInvocation<'_>,
        input: &milkdrift_capability::InputReference,
    ) -> Result<(milkdrift_capability::ArtifactReference, Vec<u8>), ControlError> {
        use milkdrift_authority::{AuthorityBudget, AuthorityExecutionProvenance, DecisionId};
        use milkdrift_workspace::ArtifactId;
        let unavailable =
            || ControlError::InvalidContract("acceptance evidence unavailable".to_owned());
        let context = invocation.context().ok_or_else(unavailable)?;
        let basis = context.authority().ok_or_else(unavailable)?;
        let entry = context.entry_authorization().ok_or_else(unavailable)?;
        let reference = match input.value() {
            milkdrift_capability::InvocationValueReference::Artifact { reference } => {
                reference.clone()
            }
            milkdrift_capability::InvocationValueReference::WorkspaceValue {
                identity,
                version,
            } => {
                let reference: milkdrift_workspace::WorkspaceValueReference =
                    serde_json::from_str(identity).map_err(|_| unavailable())?;
                if reference.scope().run() != context.run()
                    || version != &reference.version().get().to_string()
                {
                    return Err(unavailable());
                }
                let mut resources = RequestedResourceFacts::empty();
                resources.workspace_scope = Some(reference.scope().scope().clone());
                let decision = self
                    .authority
                    .evaluate(&basis.request(
                        DecisionId::new(format!(
                                "decision:{}",
                                blake3::hash(
                                    format!(
                                        "{}:{}:acceptance-workspace",
                                        invocation.request().invocation(),
                                        input.name()
                                    )
                                    .as_bytes()
                                )
                            ))?,
                        AuthorityOperation::ReadWorkspaceValue,
                        resources,
                        AuthorityBudget::default(),
                        entry.request().evaluated_at,
                        AuthorityExecutionProvenance {
                            revision: Some(context.revision().clone()),
                            node: Some(context.node().clone()),
                            ..AuthorityExecutionProvenance::default()
                        },
                    ))
                    .map_err(|_| unavailable())?;
                if !decision.is_allowed() {
                    return Err(unavailable());
                }
                let value = self
                    .workspace
                    .value(&reference)
                    .map_err(|_| unavailable())?
                    .ok_or_else(unavailable)?;
                let milkdrift_workspace::WorkspaceValue::Artifact(reference) = value.value() else {
                    return Err(unavailable());
                };
                milkdrift_capability::ArtifactReference::new(
                    reference.artifact().as_str(),
                    reference.digest().to_hex(),
                    Some(reference.media_type().as_str().to_owned()),
                    Some(reference.size_bytes()),
                )
                .map_err(|_| unavailable())?
            }
            _ => return Err(unavailable()),
        };
        let identity = ArtifactId::new(reference.identity()).map_err(|_| unavailable())?;
        let metadata = self
            .store
            .metadata(&identity)
            .map_err(|_| unavailable())?
            .ok_or_else(unavailable)?;
        if reference.digest() != metadata.reference().digest().to_hex()
            || reference.size_bytes() != Some(metadata.reference().size_bytes())
            || reference.media_type() != Some(metadata.reference().media_type().as_str())
        {
            return Err(unavailable());
        }
        let mut resources = RequestedResourceFacts::empty();
        resources.revision = Some(context.revision().clone());
        resources.artifact = Some(identity);
        resources.artifact_sensitivity = Some(metadata.sensitivity());
        let decision = self
            .authority
            .evaluate(&basis.request(
                DecisionId::new(format!(
                        "decision:{}",
                        blake3::hash(
                            format!(
                                "{}:{}:acceptance-read",
                                invocation.request().invocation(),
                                reference.identity()
                            )
                            .as_bytes()
                        )
                    ))?,
                AuthorityOperation::ReadArtifactContent,
                resources,
                AuthorityBudget {
                    artifact_bytes: reference.size_bytes(),
                    ..AuthorityBudget::default()
                },
                entry.request().evaluated_at,
                AuthorityExecutionProvenance {
                    revision: Some(context.revision().clone()),
                    node: Some(context.node().clone()),
                    ..AuthorityExecutionProvenance::default()
                },
            ))
            .map_err(|_| unavailable())?;
        if !decision.is_allowed() {
            return Err(unavailable());
        }
        let bytes = self
            .data
            .read_artifact_bytes(
                context,
                &reference,
                MaterializationLimits {
                    max_files: 1,
                    max_file_bytes: milkdrift_control::MAX_ACCEPTANCE_INPUT_BYTES,
                    max_total_bytes: milkdrift_control::MAX_ACCEPTANCE_INPUT_BYTES,
                    max_path_bytes: 256,
                    max_directory_depth: 8,
                    chunk_bytes: 262_144,
                },
            )
            .map_err(|_| unavailable())?;
        Ok((reference, bytes))
    }

    fn publish(
        &self,
        invocation: &AdapterInvocation<'_>,
        output_name: &str,
        bytes: &[u8],
    ) -> Result<milkdrift_capability::ArtifactReference, ControlError> {
        let context = invocation.context().ok_or_else(|| {
            ControlError::InvalidContract(
                "control result publication requires durable context".to_owned(),
            )
        })?;
        self.data
            .publish_bytes(
                context,
                invocation.request(),
                output_name,
                "application/vnd.milkdrift.control-result+json",
                bytes,
                MaterializationLimits {
                    max_files: 1,
                    max_file_bytes: MAX_CONTROL_RESULT_BYTES,
                    max_total_bytes: MAX_CONTROL_RESULT_BYTES,
                    max_path_bytes: 256,
                    max_directory_depth: 8,
                    chunk_bytes: 262_144,
                }
                .validate()
                .map_err(|error| ControlError::InvalidContract(error.to_string()))?,
            )
            .map_err(|error| ControlError::InvalidContract(error.to_string()))
    }
}
