//! Preparation must precede serving entry, and cannot preserve stale entry permission.
use super::faults::ControlledPeerClock;
use super::support::*;
use milkdrift_capability_host::PreparedAdapterExecution;
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
}

struct PreparingAdapter {
    inner: TerminalAdapter,
    preparations: AtomicUsize,
    prepared: SyncSender<()>,
    release: Mutex<Receiver<()>>,
    interruption: Interruption,
    identities: Mutex<Vec<InvocationId>>,
}

impl CapabilityAdapter for PreparingAdapter {
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
            ),
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
    let directory = tempfile::tempdir()?;
    let store = Arc::new(RedbStore::open(directory.path())?);
    let clock = Arc::new(ControlledPeerClock::new(now()));
    let calls = Arc::new(AtomicUsize::new(0));
    let (prepared, prepared_receiver) = sync_channel(1);
    let (release, release_receiver) = sync_channel(1);
    let adapter = Arc::new(PreparingAdapter {
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
    let peer = PeerId::new("preparation-client")?;
    let target = PeerId::new("preparation-host")?;
    let service = PeerService::new(
        server_config(peer.clone(), target.clone(), 1, 2)?,
        host.clone(),
        store.clone(),
        clock.clone(),
    )?;
    service.recover(1_024)?;
    let catalog = service.catalog(&peer)?;
    let request = request(
        &peer,
        &target,
        &descriptor,
        catalog.generation,
        catalog.digest,
        "preparation-request",
        "preparation-invocation",
    )?;
    let InvocationAcceptance::Accepted { execution, .. } =
        service.invoke(&peer, request.clone())?
    else {
        return Err("preparation fixture was not accepted".into());
    };
    prepared_receiver.recv_timeout(Duration::from_secs(5))?;
    let Some(PeerExecutionSnapshot::Hot(claimed)) = store.peer_execution(
        &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        &execution,
    )?
    else {
        return Err("preparing claim disappeared".into());
    };
    assert!(matches!(
        claimed.phase,
        PeerExecutionPhase::DispatchClaimed { .. }
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
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
        let Some(PeerExecutionSnapshot::Hot(record)) = store.peer_execution(
            &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
            &execution,
        )?
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
    let observations = store.peer_observations(
        &milkdrift_peer_protocol::ServingCaller::peer(&target, &peer),
        &execution,
        0,
        PageSize::new(8)?,
    )?;
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
        assert_eq!(identities.len(), 2);
        assert_ne!(identities[0], identities[1]);
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
