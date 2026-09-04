//! Durable dispatch-worker execution and adapter reporting.

use std::sync::{Arc, atomic::Ordering};

use milkdrift_capability::{
    ErrorClass, InvocationEvent, InvocationEventKind, InvocationFailure, InvocationTerminal,
    PeerId, TerminalStatus,
};
use milkdrift_capability_host::{AdapterError, AdapterReporter};
use milkdrift_peer_protocol::{
    CancellationDisposition, DrainState, ObservationCategory, PeerCancellationAcknowledgement,
    PeerExecutionId, PeerObservation,
};
use milkdrift_persistence::{
    PageSize, PeerClaimOutcome, PeerDispatchClaimRequest, PeerEntryOutcome, PeerEntryRequest,
    PeerExecutionPhase, PeerExecutionRecord, PeerExecutionSnapshot, PeerExecutionStore, WorkerId,
};
use milkdrift_runtime::ExecutorError;

use super::{
    PeerClock, PeerHttpError, PeerService, adapter_execution_context, bounded,
    map_execution_persistence, relationship_generation,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PeerUncertainty {
    AdapterFailure { detail: String },
    MissingTerminalEvidence,
    AdapterPanicked,
    ServiceInterrupted { detail: String },
    WorkerPanicked,
}

impl PeerUncertainty {
    fn from_execution(result: &Result<(), ExecutorError>) -> Self {
        match result {
            Ok(()) => Self::MissingTerminalEvidence,
            Err(ExecutorError::BoundaryAfterEntry(detail)) => Self::AdapterFailure {
                detail: bounded(detail, 1_920),
            },
            Err(ExecutorError::AdapterPanicked { after_entry: true }) => Self::AdapterPanicked,
            Err(error) => Self::ServiceInterrupted {
                detail: bounded(&error.to_string(), 1_920),
            },
        }
    }

    fn reason(&self) -> String {
        match self {
            Self::AdapterFailure { detail } => bounded(
                &format!("peer adapter failed after durable entry: {detail}"),
                2_048,
            ),
            Self::MissingTerminalEvidence => {
                "peer adapter returned without terminal evidence".to_owned()
            }
            Self::AdapterPanicked => "peer adapter panicked after durable entry".to_owned(),
            Self::ServiceInterrupted { detail } => bounded(
                &format!("peer execution service failed after durable entry: {detail}"),
                2_048,
            ),
            Self::WorkerPanicked => "peer worker panicked after durable entry".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PeerRecoveryTransition {
    InspectClaim { post_entry: PeerUncertainty },
    CompletePreEntryCancellation,
    ReleasePreEntryClaim,
    MarkPostEntryUncertain(PeerUncertainty),
}

#[derive(Debug)]
pub(crate) struct PeerWorkerRecovery {
    owner_peer: PeerId,
    execution: PeerExecutionId,
    transition: PeerRecoveryTransition,
}

impl PeerWorkerRecovery {
    pub(crate) fn inspect(record: PeerExecutionRecord, post_entry: PeerUncertainty) -> Self {
        Self {
            owner_peer: record.owner_peer,
            execution: record.execution,
            transition: PeerRecoveryTransition::InspectClaim { post_entry },
        }
    }

    fn exact(record: PeerExecutionRecord, transition: PeerRecoveryTransition) -> Self {
        Self {
            owner_peer: record.owner_peer,
            execution: record.execution,
            transition,
        }
    }
}

pub(crate) enum PeerWorkerRun {
    Settled,
    Recover(PeerWorkerRecovery),
}

enum ClaimedRunFailure {
    Inspect(PeerUncertainty),
    Exact(PeerRecoveryTransition),
}

impl From<PeerHttpError> for ClaimedRunFailure {
    fn from(error: PeerHttpError) -> Self {
        Self::Inspect(PeerUncertainty::ServiceInterrupted {
            detail: bounded(&error.to_string(), 1_920),
        })
    }
}

impl PeerService {
    pub(crate) fn worker_claims_enabled(&self) -> bool {
        self.drain.load(Ordering::SeqCst) == 0
    }

    pub(crate) fn claim_for_worker(
        &self,
        worker: &WorkerId,
    ) -> Result<PeerClaimOutcome, PeerHttpError> {
        let now = self.now()?;
        self.executions
            .claim_peer_dispatch(&PeerDispatchClaimRequest {
                worker,
                claimed_at_unix_ms: now,
                lease_expires_at_unix_ms: now.saturating_add(self.config.lease.execution_lease_ms),
            })
            .map_err(map_execution_persistence)
    }

    pub(crate) fn run_claimed(&self, record: PeerExecutionRecord) -> PeerWorkerRun {
        match self.run_claimed_once(&record) {
            Ok(()) => PeerWorkerRun::Settled,
            Err(ClaimedRunFailure::Inspect(post_entry)) => {
                PeerWorkerRun::Recover(PeerWorkerRecovery::inspect(record, post_entry))
            }
            Err(ClaimedRunFailure::Exact(transition)) => {
                PeerWorkerRun::Recover(PeerWorkerRecovery::exact(record, transition))
            }
        }
    }

    fn run_claimed_once(&self, record: &PeerExecutionRecord) -> Result<(), ClaimedRunFailure> {
        let claim = record.phase.claim().cloned().ok_or_else(|| {
            PeerHttpError::Persistence("claimed peer work lacks a claim".to_owned())
        })?;
        if matches!(
            &record.phase,
            PeerExecutionPhase::CancellationRequested { evidence: None, .. }
        ) {
            self.complete_pre_entry_cancellation(record)
                .map_err(|_error| {
                    ClaimedRunFailure::Exact(PeerRecoveryTransition::CompletePreEntryCancellation)
                })?;
            return Ok(());
        }
        if self.now()? > record.request.deadline_unix_ms {
            return self
                .append_pre_entry_failure(
                    record,
                    "peer execution deadline elapsed before adapter entry",
                )
                .map_err(ClaimedRunFailure::from);
        }
        if self.drain_state() != DrainState::Ready {
            return self
                .append_pre_entry_failure(
                    record,
                    "peer service stopped admission before adapter entry",
                )
                .map_err(ClaimedRunFailure::from);
        }
        let relationship = match self.relationship(&record.owner_peer) {
            Ok(relationship) => relationship,
            Err(error @ PeerHttpError::Unavailable(_)) => return Err(error.into()),
            Err(_) => {
                return self
                    .append_pre_entry_failure(
                        record,
                        "peer relationship was revoked or expired before adapter entry",
                    )
                    .map_err(ClaimedRunFailure::from);
            }
        };
        let generation = match self.exact_generation(&relationship, &record.request) {
            Ok(generation) => generation,
            Err(_) => {
                return self
                    .append_pre_entry_failure(
                        record,
                        "selected capability generation was unavailable before adapter entry",
                    )
                    .map_err(ClaimedRunFailure::from);
            }
        };
        let entry_now = self.now()?;
        let entry_authority = match self.authorize_invocation(
            &relationship,
            &record.request,
            &generation.descriptor,
            &generation.authority_requirements,
            entry_now,
        ) {
            Ok(decision) => decision,
            Err(error @ PeerHttpError::Unavailable(_)) => return Err(error.into()),
            Err(_) => {
                return self
                    .append_pre_entry_failure(
                        record,
                        "peer execution authority was denied before adapter entry",
                    )
                    .map_err(ClaimedRunFailure::from);
            }
        };
        let entered = match self
            .executions
            .mark_peer_entered(&PeerEntryRequest {
                owner: &record.owner_peer,
                execution: &record.execution,
                worker: &claim.worker,
                claim_generation: claim.generation,
                relationship_generation: relationship_generation(&relationship),
                entered_at_unix_ms: entry_now,
                authority: &entry_authority,
            })
            .map_err(map_execution_persistence)?
        {
            PeerEntryOutcome::Entered(entered) => *entered,
            PeerEntryOutcome::AdmissionClosed => {
                return self
                    .append_pre_entry_failure(
                        record,
                        "peer service stopped admission before adapter entry",
                    )
                    .map_err(ClaimedRunFailure::from);
            }
            PeerEntryOutcome::RelationshipUnavailable => {
                return self
                    .append_pre_entry_failure(
                        record,
                        "peer relationship was revoked or expired before adapter entry",
                    )
                    .map_err(ClaimedRunFailure::from);
            }
        };
        let reporter = PeerStoreReporter {
            owner_peer: entered.owner_peer.clone(),
            execution: entered.execution.clone(),
            executions: self.executions.clone(),
            clock: self.clock.clone(),
            lease_ms: self.config.lease.execution_lease_ms,
            limits: entered.request.limits,
            input_artifact_bytes: entered
                .request
                .input_artifact_bytes()
                .map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            deadline_unix_ms: entered.request.deadline_unix_ms,
            worker: claim.worker.clone(),
            claim_generation: claim.generation,
        };
        let context = adapter_execution_context(&entered.request)?;
        let result = self.capability_host.execute_exact_with_context(
            &entered.request.selection,
            &entered.request.request,
            &context,
            &reporter,
        );
        let uncertainty = PeerUncertainty::from_execution(&result);
        let recovery = PeerRecoveryTransition::MarkPostEntryUncertain(uncertainty.clone());
        let current = self
            .executions
            .peer_execution(&entered.owner_peer, &entered.execution)
            .map_err(map_execution_persistence)
            .map_err(|_| ClaimedRunFailure::Exact(recovery.clone()))?
            .ok_or_else(|| ClaimedRunFailure::Exact(recovery.clone()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Ok(());
        };
        if matches!(
            current.phase,
            PeerExecutionPhase::Terminal { .. } | PeerExecutionPhase::Uncertain { .. }
        ) {
            return Ok(());
        }
        let reason = uncertainty.reason();
        let uncertain_at_unix_ms = self
            .now()
            .map_err(|_| ClaimedRunFailure::Exact(recovery.clone()))?;
        self.executions
            .mark_peer_uncertain(
                &entered.owner_peer,
                &entered.execution,
                &claim.worker,
                claim.generation,
                uncertain_at_unix_ms,
                &reason,
            )
            .map_err(map_execution_persistence)
            .map_err(|_| ClaimedRunFailure::Exact(recovery))?;
        Ok(())
    }

    pub(crate) fn recover_worker(
        &self,
        recovery: &mut PeerWorkerRecovery,
        worker: &WorkerId,
    ) -> Result<(), PeerHttpError> {
        let current = self
            .executions
            .peer_execution(&recovery.owner_peer, &recovery.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Ok(());
        };
        if matches!(
            recovery.transition,
            PeerRecoveryTransition::CompletePreEntryCancellation
        ) {
            return self.complete_pre_entry_cancellation(&current);
        }
        let Some(claim) = current.phase.claim() else {
            return Ok(());
        };
        if claim.worker != *worker {
            return Ok(());
        }
        if let PeerRecoveryTransition::InspectClaim { post_entry } = &recovery.transition {
            recovery.transition = if current.phase.entry_evidence().is_some() {
                PeerRecoveryTransition::MarkPostEntryUncertain(post_entry.clone())
            } else {
                PeerRecoveryTransition::ReleasePreEntryClaim
            };
        }
        match &recovery.transition {
            PeerRecoveryTransition::InspectClaim { .. } => unreachable!("recovery is resolved"),
            PeerRecoveryTransition::CompletePreEntryCancellation => {
                unreachable!("pre-entry cancellation recovery returned above")
            }
            PeerRecoveryTransition::ReleasePreEntryClaim => {
                if current.phase.entry_evidence().is_some() {
                    return Err(PeerHttpError::Persistence(
                        "pre-entry recovery found durable adapter entry".to_owned(),
                    ));
                }
                self.executions
                    .release_peer_claim(
                        &current.owner_peer,
                        &current.execution,
                        worker,
                        claim.generation,
                        self.now()?,
                    )
                    .map_err(map_execution_persistence)?;
                self.notify_workers();
            }
            PeerRecoveryTransition::MarkPostEntryUncertain(uncertainty) => {
                if current.phase.entry_evidence().is_none() {
                    return Err(PeerHttpError::Persistence(
                        "post-entry recovery lacks durable adapter entry".to_owned(),
                    ));
                }
                self.executions
                    .mark_peer_uncertain(
                        &current.owner_peer,
                        &current.execution,
                        worker,
                        claim.generation,
                        self.now()?,
                        &uncertainty.reason(),
                    )
                    .map_err(map_execution_persistence)?;
            }
        }
        Ok(())
    }

    fn complete_pre_entry_cancellation(
        &self,
        record: &PeerExecutionRecord,
    ) -> Result<(), PeerHttpError> {
        let terminal = self.append_cancelled_before_entry(record)?;
        let cancellation = record.cancellation.as_ref().ok_or_else(|| {
            PeerHttpError::Persistence("cancellation facts disappeared".to_owned())
        })?;
        if cancellation.acknowledgement.is_some() {
            return Ok(());
        }
        let terminal_fact = terminal.event.kind().terminal().ok_or_else(|| {
            PeerHttpError::Persistence("pre-entry cancellation evidence is not terminal".to_owned())
        })?;
        if terminal_fact.status() != TerminalStatus::Cancelled
            || terminal_fact.side_effect() != milkdrift_capability::SideEffectClass::None
        {
            return Err(PeerHttpError::Persistence(
                "pre-entry cancellation evidence has incompatible terminal semantics".to_owned(),
            ));
        }
        self.executions
            .acknowledge_peer_cancellation(
                &record.owner_peer,
                &PeerCancellationAcknowledgement {
                    request_id: cancellation.request.request_id.clone(),
                    execution: record.execution.clone(),
                    disposition: CancellationDisposition::Accepted,
                    terminal_boundary: true,
                    terminal_evidence: Some(terminal),
                    detail: Some("durable cancellation prevented adapter entry".to_owned()),
                },
                self.now()?,
            )
            .map_err(map_execution_persistence)?;
        Ok(())
    }

    fn append_pre_entry_failure(
        &self,
        record: &PeerExecutionRecord,
        reason: &str,
    ) -> Result<(), PeerHttpError> {
        let sequence = record.last_observation_sequence.saturating_add(1);
        let failure = InvocationFailure::new(
            ErrorClass::Adapter,
            false,
            "peer_host_failure_before_entry",
            bounded(reason, 2_048),
            None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let terminal = InvocationTerminal::new(
            TerminalStatus::Failure,
            Vec::new(),
            Some(failure),
            None,
            milkdrift_capability::SideEffectClass::None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let event = InvocationEvent::new(
            record.request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        self.executions
            .append_peer_observation(
                &record.owner_peer,
                &record.execution,
                &PeerObservation {
                    execution: record.execution.clone(),
                    sequence,
                    category: ObservationCategory::Terminal,
                    event,
                    observed_at_unix_ms: self.now()?,
                },
            )
            .map_err(map_execution_persistence)?;
        Ok(())
    }

    pub(super) fn append_cancelled_before_entry(
        &self,
        record: &PeerExecutionRecord,
    ) -> Result<PeerObservation, PeerHttpError> {
        let current = self
            .executions
            .peer_execution(&record.owner_peer, &record.execution)
            .map_err(map_execution_persistence)?
            .ok_or_else(|| PeerHttpError::NotFound("remote execution was not found".to_owned()))?;
        let PeerExecutionSnapshot::Hot(current) = current else {
            return Err(PeerHttpError::Persistence(
                "active cancellation unexpectedly resolved an archived execution".to_owned(),
            ));
        };
        if let PeerExecutionPhase::Terminal { sequence, .. } = current.phase {
            return self
                .terminal_observation(&record.owner_peer, &current)?
                .filter(|observation| observation.sequence == sequence)
                .ok_or_else(|| {
                    PeerHttpError::Persistence(
                        "terminal cancellation evidence is missing".to_owned(),
                    )
                });
        }
        let sequence = current.last_observation_sequence.saturating_add(1);
        let terminal = InvocationTerminal::new(
            TerminalStatus::Cancelled,
            Vec::new(),
            None,
            None,
            milkdrift_capability::SideEffectClass::None,
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let event = InvocationEvent::new(
            current.request.request.invocation().clone(),
            sequence,
            InvocationEventKind::Terminal { terminal },
        )
        .map_err(|error| PeerHttpError::Protocol(error.to_string()))?;
        let observation = PeerObservation {
            execution: current.execution.clone(),
            sequence,
            category: ObservationCategory::Terminal,
            event,
            observed_at_unix_ms: self.now()?,
        };
        self.executions
            .append_peer_observation(&current.owner_peer, &current.execution, &observation)
            .map_err(map_execution_persistence)?;
        Ok(observation)
    }

    pub(super) fn terminal_observation(
        &self,
        owner: &PeerId,
        record: &PeerExecutionRecord,
    ) -> Result<Option<PeerObservation>, PeerHttpError> {
        if record.last_observation_sequence == 0 {
            return Ok(None);
        }
        let page = self
            .executions
            .peer_observations(
                owner,
                &record.execution,
                record.last_observation_sequence.saturating_sub(1),
                PageSize::new(1).map_err(|error| PeerHttpError::Protocol(error.to_string()))?,
            )
            .map_err(map_execution_persistence)?;
        Ok(page
            .observations
            .into_iter()
            .next()
            .filter(|observation| observation.event.kind().terminal().is_some()))
    }

    pub(super) fn notify_workers(&self) {
        if let Ok(workers) = self.workers.lock()
            && let Some(workers) = workers.as_ref()
        {
            workers.notify();
        }
    }
}

struct PeerStoreReporter {
    owner_peer: PeerId,
    execution: PeerExecutionId,
    executions: Arc<dyn PeerExecutionStore>,
    clock: Arc<dyn PeerClock>,
    lease_ms: u64,
    limits: milkdrift_peer_protocol::ExecutionLimits,
    input_artifact_bytes: u64,
    deadline_unix_ms: u64,
    worker: WorkerId,
    claim_generation: u64,
}

impl PeerStoreReporter {
    fn now(&self) -> Result<u64, AdapterError> {
        self.clock.now_unix_ms().map_err(|error| {
            AdapterError::external_failure(format!("peer boundary clock unavailable: {error}"))
        })
    }

    fn reject_report(&self, code: &str, detail: &str) -> AdapterError {
        let reason = bounded(&format!("{code}: {detail}"), 2_048);
        let observed_at_unix_ms = match self.now() {
            Ok(now) => now,
            Err(error) => return error,
        };
        let _ = self.executions.mark_peer_uncertain(
            &self.owner_peer,
            &self.execution,
            &self.worker,
            self.claim_generation,
            observed_at_unix_ms,
            &reason,
        );
        AdapterError::external_failure(reason)
    }
}

impl AdapterReporter for PeerStoreReporter {
    fn invocation(&self, event: InvocationEvent) -> Result<(), AdapterError> {
        let now = self.now()?;
        if now > self.deadline_unix_ms {
            return Err(
                self.reject_report("peer_report_deadline", "peer execution deadline elapsed")
            );
        }
        let maximum = u64::from(self.limits.observations);
        if event.sequence() > maximum
            || (event.sequence() == maximum && event.kind().terminal().is_none())
        {
            return Err(self.reject_report(
                "peer_report_observation_quota",
                "peer observation quota reached before terminal evidence",
            ));
        }
        if let InvocationEventKind::Terminal { terminal } = event.kind() {
            if terminal.usage().is_some_and(|usage| {
                usage
                    .duration_ms()
                    .is_some_and(|duration| duration > self.limits.duration_ms)
                    || usage
                        .cost_micros()
                        .is_some_and(|cost| cost > self.limits.cost_micros)
            }) {
                return Err(self.reject_report(
                    "peer_report_usage_quota",
                    "peer terminal usage exceeds the accepted duration or cost quota",
                ));
            }
            let output_bytes = terminal
                .outputs()
                .iter()
                .try_fold(self.input_artifact_bytes, |total, output| {
                    output.size_bytes().and_then(|size| total.checked_add(size))
                });
            if output_bytes.is_none_or(|bytes| bytes > self.limits.artifact_bytes) {
                return Err(self.reject_report(
                    "peer_report_artifact_quota",
                    "peer output artifact bytes are absent or exceed the accepted quota",
                ));
            }
        }
        let category = match event.kind() {
            InvocationEventKind::Progress { .. } => ObservationCategory::Progress,
            InvocationEventKind::Output { .. } => ObservationCategory::Artifact,
            InvocationEventKind::Terminal { terminal }
                if terminal.status() == TerminalStatus::Uncertain =>
            {
                ObservationCategory::Uncertainty
            }
            InvocationEventKind::Terminal { .. } => ObservationCategory::Terminal,
        };
        self.executions
            .append_peer_observation(
                &self.owner_peer,
                &self.execution,
                &PeerObservation {
                    execution: self.execution.clone(),
                    sequence: event.sequence(),
                    category,
                    event,
                    observed_at_unix_ms: now,
                },
            )
            .map(|_outcome| ())
            .map_err(|error| {
                self.reject_report("peer_report_rejected", &bounded(&error.to_string(), 1_900))
            })
    }

    fn heartbeat(&self) -> Result<(), AdapterError> {
        let now = self.now()?;
        if now > self.deadline_unix_ms {
            return Err(
                self.reject_report("peer_heartbeat_deadline", "peer execution deadline elapsed")
            );
        }
        self.executions
            .extend_peer_claim(
                &self.owner_peer,
                &self.execution,
                &self.worker,
                self.claim_generation,
                now.saturating_add(self.lease_ms),
            )
            .map_err(|error| AdapterError::external_failure(error.to_string()))
    }
}
