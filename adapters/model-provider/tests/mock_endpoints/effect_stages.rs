//! Frozen request preparation and durable failure windows through the production host/runtime.
use super::runtime_session::{Allow, ModelFixture, NoFaults};
use super::*;
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityRequest, DecisionId, DecisionReasonCode,
};
use milkdrift_capability_host::{CapabilityHost, CapabilitySelectionPolicy, HostConfig};
use milkdrift_persistence::{NodeOutcome, PageSize, RunEventKind};
use milkdrift_redb_store::{FaultInjector, FaultPoint};
use milkdrift_runtime::{
    EffectAction, ExecutionDispatch, ExecutionReporter, ExecutorError, ObservationDisposition,
    TaskExecutor,
};
use std::sync::atomic::AtomicUsize;

fn endpoint(address: &str, streaming: bool) -> TestResult<EndpointProfile> {
    let mut features = BTreeSet::from([ModelFeature::SystemRole]);
    if streaming {
        features.insert(ModelFeature::Streaming);
    }
    profile(
        address,
        "session-profile",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".to_owned(),
        },
        AuthMode::NoAuth,
        features,
    )
}

fn document(streaming: bool) -> TestResult<Vec<u8>> {
    Ok(
        ModelTaskRequestDocument::new(super::uncertainty::ordinary_task(streaming)?)
            .to_canonical_json()?,
    )
}

fn claim(fixture: &ModelFixture) -> TestResult<ExecutionDispatch> {
    fixture.runtime.scheduler_tick()?;
    match fixture
        .runtime
        .claim_execution_effects(PageSize::new(1)?)?
        .pop()
    {
        Some(EffectAction::Execute(dispatch)) => Ok(*dispatch),
        _ => Err("model dispatch was not claimed".into()),
    }
}

fn no_connections(listener: &TcpListener) -> TestResult {
    listener.set_nonblocking(true)?;
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    Ok(())
}

fn assert_released(fixture: &ModelFixture) -> TestResult {
    assert!(
        fixture
            .runtime
            .projection(&fixture.run)?
            .leases()
            .values()
            .all(|lease| !lease.is_active())
    );
    use milkdrift_authority::CapabilityAuthorityScope;
    assert!(
        fixture
            .host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                1000
            )?
            .iter()
            .all(|generation| generation.active_permits == 0)
    );
    Ok(())
}

#[test]
fn local_feature_and_encoded_request_refusals_are_terminal_without_intent_or_send() -> TestResult {
    for oversized in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut profile =
            serde_json::to_value(endpoint(&listener.local_addr()?.to_string(), false)?)?;
        if oversized {
            profile["limits"]["max_request_bytes"] = json!(1);
        }
        let fixture = ModelFixture::new(
            serde_json::from_value(profile)?,
            "fresh",
            document(!oversized)?,
            false,
            Arc::new(Allow),
            Arc::new(NoFaults),
        )?;
        let dispatch = claim(&fixture)?;
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch.clone())))?;
        let history = fixture.runtime.history(&fixture.run)?;
        assert!(history.iter().any(|event| matches!(event.kind(), RunEventKind::NodeTerminal {
            outcome: NodeOutcome::Rejected, detail: Some(detail), ..
        } if detail.as_str().contains(if oversized { "request-body bound" } else { "advertise streaming" }))));
        assert!(history.iter().all(|event| !matches!(
            event.kind(),
            RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
                | RunEventKind::ExternalOutcomeUncertain { .. }
        )));
        assert!(
            fixture
                .runtime
                .execute_effect(EffectAction::Execute(Box::new(dispatch)))
                .is_err()
        );
        assert_released(&fixture)?;
        fixture.runtime.recover()?;
        assert_eq!(history, fixture.runtime.history(&fixture.run)?);
        no_connections(&listener)?;
    }
    Ok(())
}

#[test]
fn malformed_task_and_removed_generation_leave_no_external_obligation() -> TestResult {
    for malformed in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let fixture = ModelFixture::new(
            endpoint(&listener.local_addr()?.to_string(), false)?,
            "fresh",
            if malformed {
                br#"{"schema_version":999}"#.to_vec()
            } else {
                document(false)?
            },
            false,
            Arc::new(Allow),
            Arc::new(NoFaults),
        )?;
        if malformed {
            fixture.runtime.scheduler_tick()?;
            assert!(
                fixture
                    .runtime
                    .claim_execution_effects(PageSize::new(1)?)?
                    .is_empty()
            );
        } else {
            let dispatch = claim(&fixture)?;
            fixture.host.force_remove(
                dispatch.resolution().capability(),
                dispatch.resolution().descriptor_revision(),
            )?;
            fixture
                .runtime
                .execute_effect(EffectAction::Execute(Box::new(dispatch)))?;
        }
        assert_released(&fixture)?;
        no_connections(&listener)?;
        reopen_and_assert(fixture, false)?;
    }
    Ok(())
}

