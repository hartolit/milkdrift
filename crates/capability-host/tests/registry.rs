//! Deterministic live-registry, generation, admission, cancellation, and shutdown evidence.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use milkdrift_authority::{AuthorityBudget, CapabilityAuthorityScope, SecretRef};
use milkdrift_capability::{
    AdmissionConstraints, CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor,
    CapabilityDescriptorDocument, CapabilityId, CapabilityObservation, CapabilityRequirement,
    DescriptorBuilder, InvocationEvent, InvocationEventKind, InvocationId, InvocationRequest,
    InvocationTerminal, OperationId, ProviderProfileRef, ResolvedCapabilitySnapshot,
    SideEffectClass, TerminalStatus,
};
use milkdrift_capability_host::{
    AdapterError, AdapterFailureKind, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    CapabilityHost, CapabilitySelectionPolicy, GenerationHealth, HostConfig, HostError,
    InMemorySecretResolver, RegistrationOutcome, SecretResolver,
};
use milkdrift_runtime::ExecutorError;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn descriptor(
    identity: &str,
    revision: u64,
    profile: &str,
    limit: u32,
) -> TestResult<CapabilityDescriptor> {
    let base = CapabilityDescriptorDocument::from_json(include_bytes!(
        "../../capability/tests/fixtures/descriptor-v1.json"
    ))?
    .body()
    .clone();
    Ok(DescriptorBuilder::new(
        CapabilityId::new(identity)?,
        revision,
        base.category().clone(),
        AdmissionConstraints::new(limit, 0)?,
        base.locality(),
    )
    .provider_profile(Some(ProviderProfileRef::new(profile)?))
    .operations(base.operations().clone())
    .trust_zones(base.trust_zones().clone())
    .resource_observations(base.resource_observations().cloned())
    .labels(base.labels().clone())
    .extensions(base.extensions().clone())
    .build()?)
}

fn observation(
    descriptor: &CapabilityDescriptor,
    at: u64,
    available: bool,
) -> TestResult<CapabilityObservation> {
    Ok(CapabilityObservation::new(
        descriptor.identity().clone(),
        at,
        available,
        0,
        if available { "healthy" } else { "unavailable" },
    )?)
}

fn host(
    priorities: BTreeMap<CapabilityId, i32>,
    max_generations: usize,
) -> TestResult<CapabilityHost> {
    Ok(CapabilityHost::new(
        HostConfig {
            max_registrations: 8,
            max_generations_per_capability: max_generations,
            max_concurrent_per_generation: 2,
            observation_stale_after_ms: 100,
        },
        CapabilitySelectionPolicy::new(
            CapabilityAuthorityScope::any(SideEffectClass::Unknown),
            AuthorityBudget {
                concurrency: Some(8),
                ..AuthorityBudget::default()
            },
            priorities,
        ),
    )?)
}

#[derive(Default)]
struct AdapterControl {
    entered: bool,
    release: bool,
}

struct FakeAdapter {
    capability: CapabilityId,
    execute_count: AtomicUsize,
    cancel_count: AtomicUsize,
    panic_execute: AtomicBool,
    block: Option<Arc<(Mutex<AdapterControl>, Condvar)>>,
}

impl FakeAdapter {
    fn new(capability: CapabilityId) -> Self {
        Self {
            capability,
            execute_count: AtomicUsize::new(0),
            cancel_count: AtomicUsize::new(0),
            panic_execute: AtomicBool::new(false),
            block: None,
        }
    }

    fn blocking(capability: CapabilityId, block: Arc<(Mutex<AdapterControl>, Condvar)>) -> Self {
        Self {
            block: Some(block),
            ..Self::new(capability)
        }
    }
}

