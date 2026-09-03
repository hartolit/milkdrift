//! Reusable adapter contract assertions for production implementations.
//!
//! This module is available only under explicit test support. It is not part of the default
//! product surface and owns no production lifecycle or execution behavior.

use std::{
    any::Any,
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
};

use milkdrift_authority::{CapabilityAuthorityScope, CapabilityExecutionRequirements};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, CapabilityObservation,
    CapabilityRequirement, InvocationAdmissionEnvelope, InvocationEvent, InvocationRequest,
    ResolvedCapabilitySnapshot, SideEffectClass,
};

use crate::{
    AdapterError, AdapterExecutionContext, AdapterFailureKind, AdapterInvocation, AdapterReporter,
    CapabilityAdapter, CapabilityHost, CapabilitySelectionPolicy, HostConfig,
};

/// Fresh fixture purpose supplied to a production adapter's mechanism-specific factory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConformanceScenario {
    /// Immutable facts and direct lifecycle replay.
    Lifecycle,
    /// Successful exact execution and terminal reporting.
    Execution,
    /// A durable reporter rejection that must propagate.
    ReporterFailure,
    /// Unknown and duplicate cancellation behavior.
    Cancellation,
    /// Canonical host registration, drain, exact pinned execution, and removal.
    HostDrain,
}

impl ConformanceScenario {
    /// Whether the fixture must provide a live external mechanism for one execution.
    #[must_use]
    pub const fn executes(self) -> bool {
        matches!(
            self,
            Self::Execution | Self::ReporterFailure | Self::HostDrain
        )
    }
}

/// Permitted repeated-start behavior declared by one implementation fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartReplayExpectation {
    /// Starting an already started generation is an exact idempotent replay.
    Idempotent,
    /// Starting an already started generation returns a typed rejected conflict.
    Rejected,
}

/// Explicit behavior for cancellation of an invocation the adapter does not currently own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownCancellationExpectation {
    /// The adapter returns an exact negative acknowledgement.
    NegativeAcknowledgement,
    /// The adapter returns a typed unavailable result because no mechanism identity can be found.
    Unavailable,
}

/// Legitimate lifecycle differences that the common suite must assert rather than erase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterConformanceExpectations {
    /// Repeated direct-start behavior.
    pub start_replay: StartReplayExpectation,
    /// Health availability after an explicit drain hook.
    pub available_while_draining: bool,
    /// Health availability after shutdown.
    pub available_after_shutdown: bool,
    /// Unknown cancellation behavior.
    pub unknown_cancellation: UnknownCancellationExpectation,
}

/// One fresh production adapter generation and exact invocation.
pub struct AdapterConformanceCase {
    adapter: Arc<dyn CapabilityAdapter>,
    descriptor: CapabilityDescriptor,
    snapshot: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
    context: AdapterExecutionContext,
    expectations: AdapterConformanceExpectations,
    cleanup: Option<Box<dyn FnOnce() -> Result<(), String> + Send>>,
    keepalive: Vec<Box<dyn Any + Send>>,
}

impl AdapterConformanceCase {
    /// Constructs one exact case. The request must name an operation on the descriptor.
    pub fn new(
        adapter: Arc<dyn CapabilityAdapter>,
        descriptor: CapabilityDescriptor,
        request: InvocationRequest,
        context: AdapterExecutionContext,
        expectations: AdapterConformanceExpectations,
    ) -> Result<Self, AdapterConformanceError> {
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&descriptor, request.operation())
                .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
        if request.capability() != snapshot.capability()
            || request.provider_profile() != snapshot.provider_profile()
        {
            return Err(AdapterConformanceError::new(
                "conformance request does not name the exact descriptor generation",
            ));
        }
        Ok(Self {
            adapter,
            descriptor,
            snapshot,
            request,
            context,
            expectations,
            cleanup: None,
            keepalive: Vec::new(),
        })
    }

    /// Installs mechanism-specific cleanup, such as joining a bounded mock listener.
    #[must_use]
    pub fn with_cleanup(
        mut self,
        cleanup: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) -> Self {
        self.cleanup = Some(Box::new(cleanup));
        self
    }

    /// Retains a fixture owner until after the adapter and its dependencies are dropped.
    #[must_use]
    pub fn with_keepalive(mut self, owner: impl Any + Send) -> Self {
        self.keepalive.push(Box::new(owner));
        self
    }

    fn invocation(&self) -> AdapterInvocation<'_> {
        AdapterInvocation::with_context(&self.snapshot, &self.request, &self.context)
    }

    fn cleanup(&mut self) -> Result<(), AdapterConformanceError> {
        self.cleanup.take().map_or(Ok(()), |cleanup| {
            cleanup().map_err(AdapterConformanceError::new)
        })
    }
}

