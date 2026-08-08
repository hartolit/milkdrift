//! Request-local E1 generation observation and bounded polling.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationEvent, ApplicationOutputRecordKind, ApplicationOutputState, ApplicationRuntime,
    GenerationTerminal, GenerationTerminalKind, GenerationTerminalOutcome,
};
use domain_contracts::{FinishReason, GenerationUsage, RequestId};

use super::validation::{self, GenerationExpectation};
use crate::error::{BenchmarkError, BenchmarkResult};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const GENERATION_TIMEOUT: Duration = Duration::from_mins(10);

pub(super) struct GenerationEvidence {
    pub(super) terminal: GenerationTerminal,
    pub(super) finish_reason: FinishReason,
    pub(super) usage: GenerationUsage,
    pub(super) decoded_byte_count: u64,
    pub(super) normal_timings: NormalTimings,
    pub(super) cancellation_timings: Option<CancellationTimings>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NormalTimings {
    pub(super) started: Duration,
    pub(super) first_decoded: Duration,
    pub(super) terminal_event: Duration,
    pub(super) release: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CancellationTimings {
    pub(super) generation_submission_to_cancellation: Duration,
    pub(super) cancellation_submission_to_acknowledgement: Duration,
    pub(super) cancellation_submission_to_terminal_output: Duration,
    pub(super) cancellation_submission_to_terminal_event: Duration,
    pub(super) cancellation_submission_to_release: Duration,
}

#[derive(Clone, Copy)]
struct Timed<T> {
    value: T,
    observed_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputPhase {
    Active,
    Terminal,
    Released,
}

struct GenerationObserver {
    request_id: RequestId,
    expectation: GenerationExpectation,
    submitted_at: Instant,
    output_phase: OutputPhase,
    started_at: Option<Instant>,
    first_decoded_at: Option<Instant>,
    cancellation_submitted_at: Option<Instant>,
    cancellation_acknowledged_at: Option<Instant>,
    terminal_output: Option<Timed<GenerationTerminalKind>>,
    released_output: Option<Timed<GenerationTerminalKind>>,
    terminal_event: Option<Timed<GenerationTerminal>>,
    decoded_byte_count: u64,
}

impl GenerationObserver {
    fn new(
        request_id: RequestId,
        expectation: GenerationExpectation,
        submitted_at: Instant,
    ) -> Self {
        Self {
            request_id,
            expectation,
            submitted_at,
            output_phase: OutputPhase::Active,
            started_at: None,
            first_decoded_at: None,
            cancellation_submitted_at: None,
            cancellation_acknowledged_at: None,
            terminal_output: None,
            released_output: None,
            terminal_event: None,
            decoded_byte_count: 0,
        }
    }

    fn pull(&mut self, runtime: &mut ApplicationRuntime) -> BenchmarkResult {
        runtime
            .pull_output(|batch| {
                for record in batch.records() {
                    let fragment = match record.kind {
                        ApplicationOutputRecordKind::Text(_) => batch.text_for(record),
                        ApplicationOutputRecordKind::State(_) => None,
                    };
                    self.observe_output_at(
                        record.request_id,
                        record.kind,
                        fragment,
                        Instant::now(),
                    )?;
                }
                Ok(())
            })
            .map_err(|error| {
                BenchmarkError::new(format!(
                    "bounded decoded application output could not be pulled: {error}"
                ))
            })?
    }

    fn observe_output_at(
        &mut self,
        request_id: RequestId,
        kind: ApplicationOutputRecordKind,
        fragment: Option<&str>,
        observed_at: Instant,
    ) -> BenchmarkResult {
        self.require_request_id(request_id, "generation output")?;
        match kind {
            ApplicationOutputRecordKind::Text(_) => {
                self.require_active_output("decoded text")?;
                let fragment = fragment.ok_or_else(|| {
                    BenchmarkError::new("decoded output contained an invalid UTF-8 text range")
                })?;
                if !fragment.is_empty() {
                    let bytes = u64::try_from(fragment.len()).map_err(|_| {
                        BenchmarkError::new("decoded fragment length conversion failed")
                    })?;
                    self.decoded_byte_count = self
                        .decoded_byte_count
                        .checked_add(bytes)
                        .ok_or_else(|| BenchmarkError::new("decoded byte count overflowed"))?;
                    if self.first_decoded_at.is_none() {
                        self.first_decoded_at = Some(observed_at);
                    }
                }
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::Yielded(_)) => {
                self.require_active_output("yielded state")?;
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(kind)) => {
                self.require_active_output("terminal state")?;
                if self.terminal_output.is_some() {
                    return Err(BenchmarkError::new(
                        "generation published more than one terminal output state",
                    ));
                }
                self.terminal_output = Some(Timed {
                    value: kind,
                    observed_at,
                });
                self.output_phase = OutputPhase::Terminal;
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::CleanupPending) => {
                return Err(validation::output_cleanup_pending_error());
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::CleanupExhausted) => {
                return Err(validation::output_cleanup_exhausted_error());
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::Released(kind)) => {
                if self.output_phase != OutputPhase::Terminal {
                    return Err(BenchmarkError::new(
                        "generation published Released before exactly one Terminal output state or after release",
                    ));
                }
                let terminal_kind = self
                    .terminal_output
                    .as_ref()
                    .map(|observation| observation.value)
                    .ok_or_else(|| {
                        BenchmarkError::new(
                            "generation output phase reached Terminal without terminal evidence",
                        )
                    })?;
                if kind != terminal_kind {
                    return Err(BenchmarkError::new(format!(
                        "generation Released outcome {kind:?} did not match prior Terminal outcome {terminal_kind:?}"
                    )));
                }
                self.released_output = Some(Timed {
                    value: kind,
                    observed_at,
                });
                self.output_phase = OutputPhase::Released;
            }
        }
        Ok(())
    }

    fn observe_event_at(
        &mut self,
        event: ApplicationEvent,
        observed_at: Instant,
    ) -> BenchmarkResult {
        match event {
            ApplicationEvent::GenerationStarted { request_id } => {
                self.require_request_id(request_id, "GenerationStarted")?;
                if self.started_at.replace(observed_at).is_some() {
                    return Err(BenchmarkError::new(
                        "generation published more than one matching GenerationStarted event",
                    ));
                }
                Ok(())
            }
            ApplicationEvent::GenerationCancellationRequested { request_id } => {
                self.require_request_id(request_id, "GenerationCancellationRequested")?;
                if !self.expectation.requires_cancellation() {
                    return Err(BenchmarkError::new(
                        "normal generation unexpectedly acknowledged cancellation",
                    ));
                }
                if self.cancellation_submitted_at.is_none() {
                    return Err(BenchmarkError::new(
                        "generation acknowledged cancellation before the E1 request was submitted",
                    ));
                }
                if self
                    .cancellation_acknowledged_at
                    .replace(observed_at)
                    .is_some()
                {
                    return Err(BenchmarkError::new(
                        "generation published more than one matching cancellation acknowledgement",
                    ));
                }
                Ok(())
            }
            ApplicationEvent::GenerationCancellationFailed {
                request_id,
                failure,
            } => {
                self.require_request_id(request_id, "GenerationCancellationFailed")?;
                Err(BenchmarkError::new(format!(
                    "UserRequested generation cancellation failed: {failure}"
                )))
            }
            ApplicationEvent::GenerationCleanupPending {
                request_id,
                exhausted,
                failure,
            } => {
                self.require_request_id(request_id, "GenerationCleanupPending")?;
                Err(validation::generation_cleanup_pending_error(
                    exhausted, &failure,
                ))
            }
            ApplicationEvent::GenerationFinished { terminal } => {
                self.require_request_id(terminal.request_id, "GenerationFinished")?;
                if self
                    .terminal_event
                    .replace(Timed {
                        value: terminal,
                        observed_at,
                    })
                    .is_some()
                {
                    return Err(BenchmarkError::new(
                        "generation published more than one matching GenerationFinished event",
                    ));
                }
                Ok(())
            }
            ApplicationEvent::ModelCleanupPending { exhausted, failure } => {
                Err(BenchmarkError::new(format!(
                    "model cleanup retained E0 ownership during generation observation (exhausted={exhausted}): {failure}"
                )))
            }
            ApplicationEvent::HubDisconnected => Err(BenchmarkError::new(
                "Hub worker disconnected during generation",
            )),
            ApplicationEvent::RuntimeDisconnected => Err(BenchmarkError::new(
                "inference worker disconnected during generation",
            )),
            unexpected => Err(BenchmarkError::new(format!(
                "unexpected application event during generation: {unexpected:?}"
            ))),
        }
    }

    fn require_active_output(&self, record: &'static str) -> BenchmarkResult {
        if self.output_phase != OutputPhase::Active {
            return Err(BenchmarkError::new(format!(
                "generation published {record} after terminal output progression reached {:?}",
                self.output_phase
            )));
        }
        Ok(())
    }

    fn require_request_id(&self, observed: RequestId, source: &'static str) -> BenchmarkResult {
        if observed != self.request_id {
            return Err(BenchmarkError::new(format!(
                "{source} addressed request {}, expected {}",
                observed.get(),
                self.request_id.get()
            )));
        }
        Ok(())
    }

    fn cancellation_was_preempted(&self) -> bool {
        self.expectation.requires_cancellation()
            && self.cancellation_submitted_at.is_none()
            && (self.terminal_output.is_some()
                || self.released_output.is_some()
                || self.terminal_event.is_some())
    }

    fn ready_to_request_cancellation(&self) -> bool {
        self.expectation.requires_cancellation()
            && self.cancellation_submitted_at.is_none()
            && self.started_at.is_some()
            && self.first_decoded_at.is_some()
            && !self.cancellation_was_preempted()
    }

    fn record_cancellation_submission(&mut self, submitted_at: Instant) -> BenchmarkResult {
        if !self.ready_to_request_cancellation() {
            return Err(BenchmarkError::new(
                "cancellation was not requested after both GenerationStarted and decoded progress",
            ));
        }
        elapsed_since(
            self.submitted_at,
            Some(submitted_at),
            "cancellation submission",
        )?;
        self.cancellation_submitted_at = Some(submitted_at);
        Ok(())
    }

    fn has_all_terminal_facts(&self) -> bool {
        self.started_at.is_some()
            && self.terminal_output.is_some()
            && self.released_output.is_some()
            && self.terminal_event.is_some()
            && (!self.expectation.requires_cancellation()
                || (self.cancellation_submitted_at.is_some()
                    && self.cancellation_acknowledged_at.is_some()))
    }

    fn finish(self) -> BenchmarkResult<GenerationEvidence> {
        let terminal_observation = self.terminal_event.as_ref().ok_or_else(|| {
            BenchmarkError::new("matching GenerationFinished event was not observed")
        })?;
        let terminal_observed_at = terminal_observation.observed_at;
        let terminal = terminal_observation.value.clone();
        let event_kind = validation::terminal_kind_for_outcome(&terminal.outcome);
        validation::validate_terminal_consistency(
            self.terminal_output.map(|observation| observation.value),
            self.released_output.map(|observation| observation.value),
            event_kind,
        )?;

        let finish_reason = match &terminal.outcome {
            GenerationTerminalOutcome::Finished(reason) => *reason,
            GenerationTerminalOutcome::Failed(failure) => {
                return Err(BenchmarkError::new(format!(
                    "generation terminal event reported failure: {failure}"
                )));
            }
        };
        validation::validate_nonempty_generation(terminal.usage, self.decoded_byte_count)?;
        validation::validate_expected_outcome(self.expectation, finish_reason, terminal.usage)?;

        let normal_timings = normal_timings(
            self.submitted_at,
            NormalObservationTimes {
                started: self.started_at,
                first_decoded: self.first_decoded_at,
                terminal_event: Some(terminal_observed_at),
                release: self
                    .released_output
                    .map(|observation| observation.observed_at),
            },
        )?;
        let cancellation_timings = if self.expectation.requires_cancellation() {
            Some(cancellation_timings(
                self.submitted_at,
                CancellationObservationTimes {
                    submitted: self.cancellation_submitted_at,
                    acknowledged: self.cancellation_acknowledged_at,
                    terminal_output: self
                        .terminal_output
                        .map(|observation| observation.observed_at),
                    terminal_event: Some(terminal_observed_at),
                    release: self
                        .released_output
                        .map(|observation| observation.observed_at),
                },
            )?)
        } else {
            if self.cancellation_submitted_at.is_some()
                || self.cancellation_acknowledged_at.is_some()
            {
                return Err(BenchmarkError::new(
                    "normal generation retained unexpected cancellation timing facts",
                ));
            }
            None
        };

        Ok(GenerationEvidence {
            usage: terminal.usage,
            terminal,
            finish_reason,
            decoded_byte_count: self.decoded_byte_count,
            normal_timings,
            cancellation_timings,
        })
    }
}

pub(super) fn drive_generation(
    runtime: &mut ApplicationRuntime,
    request_id: RequestId,
    submitted_at: Instant,
    expectation: GenerationExpectation,
    before_cancellation_checkpoint: Option<&'static str>,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<GenerationEvidence> {
    if expectation.requires_cancellation() != before_cancellation_checkpoint.is_some() {
        return Err(BenchmarkError::new(
            "generation driver received an inconsistent cancellation plan",
        ));
    }

    let deadline = checked_deadline(GENERATION_TIMEOUT, "generation terminal release")?;
    let mut observer = GenerationObserver::new(request_id, expectation, submitted_at);
    loop {
        if let Some(event) = runtime.poll_event() {
            observer.observe_event_at(event, Instant::now())?;
        }
        observer.pull(runtime)?;

        if observer.cancellation_was_preempted() {
            return Err(BenchmarkError::new(
                "generation reached a terminal fact before progress-triggered cancellation could be submitted",
            ));
        }
        if observer.ready_to_request_cancellation() {
            let checkpoint = before_cancellation_checkpoint.ok_or_else(|| {
                BenchmarkError::new("cancellation checkpoint label disappeared before submission")
            })?;
            observe(checkpoint)?;
            let cancellation_submitted_at = Instant::now();
            runtime.cancel_generation(request_id).map_err(|error| {
                BenchmarkError::new(format!(
                    "UserRequested cancellation could not be submitted after generation progress: {error}"
                ))
            })?;
            observer.record_cancellation_submission(cancellation_submitted_at)?;
        }

        if observer.has_all_terminal_facts() {
            let evidence = observer.finish()?;
            validation::validate_released_runtime(runtime, &evidence.terminal)?;
            return Ok(evidence);
        }
        wait_for_next_poll(deadline, "generation terminal release")?;
    }
}

#[derive(Clone, Copy)]
struct NormalObservationTimes {
    started: Option<Instant>,
    first_decoded: Option<Instant>,
    terminal_event: Option<Instant>,
    release: Option<Instant>,
}

fn normal_timings(
    submitted_at: Instant,
    observations: NormalObservationTimes,
) -> BenchmarkResult<NormalTimings> {
    Ok(NormalTimings {
        started: elapsed_since(submitted_at, observations.started, "GenerationStarted")?,
        first_decoded: elapsed_since(
            submitted_at,
            observations.first_decoded,
            "first non-empty decoded output",
        )?,
        terminal_event: elapsed_since(
            submitted_at,
            observations.terminal_event,
            "GenerationFinished event",
        )?,
        release: elapsed_since(submitted_at, observations.release, "Released output state")?,
    })
}

#[derive(Clone, Copy)]
struct CancellationObservationTimes {
    submitted: Option<Instant>,
    acknowledged: Option<Instant>,
    terminal_output: Option<Instant>,
    terminal_event: Option<Instant>,
    release: Option<Instant>,
}

fn cancellation_timings(
    generation_submitted_at: Instant,
    observations: CancellationObservationTimes,
) -> BenchmarkResult<CancellationTimings> {
    let cancellation_submitted_at = observations.submitted.ok_or_else(|| {
        BenchmarkError::new("UserRequested cancellation submission was not observed")
    })?;
    Ok(CancellationTimings {
        generation_submission_to_cancellation: elapsed_since(
            generation_submitted_at,
            Some(cancellation_submitted_at),
            "cancellation submission",
        )?,
        cancellation_submission_to_acknowledgement: elapsed_since(
            cancellation_submitted_at,
            observations.acknowledged,
            "GenerationCancellationRequested acknowledgement",
        )?,
        cancellation_submission_to_terminal_output: elapsed_since(
            cancellation_submitted_at,
            observations.terminal_output,
            "Terminal output state after cancellation",
        )?,
        cancellation_submission_to_terminal_event: elapsed_since(
            cancellation_submitted_at,
            observations.terminal_event,
            "GenerationFinished event after cancellation",
        )?,
        cancellation_submission_to_release: elapsed_since(
            cancellation_submitted_at,
            observations.release,
            "Released output state after cancellation",
        )?,
    })
}

fn checked_deadline(timeout: Duration, operation: &'static str) -> BenchmarkResult<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        BenchmarkError::new(format!(
            "deadline overflow while preparing to wait for {operation}"
        ))
    })
}

