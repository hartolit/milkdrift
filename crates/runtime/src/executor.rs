use std::{collections::BTreeMap, sync::Mutex};

use milkdrift_blueprint::{NodeId, RevisionId};
use milkdrift_capability::{
    CancellationAcknowledgement, CancellationRequest, CapabilityDescriptor, CapabilityRequirement,
    ContractError, IdempotencyBehavior, InvocationEvent, InvocationEventKind, InvocationRequest,
    InvocationTerminal, OperationId, ResolvedCapabilitySnapshot, SideEffectClass, TerminalStatus,
};
use milkdrift_persistence::{AttemptId, LeaseId, NodeExecutionId, TimestampMillis};
use milkdrift_workspace::RunId;
use thiserror::Error;

/// Maximum executor reports accepted from one dispatch before backpressure rejects it.
pub const MAX_REPORTS_PER_DISPATCH: usize = 1_024;

/// Error at the narrow capability-execution boundary.
#[derive(Debug, Error)]
pub enum ExecutorError {
    /// An immutable request and its exact capability snapshot disagree.
    #[error("invalid execution dispatch: {0}")]
    InvalidDispatch(String),
    /// No descriptor matched the exact semantic and policy constraints.
    #[error("capability resolution mismatch: {reasons:?}")]
    ResolutionMismatch {
        /// Stable mismatch fields.
        reasons: Vec<String>,
    },
    /// Authority policy denied capability selection.
    #[error("capability selection denied by authority policy: {0}")]
    AuthorityDenied(String),
    /// The exact persisted descriptor generation is no longer hosted.
    #[error("capability generation {capability}/{descriptor_revision} is unavailable")]
    UnavailableGeneration {
        /// Exact capability identity.
        capability: milkdrift_capability::CapabilityId,
        /// Exact descriptor revision.
        descriptor_revision: u64,
    },
    /// A generation is unhealthy, stale, draining for resolution, or otherwise unavailable.
    #[error("capability unavailable: {0}")]
    Unavailable(String),
    /// Admission was refused without entering adapter code.
    #[error("capability generation overloaded: {0}")]
    Overloaded(String),
    /// Host shutdown closed new admission before adapter entry.
    #[error("capability host admission is closed")]
    AdmissionClosed,
    /// A typed host failure occurred before external adapter entry.
    #[error("executor failed before adapter entry: {0}")]
    BoundaryBeforeEntry(String),
    /// Adapter code was entered and returned without a terminal durable observation.
    #[error("executor outcome is unknown after adapter entry: {0}")]
    BoundaryAfterEntry(String),
    /// Adapter code panicked; `after_entry` distinguishes uncertainty classification.
    #[error("adapter panicked (after_entry={after_entry})")]
    AdapterPanicked {
        /// Whether external adapter code had been entered.
        after_entry: bool,
    },
    /// Executor admission or execution failed before a valid report batch existed.
    #[error("executor boundary failed: {0}")]
    Boundary(String),
    /// An executor returned malformed, unbounded, or out-of-order reports.
    #[error("invalid executor reports: {0}")]
    InvalidReports(String),
    /// Capability contract validation failed.
    #[error(transparent)]
    Contract(#[from] ContractError),
}

/// Exact immutable resolution used by scheduling and dispatch.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedCapability {
    descriptor: CapabilityDescriptor,
    snapshot: ResolvedCapabilitySnapshot,
}

impl ResolvedCapability {
    /// Constructs and cross-checks an exact descriptor/snapshot pair.
    pub fn new(
        descriptor: CapabilityDescriptor,
        snapshot: ResolvedCapabilitySnapshot,
    ) -> Result<Self, ExecutorError> {
        snapshot.validate_against(&descriptor)?;
        Ok(Self {
            descriptor,
            snapshot,
        })
    }

    /// Exact immutable descriptor selected by the boundary.
    #[must_use]
    pub const fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }

    /// Domain-separated operation snapshot supplied before dispatch.
    #[must_use]
    pub const fn snapshot(&self) -> &ResolvedCapabilitySnapshot {
        &self.snapshot
    }
}

/// Dispatch value delivered to an executor after schedule and lease facts are durable.
#[derive(Debug, PartialEq)]
pub struct ExecutionDispatch {
    run: RunId,
    revision: RevisionId,
    node: NodeId,
    execution: NodeExecutionId,
    attempt: AttemptId,
    lease: LeaseId,
    lease_expires_at: TimestampMillis,
    resolution: ResolvedCapabilitySnapshot,
    request: InvocationRequest,
}

