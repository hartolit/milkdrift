//! Existing provider mapping with a durable service-generation lifetime hold.
use crate::{LinuxManagedPlatform, ModelService, rejected, units};
use milkdrift_capability::managed::{
    MANAGED_BINDING_EXTENSION, ManagedBinding, ManagedName, ResourceRequirement,
};
use milkdrift_capability::{
    CapabilityDescriptor, CapabilityId, InvocationEvent, InvocationEventKind, ProviderProfileRef,
    TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterInvocation, AdapterReporter, CapabilityAdapter, InvocationDataAccess,
    SecretResolver, managed::ManagedError,
};
use milkdrift_model_provider::{
    AuthMode, BillingTerms, EndpointProfile, ModelEndpointAdapter, ModelFeature, ProviderProtocol,
    ProxyPolicy, RedirectPolicy, TlsPolicy, descriptor_for_profile,
};
use milkdrift_persistence::managed::{
    ApprovedSetup, ManagedResourceStore, QuiescenceEvidence, managed_use_id,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool},
};

fn profile(setup: &ApprovedSetup) -> Result<Option<EndpointProfile>, ManagedError> {
    let d = units::deployment(setup)?;
    profile_for_service(&d.recipe.model_service, &d.installation, d.generation)
}

pub(crate) fn profile_for_service(
    service: &ModelService,
    installation: &ManagedName,
    generation: u64,
) -> Result<Option<EndpointProfile>, ManagedError> {
    let (base, alias, billing, token_limits, endpoint_limits) = match service {
        ModelService::Disabled {} => return Ok(None),
        ModelService::Attached {
            api_base,
            model_alias,
            billing,
            token_limits,
            endpoint_limits,
        } => (
            format!("{}/", api_base.trim_end_matches('/')),
            model_alias.clone(),
            billing.clone(),
            token_limits.clone(),
            *endpoint_limits,
        ),
        ModelService::Owned {
            port,
            token_limits,
            model_alias,
            endpoint_limits,
            ..
        } => (
            format!("http://127.0.0.1:{port}/v1/"),
            model_alias.clone(),
            BillingTerms::Unbilled {
                source: "Owned self-hosted service; no provider billing".to_owned(),
            },
            token_limits.clone(),
            *endpoint_limits,
        ),
    };
    if matches!(billing, BillingTerms::Unknown)
        || matches!(
            token_limits,
            milkdrift_model_provider::ModelTokenLimits::Unknown
        )
    {
        return Err(rejected(
            "model_service requires explicit finite billing and token contracts",
        ));
    }
    let host = url::Url::parse(&base)
        .map_err(rejected)?
        .host_str()
        .ok_or_else(|| rejected("model endpoint requires a host"))?
        .to_owned();
    EndpointProfile::new(
        ProviderProfileRef::new(format!("managed.{installation}.model")).map_err(rejected)?,
        generation,
        ProviderProtocol::OpenAiCompatible {
            path: "chat/completions".to_owned(),
        },
        base,
        alias,
        AuthMode::NoAuth,
        endpoint_limits,
        RedirectPolicy::Deny,
        TlsPolicy::WebPkiRoots,
        ProxyPolicy::Disabled,
        BTreeSet::from([ModelFeature::Streaming, ModelFeature::SystemRole]),
        1,
        true,
        BTreeSet::from([host]),
        BTreeSet::new(),
        BTreeMap::new(),
        billing,
        token_limits,
    )
    .map(Some)
    .map_err(rejected)
}

pub(crate) fn descriptor(
    setup: &ApprovedSetup,
) -> Result<Option<CapabilityDescriptor>, ManagedError> {
    let Some(profile) = profile(setup)? else {
        return Ok(None);
    };
    let d = units::deployment(setup)?;
    let descriptor = descriptor_for_profile(
        CapabilityId::new(format!("managed.{}.model", d.installation)).map_err(rejected)?,
        &profile,
    )
    .map_err(rejected)?;
    let mut document = serde_json::to_value(descriptor).map_err(rejected)?;
    let binding = ManagedBinding {
        installation: d.installation,
        generation: d.generation,
        recipe_digest: setup.recipe.digest.clone(),
        resources: vec![ResourceRequirement {
            resource: ManagedName::new("model").map_err(rejected)?,
            mutation: false,
        }],
    };
    document["extensions"][MANAGED_BINDING_EXTENSION] =
        serde_json::to_value(binding).map_err(rejected)?;
    serde_json::from_value(document).map(Some).map_err(rejected)
}

/// Service calls reuse the normal model adapter. Loss of a response keeps the use uncertain;
/// stopping an owned server is a separate authorized resource action and invents no model result.
pub struct ManagedModelAdapter {
    inner: Arc<ModelEndpointAdapter>,
    platform: Arc<LinuxManagedPlatform>,
    store: Arc<dyn ManagedResourceStore>,
    setup: ApprovedSetup,
}
impl ManagedModelAdapter {
    /// Construct the exact model descriptor already committed by installation verification.
    pub fn new(
        platform: Arc<LinuxManagedPlatform>,
        store: Arc<dyn ManagedResourceStore>,
        setup: ApprovedSetup,
        secrets: Arc<dyn SecretResolver>,
        data: Arc<dyn InvocationDataAccess>,
    ) -> Result<Self, ManagedError> {
        let d = platform.check_owner(&setup)?;
        let profile = profile(&setup)?.ok_or_else(|| rejected("model service is disabled"))?;
        let inner = ModelEndpointAdapter::new(
            CapabilityId::new(format!("managed.{}.model", d.installation)).map_err(rejected)?,
            profile,
            secrets,
            data,
        )
        .map_err(rejected)?;
        Ok(Self {
            inner: Arc::new(inner),
            platform,
            store,
            setup,
        })
    }
}
fn fail(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::external_failure(error.to_string())
}