/// Failure from the reusable adapter test contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdapterConformanceError(String);

impl AdapterConformanceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for AdapterConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for AdapterConformanceError {}

/// Runs the same common contract against one fresh production fixture per scenario.
pub fn run_adapter_conformance<E>(
    mut factory: impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
) -> Result<(), AdapterConformanceError>
where
    E: fmt::Display,
{
    let mut make = |scenario| {
        factory(scenario).map_err(|error| {
            AdapterConformanceError::new(format!("{scenario:?} fixture failed: {error}"))
        })
    };
    lifecycle_contract(&mut make)?;
    failed_start_contract(&mut make)?;
    execution_contract(&mut make, ConformanceScenario::Execution, false)?;
    execution_contract(&mut make, ConformanceScenario::ReporterFailure, true)?;
    cancellation_contract(&mut make)?;
    host_drain_contract(&mut make)?;
    Ok(())
}

fn failed_start_contract<E>(
    make: &mut impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
) -> Result<(), AdapterConformanceError>
where
    E: Into<AdapterConformanceError>,
{
    let mut case = make(ConformanceScenario::Lifecycle).map_err(Into::into)?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 1,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 1,
            observation_stale_after_ms: 1_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    let shutdowns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let adapter = Arc::new(FailingStartAdapter {
        delegate: case.adapter.clone(),
        host: host.clone(),
        visible_during_start: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        shutdowns: shutdowns.clone(),
    });
    let visible_during_start = adapter.visible_during_start.clone();
    require(
        host.register(case.descriptor.clone(), adapter, None)
            .is_err(),
        "an injected start failure unexpectedly registered a generation",
    )?;
    require(
        !visible_during_start.load(std::sync::atomic::Ordering::SeqCst)
            && host
                .generations(
                    &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                    0,
                )
                .map_err(|error| AdapterConformanceError::new(error.to_string()))?
                .is_empty(),
        "a failed start made a partial generation visible",
    )?;
    require(
        shutdowns.load(std::sync::atomic::Ordering::SeqCst) == 1,
        "a failed start did not invoke exactly one cleanup hook",
    )?;
    case.cleanup()
}

fn lifecycle_contract<E>(
    make: &mut impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
) -> Result<(), AdapterConformanceError>
where
    E: Into<AdapterConformanceError>,
{
    let mut case = make(ConformanceScenario::Lifecycle).map_err(Into::into)?;
    let first_requirements = case.adapter.authority_requirements();
    let second_requirements = case.adapter.authority_requirements();
    require(
        first_requirements == second_requirements,
        "authority requirements changed for one immutable generation",
    )?;
    let first_envelope = case
        .adapter
        .admission_envelope(&case.invocation())
        .map_err(adapter_failure)?;
    let second_envelope = case
        .adapter
        .admission_envelope(&case.invocation())
        .map_err(adapter_failure)?;
    require(
        first_envelope == second_envelope,
        "admission envelope changed for one exact request",
    )?;
    case.adapter.start().map_err(adapter_failure)?;
    match (case.expectations.start_replay, case.adapter.start()) {
        (StartReplayExpectation::Idempotent, Ok(())) => {}
        (StartReplayExpectation::Rejected, Err(error))
            if error.kind() == AdapterFailureKind::Rejected => {}
        (expected, observed) => {
            return Err(AdapterConformanceError::new(format!(
                "unexpected repeated-start behavior: expected {expected:?}, observed {observed:?}"
            )));
        }
    }
    assert_health(&case, 41, true)?;
    case.adapter.begin_drain().map_err(adapter_failure)?;
    case.adapter.begin_drain().map_err(adapter_failure)?;
    assert_health(&case, 42, case.expectations.available_while_draining)?;
    case.adapter.shutdown().map_err(adapter_failure)?;
    case.adapter.shutdown().map_err(adapter_failure)?;
    assert_health(&case, 43, case.expectations.available_after_shutdown)?;
    case.cleanup()
}

