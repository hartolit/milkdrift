//! Authoritative public-E1 generation workloads for the external baseline.

use std::time::{Duration, Instant};

use application_runtime::{
    ApplicationActivity, ApplicationEvent, ApplicationOutputRecordKind, ApplicationOutputState,
    ApplicationRuntime, ConversationProvenance, ConversationRole, ConversationTokenEstimate,
    GenerationSeed, GenerationSettings, GenerationTerminal, GenerationTerminalKind,
    GenerationTerminalOutcome, LoadedModel, ResponseAttemptState,
};
use domain_contracts::{CancellationReason, FinishReason, GenerationUsage, RequestId};

use crate::error::{BenchmarkError, BenchmarkResult};
use crate::external::report::{
    CancellationResult, CancellationSubmissionTimings, ChatProofResult, ConversationProof,
    DirectCompletionResults, DirectCompletionSample, DirectCompletionSummary,
    DirectCompletionWarmupResult, GenerationOutcomeMatch, GenerationSubmissionTimings,
};
use crate::report::duration_ns;

pub(super) const CHAT_MESSAGE: &str =
    "Reply with one short sentence confirming that local inference is working.";
pub(super) const CHAT_MESSAGE_IDENTIFIER: &str = "tinyllama-local-inference-chat-proof-v1";
pub(super) const DIRECT_COMPLETION_PROMPT: &str =
    "The following is a concise explanation of deterministic resource cleanup in systems software:";
pub(super) const DIRECT_COMPLETION_PROMPT_IDENTIFIER: &str =
    "deterministic-resource-cleanup-completion-v1";
pub(super) const CHAT_MAXIMUM_NEW_TOKENS: u32 = 24;
pub(super) const DIRECT_MAXIMUM_NEW_TOKENS: u32 = 32;
pub(super) const CANCELLATION_MAXIMUM_NEW_TOKENS: u32 = 128;
pub(super) const WARMUP_COUNT: u32 = 1;
pub(super) const SAMPLE_COUNT: u32 = 3;
pub(super) const FIXED_SEED: u64 = 39;

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const GENERATION_TIMEOUT: Duration = Duration::from_mins(10);

const AFTER_CHAT_RELEASE_CHECKPOINT: &str = "after-chat-release";
const AFTER_WARMUP_RELEASE_CHECKPOINT: &str = "after-warmup-release";
const PRIMARY_DIRECT_RELEASE_CHECKPOINTS: [&str; 3] = [
    "after-direct-sample-1-release",
    "after-direct-sample-2-release",
    "after-direct-sample-3-release",
];
const AFTER_STABILITY_DIRECT_RELEASE_CHECKPOINT: &str = "after-stability-direct-release";
const BEFORE_CANCELLATION_REQUEST_CHECKPOINT: &str = "before-cancellation-request";
const AFTER_CANCELLATION_RELEASE_CHECKPOINT: &str = "after-cancellation-release";

pub(super) struct PrimaryWorkloadEvidence {
    pub(super) chat_compatibility: ChatProofResult,
    pub(super) direct_completion: DirectCompletionResults,
    pub(super) cancellation: CancellationResult,
}

