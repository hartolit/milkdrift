//! Preparation, durable entry, and failure windows through the ordinary task runtime.
use super::runtime_session::{Allow, ModelFixture, NoFaults};
use super::*;
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityRequest, DecisionReasonCode,
};
use milkdrift_capability_host::{CapabilityHost, CapabilitySelectionPolicy, HostConfig};
use milkdrift_redb_store::{FaultInjector, FaultPoint};
use milkdrift_runtime::{
    ExecutionDispatch, ExecutionReporter, ExecutorError, ObservationDisposition, TaskExecutor,
    TaskState,
};
use std::sync::atomic::AtomicUsize;

pub(super) fn endpoint(address: &str) -> TestResult<EndpointProfile> {
    let mut value = serde_json::to_value(profile(
        address,
        "session-profile",
        ProviderProtocol::OpenAiCompatible {
            path: "v1/chat/completions".into(),
        },
        AuthMode::NoAuth,
        BTreeSet::from([ModelFeature::SystemRole]),
    )?)?;
    let object = value.as_object_mut().ok_or("profile object absent")?;
    object.insert(
        "billing".into(),
        json!({"type":"unbilled","source":"operator controlled fault fixture v1"}),
    );
    object.insert("token_limits".into(),json!({"type":"byte_bpe","template_tokens_per_message":64,"template_tokens_per_request":64,"maximum_input_tokens":32768,"maximum_output_tokens":65536,"output_control":"max_tokens","source":"fault fixture v1; single-choice total generation bound"}));
    Ok(serde_json::from_value(value)?)
}
fn claim(fixture: &ModelFixture) -> TestResult<ExecutionDispatch> {
    fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or_else(|| "model dispatch was not claimed".into())
}
fn no_connections(listener: &TcpListener) -> TestResult {
    listener.set_nonblocking(true)?;
    assert!(matches!(listener.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock));
    Ok(())
}
fn entered(fixture: &ModelFixture) -> TestResult<bool> {
    Ok(fixture
        .state()?
        .frozen
        .ok_or("frozen start absent")?
        .entered)
}
fn assert_released(fixture: &ModelFixture) -> TestResult {
    assert!(
        fixture
            .host
            .generations(
                &milkdrift_authority::CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                1000
            )?
            .iter()
            .all(|generation| generation.active_permits == 0)
    );
    Ok(())
}
fn reopen_and_assert(fixture: ModelFixture, expected: TaskState) -> TestResult {
    let fixture = fixture.reopen(endpoint("127.0.0.1:9")?, Arc::new(Allow), 100_000)?;
    assert_eq!(fixture.state()?.state, expected);
    assert!(fixture.runtime.tick(8)?.is_empty());
    Ok(())
}
pub(super) fn complete_response() -> String {
    json!({"id":"response-stage","model":"mock-model","choices":[{"message":{"content":"complete"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"prompt_tokens_details":{"cached_tokens":0}}}).to_string()
}
struct CostCeiling(u64);
impl AuthorityEvaluator for CostCeiling {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let allowed = Allow.evaluate(request)?;
        if request.budget.cost_minor.unwrap_or(0) > self.0 {
            return AuthorityDecisionSnapshot::from_evaluation(
                allowed.policy().clone(),
                allowed.policy_version(),
                request.clone(),
                vec![DecisionReasonCode::BudgetExcess],
                AuthorityBudget::default(),
                SideEffectClass::Unknown,
            );
        }
        Ok(allowed)
    }
}
#[test]
fn billed_profile_declares_rounded_permission_cost_and_refuses_insufficient_authority() -> TestResult
{
    for ceiling in [7, 8] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut value = serde_json::to_value(endpoint(&listener.local_addr()?.to_string())?)?;
        *value
            .pointer_mut("/token_limits/maximum_output_tokens")
            .ok_or("output bound")? = json!(4096);
        value.as_object_mut().ok_or("profile object")?.insert("billing".into(),json!({"type":"text_tariff","currency":"EUR","input_micros_per_million":1_000_000,"cached_input_micros_per_million":2_000_000,"output_micros_per_million":3_000_000,"source":"fixture v1, all text charges"}));
        let fixture = ModelFixture::new(
            serde_json::from_value(value)?,
            Arc::new(CostCeiling(ceiling)),
            Arc::new(NoFaults),
        );
        if ceiling == 7 {
            assert!(
                fixture.is_err(),
                "insufficient authority must refuse future task admission"
            );
            no_connections(&listener)?;
            continue;
        }
        let fixture = fixture?;
        // The larger input tariff plus 4,096 output tokens reserves eight hundredths.
        assert_eq!(
            fixture.adapter.authority_requirements().budget.cost_minor,
            Some(8)
        );
        if ceiling == 7 {
            assert!(fixture.runtime.tick(1)?.is_empty());
            assert!(fixture.state()?.frozen.is_none());
            no_connections(&listener)?;
        } else {
            let dispatch = claim(&fixture)?;
            assert_eq!(
                dispatch
                    .resolution_authorization()
                    .request()
                    .budget
                    .cost_minor,
                Some(8)
            );
            let server = thread::spawn(move || -> std::io::Result<()> {
                let (mut stream, _) = listener.accept()?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                read_request(&mut stream)?;
                let body = complete_response();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )?;
                Ok(())
            });
            fixture.runtime.execute(dispatch)?;
            server.join().map_err(|_| "server panic")??;
            assert_eq!(fixture.state()?.state, TaskState::Completed);
        }
        assert_released(&fixture)?;
    }
    Ok(())
}
#[test]
fn encoded_request_refusal_is_terminal_without_entry_or_send() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let mut value = serde_json::to_value(endpoint(&listener.local_addr()?.to_string())?)?;
    *value
        .pointer_mut("/limits/max_request_bytes")
        .ok_or("request bound")? = json!(1);
    let fixture = ModelFixture::new(
        serde_json::from_value(value)?,
        Arc::new(Allow),
        Arc::new(NoFaults),
    )?;
    let dispatch = claim(&fixture)?;
    fixture.runtime.execute(dispatch.clone())?;
    let state = fixture.state()?;
    assert_eq!(state.state, TaskState::Failed);
    assert!(
        state
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("request-body bound"))
    );
    assert!(!entered(&fixture)?);
    assert!(fixture.runtime.execute(dispatch).is_err());
    fixture.runtime.recover()?;
    assert_eq!(state, fixture.state()?);
    assert_released(&fixture)?;
    no_connections(&listener)
}
#[test]
fn removed_generation_leaves_no_external_obligation() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string())?,
        Arc::new(Allow),
        Arc::new(NoFaults),
    )?;
    let dispatch = claim(&fixture)?;
    fixture.host.force_remove(
        dispatch.resolution().capability(),
        dispatch.resolution().descriptor_revision(),
    )?;
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, TaskState::Failed);
    assert!(!entered(&fixture)?);
    assert_released(&fixture)?;
    reopen_and_assert(fixture, TaskState::Failed)?;
    no_connections(&listener)
}
#[derive(Default)]
struct RevokeOnFinalCheck {
    checks: AtomicUsize,
}
impl AuthorityEvaluator for RevokeOnFinalCheck {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let allowed = Allow.evaluate(request)?;
        if request.decision.as_str().starts_with("entry_") {
            self.checks.fetch_add(1, Ordering::SeqCst);
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
fn authority_is_checked_after_preparation_and_releases_the_exact_permit() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let authority = Arc::new(RevokeOnFinalCheck::default());
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string())?,
        authority.clone(),
        Arc::new(NoFaults),
    )?;
    fixture.runtime.execute(claim(&fixture)?)?;
    assert_eq!(authority.checks.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.state()?.state, TaskState::Failed);
    assert!(!entered(&fixture)?);
    assert!(
        fixture
            .state()?
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("revoked"))
    );
    assert_released(&fixture)?;
    no_connections(&listener)
}
#[derive(Default)]
struct Observations(Mutex<Vec<InvocationEvent>>);
impl ExecutionReporter for Observations {
    fn invocation(&self, event: InvocationEvent) -> Result<ObservationDisposition, ExecutorError> {
        self.0
            .lock()
            .map_err(|_| ExecutorError::BoundaryAfterEntry("report lock".into()))?
            .push(event);
        Ok(ObservationDisposition::Applied)
    }
    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError> {
        Ok(ObservationDisposition::Applied)
    }
}
#[test]
fn preparation_is_exclusive_and_changed_dispatch_or_generation_cannot_enter() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let fixture = ModelFixture::new(endpoint(&address)?, Arc::new(Allow), Arc::new(NoFaults))?;
    let dispatch = claim(&fixture)?;
    let prepared = fixture.host.prepare_exact_entry(&dispatch)?;
    assert!(matches!(
        fixture.host.prepare_exact_entry(&dispatch),
        Err(ExecutorError::Overloaded(_))
    ));
    let other = ModelFixture::with_instructions(
        endpoint(&address)?,
        "different captured instructions",
        Arc::new(Allow),
        Arc::new(NoFaults),
    )?;
    assert!(matches!(
        prepared.enter(&claim(&other)?, &Observations::default()),
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
    assert_released(&fixture)?;
    no_connections(&listener)
}
#[derive(Default)]
struct DenyReadAfterIntent {
    target: Mutex<Option<Arc<AtomicBool>>>,
}
impl FaultInjector for DenyReadAfterIntent {
    fn check(&self, point: FaultPoint) -> Result<(), milkdrift_persistence::PersistenceError> {
        if point == FaultPoint::AfterCommandCommit {
            let target = self
                .target
                .lock()
                .map_err(|_| milkdrift_redb_store::injected_failure(point))?
                .take();
            if let Some(target) = target {
                target.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}
#[test]
fn prepared_request_enters_without_reloading_selected_bytes() -> TestResult {
    let (address, server) = serve(complete_response(), "application/json")?;
    let fault = Arc::new(DenyReadAfterIntent::default());
    let fixture = ModelFixture::new(endpoint(&address)?, Arc::new(Allow), fault.clone())?;
    let dispatch = claim(&fixture)?;
    *fault.target.lock().map_err(|_| "target lock")? = Some(fixture.read_denial.clone());
    fixture.runtime.execute(dispatch)?;
    let captured = server.join().map_err(|_| "server panic")??;
    assert!(captured.contains("bounded hostile endpoint"));
    assert!(captured.contains("captured project evidence"));
    assert!(fixture.read_denial.load(Ordering::SeqCst));
    assert_eq!(fixture.state()?.state, TaskState::Completed);
    assert!(entered(&fixture)?);
    assert_released(&fixture)
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
        endpoint(&listener.local_addr()?.to_string())?,
        Arc::new(Allow),
        fault.clone(),
    )?;
    let dispatch = claim(&fixture)?;
    fault.arm(1);
    assert!(fixture.runtime.execute(dispatch.clone()).is_err());
    assert!(entered(&fixture)?);
    assert!(fixture.runtime.execute(dispatch).is_err());
    assert_released(&fixture)?;
    reopen_and_assert(fixture, TaskState::Uncertain)?;
    no_connections(&listener)
}
#[test]
fn local_publication_and_terminal_commit_faults_preserve_observed_response_without_second_send()
-> TestResult {
    for (point, count, expected) in [
        (
            FaultPoint::BeforeArtifactMetadataCommit,
            1,
            TaskState::Failed,
        ),
        (FaultPoint::BeforeCommandCommit, 2, TaskState::Uncertain),
        (FaultPoint::BeforeCommandCommit, 5, TaskState::Uncertain),
        (FaultPoint::AfterCommandCommit, 5, TaskState::Completed),
    ] {
        let (address, server) = serve(complete_response(), "application/json")?;
        let fault = Arc::new(CommitFailure::new(point));
        let fixture = ModelFixture::new(endpoint(&address)?, Arc::new(Allow), fault.clone())?;
        let dispatch = claim(&fixture)?;
        fault.arm(count);
        let _outcome = fixture.runtime.execute(dispatch.clone());
        server.join().map_err(|_| "server panic")??;
        assert_eq!(
            fault.remaining.load(Ordering::SeqCst),
            0,
            "fault window not reached: {point:?}/{count}"
        );
        assert_eq!(
            fixture.state()?.state,
            expected,
            "{point:?}/{count}: {:?}",
            fixture.state()?
        );
        assert!(entered(&fixture)?);
        assert!(fixture.runtime.execute(dispatch).is_err());
        assert_released(&fixture)?;
        reopen_and_assert(fixture, expected)?;
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
            self.adapter
                .lock()
                .map_err(|_| milkdrift_redb_store::injected_failure(point))?
                .upgrade()
                .ok_or_else(|| milkdrift_redb_store::injected_failure(point))?
                .shutdown()
                .map_err(|_| milkdrift_redb_store::injected_failure(point))?;
        }
        Ok(())
    }
}
#[test]
fn stopped_adapter_after_intent_is_uncertain_even_when_server_observes_zero_requests() -> TestResult
{
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fault = Arc::new(StopBeforeSend {
        armed: AtomicBool::new(false),
        adapter: Mutex::new(std::sync::Weak::new()),
    });
    let fixture = ModelFixture::new(
        endpoint(&listener.local_addr()?.to_string())?,
        Arc::new(Allow),
        fault.clone(),
    )?;
    let dispatch = claim(&fixture)?;
    *fault.adapter.lock().map_err(|_| "adapter lock")? = Arc::downgrade(&fixture.adapter);
    fault.armed.store(true, Ordering::SeqCst);
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, TaskState::Uncertain);
    assert!(entered(&fixture)?);
    assert_released(&fixture)?;
    no_connections(&listener)?;
    reopen_and_assert(fixture, TaskState::Uncertain)
}
#[test]
fn failed_refusal_commit_preserves_unentered_frozen_start_on_restart() -> TestResult {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let fault = Arc::new(CommitFailure::new(FaultPoint::BeforeCommandCommit));
    let mut value = serde_json::to_value(endpoint(&listener.local_addr()?.to_string())?)?;
    *value
        .pointer_mut("/limits/max_request_bytes")
        .ok_or("request bound")? = json!(1);
    let profile: EndpointProfile = serde_json::from_value(value)?;
    let fixture = ModelFixture::new(profile.clone(), Arc::new(Allow), fault.clone())?;
    let dispatch = claim(&fixture)?;
    fault.arm(1);
    assert!(fixture.runtime.execute(dispatch.clone()).is_err());
    assert_eq!(fixture.state()?.state, TaskState::Running);
    assert!(!entered(&fixture)?);
    assert!(fixture.state()?.result.is_none());
    assert_released(&fixture)?;
    let fixture = fixture.reopen(profile, Arc::new(Allow), 1000)?;
    assert_eq!(
        fixture.state()?.frozen.ok_or("frozen start")?.dispatch,
        dispatch
    );
    let recovered = fixture.runtime.recover()?;
    assert_eq!(recovered, vec![dispatch.clone()]);
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, TaskState::Failed);
    assert!(!entered(&fixture)?);
    no_connections(&listener)
}
#[test]
fn interrupted_reception_acceptance_and_response_never_repeat_after_reopen() -> TestResult {
    for stage in ["receiving", "accepted", "partial_response"] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?.to_string();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
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
                            if stage == "partial_response" {
                                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 200\r\nConnection: close\r\n\r\n{\"choices\":[")?;
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error),
                }
                thread::sleep(Duration::from_millis(5));
                if stopped.load(Ordering::SeqCst) {
                    return Ok(());
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "endpoint observer expired",
            ))
        });
        let fixture = ModelFixture::new(endpoint(&address)?, Arc::new(Allow), Arc::new(NoFaults))?;
        let dispatch = claim(&fixture)?;
        fixture.runtime.execute(dispatch.clone())?;
        assert_eq!(fixture.state()?.state, TaskState::Uncertain);
        assert!(entered(&fixture)?);
        assert!(fixture.runtime.execute(dispatch).is_err());
        assert_released(&fixture)?;
        reopen_and_assert(fixture, TaskState::Uncertain)?;
        stop.store(true, Ordering::SeqCst);
        server.join().map_err(|_| "counting server panic")??;
        assert_eq!(count.load(Ordering::SeqCst), 1, "{stage}");
    }
    Ok(())
}