impl ExecutionDispatch {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_snapshot(
        run: RunId,
        revision: RevisionId,
        node: NodeId,
        execution: NodeExecutionId,
        attempt: AttemptId,
        lease: LeaseId,
        lease_expires_at: TimestampMillis,
        resolution: ResolvedCapabilitySnapshot,
        request: InvocationRequest,
    ) -> Result<Self, ExecutorError> {
        let snapshot = &resolution;
        if request.capability() != snapshot.capability()
            || request.operation() != snapshot.operation()
            || request.provider_profile() != snapshot.provider_profile()
        {
            return Err(ExecutorError::InvalidDispatch(
                "invocation selection does not equal the resolved capability snapshot".to_owned(),
            ));
        }
        let contract = snapshot.operation_contract();
        if contract.idempotency() == IdempotencyBehavior::Unsupported
            && request.idempotency_key().is_some()
        {
            return Err(ExecutorError::InvalidDispatch(
                "an operation advertising unsupported idempotency cannot receive a key".to_owned(),
            ));
        }
        if contract.side_effect() == SideEffectClass::IdempotentWrite
            && (contract.idempotency() == IdempotencyBehavior::Unsupported
                || request.idempotency_key().is_none())
        {
            return Err(ExecutorError::InvalidDispatch(
                "an idempotent write requires advertised idempotency and a stable key".to_owned(),
            ));
        }
        Ok(Self {
            run,
            revision,
            node,
            execution,
            attempt,
            lease,
            lease_expires_at,
            resolution,
            request,
        })
    }

    /// Owning run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Exact revision used by this invocation.
    #[must_use]
    pub const fn revision(&self) -> &RevisionId {
        &self.revision
    }

    /// Stable semantic node.
    #[must_use]
    pub const fn node(&self) -> &NodeId {
        &self.node
    }

    /// Logical execution identity.
    #[must_use]
    pub const fn execution(&self) -> &NodeExecutionId {
        &self.execution
    }

    /// Immutable attempt identity.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Durable lease proving dispatch ownership.
    #[must_use]
    pub const fn lease(&self) -> &LeaseId {
        &self.lease
    }

    /// Recorded lease deadline.
    #[must_use]
    pub const fn lease_expires_at(&self) -> TimestampMillis {
        self.lease_expires_at
    }

    /// Exact capability resolution.
    #[must_use]
    pub const fn resolution(&self) -> &ResolvedCapabilitySnapshot {
        &self.resolution
    }

    /// Validated provider-neutral invocation request.
    #[must_use]
    pub const fn request(&self) -> &InvocationRequest {
        &self.request
    }
}

/// Exact cancellation effect claimed from durable run state.
#[derive(Clone, Debug, PartialEq)]
pub struct CancellationDispatch {
    run: RunId,
    attempt: AttemptId,
    request: CancellationRequest,
}

impl CancellationDispatch {
    pub(crate) fn new(run: RunId, attempt: AttemptId, request: CancellationRequest) -> Self {
        Self {
            run,
            attempt,
            request,
        }
    }

    /// Owning run.
    #[must_use]
    pub const fn run(&self) -> &RunId {
        &self.run
    }

    /// Exact attempt whose invocation receives cancellation.
    #[must_use]
    pub const fn attempt(&self) -> &AttemptId {
        &self.attempt
    }

    /// Provider-neutral cancellation request.
    #[must_use]
    pub const fn request(&self) -> &CancellationRequest {
        &self.request
    }
}

/// One durably claimed external action for a caller-owned effect host.
#[derive(Debug, PartialEq)]
pub enum EffectAction {
    /// Execute one immutable invocation.
    Execute(Box<ExecutionDispatch>),
    /// Submit one cancellation request.
    Cancel(CancellationDispatch),
}

/// Validated, bounded executor reports for one invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutionReportBatch(Vec<InvocationEvent>);

impl ExecutionReportBatch {
    /// Validates correlation, contiguity, terminal uniqueness, and the hard report bound.
    pub fn new(
        request: &InvocationRequest,
        reports: Vec<InvocationEvent>,
    ) -> Result<Self, ExecutorError> {
        if reports.is_empty() || reports.len() > MAX_REPORTS_PER_DISPATCH {
            return Err(ExecutorError::InvalidReports(format!(
                "a dispatch must return 1..={MAX_REPORTS_PER_DISPATCH} reports"
            )));
        }
        let mut terminal_seen = false;
        for (index, report) in reports.iter().enumerate() {
            let expected_sequence = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    ExecutorError::InvalidReports("report sequence overflow".to_owned())
                })?;
            if report.invocation() != request.invocation() || report.sequence() != expected_sequence
            {
                return Err(ExecutorError::InvalidReports(
                    "reports must match the invocation and be contiguous from sequence one"
                        .to_owned(),
                ));
            }
            if terminal_seen {
                return Err(ExecutorError::InvalidReports(
                    "no report may follow a terminal report".to_owned(),
                ));
            }
            terminal_seen = report.kind().terminal().is_some();
        }
        if !terminal_seen {
            return Err(ExecutorError::InvalidReports(
                "a synchronous report batch must end in exactly one terminal report".to_owned(),
            ));
        }
        Ok(Self(reports))
    }

    /// Ordered immutable reports.
    #[must_use]
    pub fn reports(&self) -> &[InvocationEvent] {
        &self.0
    }
}