fn wait_for_next_poll(deadline: Instant, operation: &'static str) -> BenchmarkResult {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| BenchmarkError::new(format!("timed out waiting for {operation}")))?;
    std::thread::sleep(POLL_INTERVAL.min(remaining));
    Ok(())
}

fn elapsed_since(
    submitted_at: Instant,
    observed_at: Option<Instant>,
    label: &'static str,
) -> BenchmarkResult<Duration> {
    observed_at
        .ok_or_else(|| BenchmarkError::new(format!("{label} was not observed")))?
        .checked_duration_since(submitted_at)
        .ok_or_else(|| BenchmarkError::new(format!("{label} preceded request submission")))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use application_runtime::{
        ApplicationEvent, ApplicationOutputRecordKind, ApplicationOutputState,
        ApplicationTextRange, GenerationTerminal, GenerationTerminalKind,
        GenerationTerminalOutcome,
    };
    use domain_contracts::{CancellationReason, FinishReason, GenerationUsage, RequestId};

    use super::{GenerationObserver, NormalObservationTimes, normal_timings};
    use crate::external::generation::validation::GenerationExpectation;

    fn after(origin: Instant, milliseconds: u64) -> Result<Instant, String> {
        origin
            .checked_add(Duration::from_millis(milliseconds))
            .ok_or_else(|| "test instant overflowed".to_owned())
    }

    fn terminal_event(
        request_id: RequestId,
        reason: FinishReason,
        generated_tokens: u64,
    ) -> ApplicationEvent {
        ApplicationEvent::GenerationFinished {
            terminal: GenerationTerminal {
                request_id,
                outcome: GenerationTerminalOutcome::Finished(reason),
                usage: GenerationUsage {
                    prompt_tokens: 12,
                    generated_tokens,
                },
            },
        }
    }

    fn text_record(length: usize) -> ApplicationOutputRecordKind {
        ApplicationOutputRecordKind::Text(ApplicationTextRange { start: 0, length })
    }

    #[test]
    fn valid_normal_transition_collects_request_local_evidence() -> Result<(), String> {
        let submitted_at = Instant::now();
        let request_id = RequestId::new(1);
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let mut observer = GenerationObserver::new(
            request_id,
            GenerationExpectation::direct_token_limit(32),
            submitted_at,
        );

        observer
            .observe_event_at(
                ApplicationEvent::GenerationStarted { request_id },
                after(submitted_at, 1)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                text_record(2),
                Some("ok"),
                after(submitted_at, 2)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(terminal)),
                None,
                after(submitted_at, 3)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_event_at(
                terminal_event(request_id, FinishReason::TokenLimit, 32),
                after(submitted_at, 4)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                None,
                after(submitted_at, 5)?,
            )
            .map_err(|error| error.to_string())?;

        assert!(observer.has_all_terminal_facts());
        let evidence = observer.finish().map_err(|error| error.to_string())?;
        assert_eq!(evidence.terminal.request_id, request_id);
        assert_eq!(evidence.decoded_byte_count, 2);
        assert_eq!(evidence.normal_timings.started, Duration::from_millis(1));
        assert_eq!(
            evidence.normal_timings.first_decoded,
            Duration::from_millis(2)
        );
        assert_eq!(
            evidence.normal_timings.terminal_event,
            Duration::from_millis(4)
        );
        assert_eq!(evidence.normal_timings.release, Duration::from_millis(5));
        assert_eq!(evidence.cancellation_timings, None);
        Ok(())
    }

    #[test]
    fn duplicate_terminal_output_is_rejected() -> Result<(), String> {
        let observed_at = Instant::now();
        let request_id = RequestId::new(1);
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let mut observer = GenerationObserver::new(
            request_id,
            GenerationExpectation::direct_token_limit(32),
            observed_at,
        );
        observer
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(terminal)),
                None,
                observed_at,
            )
            .map_err(|error| error.to_string())?;
        assert!(
            observer
                .observe_output_at(
                    request_id,
                    ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(terminal)),
                    None,
                    observed_at,
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn release_before_terminal_is_rejected() {
        let observed_at = Instant::now();
        let request_id = RequestId::new(1);
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let mut observer = GenerationObserver::new(
            request_id,
            GenerationExpectation::direct_token_limit(32),
            observed_at,
        );
        assert!(
            observer
                .observe_output_at(
                    request_id,
                    ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                    None,
                    observed_at,
                )
                .is_err()
        );
    }

    #[test]
    fn mismatched_request_identity_is_rejected_for_output_and_events() {
        let observed_at = Instant::now();
        let mut observer = GenerationObserver::new(
            RequestId::new(1),
            GenerationExpectation::direct_token_limit(32),
            observed_at,
        );
        assert!(
            observer
                .observe_output_at(RequestId::new(2), text_record(1), Some("x"), observed_at)
                .is_err()
        );
        assert!(
            observer
                .observe_event_at(
                    ApplicationEvent::GenerationStarted {
                        request_id: RequestId::new(2),
                    },
                    observed_at,
                )
                .is_err()
        );
    }

    #[test]
    fn cancellation_transitions_preserve_submission_origins_and_order() -> Result<(), String> {
        let submitted_at = Instant::now();
        let request_id = RequestId::new(7);

        let mut out_of_order = GenerationObserver::new(
            request_id,
            GenerationExpectation::cancellation(128),
            submitted_at,
        );
        assert!(
            out_of_order
                .observe_event_at(
                    ApplicationEvent::GenerationCancellationRequested { request_id },
                    after(submitted_at, 1)?,
                )
                .is_err()
        );

        let terminal_reason = FinishReason::Cancelled(CancellationReason::UserRequested);
        let terminal = GenerationTerminalKind::Finished(terminal_reason);
        let mut observer = GenerationObserver::new(
            request_id,
            GenerationExpectation::cancellation(128),
            submitted_at,
        );
        observer
            .observe_event_at(
                ApplicationEvent::GenerationStarted { request_id },
                after(submitted_at, 1)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                text_record(1),
                Some("x"),
                after(submitted_at, 2)?,
            )
            .map_err(|error| error.to_string())?;
        assert!(observer.ready_to_request_cancellation());
        observer
            .record_cancellation_submission(after(submitted_at, 3)?)
            .map_err(|error| error.to_string())?;
        observer
            .observe_event_at(
                ApplicationEvent::GenerationCancellationRequested { request_id },
                after(submitted_at, 4)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(terminal)),
                None,
                after(submitted_at, 5)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_event_at(
                terminal_event(request_id, terminal_reason, 7),
                after(submitted_at, 6)?,
            )
            .map_err(|error| error.to_string())?;
        observer
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                None,
                after(submitted_at, 7)?,
            )
            .map_err(|error| error.to_string())?;

        let evidence = observer.finish().map_err(|error| error.to_string())?;
        let timings = evidence
            .cancellation_timings
            .ok_or_else(|| "cancellation timings disappeared".to_owned())?;
        assert_eq!(
            timings.generation_submission_to_cancellation,
            Duration::from_millis(3)
        );
        assert_eq!(
            timings.cancellation_submission_to_acknowledgement,
            Duration::from_millis(1)
        );
        assert_eq!(
            timings.cancellation_submission_to_terminal_output,
            Duration::from_millis(2)
        );
        assert_eq!(
            timings.cancellation_submission_to_terminal_event,
            Duration::from_millis(3)
        );
        assert_eq!(
            timings.cancellation_submission_to_release,
            Duration::from_millis(4)
        );
        Ok(())
    }

    #[test]
    fn timing_helpers_reject_missing_or_pre_submission_observations() -> Result<(), String> {
        let submitted_at = Instant::now();
        let before_submission = submitted_at
            .checked_sub(Duration::from_millis(1))
            .ok_or_else(|| "test instant underflowed".to_owned())?;
        assert!(
            normal_timings(
                submitted_at,
                NormalObservationTimes {
                    started: Some(before_submission),
                    first_decoded: Some(submitted_at),
                    terminal_event: Some(submitted_at),
                    release: Some(submitted_at),
                },
            )
            .is_err()
        );
        assert!(
            normal_timings(
                submitted_at,
                NormalObservationTimes {
                    started: Some(submitted_at),
                    first_decoded: None,
                    terminal_event: Some(submitted_at),
                    release: Some(submitted_at),
                },
            )
            .is_err()
        );
        Ok(())
    }
}
