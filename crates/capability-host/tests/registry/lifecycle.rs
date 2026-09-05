use super::*;

#[test]
fn registration_and_lifecycle_failures_are_contained_without_partial_visibility() -> TestResult {
    let visible = CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown);

    let failed_descriptor = descriptor("cap-failed-start", 1, "profile-lifecycle", 1)?;
    let failed_host = host(BTreeMap::new(), 1)?;
    let mut failed_adapter =
        LifecycleProbeAdapter::new(failed_descriptor.identity().clone(), LifecyclePanic::None);
    failed_adapter.fail_start = true;
    let failed_adapter = Arc::new(failed_adapter);
    assert!(matches!(
        failed_host.register(failed_descriptor, failed_adapter.clone(), None),
        Err(HostError::Adapter(_))
    ));
    assert!(failed_host.generations(&visible, 100)?.is_empty());
    assert_eq!(failed_adapter.shutdowns.load(Ordering::SeqCst), 1);

    for (identity, panic_at, expected_shutdowns) in [
        ("cap-authority-panic", LifecyclePanic::Authority, 0),
        ("cap-start-panic", LifecyclePanic::Start, 1),
    ] {
        let descriptor = descriptor(identity, 1, "profile-lifecycle", 1)?;
        let host = host(BTreeMap::new(), 1)?;
        let adapter = Arc::new(LifecycleProbeAdapter::new(
            descriptor.identity().clone(),
            panic_at,
        ));
        assert!(matches!(
            host.register(descriptor, adapter.clone(), None),
            Err(HostError::AdapterPanicked)
        ));
        assert!(host.generations(&visible, 100)?.is_empty());
        assert_eq!(adapter.shutdowns.load(Ordering::SeqCst), expected_shutdowns);
    }

    let health_descriptor = descriptor("cap-health-panic", 1, "profile-lifecycle", 1)?;
    let health_host = host(BTreeMap::new(), 1)?;
    health_host.register(
        health_descriptor.clone(),
        Arc::new(LifecycleProbeAdapter::new(
            health_descriptor.identity().clone(),
            LifecyclePanic::Health,
        )),
        None,
    )?;
    assert!(matches!(
        health_host.refresh_health(
            health_descriptor.identity(),
            health_descriptor.descriptor_revision(),
            100,
        ),
        Err(HostError::AdapterPanicked)
    ));
    health_host.force_remove(
        health_descriptor.identity(),
        health_descriptor.descriptor_revision(),
    )?;

    let time_descriptor = descriptor("cap-health-time", 1, "profile-lifecycle", 1)?;
    let time_host = host(BTreeMap::new(), 1)?;
    let mut time_adapter =
        LifecycleProbeAdapter::new(time_descriptor.identity().clone(), LifecyclePanic::None);
    time_adapter.wrong_health_time = true;
    time_host.register(time_descriptor.clone(), Arc::new(time_adapter), None)?;
    assert!(matches!(
        time_host.refresh_health(
            time_descriptor.identity(),
            time_descriptor.descriptor_revision(),
            100,
        ),
        Err(HostError::ObservationTimeMismatch)
    ));
    time_host.force_remove(
        time_descriptor.identity(),
        time_descriptor.descriptor_revision(),
    )?;

    let drain_descriptor = descriptor("cap-drain-panic", 1, "profile-lifecycle", 1)?;
    let drain_host = host(BTreeMap::new(), 1)?;
    drain_host.register(
        drain_descriptor.clone(),
        Arc::new(LifecycleProbeAdapter::new(
            drain_descriptor.identity().clone(),
            LifecyclePanic::Drain,
        )),
        Some(observation(&drain_descriptor, 100, true)?),
    )?;
    assert!(matches!(
        drain_host.begin_drain(
            drain_descriptor.identity(),
            drain_descriptor.descriptor_revision(),
        ),
        Err(HostError::AdapterPanicked)
    ));
    let view = &drain_host.generations(&visible, 100)?[0];
    assert!(view.draining);
    assert!(!view.current);
    drain_host.finish_drain(
        drain_descriptor.identity(),
        drain_descriptor.descriptor_revision(),
    )?;

    let shutdown_descriptor = descriptor("cap-shutdown-panic", 1, "profile-lifecycle", 1)?;
    let shutdown_host = host(BTreeMap::new(), 1)?;
    shutdown_host.register(
        shutdown_descriptor.clone(),
        Arc::new(LifecycleProbeAdapter::new(
            shutdown_descriptor.identity().clone(),
            LifecyclePanic::Shutdown,
        )),
        None,
    )?;
    shutdown_host.begin_drain(
        shutdown_descriptor.identity(),
        shutdown_descriptor.descriptor_revision(),
    )?;
    assert!(matches!(
        shutdown_host.finish_drain(
            shutdown_descriptor.identity(),
            shutdown_descriptor.descriptor_revision(),
        ),
        Err(HostError::AdapterPanicked)
    ));
    assert!(shutdown_host.generations(&visible, 100)?.is_empty());
    Ok(())
}