pub(super) fn run_primary_workload(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<PrimaryWorkloadEvidence> {
    eprintln!("running the exact compatible-chat proof");
    let chat_compatibility = run_chat_proof(runtime, loaded, observe)?;
    observe(AFTER_CHAT_RELEASE_CHECKPOINT)?;

    eprintln!("running one controlled direct-completion warmup");
    let warmup_evidence = run_direct_completion(
        runtime,
        GenerationExpectation::DirectTokenLimit,
        None,
        observe,
    )?;
    let warmup = warmup_record(&warmup_evidence);
    observe(AFTER_WARMUP_RELEASE_CHECKPOINT)?;

    let expected_sample_count = usize::try_from(SAMPLE_COUNT)
        .map_err(|_| BenchmarkError::new("external sample count conversion to usize failed"))?;
    if PRIMARY_DIRECT_RELEASE_CHECKPOINTS.len() != expected_sample_count {
        return Err(BenchmarkError::new(
            "direct-completion checkpoint labels did not match the configured sample count",
        ));
    }

    eprintln!("running three sequential controlled direct-completion samples");
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(expected_sample_count)
        .map_err(|error| BenchmarkError::new(format!("sample allocation failed: {error}")))?;
    for (index, checkpoint) in PRIMARY_DIRECT_RELEASE_CHECKPOINTS
        .iter()
        .copied()
        .enumerate()
    {
        let ordinal = u32::try_from(index.saturating_add(1)).map_err(|_| {
            BenchmarkError::new("direct-completion sample ordinal conversion failed")
        })?;
        eprintln!("running direct-completion sample {ordinal} of {SAMPLE_COUNT}");
        let evidence = run_direct_completion(
            runtime,
            GenerationExpectation::DirectTokenLimit,
            None,
            observe,
        )?;
        let sample = sample_record(ordinal, &evidence)?;
        observe(checkpoint)?;
        samples.push(sample);
    }
    let summary = summarize_samples(&samples)?;

    eprintln!("running cancellation after observable generation progress");
    let cancellation = run_cancellation(runtime, observe)?;

    Ok(PrimaryWorkloadEvidence {
        chat_compatibility,
        direct_completion: DirectCompletionResults {
            warmup,
            samples,
            summary,
        },
        cancellation,
    })
}

pub(super) fn run_stability_workload(
    runtime: &mut ApplicationRuntime,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<(DirectCompletionSample, CancellationResult)> {
    eprintln!("running one exact direct completion for the stability cycle");
    let evidence = run_direct_completion(
        runtime,
        GenerationExpectation::DirectTokenLimit,
        None,
        observe,
    )?;
    let direct_completion = sample_record(1, &evidence)?;
    observe(AFTER_STABILITY_DIRECT_RELEASE_CHECKPOINT)?;

    eprintln!("running one cancellation for the stability cycle");
    let cancellation = run_cancellation(runtime, observe)?;
    Ok((direct_completion, cancellation))
}

fn run_chat_proof(
    runtime: &mut ApplicationRuntime,
    loaded: &LoadedModel,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<ChatProofResult> {
    validate_chat_ready(runtime, loaded)?;
    let submitted_at = Instant::now();
    let request_id = runtime
        .submit_user_message(CHAT_MESSAGE, generation_settings(CHAT_MAXIMUM_NEW_TOKENS))
        .map_err(|error| {
            BenchmarkError::new(format!(
                "exact compatible-chat request could not be submitted: {error}"
            ))
        })?;
    let evidence = drive_generation(
        runtime,
        request_id,
        submitted_at,
        GenerationExpectation::Chat,
        None,
        observe,
    )?;
    validate_chat_conversation(runtime, loaded, &evidence)?;
    clear_chat_conversation(runtime)?;

    Ok(ChatProofResult {
        submission_to_generation_started_ns: duration_ns(evidence.normal_timings.started),
        submission_to_first_decoded_output_ns: duration_ns(evidence.normal_timings.first_decoded),
        submission_to_terminal_event_ns: duration_ns(evidence.normal_timings.terminal_event),
        submission_to_release_ns: duration_ns(evidence.normal_timings.release),
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: evidence.terminal_kind,
        outcome_match: matched_outcomes(),
        conversation: ConversationProof {
            validated: true,
            cleared: true,
        },
    })
}

fn run_direct_completion(
    runtime: &mut ApplicationRuntime,
    expectation: GenerationExpectation,
    before_cancellation_checkpoint: Option<&'static str>,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<GenerationEvidence> {
    validate_direct_ready(runtime)?;
    let maximum_new_tokens = expectation.maximum_new_tokens();
    let submitted_at = Instant::now();
    let request_id = runtime
        .start_generation(
            DIRECT_COMPLETION_PROMPT,
            generation_settings(maximum_new_tokens),
        )
        .map_err(|error| {
            BenchmarkError::new(format!(
                "controlled direct completion could not be submitted: {error}"
            ))
        })?;
    let evidence = drive_generation(
        runtime,
        request_id,
        submitted_at,
        expectation,
        before_cancellation_checkpoint,
        observe,
    )?;
    validate_direct_conversation_state(runtime)?;
    Ok(evidence)
}

fn run_cancellation(
    runtime: &mut ApplicationRuntime,
    observe: &mut impl FnMut(&'static str) -> BenchmarkResult,
) -> BenchmarkResult<CancellationResult> {
    let evidence = run_direct_completion(
        runtime,
        GenerationExpectation::Cancellation,
        Some(BEFORE_CANCELLATION_REQUEST_CHECKPOINT),
        observe,
    )?;
    let result = cancellation_record(&evidence)?;
    observe(AFTER_CANCELLATION_RELEASE_CHECKPOINT)?;
    Ok(result)
}

fn generation_settings(maximum_new_tokens: u32) -> GenerationSettings {
    GenerationSettings {
        maximum_new_tokens,
        temperature: 1.0,
        top_k: 1,
        top_p: 1.0,
        min_p: 0.0,
        repetition_penalty: 1.0,
        repetition_window: 0,
        seed: GenerationSeed::Fixed(FIXED_SEED),
        eos_tokens: Vec::new(),
        stop_sequences: Vec::new(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationExpectation {
    Chat,
    DirectTokenLimit,
    Cancellation,
}

impl GenerationExpectation {
    const fn maximum_new_tokens(self) -> u32 {
        match self {
            Self::Chat => CHAT_MAXIMUM_NEW_TOKENS,
            Self::DirectTokenLimit => DIRECT_MAXIMUM_NEW_TOKENS,
            Self::Cancellation => CANCELLATION_MAXIMUM_NEW_TOKENS,
        }
    }

    const fn requires_cancellation(self) -> bool {
        matches!(self, Self::Cancellation)
    }
}

struct GenerationEvidence {
    finish_reason: FinishReason,
    terminal_kind: &'static str,
    usage: GenerationUsage,
    decoded_byte_count: u64,
    normal_timings: NormalTimings,
    cancellation_timings: Option<CancellationTimings>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NormalTimings {
    started: Duration,
    first_decoded: Duration,
    terminal_event: Duration,
    release: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CancellationTimings {
    generation_submission_to_cancellation: Duration,
    cancellation_submission_to_acknowledgement: Duration,
    cancellation_submission_to_terminal_output: Duration,
    cancellation_submission_to_terminal_event: Duration,
    cancellation_submission_to_release: Duration,
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
                return Err(BenchmarkError::new(
                    "generation output entered cleanup-pending state",
                ));
            }
            ApplicationOutputRecordKind::State(ApplicationOutputState::CleanupExhausted) => {
                return Err(BenchmarkError::new(
                    "generation output exhausted cleanup while retaining ownership",
                ));
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
                Err(BenchmarkError::new(format!(
                    "generation cleanup remained pending (exhausted={exhausted}): {failure}"
                )))
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

    fn finish(self, runtime: &ApplicationRuntime) -> BenchmarkResult<GenerationEvidence> {
        let terminal_observation = self.terminal_event.as_ref().ok_or_else(|| {
            BenchmarkError::new("matching GenerationFinished event was not observed")
        })?;
        let terminal = &terminal_observation.value;
        let event_kind = terminal_kind_for_outcome(&terminal.outcome);
        validate_terminal_consistency(
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
        if terminal.usage.prompt_tokens == 0
            || terminal.usage.generated_tokens == 0
            || self.decoded_byte_count == 0
        {
            return Err(BenchmarkError::new(format!(
                "generation did not publish non-zero prompt usage, generated usage, and decoded bytes: usage={:?}, decoded_bytes={}",
                terminal.usage, self.decoded_byte_count
            )));
        }
        validate_expected_outcome(self.expectation, finish_reason, terminal.usage)?;
        validate_released_runtime(runtime, terminal)?;

        let normal_timings = normal_timings(
            self.submitted_at,
            NormalObservationTimes {
                started: self.started_at,
                first_decoded: self.first_decoded_at,
                terminal_event: Some(terminal_observation.observed_at),
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
                    terminal_event: Some(terminal_observation.observed_at),
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
            finish_reason,
            terminal_kind: finish_reason_label(finish_reason),
            usage: terminal.usage,
            decoded_byte_count: self.decoded_byte_count,
            normal_timings,
            cancellation_timings,
        })
    }
}

fn drive_generation(
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
            return observer.finish(runtime);
        }
        wait_for_next_poll(deadline, "generation terminal release")?;
    }
}

fn validate_chat_ready(runtime: &ApplicationRuntime, loaded: &LoadedModel) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded() != Some(loaded)
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
        || !runtime.can_submit_chat_message()
        || !runtime.conversation().is_empty()
        || runtime.context_diagnostics().is_some()
    {
        return Err(BenchmarkError::new(
            "compatible-chat proof did not begin from a connected, loaded, empty E1 state",
        ));
    }
    Ok(())
}

fn validate_direct_ready(runtime: &ApplicationRuntime) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded().is_none()
        || state.active_generation().is_some()
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "direct completion did not begin from a connected, loaded, idle E1 state",
        ));
    }
    validate_direct_conversation_state(runtime)
}

fn validate_direct_conversation_state(runtime: &ApplicationRuntime) -> BenchmarkResult {
    if !runtime.conversation().is_empty() || runtime.context_diagnostics().is_some() {
        return Err(BenchmarkError::new(
            "direct completion unexpectedly retained chat conversation state or diagnostics",
        ));
    }
    Ok(())
}

fn validate_released_runtime(
    runtime: &ApplicationRuntime,
    terminal: &GenerationTerminal,
) -> BenchmarkResult {
    let state = runtime.state();
    if state.activity() != ApplicationActivity::Idle
        || state.loaded().is_none()
        || state.active_generation().is_some()
        || state.last_generation() != Some(terminal)
        || !state.hub_available()
        || !state.inference_available()
    {
        return Err(BenchmarkError::new(
            "public E1 state did not retain the matching released terminal generation cleanly",
        ));
    }
    Ok(())
}

fn validate_chat_conversation(
    runtime: &ApplicationRuntime,
    loaded: &LoadedModel,
    evidence: &GenerationEvidence,
) -> BenchmarkResult {
    let records = runtime.conversation();
    if records.len() != 2 {
        return Err(BenchmarkError::new(format!(
            "compatible-chat proof retained {} conversation records instead of one user and one assistant record",
            records.len()
        )));
    }
    let user = records
        .first()
        .ok_or_else(|| BenchmarkError::new("compatible-chat user record disappeared"))?;
    let assistant = records
        .get(1)
        .ok_or_else(|| BenchmarkError::new("compatible-chat assistant record disappeared"))?;
    if user.role != ConversationRole::User
        || user.provenance != ConversationProvenance::User
        || user.content != CHAT_MESSAGE
        || user.response_attempt.is_some()
        || assistant.role != ConversationRole::Assistant
        || assistant.provenance != ConversationProvenance::Model
        || assistant.content.is_empty()
        || u64::try_from(assistant.content.len()).ok() != Some(evidence.decoded_byte_count)
        || assistant.token_estimate
            != ConversationTokenEstimate::Generated(
                u32::try_from(evidence.usage.generated_tokens).map_err(|_| {
                    BenchmarkError::new("chat generated usage could not fit its token estimate")
                })?,
            )
        || !assistant.is_active_context()
    {
        return Err(BenchmarkError::new(
            "compatible-chat conversation did not retain the expected user turn and non-empty active model response",
        ));
    }
    let attempt = assistant.response_attempt.as_ref().ok_or_else(|| {
        BenchmarkError::new("compatible-chat assistant record had no response-attempt provenance")
    })?;
    if attempt.responding_to != user.id
        || attempt.superseded
        || attempt.state != ResponseAttemptState::Completed(evidence.finish_reason)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat assistant attempt did not match the released terminal outcome: {attempt:?}"
        )));
    }

    let diagnostics = runtime.context_diagnostics().ok_or_else(|| {
        BenchmarkError::new("compatible-chat context diagnostics were not retained")
    })?;
    if diagnostics.actual_input_tokens == 0
        || u64::from(diagnostics.actual_input_tokens) != evidence.usage.prompt_tokens
        || diagnostics.reserved_output_tokens != CHAT_MAXIMUM_NEW_TOKENS
        || diagnostics.maximum_context_tokens != loaded.maximum_context_tokens()
        || diagnostics
            .actual_input_tokens
            .checked_add(diagnostics.reserved_output_tokens)
            .is_none_or(|required| required > diagnostics.maximum_context_tokens)
    {
        return Err(BenchmarkError::new(format!(
            "compatible-chat context diagnostics were incomplete: {diagnostics:?}"
        )));
    }
    Ok(())
}