/// Durable result of submitting one executor observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationDisposition {
    /// The observation advanced authoritative run state.
    Applied,
    /// An exact previously committed observation was returned idempotently.
    Replayed,
    /// A terminal observation arrived after ownership became uncertain and was kept as evidence.
    LateEvidence,
}

/// Incremental durable observation boundary supplied to long-running executors.
///
/// Calls may apply storage backpressure. A successful return means the observation is
/// already durable; adapters must not treat merely writing to an in-memory channel as
/// completion.
pub trait ExecutionReporter: Send + Sync {
    /// Persists one exactly correlated invocation observation.
    fn invocation(&self, report: InvocationEvent) -> Result<ObservationDisposition, ExecutorError>;

    /// Persists a policy-bounded lease extension while the invocation is still alive.
    ///
    /// The runtime chooses the new expiry from its trusted boundary clock and lease policy;
    /// an adapter cannot pin ownership arbitrarily far into the future.
    fn heartbeat(&self) -> Result<ObservationDisposition, ExecutorError>;
}

/// Narrow object-safe boundary implemented by Pass 3 registries/adapters.
pub trait TaskExecutor: Send + Sync {
    /// Deterministically resolves an exact immutable descriptor and operation snapshot.
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError>;

    /// Compatibility hook for bounded synchronous executors.
    ///
    /// Long-running adapters should implement [`Self::execute_streaming`]. This method
    /// is never invoked by a scheduler tick.
    fn execute(
        &self,
        _dispatch: &ExecutionDispatch,
    ) -> Result<ExecutionReportBatch, ExecutorError> {
        Err(ExecutorError::Boundary(
            "executor implements neither streaming nor bounded execution".to_owned(),
        ))
    }

    /// Executes after immutable request, resolution, side-effect, and lease facts are durable.
    ///
    /// The caller owns the effect-host thread/task. The runtime itself never spawns hidden
    /// work. The default implementation adapts a bounded legacy batch into independently
    /// durable observations.
    fn execute_streaming(
        &self,
        dispatch: &ExecutionDispatch,
        reporter: &dyn ExecutionReporter,
    ) -> Result<(), ExecutorError> {
        let reports = self.execute(dispatch)?;
        for report in reports.reports() {
            let _ = reporter.invocation(report.clone())?;
        }
        Ok(())
    }

    /// Requests cancellation without implying a terminal outcome.
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError>;
}

/// Deterministic bounded executor used by runtime and crash-recovery tests.
pub struct DeterministicExecutor {
    descriptor: CapabilityDescriptor,
    scripts: Mutex<BTreeMap<OperationId, Vec<InvocationEventKind>>>,
}

impl DeterministicExecutor {
    /// Creates a deterministic executor around one immutable descriptor.
    #[must_use]
    pub fn new(descriptor: CapabilityDescriptor) -> Self {
        Self {
            descriptor,
            scripts: Mutex::new(BTreeMap::new()),
        }
    }

    /// Installs a bounded deterministic script for an operation.
    pub fn set_script(
        &self,
        operation: OperationId,
        events: Vec<InvocationEventKind>,
    ) -> Result<(), ExecutorError> {
        if events.is_empty() || events.len() > MAX_REPORTS_PER_DISPATCH {
            return Err(ExecutorError::InvalidReports(format!(
                "a script must contain 1..={MAX_REPORTS_PER_DISPATCH} events"
            )));
        }
        let terminal_count = events
            .iter()
            .filter(|event| event.terminal().is_some())
            .count();
        if terminal_count != 1
            || events
                .last()
                .and_then(InvocationEventKind::terminal)
                .is_none()
        {
            return Err(ExecutorError::InvalidReports(
                "a script must end in exactly one terminal event".to_owned(),
            ));
        }
        let mut scripts = self.scripts.lock().map_err(|_| {
            ExecutorError::Boundary("deterministic executor lock poisoned".to_owned())
        })?;
        scripts.insert(operation, events);
        Ok(())
    }

    fn default_script(
        &self,
        operation: &OperationId,
    ) -> Result<Vec<InvocationEventKind>, ExecutorError> {
        let contract = self.descriptor.operation(operation).ok_or_else(|| {
            ExecutorError::ResolutionMismatch {
                reasons: vec![format!("operation '{operation}' is not advertised")],
            }
        })?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Success,
            Vec::new(),
            None,
            None,
            contract.side_effect(),
        )?;
        Ok(vec![InvocationEventKind::Terminal { terminal }])
    }
}

