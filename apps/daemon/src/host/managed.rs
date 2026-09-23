//! Production composition for approved Linux resources and their registry projection.
use super::clock::DurableClock;
use milkdrift_authority::CapabilityAuthorityScope;
use milkdrift_capability::SideEffectClass;
use milkdrift_capability_host::{
    CapabilityHost, InvocationDataAccess,
    managed::{
        ManagedError, ManagedGenerationPublisher, ManagedLifecycleAdapter, ManagedResources,
        managed_lifecycle_descriptor,
    },
};
use milkdrift_managed_linux::{LinuxManagedPlatform, LinuxManagerConfig, ManagedWorkerAdapter};
use milkdrift_persistence::managed::InstallationRecord;
use milkdrift_redb_store::RedbStore;
use std::sync::Arc;

pub(super) struct ManagedHost {
    pub(super) resources: Arc<ManagedResources>,
    // Registry adapters retain only a weak publisher to avoid a host/adapter ownership cycle.
    _publisher: Arc<dyn ManagedGenerationPublisher>,
}

struct RegistryProjection {
    host: CapabilityHost,
    platform: Arc<LinuxManagedPlatform>,
    store: Arc<RedbStore>,
    data: Arc<dyn InvocationDataAccess>,
    clock: DurableClock,
    secrets: Arc<dyn milkdrift_capability_host::SecretResolver>,
}

impl ManagedGenerationPublisher for RegistryProjection {
    fn synchronize(&self, record: &InstallationRecord) -> Result<(), ManagedError> {
        let now = self.clock.now().map_err(failure)?.get();
        let descriptors = record
            .current
            .as_ref()
            .map(|setup| setup.capabilities.as_slice())
            .unwrap_or_default();
        let prefix = format!("managed.{}.", record.name);
        for generation in self
            .host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                now,
            )
            .map_err(failure)?
        {
            if !generation.capability.as_str().starts_with(&prefix) {
                continue;
            }
            if record.removed
                || (record.pending.is_none()
                    && !descriptors.iter().any(|d| {
                        d.identity() == &generation.capability
                            && d.descriptor_revision() == generation.descriptor_revision
                    }))
            {
                self.host
                    .begin_drain(&generation.capability, generation.descriptor_revision)
                    .map_err(failure)?;
                if generation.active_permits == 0 {
                    self.host
                        .finish_drain(&generation.capability, generation.descriptor_revision)
                        .map_err(failure)?;
                }
            }
        }
        if !record.admission_open || record.removed || record.pending.is_some() {
            return Ok(());
        }
        let setup = record
            .current
            .as_ref()
            .ok_or_else(|| failure("verified setup is absent"))?;
        for descriptor in descriptors {
            let adapter: Arc<dyn milkdrift_capability_host::CapabilityAdapter> =
                if *descriptor.category() == milkdrift_capability::CapabilityCategory::Model {
                    Arc::new(milkdrift_managed_linux::ManagedModelAdapter::new(
                        self.platform.clone(),
                        self.store.clone(),
                        setup.clone(),
                        self.secrets.clone(),
                        self.data.clone(),
                    )?)
                } else {
                    Arc::new(ManagedWorkerAdapter::new(
                        self.platform.clone(),
                        self.store.clone(),
                        setup.clone(),
                        self.data.clone(),
                    )?)
                };
            self.host
                .register(descriptor.clone(), adapter, None)
                .map_err(failure)?;
            self.host
                .refresh_health(descriptor.identity(), descriptor.descriptor_revision(), now)
                .map_err(failure)?;
        }
        Ok(())
    }
}

impl ManagedHost {
    pub(super) fn open(
        config: LinuxManagerConfig,
        host: CapabilityHost,
        store: Arc<RedbStore>,
        data: Arc<dyn InvocationDataAccess>,
        authority: Arc<dyn milkdrift_authority::AuthorityEvaluator>,
        secrets: Arc<dyn milkdrift_capability_host::SecretResolver>,
        clock: DurableClock,
    ) -> Result<Arc<Self>, String> {
        let platform = Arc::new(LinuxManagedPlatform::new(config).map_err(|e| e.to_string())?);
        let publisher: Arc<dyn ManagedGenerationPublisher> = Arc::new(RegistryProjection {
            host: host.clone(),
            platform: platform.clone(),
            store: store.clone(),
            data: data.clone(),
            secrets,
            clock: clock.clone(),
        });
        let resources = Arc::new(
            ManagedResources::new(store.clone(), platform, authority, clock.runtime_adapter())
                .with_artifacts(store)
                .with_publisher(Arc::downgrade(&publisher)),
        );
        resources.recover_startup().map_err(|e| e.to_string())?;
        let descriptor = managed_lifecycle_descriptor().map_err(|e| e.to_string())?;
        host.register(
            descriptor.clone(),
            Arc::new(ManagedLifecycleAdapter::new(resources.clone(), data)),
            None,
        )
        .map_err(|e| e.to_string())?;
        host.refresh_health(
            descriptor.identity(),
            descriptor.descriptor_revision(),
            clock.now().map_err(|e| e.to_string())?.get(),
        )
        .map_err(|e| e.to_string())?;
        Ok(Arc::new(Self {
            resources,
            _publisher: publisher,
        }))
    }
}

fn failure(error: impl std::fmt::Display) -> ManagedError {
    ManagedError::Platform(error.to_string())
}

impl super::DaemonHost {
    pub(crate) fn manage_resources(
        &self,
        session: &crate::auth::ActorSession,
        request: &milkdrift_capability::managed::ManagedRequest,
    ) -> Result<milkdrift_capability::managed::ManagedResponse, ManagedError> {
        use milkdrift_authority::{
            AuthorityOperation, AuthorityRequest, BoundaryTimeMillis, DecisionId,
            RequestedResourceFacts,
        };
        if !self.accepting_mutations() {
            return Err(ManagedError::Conflict("host is draining".to_owned()));
        }
        let managed = self.managed.as_ref().ok_or_else(|| {
            ManagedError::Rejected("managed Linux setup is not configured".to_owned())
        })?;
        let claim = session.context.authority();
        let caller = AuthorityRequest {
            decision: DecisionId::new("managed-api").map_err(failure)?,
            actor: session.actor.clone(),
            grant: claim.grant().clone(),
            grant_revision: claim.grant_revision(),
            grant_digest: claim.grant_digest().clone(),
            revocation_generation: claim.revocation_generation(),
            operation: AuthorityOperation::AdministerCapabilities,
            resources: RequestedResourceFacts::empty(),
            budget: Default::default(),
            evaluated_at: BoundaryTimeMillis::new(0),
            provenance: Default::default(),
        };
        managed.resources.execute(&caller, request)
    }
}