#[derive(Default)]
struct RevokeOnFinalCheck {
    decision: Mutex<Option<DecisionId>>,
    checks: AtomicUsize,
}

impl AuthorityEvaluator for RevokeOnFinalCheck {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let allowed = Allow.evaluate(request)?;
        if self
            .decision
            .lock()
            .ok()
            .as_deref()
            .and_then(|value| value.as_ref())
            == Some(&request.decision)
            && self.checks.fetch_add(1, Ordering::SeqCst) > 0
        {
            return AuthorityDecisionSnapshot::from_evaluation(
                allowed.policy().clone(),
                allowed.policy_version(),
                request.clone(),
                vec![DecisionReasonCode::Revoked],
                AuthorityBudget::default(),
                SideEffectClass::Unknown,
            );
        }
        Ok(allowed)
    }
}

#[test]
fn authority_is_checked_again_after_preparation_and_releases_the_exact_permit() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let authority = Arc::new(RevokeOnFinalCheck::default());
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string(), false)?,
        "fresh",
        document(false)?,
        false,
        authority.clone(),
        Arc::new(NoFaults),
    )?;
    let dispatch = claim(&fixture)?;
    *authority.decision.lock().map_err(|_| "decision lock")? = Some(DecisionId::new(format!(
        "decision:{}",
        blake3::hash(
            format!("{}:adapter-entry", dispatch.entry_authorization().digest()).as_bytes()
        )
    ))?);
    fixture
        .runtime
        .execute_effect(EffectAction::Execute(Box::new(dispatch)))?;
    assert_eq!(authority.checks.load(Ordering::SeqCst), 2);
    let history = fixture.runtime.history(&fixture.run)?;
    assert!(history.iter().any(|event| matches!(event.kind(), RunEventKind::CapabilityAdapterEntryDecisionRecorded {
        authorization, controller_admission: milkdrift_persistence::ControllerAdmissionOutcome::NotControlled, ..
    } if !authorization.is_allowed())));
    assert_released(&fixture)?;
    no_connections(&listener)
}

#[derive(Default)]
struct Observations(Mutex<Vec<InvocationEvent>>);
impl ExecutionReporter for Observations {
    fn invocation(&self, event: InvocationEvent) -> Result<ObservationDisposition, ExecutorError> {
        self.0
            .lock()
            .map_err(|_| ExecutorError::BoundaryAfterEntry("report lock".to_owned()))?
            .push(event);
        Ok(ObservationDisposition::Applied)
    }
    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError> {
        Ok(ObservationDisposition::Applied)
    }
}

#[test]
fn prepared_request_is_consumed_once_and_changed_dispatch_or_generation_cannot_enter() -> TestResult
{
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let fixture = ModelFixture::new(
        endpoint(&address, false)?,
        "fresh",
        document(false)?,
        false,
        Arc::new(Allow),
        Arc::new(NoFaults),
    )?;
    let dispatch = claim(&fixture)?;
    let prepared = fixture.host.prepare_exact_entry(&dispatch)?;
    assert!(matches!(
        fixture.host.prepare_exact_entry(&dispatch),
        Err(ExecutorError::Overloaded(_))
    ));
    let other = ModelFixture::new(
        endpoint(&address, true)?,
        "fresh",
        document(true)?,
        false,
        Arc::new(Allow),
        Arc::new(NoFaults),
    )?;
    let changed = claim(&other)?;
    assert!(matches!(
        prepared.enter(&changed, &Observations::default()),
        Err(ExecutorError::InvalidDispatch(_))
    ));
    let absent = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?;
    assert!(matches!(
        absent.prepare_exact_entry(&dispatch),
        Err(ExecutorError::UnavailableGeneration { .. })
    ));
    no_connections(&listener)?;
    let prepared = fixture.host.prepare_exact_entry(&dispatch)?;
    listener.set_nonblocking(false)?;
    fixture.read_denial.store(true, Ordering::SeqCst);
    let server = thread::spawn(move || -> std::io::Result<String> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let request = read_request(&mut stream)?;
        let body = complete_response();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )?;
        Ok(request)
    });
    let observations = Observations::default();
    prepared.enter(&dispatch, &observations)?;
    let captured = server.join().map_err(|_| "server panic")??;
    let wire: Value =
        serde_json::from_str(captured.split_once("\r\n\r\n").ok_or("request body")?.1)?;
    assert_eq!(wire["model"], "mock-model");
    assert!(captured.contains("bounded hostile endpoint"));
    assert!(
        observations
            .0
            .lock()
            .map_err(|_| "reports")?
            .iter()
            .any(|event| event
                .kind()
                .terminal()
                .is_some_and(|terminal| terminal.status() == TerminalStatus::Success))
    );
    Ok(())
}

