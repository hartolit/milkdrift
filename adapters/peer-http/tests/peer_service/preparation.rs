//! Preparation must precede serving entry, and cannot preserve stale entry permission.
use super::faults::ControlledPeerClock;
use super::support::*;
use milkdrift_capability_host::PreparedAdapterExecution;
use milkdrift_persistence::NativePathClaimStore;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

#[derive(Clone, Copy)]
enum Interruption {
    None,
    Refuse,
    Panic,
    Cancel,
    Revoke,
    LeaseExpired,
    DeadlineExpired,
    Drain,
    UnknownAllowance,
    ExhaustedAllowance,
    UnknownDuration,
    ExhaustedDuration,
}

struct PreparingAdapter {
    native_path: Option<std::path::PathBuf>,
    inner: TerminalAdapter,
    preparations: AtomicUsize,
    prepared: SyncSender<()>,
    release: Mutex<Receiver<()>>,
    interruption: Interruption,
    identities: Mutex<Vec<InvocationId>>,
}

impl CapabilityAdapter for PreparingAdapter {
    fn accepts_direct_inputs(&self) -> bool {
        self.inner.accepts_direct_inputs()
    }

    fn native_path_claims(
        &self,
        _invocation: &AdapterInvocation<'_>,
    ) -> Result<Vec<milkdrift_capability_host::NativePathScope>, AdapterError> {
        Ok(self
            .native_path
            .as_ref()
            .map(|path| milkdrift_capability_host::NativePathScope {
                path: path.clone(),
                write: true,
            })
            .into_iter()
            .collect())
    }

    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedAdapterExecution, AdapterError> {
        self.preparations.fetch_add(1, Ordering::SeqCst);
        self.identities
            .lock()
            .map_err(|_| AdapterError::rejected("test identities poisoned"))?
            .push(invocation.request().invocation().clone());
        self.prepared
            .send(())
            .map_err(|_| AdapterError::rejected("test observer closed"))?;
        self.release
            .lock()
            .map_err(|_| AdapterError::rejected("test release poisoned"))?
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| AdapterError::rejected("test preparation release timed out"))?;
        match self.interruption {
            Interruption::Refuse => return Err(AdapterError::rejected("fixture input refused")),
            Interruption::Panic => std::panic::resume_unwind(Box::new("fixture preparation panic")),
            _ => {}
        }
        let envelope = match self.interruption {
            Interruption::UnknownAllowance => InvocationAdmissionEnvelope::unknown(),
            Interruption::ExhaustedAllowance => InvocationAdmissionEnvelope::new(
                milkdrift_capability::AdmissionUnit::Unknown,
                milkdrift_capability::AdmissionBound::NotApplicable,
                milkdrift_capability::AdmissionBound::NotApplicable,
                milkdrift_capability::AdmissionBound::Bounded(u64::MAX),
                milkdrift_capability::AdmissionBound::NotApplicable,
            )
            .with_execution_duration_ms(milkdrift_capability::AdmissionBound::NotApplicable),
            Interruption::UnknownDuration => InvocationAdmissionEnvelope::not_applicable()
                .with_execution_duration_ms(milkdrift_capability::AdmissionBound::Unknown),
            Interruption::ExhaustedDuration => InvocationAdmissionEnvelope::not_applicable()
                .with_execution_duration_ms(milkdrift_capability::AdmissionBound::Bounded(30_001)),
            _ => InvocationAdmissionEnvelope::not_applicable(),
        };
        Ok(PreparedAdapterExecution::new(
            envelope,
            move |invocation, reporter| self.inner.execute(invocation, reporter),
        ))
    }

    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.inner.admission_envelope(invocation)
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.inner.authority_requirements()
    }
    fn start(&self) -> Result<(), AdapterError> {
        self.inner.start()
    }
    fn execute(
        &self,
        _: &AdapterInvocation<'_>,
        _: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        Err(AdapterError::external_failure(
            "unprepared execution bypass was called",
        ))
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.inner.cancel(request)
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        self.inner.health(now)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.inner.begin_drain()
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        self.inner.shutdown()
    }
}

fn exercise(interruption: Interruption) -> TestResult {
    exercise_for(interruption, false)
}