#[test]
fn shutdown_refuses_an_incomplete_registration_and_the_started_adapter_cleans_up() -> TestResult {
    let host = host(BTreeMap::new(), 1)?;
    let descriptor = descriptor("cap-start-shutdown-race", 1, "profile-lifecycle", 1)?;
    let gate = Arc::new((Mutex::new(AdapterControl::default()), Condvar::new()));
    let mut adapter =
        LifecycleProbeAdapter::new(descriptor.identity().clone(), LifecyclePanic::None);
    adapter.start_gate = Some(gate.clone());
    let adapter = Arc::new(adapter);
    let registering_host = host.clone();
    let registering_descriptor = descriptor.clone();
    let registering_adapter = adapter.clone();
    let registration = std::thread::spawn(move || {
        registering_host.register(registering_descriptor, registering_adapter, None)
    });
    {
        let (lock, changed) = &*gate;
        let mut state = lock.lock().map_err(|_| "start gate poisoned")?;
        while !state.entered {
            state = changed.wait(state).map_err(|_| "start gate poisoned")?;
        }
    }
    assert!(
        host.generations(
            &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
            100,
        )?
        .is_empty()
    );
    assert!(matches!(
        host.shutdown(),
        Err(HostError::RegistrationInProgress(1))
    ));
    {
        let (lock, changed) = &*gate;
        let mut state = lock.lock().map_err(|_| "start gate poisoned")?;
        state.release = true;
        changed.notify_all();
    }
    assert!(matches!(
        registration
            .join()
            .map_err(|_| "registration thread panicked")?,
        Err(HostError::RegistrationClosed)
    ));
    assert_eq!(adapter.shutdowns.load(Ordering::SeqCst), 1);
    assert!(host.shutdown()?.unresolved_invocations.is_empty());
    Ok(())
}