impl CapabilityAdapter for FakeAdapter {
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.execute_count.fetch_add(1, Ordering::SeqCst);
        assert!(
            !self.panic_execute.load(Ordering::SeqCst),
            "fake adapter panic"
        );
        if let Some(block) = &self.block {
            let (lock, changed) = &**block;
            let mut state = lock
                .lock()
                .map_err(|_error| AdapterError::external_failure("block lock"))?;
            state.entered = true;
            changed.notify_all();
            while !state.release {
                state = changed
                    .wait(state)
                    .map_err(|_error| AdapterError::external_failure("block wait"))?;
            }
        }
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            invocation.resolution().operation_contract().side_effect(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        let event = InvocationEvent::new(
            invocation.request().invocation().clone(),
            1,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        reporter.invocation(event)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            None,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

struct FailingAdapter {
    capability: CapabilityId,
    kind: AdapterFailureKind,
}

impl CapabilityAdapter for FailingAdapter {
    fn execute(
        &self,
        _invocation: &AdapterInvocation<'_>,
        _reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        Err(match self.kind {
            AdapterFailureKind::Rejected => AdapterError::rejected("planned rejection"),
            AdapterFailureKind::Unavailable => AdapterError::unavailable("planned unavailable"),
            AdapterFailureKind::ExternalFailure => {
                AdapterError::external_failure("planned external failure")
            }
        })
    }

    fn cancel(
        &self,
        _request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        Err(AdapterError::unavailable("planned unavailable"))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

#[derive(Default)]
struct CountingReporter(AtomicUsize);

impl AdapterReporter for CountingReporter {
    fn invocation(&self, _event: InvocationEvent) -> Result<(), AdapterError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

fn request(snapshot: &ResolvedCapabilitySnapshot, identity: &str) -> TestResult<InvocationRequest> {
    Ok(InvocationRequest::new(
        InvocationId::new(identity)?,
        snapshot.capability().clone(),
        snapshot.operation().clone(),
        snapshot.provider_profile().cloned(),
        None,
        Vec::new(),
        BTreeMap::new(),
    )?)
}

#[test]
fn resolution_is_stable_policy_constrained_and_health_aware() -> TestResult {
    let a = descriptor("cap-a", 1, "profile-a", 2)?;
    let b = descriptor("cap-b", 1, "profile-b", 2)?;
    let host_one = host(BTreeMap::new(), 2)?;
    host_one.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    host_one.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    let host_two = host(BTreeMap::new(), 2)?;
    host_two.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    host_two.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
    assert_eq!(
        host_one
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        a.identity()
    );
    assert_eq!(
        host_two
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        a.identity()
    );

    let profile = requirement
        .clone()
        .provider_profile(ProviderProfileRef::new("profile-b")?);
    assert_eq!(
        host_one.resolve_at(&profile, 150)?.snapshot().capability(),
        b.identity()
    );
    assert!(matches!(
        host_one.resolve_at(&requirement, 201),
        Err(ExecutorError::Unavailable(_))
    ));
    let views = host_one.generations(
        &CapabilityAuthorityScope::any(SideEffectClass::Unknown),
        201,
    )?;
    assert!(
        views
            .iter()
            .all(|view| view.health == GenerationHealth::Stale)
    );

    let priority_host = host(BTreeMap::from([(b.identity().clone(), 10)]), 2)?;
    priority_host.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    priority_host.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    assert_eq!(
        priority_host
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        b.identity()
    );

    let constrained_host = CapabilityHost::new(
        HostConfig {
            max_registrations: 8,
            max_generations_per_capability: 2,
            max_concurrent_per_generation: 2,
            observation_stale_after_ms: 100,
        },
        CapabilitySelectionPolicy::new(
            CapabilityAuthorityScope::new(
                BTreeSet::from([b.identity().clone()]),
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::new(),
                BTreeSet::new(),
                SideEffectClass::Unknown,
            )?,
            AuthorityBudget {
                concurrency: Some(8),
                ..AuthorityBudget::default()
            },
            BTreeMap::new(),
        ),
    )?;
    constrained_host.register(
        a.clone(),
        Arc::new(FakeAdapter::new(a.identity().clone())),
        Some(observation(&a, 100, true)?),
    )?;
    constrained_host.register(
        b.clone(),
        Arc::new(FakeAdapter::new(b.identity().clone())),
        Some(observation(&b, 100, true)?),
    )?;
    assert_eq!(
        constrained_host
            .resolve_at(&requirement, 150)?
            .snapshot()
            .capability(),
        b.identity()
    );
    assert_eq!(
        constrained_host
            .generations(
                &CapabilityAuthorityScope::new(
                    BTreeSet::from([a.identity().clone()]),
                    BTreeSet::new(),
                    BTreeSet::new(),
                    BTreeSet::new(),
                    BTreeSet::new(),
                    BTreeSet::new(),
                    SideEffectClass::Unknown,
                )?,
                150,
            )?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn descriptor_generations_are_immutable_drained_and_never_fallback() -> TestResult {
    let host = host(BTreeMap::new(), 2)?;
    let old = descriptor("cap-generation", 1, "profile-generation", 2)?;
    let new = descriptor("cap-generation", 2, "profile-generation", 2)?;
    let old_adapter = Arc::new(FakeAdapter::new(old.identity().clone()));
    assert_eq!(
        host.register(
            old.clone(),
            old_adapter.clone(),
            Some(observation(&old, 100, true)?)
        )?,
        RegistrationOutcome::Registered
    );
    assert_eq!(
        host.register(
            old.clone(),
            old_adapter,
            Some(observation(&old, 100, true)?)
        )?,
        RegistrationOutcome::Idempotent
    );
    let conflict = descriptor("cap-generation", 1, "other-profile", 2)?;
    assert!(matches!(
        host.register(
            conflict,
            Arc::new(FakeAdapter::new(old.identity().clone())),
            None
        ),
        Err(HostError::ConflictingRevision { .. })
    ));

    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?)
        .exact(old.identity().clone());
    let old_snapshot = host.resolve_at(&requirement, 150)?.snapshot().clone();
    host.register(
        new.clone(),
        Arc::new(FakeAdapter::new(new.identity().clone())),
        Some(observation(&new, 100, true)?),
    )?;
    assert_eq!(
        host.resolve_at(&requirement, 150)?
            .snapshot()
            .descriptor_revision(),
        2
    );
    let reporter = CountingReporter::default();
    host.execute_exact(
        &old_snapshot,
        &request(&old_snapshot, "invocation-old")?,
        &reporter,
    )?;
    host.begin_drain(old.identity(), 1)?;
    host.execute_exact(
        &old_snapshot,
        &request(&old_snapshot, "invocation-old-draining")?,
        &reporter,
    )?;
    host.finish_drain(old.identity(), 1)?;
    assert!(matches!(
        host.execute_exact(
            &old_snapshot,
            &request(&old_snapshot, "invocation-removed")?,
            &reporter
        ),
        Err(ExecutorError::UnavailableGeneration { .. })
    ));
    assert_eq!(reporter.0.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn permits_cancel_exact_owner_and_release_after_panic_and_completion() -> TestResult {
    let host = host(BTreeMap::new(), 2)?;
    let descriptor = descriptor("cap-admission", 1, "profile-admission", 1)?;
    let block = Arc::new((Mutex::new(AdapterControl::default()), Condvar::new()));
    let adapter = Arc::new(FakeAdapter::blocking(
        descriptor.identity().clone(),
        block.clone(),
    ));
    host.register(
        descriptor.clone(),
        adapter.clone(),
        Some(observation(&descriptor, 100, true)?),
    )?;
    let requirement = CapabilityRequirement::new(OperationId::new("model.generate")?);
    let snapshot = host.resolve_at(&requirement, 150)?.snapshot().clone();
    let first_request = request(&snapshot, "invocation-blocked")?;
    let thread_host = host.clone();
    let thread_snapshot = snapshot.clone();
    let thread_request = first_request.clone();
    let thread = std::thread::spawn(move || {
        thread_host.execute_exact(
            &thread_snapshot,
            &thread_request,
            &CountingReporter::default(),
        )
    });
    {
        let (lock, changed) = &*block;
        let mut state = lock.lock().map_err(|_error| "block lock poisoned")?;
        while !state.entered {
            state = changed
                .wait(state)
                .map_err(|_error| "block wait poisoned")?;
        }
    }
    assert!(matches!(
        host.execute_exact(
            &snapshot,
            &request(&snapshot, "invocation-overload")?,
            &CountingReporter::default()
        ),
        Err(ExecutorError::Overloaded(_))
    ));
    let cancellation = CancellationRequest::new(first_request.invocation().clone(), 1, "stop")?;
    assert!(milkdrift_runtime::TaskExecutor::cancel(&host, &cancellation)?.accepted());
    assert_eq!(adapter.cancel_count.load(Ordering::SeqCst), 1);
    {
        let (lock, changed) = &*block;
        let mut state = lock.lock().map_err(|_error| "block lock poisoned")?;
        state.release = true;
        changed.notify_all();
    }
    thread
        .join()
        .map_err(|_panic| "adapter thread panicked")??;
    let views = host.generations(
        &CapabilityAuthorityScope::any(SideEffectClass::Unknown),
        150,
    )?;
    assert_eq!(views[0].active_permits, 0);

    adapter.panic_execute.store(true, Ordering::SeqCst);
    assert!(matches!(
        host.execute_exact(
            &snapshot,
            &request(&snapshot, "invocation-panic")?,
            &CountingReporter::default()
        ),
        Err(ExecutorError::AdapterPanicked { after_entry: true })
    ));
    let views = host.generations(
        &CapabilityAuthorityScope::any(SideEffectClass::Unknown),
        150,
    )?;
    assert_eq!(views[0].active_permits, 0);
    Ok(())
}

#[test]
fn adapter_failures_preserve_pre_entry_and_post_entry_uncertainty() -> TestResult {
    for (identity, kind, expected_after_entry) in [
        ("cap-rejected", AdapterFailureKind::Rejected, false),
        (
            "cap-external-failure",
            AdapterFailureKind::ExternalFailure,
            true,
        ),
    ] {
        let host = host(BTreeMap::new(), 1)?;
        let descriptor = descriptor(identity, 1, "profile-failure", 1)?;
        host.register(
            descriptor.clone(),
            Arc::new(FailingAdapter {
                capability: descriptor.identity().clone(),
                kind,
            }),
            Some(observation(&descriptor, 100, true)?),
        )?;
        let snapshot = host
            .resolve_at(
                &CapabilityRequirement::new(OperationId::new("model.generate")?),
                150,
            )?
            .snapshot()
            .clone();
        let error = match host.execute_exact(
            &snapshot,
            &request(&snapshot, &format!("invocation-{identity}"))?,
            &CountingReporter::default(),
        ) {
            Ok(()) => return Err("planned adapter failure did not propagate".into()),
            Err(error) => error,
        };
        assert_eq!(
            matches!(&error, ExecutorError::BoundaryAfterEntry(_)),
            expected_after_entry
        );
        assert_eq!(
            matches!(&error, ExecutorError::BoundaryBeforeEntry(_)),
            !expected_after_entry
        );
        assert_eq!(
            host.generations(
                &CapabilityAuthorityScope::any(SideEffectClass::Unknown),
                150,
            )?[0]
                .active_permits,
            0
        );
    }
    Ok(())
}

#[test]
fn registry_bounds_shutdown_and_secret_resolution_are_explicit() -> TestResult {
    let host = host(BTreeMap::new(), 1)?;
    let first = descriptor("cap-bounded", 1, "profile-bounded", 1)?;
    let second = descriptor("cap-bounded", 2, "profile-bounded", 1)?;
    host.register(
        first.clone(),
        Arc::new(FakeAdapter::new(first.identity().clone())),
        Some(observation(&first, 100, true)?),
    )?;
    assert!(matches!(
        host.register(
            second,
            Arc::new(FakeAdapter::new(first.identity().clone())),
            None
        ),
        Err(HostError::RegistryBound("retained generations"))
    ));
    assert!(host.shutdown()?.unresolved_invocations.is_empty());
    assert!(matches!(
        host.resolve_at(
            &CapabilityRequirement::new(OperationId::new("model.generate")?),
            150
        ),
        Err(ExecutorError::AdmissionClosed)
    ));

    let resolver = InMemorySecretResolver::new();
    let reference = SecretRef::new("secret:test")?;
    resolver.insert(reference.clone(), b"super-secret".to_vec())?;
    let value = resolver.resolve(&reference)?;
    assert_eq!(value.expose(|bytes| bytes.len()), 12);
    assert!(!format!("{resolver:?} {reference:?} {value:?}").contains("super-secret"));
    Ok(())
}