fn clear_chat_conversation(runtime: &mut ApplicationRuntime) -> BenchmarkResult {
    runtime.clear_conversation().map_err(|error| {
        BenchmarkError::new(format!(
            "compatible-chat conversation could not be cleared after release: {error}"
        ))
    })?;
    if !runtime.conversation().is_empty() || runtime.context_diagnostics().is_some() {
        return Err(BenchmarkError::new(
            "compatible-chat conversation or diagnostics remained after public clear",
        ));
    }
    Ok(())
}

fn validate_expected_outcome(
    expectation: GenerationExpectation,
    finish_reason: FinishReason,
    usage: GenerationUsage,
) -> BenchmarkResult {
    match expectation {
        GenerationExpectation::Chat => match finish_reason {
            FinishReason::TokenLimit
                if usage.generated_tokens == u64::from(CHAT_MAXIMUM_NEW_TOKENS) =>
            {
                Ok(())
            }
            FinishReason::EndOfSequence(_)
                if usage.generated_tokens <= u64::from(CHAT_MAXIMUM_NEW_TOKENS) =>
            {
                Ok(())
            }
            _ => Err(BenchmarkError::new(format!(
                "compatible-chat proof returned a finish reason or usage inconsistent with its {CHAT_MAXIMUM_NEW_TOKENS}-token bound: reason={finish_reason:?}, usage={usage:?}"
            ))),
        },
        GenerationExpectation::DirectTokenLimit => {
            if finish_reason == FinishReason::TokenLimit
                && usage.generated_tokens == u64::from(DIRECT_MAXIMUM_NEW_TOKENS)
            {
                Ok(())
            } else {
                Err(BenchmarkError::new(format!(
                    "controlled direct completion did not reach the exact {DIRECT_MAXIMUM_NEW_TOKENS}-token limit: reason={finish_reason:?}, usage={usage:?}"
                )))
            }
        }
        GenerationExpectation::Cancellation => {
            if finish_reason == FinishReason::Cancelled(CancellationReason::UserRequested)
                && usage.generated_tokens > 0
                && usage.generated_tokens < u64::from(CANCELLATION_MAXIMUM_NEW_TOKENS)
            {
                Ok(())
            } else {
                Err(BenchmarkError::new(format!(
                    "progress-triggered cancellation did not finish as Cancelled(UserRequested) strictly before the {CANCELLATION_MAXIMUM_NEW_TOKENS}-token bound: reason={finish_reason:?}, usage={usage:?}"
                )))
            }
        }
    }
}