#[test]
fn shutdown_attempts_every_adapter_after_one_panics() -> TestResult {
    let host = host(BTreeMap::new(), 1)?;
    let first = descriptor("cap-a-shutdown-panic", 1, "profile-lifecycle", 1)?;
    let second = descriptor("cap-b-shutdown-clean", 1, "profile-lifecycle", 1)?;
    let first_adapter = Arc::new(LifecycleProbeAdapter::new(
        first.identity().clone(),
        LifecyclePanic::Shutdown,
    ));
    let second_adapter = Arc::new(LifecycleProbeAdapter::new(
        second.identity().clone(),
        LifecyclePanic::None,
    ));
    host.register(first, first_adapter.clone(), None)?;
    host.register(second, second_adapter.clone(), None)?;
    assert!(matches!(host.shutdown(), Err(HostError::AdapterPanicked)));
    assert_eq!(first_adapter.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(second_adapter.shutdowns.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn adapter_calls_run_without_the_registry_lock() -> TestResult {
    let (finished_tx, finished_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = (|| -> TestResult {
            let host = host(BTreeMap::new(), 1)?;
            let descriptor = descriptor("cap-reentrant", 1, "profile-reentrant", 1)?;
            host.register(
                descriptor.clone(),
                Arc::new(ReentrantAdapter {
                    host: host.clone(),
                    capability: descriptor.identity().clone(),
                }),
                None,
            )?;
            host.refresh_health(descriptor.identity(), descriptor.descriptor_revision(), 100)?;
            let snapshot = host
                .resolve_at(
                    &CapabilityRequirement::new(OperationId::new("model.generate")?),
                    100,
                )?
                .snapshot()
                .clone();
            host.execute_exact(
                &snapshot,
                &request(&snapshot, "invocation-reentrant")?,
                &CountingReporter::default(),
            )?;
            host.begin_drain(descriptor.identity(), descriptor.descriptor_revision())?;
            host.finish_drain(descriptor.identity(), descriptor.descriptor_revision())?;
            Ok(())
        })()
        .map_err(|error| error.to_string());
        let _ = finished_tx.send(result);
    });
    let result = finished_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "adapter call deadlocked while re-entering the registry")?;
    worker
        .join()
        .map_err(|_| "registry probe thread panicked")?;
    result.map_err(Into::into)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LifecyclePanic {
    None,
    Authority,
    Start,
    Health,
    Drain,
    Shutdown,
}

struct LifecycleProbeAdapter {
    capability: CapabilityId,
    panic_at: LifecyclePanic,
    fail_start: bool,
    wrong_health_time: bool,
    shutdowns: AtomicUsize,
    start_gate: Option<Arc<(Mutex<AdapterControl>, Condvar)>>,
}

impl LifecycleProbeAdapter {
    fn new(capability: CapabilityId, panic_at: LifecyclePanic) -> Self {
        Self {
            capability,
            panic_at,
            fail_start: false,
            wrong_health_time: false,
            shutdowns: AtomicUsize::new(0),
            start_gate: None,
        }
    }
}

impl CapabilityAdapter for LifecycleProbeAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        assert_ne!(self.panic_at, LifecyclePanic::Authority, "authority panic");
        CapabilityExecutionRequirements::default()
    }

    fn start(&self) -> Result<(), AdapterError> {
        assert_ne!(self.panic_at, LifecyclePanic::Start, "start panic");
        if let Some((lock, changed)) = self.start_gate.as_deref() {
            let mut state = lock
                .lock()
                .map_err(|_| AdapterError::external_failure("start gate poisoned"))?;
            state.entered = true;
            changed.notify_all();
            while !state.release {
                state = changed
                    .wait(state)
                    .map_err(|_| AdapterError::external_failure("start gate poisoned"))?;
            }
        }
        if self.fail_start {
            Err(AdapterError::rejected("planned start failure"))
        } else {
            Ok(())
        }
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            invocation.resolution().operation_contract().side_effect(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?,
        )
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            false,
            false,
            None,
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        assert_ne!(self.panic_at, LifecyclePanic::Health, "health panic");
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms.saturating_add(u64::from(self.wrong_health_time)),
            true,
            0,
            "lifecycle probe healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        assert_ne!(self.panic_at, LifecyclePanic::Drain, "drain panic");
        Ok(())
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.shutdowns.fetch_add(1, Ordering::SeqCst);
        assert_ne!(self.panic_at, LifecyclePanic::Shutdown, "shutdown panic");
        Ok(())
    }
}

struct ReentrantAdapter {
    host: CapabilityHost,
    capability: CapabilityId,
}

impl ReentrantAdapter {
    fn visit_registry(&self) -> Result<(), AdapterError> {
        self.host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                100,
            )
            .map(|_views| ())
            .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}

impl CapabilityAdapter for ReentrantAdapter {
    fn admission_envelope(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.visit_registry()?;
        Ok(InvocationAdmissionEnvelope::not_applicable())
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        assert!(
            self.visit_registry().is_ok(),
            "registry lock was held while deriving authority requirements"
        );
        CapabilityExecutionRequirements::default()
    }

    fn start(&self) -> Result<(), AdapterError> {
        self.visit_registry()
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.visit_registry()?;
        if invocation.resolution().capability() != &self.capability
            || invocation.request().capability() != &self.capability
            || invocation.request().operation() != invocation.resolution().operation()
        {
            return Err(AdapterError::rejected(
                "host changed the exact invocation identity",
            ));
        }
        let cancellation = CancellationRequest::new(
            invocation.request().invocation().clone(),
            11,
            "reentrant host-lock probe",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        let acknowledgement = self
            .host
            .cancel_exact(&cancellation)
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        if acknowledgement.invocation() != cancellation.invocation()
            || acknowledgement.request_sequence() != cancellation.request_sequence()
        {
            return Err(AdapterError::external_failure(
                "reentrant cancellation lost exact correlation",
            ));
        }
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            invocation.resolution().operation_contract().side_effect(),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        reporter.invocation(
            InvocationEvent::new(
                invocation.request().invocation().clone(),
                1,
                InvocationEventKind::Terminal { terminal },
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?,
        )
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.visit_registry()?;
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            true,
            false,
            Some("reentrant cancellation accepted".to_owned()),
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        self.visit_registry()?;
        CapabilityObservation::new(
            self.capability.clone(),
            observed_at_unix_ms,
            true,
            0,
            "reentrant adapter healthy",
        )
        .map_err(|error| AdapterError::external_failure(error.to_string()))
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.visit_registry()
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.visit_registry()
    }
}