impl TaskExecutor for DeterministicExecutor {
    fn resolve(
        &self,
        requirement: &CapabilityRequirement,
        _observed_at_unix_ms: u64,
    ) -> Result<ResolvedCapability, ExecutorError> {
        let result = self.descriptor.matches(requirement);
        if !result.is_match() {
            return Err(ExecutorError::ResolutionMismatch {
                reasons: result.mismatch_reasons().to_vec(),
            });
        }
        let snapshot =
            ResolvedCapabilitySnapshot::from_descriptor(&self.descriptor, requirement.operation())?;
        ResolvedCapability::new(self.descriptor.clone(), snapshot)
    }

    fn execute(&self, dispatch: &ExecutionDispatch) -> Result<ExecutionReportBatch, ExecutorError> {
        let kinds = {
            let scripts = self.scripts.lock().map_err(|_| {
                ExecutorError::Boundary("deterministic executor lock poisoned".to_owned())
            })?;
            scripts.get(dispatch.request().operation()).cloned()
        };
        let kinds = match kinds {
            Some(kinds) => kinds,
            None => self.default_script(dispatch.request().operation())?,
        };
        let reports = kinds
            .into_iter()
            .enumerate()
            .map(|(index, kind)| {
                let sequence = u64::try_from(index)
                    .ok()
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| {
                        ExecutorError::InvalidReports("report sequence overflow".to_owned())
                    })?;
                InvocationEvent::new(dispatch.request().invocation().clone(), sequence, kind)
                    .map_err(ExecutorError::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        ExecutionReportBatch::new(dispatch.request(), reports)
    }

    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, ExecutorError> {
        CancellationAcknowledgement::new(
            request.invocation().clone(),
            request.request_sequence(),
            false,
            false,
            Some("deterministic executor has no concurrently running invocation".to_owned()),
        )
        .map_err(ExecutorError::from)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use milkdrift_capability::{CapabilityDescriptorDocument, CapabilityId, InvocationId};

    use super::*;

    type TestResult<T = ()> = Result<T, Box<dyn Error>>;

    fn request(invocation: &str) -> TestResult<InvocationRequest> {
        Ok(InvocationRequest::new(
            InvocationId::new(invocation)?,
            CapabilityId::new("capability.test")?,
            OperationId::new("tool.test")?,
            None,
            None,
            Vec::new(),
            BTreeMap::new(),
        )?)
    }

    fn progress(invocation: &str, sequence: u64) -> TestResult<InvocationEvent> {
        Ok(InvocationEvent::new(
            InvocationId::new(invocation)?,
            sequence,
            InvocationEventKind::Progress {
                message: "working".to_owned(),
                completed_units: None,
                total_units: None,
            },
        )?)
    }

    fn terminal(invocation: &str, sequence: u64) -> TestResult<InvocationEvent> {
        Ok(InvocationEvent::new(
            InvocationId::new(invocation)?,
            sequence,
            InvocationEventKind::Terminal {
                terminal: InvocationTerminal::new(
                    TerminalStatus::Success,
                    Vec::new(),
                    None,
                    None,
                    SideEffectClass::None,
                )?,
            },
        )?)
    }

    #[test]
    fn report_batches_reject_wrong_invocations_gaps_and_post_terminal_reports() -> TestResult {
        let request = request("invocation-one")?;
        assert!(
            ExecutionReportBatch::new(&request, vec![terminal("invocation-other", 1)?]).is_err()
        );
        assert!(
            ExecutionReportBatch::new(
                &request,
                vec![
                    progress("invocation-one", 1)?,
                    terminal("invocation-one", 3)?
                ]
            )
            .is_err()
        );
        assert!(
            ExecutionReportBatch::new(
                &request,
                vec![
                    terminal("invocation-one", 1)?,
                    progress("invocation-one", 2)?
                ]
            )
            .is_err()
        );
        assert!(
            ExecutionReportBatch::new(
                &request,
                vec![
                    progress("invocation-one", 1)?,
                    terminal("invocation-one", 2)?
                ]
            )
            .is_ok()
        );
        Ok(())
    }

    #[test]
    fn deterministic_cancellation_acknowledges_the_exact_request_without_claiming_terminal()
    -> TestResult {
        let document = CapabilityDescriptorDocument::from_json(include_bytes!(
            "../../capability/tests/fixtures/descriptor-v1.json"
        ))?;
        let executor = DeterministicExecutor::new(document.body().clone());
        let request = CancellationRequest::new(
            InvocationId::new("invocation-cancel")?,
            7,
            "operator requested cancellation",
        )?;
        let acknowledgement = executor.cancel(&request)?;
        assert_eq!(acknowledgement.invocation(), request.invocation());
        assert_eq!(acknowledgement.request_sequence(), 7);
        assert!(!acknowledgement.accepted());
        assert!(!acknowledgement.terminal_boundary());
        assert!(acknowledgement.detail().is_some());
        Ok(())
    }
}