fn terminal_kind_for_outcome(outcome: &GenerationTerminalOutcome) -> GenerationTerminalKind {
    match outcome {
        GenerationTerminalOutcome::Finished(reason) => GenerationTerminalKind::Finished(*reason),
        GenerationTerminalOutcome::Failed(_) => GenerationTerminalKind::Failed,
    }
}

fn validate_terminal_consistency(
    terminal_output: Option<GenerationTerminalKind>,
    released_output: Option<GenerationTerminalKind>,
    event_kind: GenerationTerminalKind,
) -> BenchmarkResult {
    if terminal_output != Some(event_kind) || released_output != Some(event_kind) {
        return Err(BenchmarkError::new(format!(
            "terminal, released, and GenerationFinished outcomes did not match: terminal={terminal_output:?}, released={released_output:?}, event={event_kind:?}"
        )));
    }
    Ok(())
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

fn warmup_record(evidence: &GenerationEvidence) -> DirectCompletionWarmupResult {
    DirectCompletionWarmupResult {
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: evidence.terminal_kind,
        clean_release: true,
    }
}

fn sample_record(
    ordinal: u32,
    evidence: &GenerationEvidence,
) -> BenchmarkResult<DirectCompletionSample> {
    let release_seconds = evidence.normal_timings.release.as_secs_f64();
    if release_seconds <= 0.0 || !release_seconds.is_finite() {
        return Err(BenchmarkError::new(
            "submission-to-release duration could not support a finite throughput calculation",
        ));
    }
    let generated_tokens = u32::try_from(evidence.usage.generated_tokens).map_err(|_| {
        BenchmarkError::new("generated-token count was too large for exact f64 conversion")
    })?;
    let effective_generated_tokens_per_second = f64::from(generated_tokens) / release_seconds;
    if !effective_generated_tokens_per_second.is_finite() {
        return Err(BenchmarkError::new(
            "effective generated-token throughput was not finite",
        ));
    }

    Ok(DirectCompletionSample {
        ordinal,
        submission_to_generation_started_ns: duration_ns(evidence.normal_timings.started),
        submission_to_first_decoded_output_ns: duration_ns(evidence.normal_timings.first_decoded),
        submission_to_terminal_event_ns: duration_ns(evidence.normal_timings.terminal_event),
        submission_to_release_ns: duration_ns(evidence.normal_timings.release),
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        decoded_byte_count: evidence.decoded_byte_count,
        terminal_kind: evidence.terminal_kind,
        terminal_state_matched: true,
        released_state_matched: true,
        terminal_event_matched: true,
        effective_generated_tokens_per_second,
    })
}

fn cancellation_record(evidence: &GenerationEvidence) -> BenchmarkResult<CancellationResult> {
    let timings = evidence.cancellation_timings.ok_or_else(|| {
        BenchmarkError::new("completed cancellation did not retain cancellation timings")
    })?;
    Ok(CancellationResult {
        generation_submission: GenerationSubmissionTimings {
            to_generation_started_ns: duration_ns(evidence.normal_timings.started),
            to_first_decoded_output_ns: duration_ns(evidence.normal_timings.first_decoded),
            to_cancellation_submission_ns: duration_ns(
                timings.generation_submission_to_cancellation,
            ),
        },
        cancellation_submission: CancellationSubmissionTimings {
            to_acknowledgement_ns: duration_ns(timings.cancellation_submission_to_acknowledgement),
            to_terminal_output_ns: duration_ns(timings.cancellation_submission_to_terminal_output),
            to_terminal_event_ns: duration_ns(timings.cancellation_submission_to_terminal_event),
            to_release_ns: duration_ns(timings.cancellation_submission_to_release),
        },
        decoded_byte_count: evidence.decoded_byte_count,
        prompt_tokens: evidence.usage.prompt_tokens,
        generated_tokens: evidence.usage.generated_tokens,
        terminal_kind: evidence.terminal_kind,
        cancellation_acknowledged: true,
        outcome_match: matched_outcomes(),
    })
}

fn summarize_samples(
    samples: &[DirectCompletionSample],
) -> BenchmarkResult<DirectCompletionSummary> {
    let sample_count = u32::try_from(samples.len())
        .map_err(|_| BenchmarkError::new("sample count conversion to u32 failed"))?;
    if sample_count != SAMPLE_COUNT {
        return Err(BenchmarkError::new(format!(
            "external baseline collected {sample_count} samples instead of {SAMPLE_COUNT}"
        )));
    }
    Ok(DirectCompletionSummary {
        sample_count,
        median_submission_to_generation_started_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_generation_started_ns),
        )?,
        median_submission_to_first_decoded_output_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_first_decoded_output_ns),
        )?,
        median_submission_to_terminal_event_ns: median_u64(
            samples
                .iter()
                .map(|sample| sample.submission_to_terminal_event_ns),
        )?,
        median_submission_to_release_ns: median_u64(
            samples.iter().map(|sample| sample.submission_to_release_ns),
        )?,
        median_effective_generated_tokens_per_second: median_f64(
            samples
                .iter()
                .map(|sample| sample.effective_generated_tokens_per_second),
        )?,
    })
}