impl CapabilityAdapter for ManagedModelAdapter {
    fn accepts_direct_inputs(&self) -> bool {
        true
    }
    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<milkdrift_capability_host::PreparedAdapterExecution, AdapterError> {
        let prepared = self.inner.clone().prepare(invocation)?;
        Ok(
            prepared.with_entry_wrapper(move |invocation, reporter, entry| {
                self.execute_guarded(invocation, reporter, |reporter| entry(invocation, reporter))
            }),
        )
    }
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<milkdrift_capability::InvocationAdmissionEnvelope, AdapterError> {
        self.inner.admission_envelope(invocation)
    }
    fn authority_requirements(&self) -> milkdrift_authority::CapabilityExecutionRequirements {
        self.inner.authority_requirements()
    }
    fn start(&self) -> Result<(), AdapterError> {
        self.inner.start()
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.execute_guarded(invocation, reporter, |reporter| {
            self.inner.execute(invocation, reporter)
        })
    }
    fn cancel(
        &self,
        request: &milkdrift_capability::CancellationRequest,
    ) -> Result<milkdrift_capability::CancellationAcknowledgement, AdapterError> {
        self.inner.cancel(request)
    }
    fn health(
        &self,
        now: u64,
    ) -> Result<milkdrift_capability::CapabilityObservation, AdapterError> {
        let d = units::deployment(&self.setup).map_err(fail)?;
        let ready = self
            .store
            .managed_installation(&d.installation)
            .map_err(fail)?
            .is_some_and(|r| {
                r.admission_open
                    && r.generation == d.generation
                    && (matches!(d.recipe.model_service, ModelService::Attached { .. })
                        || r.observation.as_ref().is_some_and(|o| o.running))
            });
        milkdrift_capability::CapabilityObservation::new(
            CapabilityId::new(format!("managed.{}.model", d.installation)).map_err(fail)?,
            now,
            ready
                && self.inner.health(now)?.available()
                && (matches!(d.recipe.model_service, ModelService::Attached { .. })
                    || milkdrift_capability_host::managed::ManagedPlatform::observe(
                        self.platform.as_ref(),
                        &self.setup,
                    )
                    .is_ok_and(|o| o.running)),
            0,
            "verified managed model generation",
        )
        .map_err(fail)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.inner.begin_drain()
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        self.inner.shutdown()
    }
}

struct ActiveCall {
    platform: Arc<LinuxManagedPlatform>,
    id: String,
}
impl Drop for ActiveCall {
    fn drop(&mut self) {
        if let Ok(mut active) = self.platform.active.lock() {
            active.remove(&self.id);
            self.platform.quiescent.notify_all();
        }
    }
}
struct ResponseProof<'a> {
    target: &'a dyn AdapterReporter,
    store: &'a dyn ManagedResourceStore,
    id: &'a str,
    claim: u64,
    physical: &'a str,
}
impl AdapterReporter for ResponseProof<'_> {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        if matches!(event.kind(), InvocationEventKind::Terminal { terminal } if terminal.status() == TerminalStatus::Success)
        {
            self.store
                .quiesce_managed_use(
                    self.id,
                    self.claim,
                    &QuiescenceEvidence {
                        physical_identity: self.physical.to_owned(),
                        observation_digest: crate::digest(
                            serde_json::to_vec(&event).map_err(fail)?,
                        ),
                        disrupted: false,
                    },
                )
                .map_err(fail)?;
        }
        self.target.invocation(event)
    }
    fn heartbeat(&self) -> Result<(), AdapterError> {
        self.target.heartbeat()
    }
}

impl ManagedModelAdapter {
    fn execute_guarded(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
        work: impl FnOnce(&dyn AdapterReporter) -> Result<(), AdapterError>,
    ) -> Result<(), AdapterError> {
        let id = managed_use_id(invocation.request().invocation());
        let resource = self
            .setup
            .resources
            .iter()
            .find(|r| r.name.as_str() == "model")
            .ok_or_else(|| fail("model service identity absent"))?;
        {
            let mut active = self.platform.active.lock().map_err(fail)?;
            if active.len() >= milkdrift_capability::managed::MAX_MANAGED_USES
                || active.contains_key(&id)
            {
                return Err(fail("model creator is duplicated or capacity is exhausted"));
            }
            active.insert(id.clone(), Arc::new(AtomicBool::new(false)));
        }
        let _active = ActiveCall {
            platform: self.platform.clone(),
            id: id.clone(),
        };
        let usage = self
            .store
            .managed_use(&id)
            .map_err(fail)?
            .ok_or_else(|| fail("model call lacks durable generation hold"))?;
        self.store
            .enter_managed_use(&id, usage.claim, &resource.identity)
            .map_err(fail)?;
        work(&ResponseProof {
            target: reporter,
            store: self.store.as_ref(),
            id: &id,
            claim: usage.claim,
            physical: &resource.identity,
        })
    }
}