fn execution_contract<E>(
    make: &mut impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
    scenario: ConformanceScenario,
    reject_reports: bool,
) -> Result<(), AdapterConformanceError>
where
    E: Into<AdapterConformanceError>,
{
    let mut case = make(scenario).map_err(Into::into)?;
    case.adapter.start().map_err(adapter_failure)?;
    if reject_reports {
        let reporter = RejectingReporter;
        let error = match case.adapter.execute(&case.invocation(), &reporter) {
            Ok(()) => {
                return Err(AdapterConformanceError::new(
                    "adapter ignored a durable reporter failure",
                ));
            }
            Err(error) => error,
        };
        require(
            error.kind() == AdapterFailureKind::ExternalFailure
                && error.summary().contains("durable reporter rejection"),
            "adapter obscured or reclassified a durable reporter failure",
        )?;
    } else {
        let reporter = RecordingReporter::default();
        case.adapter
            .execute(&case.invocation(), &reporter)
            .map_err(adapter_failure)?;
        reporter.assert_complete(case.request.invocation())?;
    }
    case.adapter.shutdown().map_err(adapter_failure)?;
    case.cleanup()
}

fn cancellation_contract<E>(
    make: &mut impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
) -> Result<(), AdapterConformanceError>
where
    E: Into<AdapterConformanceError>,
{
    let mut case = make(ConformanceScenario::Cancellation).map_err(Into::into)?;
    case.adapter.start().map_err(adapter_failure)?;
    let request = CancellationRequest::new(
        case.request.invocation().clone(),
        7,
        "adapter conformance cancellation",
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    for _duplicate in 0..2 {
        match (
            case.expectations.unknown_cancellation,
            case.adapter.cancel(&request),
        ) {
            (UnknownCancellationExpectation::NegativeAcknowledgement, Ok(acknowledgement)) => {
                require(
                    acknowledgement.invocation() == request.invocation()
                        && acknowledgement.request_sequence() == request.request_sequence()
                        && !acknowledgement.accepted()
                        && !acknowledgement.terminal_boundary(),
                    "negative cancellation acknowledgement lost exact correlation or overclaimed termination",
                )?;
            }
            (UnknownCancellationExpectation::Unavailable, Err(error))
                if error.kind() == AdapterFailureKind::Unavailable => {}
            (expected, observed) => {
                return Err(AdapterConformanceError::new(format!(
                    "unexpected unknown-cancellation behavior: expected {expected:?}, observed {observed:?}"
                )));
            }
        }
    }
    case.adapter.shutdown().map_err(adapter_failure)?;
    case.cleanup()
}

fn host_drain_contract<E>(
    make: &mut impl FnMut(ConformanceScenario) -> Result<AdapterConformanceCase, E>,
) -> Result<(), AdapterConformanceError>
where
    E: Into<AdapterConformanceError>,
{
    let mut case = make(ConformanceScenario::HostDrain).map_err(Into::into)?;
    let host = CapabilityHost::new(
        HostConfig {
            max_registrations: 2,
            max_generations_per_capability: 1,
            max_concurrent_per_generation: 2,
            observation_stale_after_ms: 1_000,
        },
        CapabilitySelectionPolicy::priorities(BTreeMap::new()),
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    host.register(case.descriptor.clone(), case.adapter.clone(), None)
        .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    host.refresh_health(
        case.descriptor.identity(),
        case.descriptor.descriptor_revision(),
        100,
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    let requirement = CapabilityRequirement::new(case.request.operation().clone())
        .exact(case.descriptor.identity().clone());
    host.resolve_at(&requirement, 100)
        .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    host.begin_drain(
        case.descriptor.identity(),
        case.descriptor.descriptor_revision(),
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    require(
        host.resolve_at(&requirement, 100).is_err(),
        "drained generation remained visible to new resolution",
    )?;
    let reporter = RecordingReporter::default();
    host.execute_exact_with_context(&case.snapshot, &case.request, &case.context, &reporter)
        .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    reporter.assert_complete(case.request.invocation())?;
    host.finish_drain(
        case.descriptor.identity(),
        case.descriptor.descriptor_revision(),
    )
    .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    require(
        host.execute_exact_with_context(
            &case.snapshot,
            &case.request,
            &case.context,
            &RecordingReporter::default(),
        )
        .is_err(),
        "removed generation accepted another exact invocation",
    )?;
    host.shutdown()
        .map_err(|error| AdapterConformanceError::new(error.to_string()))?;
    case.cleanup()
}

fn assert_health(
    case: &AdapterConformanceCase,
    observed_at_unix_ms: u64,
    expected_available: bool,
) -> Result<(), AdapterConformanceError> {
    let health = case
        .adapter
        .health(observed_at_unix_ms)
        .map_err(adapter_failure)?;
    require(
        health.capability() == case.descriptor.identity(),
        "health observation named another capability",
    )?;
    require(
        health.observed_at_unix_ms() == observed_at_unix_ms,
        "health observation replaced the supplied boundary time",
    )?;
    require(
        health.available() == expected_available,
        "health availability disagrees with the declared lifecycle semantic",
    )
}

#[derive(Default)]
struct RecordingReporter {
    events: Mutex<Vec<InvocationEvent>>,
}

impl RecordingReporter {
    fn assert_complete(
        &self,
        invocation: &milkdrift_capability::InvocationId,
    ) -> Result<(), AdapterConformanceError> {
        let events = self
            .events
            .lock()
            .map_err(|_| AdapterConformanceError::new("recording reporter lock poisoned"))?;
        require(!events.is_empty(), "adapter returned without observations")?;
        let mut terminal_count = 0_usize;
        for (index, event) in events.iter().enumerate() {
            let expected_sequence = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| AdapterConformanceError::new("report sequence overflow"))?;
            require(
                event.invocation() == invocation && event.sequence() == expected_sequence,
                "adapter forged invocation identity or emitted a non-contiguous sequence",
            )?;
            if event.kind().terminal().is_some() {
                terminal_count = terminal_count.saturating_add(1);
                require(
                    index + 1 == events.len(),
                    "adapter emitted an observation after terminal evidence",
                )?;
            }
        }
        require(
            terminal_count == 1,
            "adapter must emit exactly one terminal observation",
        )
    }
}

impl AdapterReporter for RecordingReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        self.events
            .lock()
            .map_err(|_| AdapterError::external_failure("recording reporter lock poisoned"))?
            .push(event);
        Ok(())
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

struct RejectingReporter;

impl AdapterReporter for RejectingReporter {
    fn invocation(&self, _event: InvocationEvent) -> Result<(), AdapterError> {
        Err(AdapterError::external_failure(
            "injected durable reporter rejection",
        ))
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        Err(AdapterError::external_failure(
            "injected durable heartbeat rejection",
        ))
    }
}

struct FailingStartAdapter {
    delegate: Arc<dyn CapabilityAdapter>,
    host: CapabilityHost,
    visible_during_start: Arc<std::sync::atomic::AtomicBool>,
    shutdowns: Arc<std::sync::atomic::AtomicUsize>,
}

impl CapabilityAdapter for FailingStartAdapter {
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.delegate.admission_envelope(invocation)
    }

    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.delegate.authority_requirements()
    }

    fn start(&self) -> Result<(), AdapterError> {
        let visible = self
            .host
            .generations(
                &CapabilityAuthorityScope::allow_any(SideEffectClass::Unknown),
                0,
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.visible_during_start
            .store(!visible.is_empty(), std::sync::atomic::Ordering::SeqCst);
        Err(AdapterError::rejected("injected start failure"))
    }

    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        self.delegate.execute(invocation, reporter)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        self.delegate.cancel(request)
    }

    fn health(&self, observed_at_unix_ms: u64) -> Result<CapabilityObservation, AdapterError> {
        self.delegate.health(observed_at_unix_ms)
    }

    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.delegate.begin_drain()
    }

    fn shutdown(&self) -> Result<(), AdapterError> {
        self.shutdowns
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.delegate.shutdown()
    }
}

fn adapter_failure(error: AdapterError) -> AdapterConformanceError {
    AdapterConformanceError::new(error.to_string())
}

fn require(condition: bool, message: &'static str) -> Result<(), AdapterConformanceError> {
    condition
        .then_some(())
        .ok_or_else(|| AdapterConformanceError::new(message))
}