fn median_u64(values: impl IntoIterator<Item = u64>) -> BenchmarkResult<u64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() {
        return Err(BenchmarkError::new("cannot summarize an empty sample set"));
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("summary upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("summary lower median disappeared"))?;
        Ok(lower.saturating_add(upper) / 2)
    } else {
        Ok(upper)
    }
}

fn median_f64(values: impl IntoIterator<Item = f64>) -> BenchmarkResult<f64> {
    let mut values = values.into_iter().collect::<Vec<_>>();
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return Err(BenchmarkError::new(
            "cannot summarize empty or non-finite throughput samples",
        ));
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    let upper = values
        .get(middle)
        .copied()
        .ok_or_else(|| BenchmarkError::new("throughput upper median disappeared"))?;
    if values.len() % 2 == 0 {
        let lower = values
            .get(middle.saturating_sub(1))
            .copied()
            .ok_or_else(|| BenchmarkError::new("throughput lower median disappeared"))?;
        Ok(f64::midpoint(lower, upper))
    } else {
        Ok(upper)
    }
}

const fn matched_outcomes() -> GenerationOutcomeMatch {
    GenerationOutcomeMatch {
        terminal_state_matched: true,
        released_state_matched: true,
        terminal_event_matched: true,
    }
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

const fn finish_reason_label(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::EndOfSequence(_) => "end_of_sequence",
        FinishReason::TokenLimit => "token_limit",
        FinishReason::StopCondition => "stop_condition",
        FinishReason::BufferExhausted(_) => "buffer_exhausted",
        FinishReason::Cancelled(_) => "cancelled",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use application_runtime::{
        ApplicationOutputRecordKind, ApplicationOutputState, ApplicationTextRange,
        GenerationTerminalKind,
    };
    use domain_contracts::{
        CancellationReason, FinishReason, GenerationUsage, RequestId, YieldReason,
    };

    use super::{
        CancellationObservationTimes, GenerationExpectation, GenerationObserver,
        NormalObservationTimes, cancellation_timings, normal_timings, validate_expected_outcome,
        validate_terminal_consistency,
    };

    #[test]
    fn output_validation_rejects_a_mismatched_request_identity() {
        let submitted_at = Instant::now();
        let mut observer = GenerationObserver::new(
            RequestId::new(1),
            GenerationExpectation::DirectTokenLimit,
            submitted_at,
        );
        let result = observer.observe_output_at(
            RequestId::new(2),
            ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(
                GenerationTerminalKind::Finished(FinishReason::TokenLimit),
            )),
            None,
            submitted_at,
        );
        assert!(result.is_err());
    }

    #[test]
    fn output_validation_enforces_terminal_then_release_order() -> Result<(), String> {
        let observed_at = Instant::now();
        let request_id = RequestId::new(1);
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);

        let mut release_first = GenerationObserver::new(
            request_id,
            GenerationExpectation::DirectTokenLimit,
            observed_at,
        );
        assert!(
            release_first
                .observe_output_at(
                    request_id,
                    ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                    None,
                    observed_at,
                )
                .is_err()
        );

        let mut ordered = GenerationObserver::new(
            request_id,
            GenerationExpectation::DirectTokenLimit,
            observed_at,
        );
        ordered
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Terminal(terminal)),
                None,
                observed_at,
            )
            .map_err(|error| error.to_string())?;
        for late_record in [
            ApplicationOutputRecordKind::Text(ApplicationTextRange {
                start: 0,
                length: 1,
            }),
            ApplicationOutputRecordKind::State(ApplicationOutputState::Yielded(
                YieldReason::Scheduler,
            )),
        ] {
            assert!(
                ordered
                    .observe_output_at(request_id, late_record, Some("x"), observed_at)
                    .is_err()
            );
        }
        ordered
            .observe_output_at(
                request_id,
                ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                None,
                observed_at,
            )
            .map_err(|error| error.to_string())?;
        assert!(
            ordered
                .observe_output_at(
                    request_id,
                    ApplicationOutputRecordKind::State(ApplicationOutputState::Released(terminal)),
                    None,
                    observed_at,
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn terminal_validation_rejects_mismatched_terminal_and_released_outcomes() {
        let terminal = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        let released = GenerationTerminalKind::Finished(FinishReason::StopCondition);
        assert!(validate_terminal_consistency(Some(terminal), Some(released), terminal).is_err());
        assert!(validate_terminal_consistency(Some(terminal), Some(terminal), released).is_err());
    }

    #[test]
    fn terminal_validation_accepts_one_matching_terminal_release_and_event() -> Result<(), String> {
        let expected = GenerationTerminalKind::Finished(FinishReason::TokenLimit);
        validate_terminal_consistency(Some(expected), Some(expected), expected)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn exact_direct_and_bounded_cancellation_outcomes_are_enforced() -> Result<(), String> {
        let direct_usage = GenerationUsage {
            prompt_tokens: 12,
            generated_tokens: 32,
        };
        validate_expected_outcome(
            GenerationExpectation::DirectTokenLimit,
            FinishReason::TokenLimit,
            direct_usage,
        )
        .map_err(|error| error.to_string())?;
        assert!(
            validate_expected_outcome(
                GenerationExpectation::DirectTokenLimit,
                FinishReason::TokenLimit,
                GenerationUsage {
                    generated_tokens: 31,
                    ..direct_usage
                },
            )
            .is_err()
        );

        let cancellation_usage = GenerationUsage {
            prompt_tokens: 12,
            generated_tokens: 7,
        };
        validate_expected_outcome(
            GenerationExpectation::Cancellation,
            FinishReason::Cancelled(CancellationReason::UserRequested),
            cancellation_usage,
        )
        .map_err(|error| error.to_string())?;
        assert!(
            validate_expected_outcome(
                GenerationExpectation::Cancellation,
                FinishReason::Cancelled(CancellationReason::UserRequested),
                GenerationUsage {
                    generated_tokens: 128,
                    ..cancellation_usage
                },
            )
            .is_err()
        );
        assert!(
            validate_expected_outcome(
                GenerationExpectation::Cancellation,
                FinishReason::Cancelled(CancellationReason::ParentTask),
                cancellation_usage,
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn timing_helpers_preserve_each_submission_origin() -> Result<(), String> {
        let generation_submitted_at = Instant::now();
        let after = |milliseconds| {
            generation_submitted_at
                .checked_add(Duration::from_millis(milliseconds))
                .ok_or_else(|| "test instant overflowed".to_owned())
        };
        let normal = normal_timings(
            generation_submitted_at,
            NormalObservationTimes {
                started: Some(after(2)?),
                first_decoded: Some(after(5)?),
                terminal_event: Some(after(17)?),
                release: Some(after(19)?),
            },
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(normal.started, Duration::from_millis(2));
        assert_eq!(normal.first_decoded, Duration::from_millis(5));
        assert_eq!(normal.terminal_event, Duration::from_millis(17));
        assert_eq!(normal.release, Duration::from_millis(19));

        let cancellation = cancellation_timings(
            generation_submitted_at,
            CancellationObservationTimes {
                submitted: Some(after(7)?),
                acknowledged: Some(after(9)?),
                terminal_output: Some(after(11)?),
                terminal_event: Some(after(13)?),
                release: Some(after(15)?),
            },
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(
            cancellation.generation_submission_to_cancellation,
            Duration::from_millis(7)
        );
        assert_eq!(
            cancellation.cancellation_submission_to_acknowledgement,
            Duration::from_millis(2)
        );
        assert_eq!(
            cancellation.cancellation_submission_to_terminal_output,
            Duration::from_millis(4)
        );
        assert_eq!(
            cancellation.cancellation_submission_to_terminal_event,
            Duration::from_millis(6)
        );
        assert_eq!(
            cancellation.cancellation_submission_to_release,
            Duration::from_millis(8)
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