fn exercise_for(interruption: Interruption, client: bool) -> TestResult {
    let directory = tempfile::tempdir()?;
    let native = directory.path().join("configured-working-directory");
    std::fs::create_dir(&native)?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ControlledPeerClock::new(now()));
    let calls = Arc::new(AtomicUsize::new(0));
    let (prepared, prepared_receiver) = sync_channel(1);
    let (release, release_receiver) = sync_channel(1);
    let adapter = Arc::new(PreparingAdapter {
        native_path: cfg!(unix).then_some(native.clone()),
        inner: TerminalAdapter {
            capability: CapabilityId::new("test-capability")?,
            delay: Duration::ZERO,
            active: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
            calls: calls.clone(),
            requirements: CapabilityExecutionRequirements::default(),
        },
        preparations: AtomicUsize::new(0),
        prepared,
        release: Mutex::new(release_receiver),
        interruption,
        identities: Mutex::new(Vec::new()),
    });
    let (host, descriptor) = host_with_adapter(adapter.clone())?;
    host.install_native_path_store(store.clone(), Vec::new())?;
    let peer = PeerId::new("preparation-client")?;
    let target = PeerId::new("preparation-host")?;
    let actor = ActorRef::new("preparation-client")?;
    let config = server_config(peer.clone(), target.clone(), 1, 2)?;
    let service = if client {
        PeerService::with_clients(
            config,
            host.clone(),
            store.clone(),
            milkdrift_capability_host::conformance::disabled_artifact_store(),
            None,
            Some(super::direct::client_policy(std::slice::from_ref(&actor))?),
            clock.clone(),
        )?
    } else {
        PeerService::new(config, host.clone(), store.clone(), clock.clone())?
    };
    service.recover(1_024)?;
    let catalog = if client {
        service.client_catalog(&actor)?
    } else {
        service.catalog(&peer)?
    };
    let request = request(
        &peer,
        &target,
        &descriptor,
        catalog.generation,
        catalog.digest,
        "preparation-request",
        "preparation-invocation",
    )?;
    let acceptance = if client {
        service.invoke_client(
            &actor,
            &milkdrift_peer_protocol::DirectInvocationRequest {
                host: target.clone(),
                request_id: request.request_id.clone(),
                catalog_generation: request.catalog_generation,
                catalog_digest: request.catalog_digest.clone(),
                selection: request.selection.clone(),
                request: request.request.clone(),
                limits: request.limits.clone(),
                deadline_unix_ms: request.deadline_unix_ms,
            },
        )?
    } else {
        service.invoke(&peer, request.clone())?
    };
    let InvocationAcceptance::Accepted { execution, .. } = acceptance else {
        return Err("preparation fixture was not accepted".into());
    };
    prepared_receiver.recv_timeout(Duration::from_secs(5))?;
    let caller = if client {
        milkdrift_peer_protocol::ServingCaller {
            host: target,
            principal: milkdrift_peer_protocol::ServingPrincipal::Client { actor },
        }
    } else {
        milkdrift_peer_protocol::ServingCaller::peer(&target, &peer)
    };
    let Some(PeerExecutionSnapshot::Hot(claimed)) = store.peer_execution(&caller, &execution)?
    else {
        return Err("preparing claim disappeared".into());
    };
    assert!(matches!(
        claimed.phase,
        PeerExecutionPhase::DispatchClaimed { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    #[cfg(unix)]
    {
        let claim = store
            .native_path_claim(claimed.managed_invocation()?.as_str())?
            .ok_or("configured writer claim absent")?;
        assert_eq!(claim.roots.len(), 1);
        let competing = milkdrift_persistence::NativePathClaim {
            owner: "project-reader".into(),
            roots: vec![milkdrift_capability_host::capture_native_project_root(
                &native, false,
            )?],
        };
        assert!(store.claim_native_paths(&competing).is_err());
    }
    match interruption {
        Interruption::Cancel => {
            service.cancel(
                &peer,
                &PeerCancellationRequest {
                    request_id: PeerRequestId::new("cancel-preparation")?,
                    execution: execution.clone(),
                    sequence: 1,
                    reason: "cancel while preparing".to_owned(),
                },
            )?;
        }
        Interruption::Revoke => service.revoke_peer(&peer)?,
        Interruption::LeaseExpired => clock.set(
            claimed
                .phase
                .claim()
                .ok_or("claim absent")?
                .lease_expires_at_unix_ms
                + 1,
        )?,
        Interruption::DeadlineExpired => clock.set(request.deadline_unix_ms + 1)?,
        Interruption::Drain => service.begin_drain()?,
        _ => {}
    }
    release.send(())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let terminal = loop {
        let Some(PeerExecutionSnapshot::Hot(record)) = store.peer_execution(&caller, &execution)?
        else {
            return Err("preparation result disappeared".into());
        };
        if matches!(record.phase, PeerExecutionPhase::Terminal { .. }) {
            break record;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("preparation did not settle: {:?}", record.phase).into());
        }
        thread::sleep(Duration::from_millis(5));
    };
    assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
    assert_eq!(adapter.preparations.load(Ordering::SeqCst), 1);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        usize::from(matches!(interruption, Interruption::None))
    );
    let observations = store.peer_observations(&caller, &execution, 0, PageSize::new(8)?)?;
    let final_status = observations
        .observations
        .last()
        .and_then(|item| item.event.kind().terminal())
        .ok_or("terminal absent")?
        .status();
    assert_eq!(
        final_status,
        match interruption {
            Interruption::None => TerminalStatus::Success,
            Interruption::Cancel => TerminalStatus::Cancelled,
            _ => TerminalStatus::Failure,
        }
    );
    assert_eq!(terminal.last_observation_sequence, 1);
    assert!(
        store
            .native_path_claim(claimed.managed_invocation()?.as_str())?
            .is_none()
    );
    // A completed or refused preparation releases the exact generation permit.
    assert!(host.shutdown().is_ok());
    Ok(())
}