fn complete_response() -> String {
    json!({"id":"response-stage", "model":"mock-model", "choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":1,"completion_tokens":1}}).to_string()
}

struct CommitFailure {
    point: FaultPoint,
    remaining: AtomicUsize,
}
impl CommitFailure {
    fn new(point: FaultPoint) -> Self {
        Self {
            point,
            remaining: AtomicUsize::new(0),
        }
    }
    fn arm(&self, count: usize) {
        self.remaining.store(count, Ordering::SeqCst);
    }
}
impl FaultInjector for CommitFailure {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == self.point
            && self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_sub(1)
                })
                == Ok(1)
        {
            return Err(milkdrift_redb_store::injected_failure(point));
        }
        Ok(())
    }
}

#[test]
fn crash_after_entry_intent_without_send_does_not_become_negative_proof_on_reopen() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fault = Arc::new(CommitFailure::new(FaultPoint::AfterCommandCommit));
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string(), false)?,
        "fresh",
        document(false)?,
        false,
        Arc::new(Allow),
        fault.clone(),
    )?;
    let dispatch = claim(&fixture)?;
    fault.arm(1);
    assert!(
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch.clone())))
            .is_err()
    );
    assert!(fixture.runtime.history(&fixture.run)?.iter().any(|event| matches!(event.kind(),
        RunEventKind::CapabilityAdapterEntryDecisionRecorded { authorization, .. } if authorization.is_allowed())));
    // A duplicate live call cannot erase the durable intent with a later local refusal.
    assert!(
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch)))
            .is_err()
    );
    reopen_and_assert(fixture, true)?;
    no_connections(&listener)
}

fn reopen_and_assert(fixture: ModelFixture, uncertain: bool) -> TestResult {
    let ModelFixture {
        runtime,
        host,
        store,
        run,
        directory,
        adapter,
        ..
    } = fixture;
    drop(adapter);
    drop(runtime);
    drop(host);
    drop(store);
    let store = Arc::new(RedbStore::open(directory.path().join("store"))?);
    let host = Arc::new(CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 10_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )?);
    let runtime = milkdrift_runtime::RuntimeService::new_with_authority(
        store,
        host,
        Arc::new(Allow),
        Arc::new(milkdrift_runtime::ManualClock::new(100_000)),
        Arc::new(milkdrift_runtime::SequentialIdGenerator::new(
            "stage-reopen",
            1,
        )?),
        milkdrift_runtime::RuntimeConfig::new(
            milkdrift_persistence::WorkerId::new("session-worker")?,
            milkdrift_authority::ActorRef::new("controller:session")?,
            30_000,
            16,
            milkdrift_runtime::SchedulerLimits::new(8, 4, 2, 4)?,
            milkdrift_runtime::RetryPolicy::new(1, Vec::new(), 1, 1000, 0)?,
        )?,
    )?;
    runtime.recover()?;
    let history = runtime.history(&run)?;
    assert_eq!(
        history
            .iter()
            .any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { .. })),
        uncertain,
        "{history:#?}"
    );
    assert!(
        runtime
            .claim_execution_effects(PageSize::new(8)?)?
            .is_empty()
    );
    Ok(())
}

#[test]
fn local_publication_and_terminal_commit_faults_preserve_observed_response_and_no_second_send()
-> TestResult {
    for (point, count, uncertain) in [
        (FaultPoint::BeforeArtifactMetadataCommit, 1, false),
        (FaultPoint::BeforeCommandCommit, 2, true),
        (FaultPoint::BeforeCommandCommit, 5, true),
        (FaultPoint::AfterCommandCommit, 5, false),
    ] {
        let (address, server) = serve(complete_response(), "application/json")?;
        let fault = Arc::new(CommitFailure::new(point));
        let fixture = ModelFixture::new(
            endpoint(&address, false)?,
            "fresh",
            document(false)?,
            false,
            Arc::new(Allow),
            fault.clone(),
        )?;
        let dispatch = claim(&fixture)?;
        fault.arm(count);
        let _outcome = fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch)));
        server.join().map_err(|_| "server panic")??;
        let history = fixture.runtime.history(&fixture.run)?;
        assert_eq!(
            fault.remaining.load(Ordering::SeqCst),
            0,
            "fault window was not reached: {point:?}/{count}"
        );
        if uncertain {
            assert!(history.iter().any(|event| matches!(event.kind(), RunEventKind::ExternalOutcomeUncertain { reason, .. }
                if reason.as_str().contains("complete provider response observed"))), "{history:#?}");
        } else if point == FaultPoint::BeforeArtifactMetadataCommit {
            assert!(
                history.iter().any(|event| matches!(
                    event.kind(),
                    RunEventKind::NodeTerminal {
                        outcome: NodeOutcome::Failed,
                        ..
                    }
                )),
                "{history:#?}"
            );
        } else {
            assert!(
                history.iter().any(|event| matches!(
                    event.kind(),
                    RunEventKind::NodeTerminal {
                        outcome: NodeOutcome::Succeeded,
                        ..
                    }
                )),
                "{history:#?}"
            );
        }
        assert_released(&fixture)?;
        reopen_and_assert(fixture, uncertain)?;
    }
    Ok(())
}