#[test]
fn different_callers_cannot_collide_in_the_adapter_invocation_namespace() -> TestResult {
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let calls = Arc::new(AtomicUsize::new(0));
    let (prepared, prepared_receiver) = sync_channel(2);
    let (release, release_receiver) = sync_channel(2);
    let adapter = Arc::new(PreparingAdapter {
        native_path: None,
        inner: TerminalAdapter {
            capability: CapabilityId::new("test-capability")?,
            delay: Duration::ZERO,
            active: Arc::new(AtomicUsize::new(0)),
            maximum: Arc::new(AtomicUsize::new(0)),
            calls: calls.clone(),
            requirements: CapabilityExecutionRequirements::default(),
        },
        preparations: AtomicUsize::new(0),
        prepared,
        release: Mutex::new(release_receiver),
        interruption: Interruption::None,
        identities: Mutex::new(Vec::new()),
    });
    let (host, descriptor) = host_with_adapter(adapter.clone())?;
    let peers = [PeerId::new("caller-one")?, PeerId::new("caller-two")?];
    let target = PeerId::new("shared-host")?;
    let mut config = server_config(peers[0].clone(), target.clone(), 2, 2)?;
    config
        .relationships
        .push(relationship(peers[1].clone(), 2)?);
    let service = PeerService::new(config, host.clone(), store.clone(), system_peer_clock())?;
    service.recover(32)?;
    let mut executions = Vec::new();
    for peer in &peers {
        let catalog = service.catalog(peer)?;
        let request = request(
            peer,
            &target,
            &descriptor,
            catalog.generation,
            catalog.digest,
            "same-request",
            "same-invocation",
        )?;
        let InvocationAcceptance::Accepted { execution, .. } = service.invoke(peer, request)?
        else {
            return Err("colliding caller was not accepted independently".into());
        };
        executions.push(execution);
        // Both hold preparation permits at once, before either has entered.
        prepared_receiver.recv_timeout(Duration::from_secs(5))?;
    }
    {
        let identities = adapter
            .identities
            .lock()
            .map_err(|_| "test identities poisoned")?;
        let [first, second] = identities.as_slice() else {
            return Err("expected two prepared identities".into());
        };
        assert_ne!(first, second);
        assert!(identities.iter().all(|id| id.as_str() != "same-invocation"));
    }
    for _ in &peers {
        release.send(())?;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    for (peer, execution) in peers.iter().zip(&executions) {
        loop {
            let observations = store.peer_observations(
                &milkdrift_peer_protocol::ServingCaller::peer(&target, peer),
                execution,
                0,
                PageSize::new(8)?,
            )?;
            if let Some(terminal) = observations.observations.last() {
                assert_eq!(terminal.event.invocation().as_str(), "same-invocation");
                assert_eq!(
                    terminal
                        .event
                        .kind()
                        .terminal()
                        .ok_or("terminal absent")?
                        .status(),
                    TerminalStatus::Success
                );
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("colliding calls did not settle".into());
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(service.shutdown_workers(Duration::from_secs(5)).clean);
    assert!(host.shutdown().is_ok());
    Ok(())
}

#[test]
fn serving_enters_the_prepared_handle_once() -> TestResult {
    exercise(Interruption::None)
}
#[test]
fn direct_serving_claims_configured_writer_and_releases_after_terminal() -> TestResult {
    exercise_for(Interruption::None, true)
}
#[test]
fn unknown_prepared_duration_has_no_external_entry() -> TestResult {
    exercise(Interruption::UnknownDuration)
}
#[test]
fn excessive_prepared_duration_has_no_external_entry() -> TestResult {
    exercise(Interruption::ExhaustedDuration)
}
#[test]
fn preparation_refusal_has_no_external_entry() -> TestResult {
    exercise(Interruption::Refuse)
}
#[test]
fn preparation_panic_has_no_external_entry() -> TestResult {
    exercise(Interruption::Panic)
}
#[test]
fn cancellation_during_preparation_prevents_entry() -> TestResult {
    exercise(Interruption::Cancel)
}
#[test]
fn revocation_during_preparation_prevents_entry() -> TestResult {
    exercise(Interruption::Revoke)
}
#[test]
fn expired_claim_after_preparation_prevents_entry() -> TestResult {
    exercise(Interruption::LeaseExpired)
}
#[test]
fn expired_deadline_after_preparation_prevents_entry() -> TestResult {
    exercise(Interruption::DeadlineExpired)
}
#[test]
fn drain_during_preparation_prevents_entry() -> TestResult {
    exercise(Interruption::Drain)
}

#[test]
fn unknown_prepared_allowance_prevents_external_entry() -> TestResult {
    exercise(Interruption::UnknownAllowance)
}

#[test]
fn exhausted_prepared_allowance_prevents_external_entry() -> TestResult {
    exercise(Interruption::ExhaustedAllowance)
}