struct StopBeforeSend {
    armed: AtomicBool,
    adapter: Mutex<std::sync::Weak<ModelEndpointAdapter>>,
}
impl FaultInjector for StopBeforeSend {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == FaultPoint::AfterCommandCommit && self.armed.swap(false, Ordering::SeqCst) {
            let adapter = self
                .adapter
                .lock()
                .map_err(|_| milkdrift_redb_store::injected_failure(point))?;
            adapter
                .upgrade()
                .ok_or_else(|| milkdrift_redb_store::injected_failure(point))?
                .shutdown()
                .map_err(|_| milkdrift_redb_store::injected_failure(point))?;
        }
        Ok(())
    }
}

#[test]
fn stopped_adapter_after_intent_is_uncertain_even_when_the_server_observes_zero_requests()
-> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fault = Arc::new(StopBeforeSend {
        armed: AtomicBool::new(false),
        adapter: Mutex::new(std::sync::Weak::new()),
    });
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string(), false)?,
        "fresh",
        document(false)?,
        false,
        Arc::new(Allow),
        fault.clone(),
    )?;
    let dispatch = claim(&fixture)?;
    *fault.adapter.lock().map_err(|_| "adapter lock")? = Arc::downgrade(&fixture.adapter);
    fault.armed.store(true, Ordering::SeqCst);
    assert!(matches!(
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch)))?,
        milkdrift_runtime::EffectExecutionResult::Uncertain { .. }
    ));
    assert_released(&fixture)?;
    no_connections(&listener)?;
    reopen_and_assert(fixture, true)
}

#[test]
fn failed_refusal_commit_cannot_be_invented_as_negative_proof_after_restart() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fault = Arc::new(CommitFailure::new(FaultPoint::BeforeCommandCommit));
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string(), false)?,
        "fresh",
        document(true)?,
        false,
        Arc::new(Allow),
        fault.clone(),
    )?;
    let dispatch = claim(&fixture)?;
    fault.arm(1);
    assert!(
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch)))
            .is_err()
    );
    assert!(
        fixture
            .runtime
            .history(&fixture.run)?
            .iter()
            .all(|event| !matches!(
                event.kind(),
                RunEventKind::NodeTerminal { .. }
                    | RunEventKind::CapabilityAdapterEntryDecisionRecorded { .. }
            ))
    );
    no_connections(&listener)?;
    reopen_and_assert(fixture, true)
}

#[test]
fn interrupted_request_reception_acceptance_and_stream_never_repeat_after_reopen() -> TestResult {
    for stage in ["receiving", "accepted", "stream"] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        listener.set_nonblocking(true)?;
        let (stop, stopped) = mpsc::channel();
        let count = Arc::new(AtomicUsize::new(0));
        let entries = count.clone();
        let server = thread::spawn(move || -> std::io::Result<()> {
            for _ in 0..1000 {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        entries.fetch_add(1, Ordering::SeqCst);
                        stream.set_nonblocking(false)?;
                        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                        if stage == "receiving" {
                            let mut prefix = [0; 1];
                            stream.read_exact(&mut prefix)?;
                            stream.shutdown(std::net::Shutdown::Both)?;
                        } else {
                            read_request(&mut stream)?;
                            if stage == "stream" {
                                let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"},\"finish_reason\":null}]}\n\n";
                                write!(
                                    stream,
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                                    body.len(),
                                    body
                                )?;
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error),
                }
                if stopped.recv_timeout(Duration::from_millis(5)).is_ok() {
                    return Ok(());
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "endpoint observer expired",
            ))
        });
        let fixture = ModelFixture::new(
            endpoint(&address, true)?,
            "fresh",
            document(stage == "stream")?,
            false,
            Arc::new(Allow),
            Arc::new(NoFaults),
        )?;
        let dispatch = claim(&fixture)?;
        fixture
            .runtime
            .execute_effect(EffectAction::Execute(Box::new(dispatch.clone())))?;
        assert!(
            fixture
                .runtime
                .execute_effect(EffectAction::Execute(Box::new(dispatch)))
                .is_err()
        );
        assert_released(&fixture)?;
        reopen_and_assert(fixture, true)?;
        let _ = stop.send(());
        server.join().map_err(|_| "counting server panic")??;
        assert_eq!(count.load(Ordering::SeqCst), 1, "{stage}");
    }
    Ok(())
}
