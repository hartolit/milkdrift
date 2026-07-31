//! Frontend-neutral direct-completion admission, decoding, and bounded text output.

use std::collections::VecDeque;
use std::num::{NonZeroU32, NonZeroUsize};

use domain_contracts::{
    CancellationReason, FinishReason, GenerationUsage, RequestId, SequenceConfiguration,
    SequenceId, TokenId, YieldReason,
};

use host_runtime::{
    OutputPullError, OutputPushError, TextOutputBatch, TextOutputConsumer,
    TextOutputInitializationError, TextOutputProducer, TextOutputRecordKind, TokenOutputBatch,
    TokenOutputRecordKind, text_output_accumulator,
};
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputCapacityPolicy, GenerationOutputState,
    GenerationRequest, GenerationStopSequence, RuntimeCommand, RuntimeEvent, SamplingConfig,
};
use tokenization::{
    DecodeOptions, EncodeOptions, SpecialTokenPolicy, StreamingDecoder, TextBuffer, TokenBuffer,
    TokenizationError, Tokenizer,
};

use crate::{
    ApplicationConfigurationField, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationRuntime, ApplicationRuntimeConfiguration, GenerationPhase,
    GenerationSettingsField, GenerationSummary, GenerationTerminal, GenerationTerminalOutcome,
};
use hf_tokenizer::HfOwnedStreamingDecoder;

const FIRST_STOP_SEQUENCE_CODE: u32 = 1;
const INTERNAL_SCHEDULER_QUANTUM: NonZeroU32 = NonZeroU32::MIN;

/// Stable seed policy for one direct-completion request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GenerationSeed {
    /// Derive a deterministic seed from the application request identity.
    #[default]
    RequestId,
    /// Use one exact caller-provided seed.
    Fixed(u64),
}

/// Stable E1 settings translated into E0 sampling and generation contracts.
#[derive(Clone, Debug, PartialEq)]
pub struct GenerationSettings {
    /// Maximum sampled continuation tokens.
    pub maximum_new_tokens: u32,
    /// Positive finite softmax temperature.
    pub temperature: f32,
    /// Maximum retained candidates after ordering. Zero retains all.
    pub top_k: u32,
    /// Cumulative probability threshold in `(0, 1]`.
    pub top_p: f32,
    /// Minimum probability relative to the highest-probability token in `[0, 1]`.
    pub min_p: f32,
    /// Positive finite repetition penalty. One disables the penalty.
    pub repetition_penalty: f32,
    /// Number of trailing tokens considered for repetition. Zero uses full history.
    pub repetition_window: u32,
    /// Deterministic seed policy.
    pub seed: GenerationSeed,
    /// Explicit token identifiers treated as end-of-sequence markers.
    pub eos_tokens: Vec<TokenId>,
    /// Textual stop suffixes encoded once before E0 admission.
    pub stop_sequences: Vec<String>,
}

impl Default for GenerationSettings {
    fn default() -> Self {
        Self {
            maximum_new_tokens: 128,
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
            min_p: 0.0,
            repetition_penalty: 1.0,
            repetition_window: 64,
            seed: GenerationSeed::RequestId,
            eos_tokens: Vec::new(),
            stop_sequences: Vec::new(),
        }
    }
}

impl GenerationSettings {
    fn validate(&self) -> Result<SamplingConfig, ApplicationError> {
        if self.maximum_new_tokens == 0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MaximumNewTokens,
            ));
        }
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::Temperature,
            ));
        }
        if !self.top_p.is_finite() || self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::TopP,
            ));
        }
        if !self.min_p.is_finite() || self.min_p < 0.0 || self.min_p > 1.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::MinP,
            ));
        }
        if !self.repetition_penalty.is_finite() || self.repetition_penalty <= 0.0 {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::RepetitionPenalty,
            ));
        }
        if self
            .stop_sequences
            .iter()
            .any(std::string::String::is_empty)
        {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::StopSequence,
            ));
        }
        Ok(SamplingConfig {
            temperature: self.temperature,
            top_k: self.top_k,
            top_p: self.top_p,
            min_p: self.min_p,
            repetition_penalty: self.repetition_penalty,
            repetition_window: self.repetition_window,
        })
    }
}

/// Compact state payload published beside decoded text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOutputState {
    /// E0 yielded without completing the request.
    Yielded(YieldReason),
    /// Generation ended and sequence cleanup is beginning.
    Terminal(GenerationTerminalKind),
    /// Explicit sequence cleanup failed but remains retryable.
    CleanupPending,
    /// Automatic cleanup attempts are exhausted.
    CleanupExhausted,
    /// Sequence cleanup completed and request accounting was released.
    Released(GenerationTerminalKind),
}

/// Allocation-free terminal classification used in pulled output records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationTerminalKind {
    /// Generation completed with a stable finish reason.
    Finished(FinishReason),
    /// Generation failed; the detailed normalized failure is available as an event/state summary.
    Failed,
}

/// Absolute UTF-8 byte range within the application output stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ApplicationTextRange {
    /// Inclusive absolute byte position.
    pub start: u64,
    /// Number of UTF-8 bytes in the range.
    pub length: usize,
}

/// One request-scoped decoded output record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationOutputRecordKind {
    /// UTF-8 text committed to the batch byte storage.
    Text(ApplicationTextRange),
    /// Application generation or cleanup state.
    State(ApplicationOutputState),
}

/// One request-scoped decoded output record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationOutputRecord {
    /// Request that produced the record.
    pub request_id: RequestId,
    /// Text or state payload.
    pub kind: ApplicationOutputRecordKind,
}

/// Borrowed decoded output batch exposed without host-runtime implementation types.
pub struct ApplicationOutputBatch<'a> {
    inner: TextOutputBatch<'a, ApplicationOutputState>,
}

impl<'a> ApplicationOutputBatch<'a> {
    /// Returns the absolute byte cursor before this batch.
    #[must_use]
    pub const fn start(&self) -> u64 {
        self.inner.start.get()
    }

    /// Returns the absolute byte cursor after this batch.
    #[must_use]
    pub const fn end(&self) -> u64 {
        self.inner.end.get()
    }

    /// Returns the contiguous UTF-8 bytes retained by this batch.
    #[must_use]
    pub const fn bytes(&self) -> &'a [u8] {
        let bytes: &'a [u8] = self.inner.bytes;
        bytes
    }

    /// Iterates over copied frontend-neutral record descriptions.
    pub fn records(&self) -> impl Iterator<Item = ApplicationOutputRecord> + '_ {
        self.inner.records.iter().map(|record| {
            let kind = match record.kind {
                TextOutputRecordKind::Text(range) => {
                    ApplicationOutputRecordKind::Text(ApplicationTextRange {
                        start: range.start.get(),
                        length: range.length,
                    })
                }
                TextOutputRecordKind::State(state) => ApplicationOutputRecordKind::State(state),
            };
            ApplicationOutputRecord {
                request_id: record.request_id,
                kind,
            }
        })
    }

    /// Resolves one text record to its UTF-8 fragment when it belongs to this batch.
    #[must_use]
    pub fn text_for(&self, record: ApplicationOutputRecord) -> Option<&'a str> {
        let ApplicationOutputRecordKind::Text(range) = record.kind else {
            return None;
        };
        let offset = range.start.checked_sub(self.start())?;
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(range.length)?;
        let bytes: &'a [u8] = self.inner.bytes.get(offset..end)?;
        std::str::from_utf8(bytes).ok()
    }
}

pub struct GenerationBridge {
    output_producer: TextOutputProducer<ApplicationOutputState>,
    output_consumer: TextOutputConsumer<ApplicationOutputState>,
    pending_capacity: usize,
    pending: VecDeque<PendingOutput>,
    session: Option<GenerationSession>,
    pending_event: Option<ApplicationEvent>,
}

struct GenerationSession {
    request_id: RequestId,
    admission_ticket: CommandTicket,
    decoder: HfOwnedStreamingDecoder,
    decode_storage: Vec<u8>,
    pending_text_length: usize,
    local_failure: Option<ApplicationFailure>,
    cancellation_requested: bool,
}

#[derive(Clone, Copy)]
enum PendingOutput {
    Token(TokenId),
    State(GenerationOutputState),
}

impl GenerationBridge {
    pub(crate) fn new(
        configuration: &ApplicationRuntimeConfiguration,
    ) -> Result<Self, ApplicationError> {
        let bytes = nonzero_capacity(
            configuration.text_output_byte_capacity,
            ApplicationConfigurationField::TextOutputByteCapacity,
        )?;
        let records = nonzero_capacity(
            configuration.text_output_record_capacity,
            ApplicationConfigurationField::TextOutputRecordCapacity,
        )?;
        let (output_producer, output_consumer) =
            text_output_accumulator(bytes, records).map_err(text_output_initialization_failure)?;
        let pending_capacity = configuration
            .token_output_capacity
            .checked_add(configuration.token_output_record_capacity)
            .ok_or(ApplicationError::InvalidConfiguration(
                ApplicationConfigurationField::PendingGenerationOutputCapacity,
            ))?;
        let mut pending = VecDeque::new();
        pending
            .try_reserve_exact(pending_capacity)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Worker, error))?;
        Ok(Self {
            output_producer,
            output_consumer,
            pending_capacity,
            pending,
            session: None,
            pending_event: None,
        })
    }

    fn output_batch<R, F>(&self, consume: F) -> Result<R, ApplicationError>
    where
        F: for<'batch> FnOnce(ApplicationOutputBatch<'batch>) -> R,
    {
        self.output_consumer
            .pull(|batch| consume(ApplicationOutputBatch { inner: batch }))
            .map_err(output_pull_failure)
    }
}

impl ApplicationRuntime {
    /// Starts one direct-completion request against the resident single model.
    ///
    /// This mode intentionally performs no chat rendering and remains available
    /// when the resolved model has no verified chat compatibility profile.
    ///
    /// # Errors
    ///
    /// Returns an error when lifecycle state, settings, prompt capacity, tokenizer
    /// state, or bounded runtime command capacity prevents admission.
    pub fn start_generation(
        &mut self,
        input: &str,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        let loaded = self.state.loaded().ok_or(ApplicationError::NoLoadedModel)?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?;
        let prompt_tokens =
            encode_direct_completion_prompt(tokenizer, input, loaded.maximum_context_tokens())?;
        self.start_generation_tokens(prompt_tokens, settings)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "generation admission keeps validation, allocation, translation, submission, and state publication contiguous"
    )]
    pub(crate) fn start_generation_tokens(
        &mut self,
        prompt_tokens: Box<[TokenId]>,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        if let Some(active) = self.state.active_generation() {
            return Err(ApplicationError::GenerationAlreadyActive(active.request_id));
        }
        if self.state.activity() != crate::ApplicationActivity::Idle {
            return Err(ApplicationError::Busy(self.state.activity()));
        }
        if !self.state.inference_available() {
            return Err(ApplicationError::RuntimeDisconnected);
        }
        let loaded = self
            .state
            .loaded()
            .cloned()
            .ok_or(ApplicationError::NoLoadedModel)?;
        let sampling = settings.validate()?;
        if settings
            .eos_tokens
            .iter()
            .any(|token| token.get() >= loaded.vocabulary_size())
        {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::EndOfSequenceToken,
            ));
        }
        if prompt_tokens.is_empty() {
            return Err(ApplicationError::EmptyPrompt);
        }
        let stop_sequences = {
            let tokenizer = self
                .tokenizer
                .as_ref()
                .ok_or(ApplicationError::NoTokenizer)?;
            encode_stop_sequences(
                tokenizer,
                settings.stop_sequences.as_slice(),
                loaded.maximum_context_tokens(),
            )?
        };
        let prompt_len = prompt_tokens.len();
        let maximum_prefill = usize::try_from(loaded.maximum_prefill_batch()).unwrap_or(usize::MAX);
        if prompt_len > maximum_prefill {
            return Err(ApplicationError::PromptTooLong {
                required: prompt_len,
                available: maximum_prefill,
            });
        }
        let maximum_new_tokens = NonZeroU32::new(settings.maximum_new_tokens).ok_or(
            ApplicationError::InvalidGenerationSettings(GenerationSettingsField::MaximumNewTokens),
        )?;
        let required_tokens = u64::try_from(prompt_len)
            .unwrap_or(u64::MAX)
            .saturating_add(u64::from(maximum_new_tokens.get()));
        if required_tokens > u64::from(loaded.maximum_context_tokens()) {
            return Err(ApplicationError::ContextCapacityExceeded {
                required: required_tokens,
                available: u64::from(loaded.maximum_context_tokens()),
            });
        }
        let sequence_tokens = u32::try_from(required_tokens).map_err(|_| {
            ApplicationError::ContextCapacityExceeded {
                required: required_tokens,
                available: u64::from(loaded.maximum_context_tokens()),
            }
        })?;
        let prompt_batch =
            u32::try_from(prompt_len).map_err(|_| ApplicationError::PromptTooLong {
                required: prompt_len,
                available: maximum_prefill,
            })?;
        let ticket = self.next_ticket()?;
        let request_id = RequestId::new(ticket.get());
        let seed = match settings.seed {
            GenerationSeed::RequestId => request_id.get(),
            GenerationSeed::Fixed(seed) => seed,
        };
        let request = GenerationRequest {
            request_id,
            sequence_id: SequenceId::new(request_id.get()),
            prompt_tokens,
            sequence: SequenceConfiguration::new(
                NonZeroU32::new(sequence_tokens).ok_or(ApplicationError::EmptyPrompt)?,
                NonZeroU32::new(prompt_batch).ok_or(ApplicationError::EmptyPrompt)?,
            ),
            maximum_generated_tokens: maximum_new_tokens,
            sampling,
            seed,
            eos_tokens: settings.eos_tokens.into_boxed_slice(),
            stop_sequences,
            scheduler_quantum: INTERNAL_SCHEDULER_QUANTUM,
            output_capacity: GenerationOutputCapacityPolicy::default(),
        };
        let decoder = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?
            .owned_decoder(DecodeOptions {
                skip_special_tokens: true,
            });
        let mut decode_storage = Vec::new();
        decode_storage
            .try_reserve_exact(self.configuration.text_output_byte_capacity)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Worker, error))?;
        decode_storage.resize(self.configuration.text_output_byte_capacity, 0);
        self.submit_inference(RuntimeCommand::Generate {
            ticket,
            handle: loaded.handle(),
            request,
        })?;
        self.generation.pending.clear();
        self.generation.pending_event = None;
        self.generation.session = Some(GenerationSession {
            request_id,
            admission_ticket: ticket,
            decoder,
            decode_storage,
            pending_text_length: 0,
            local_failure: None,
            cancellation_requested: false,
        });
        self.state.begin_generation(GenerationSummary {
            request_id,
            phase: GenerationPhase::Starting,
            usage: GenerationUsage {
                prompt_tokens: u64::try_from(prompt_len).unwrap_or(u64::MAX),
                generated_tokens: 0,
            },
        });
        Ok(request_id)
    }

    /// Requests cancellation of the active generation at the next safe E0 boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when `request_id` is not active or the bounded inference
    /// command queue cannot accept the cancellation request.
    pub fn cancel_generation(&mut self, request_id: RequestId) -> Result<(), ApplicationError> {
        let active = self
            .state
            .active_generation()
            .ok_or(ApplicationError::GenerationNotActive(request_id))?;
        if active.request_id != request_id {
            return Err(ApplicationError::GenerationNotActive(request_id));
        }
        if active.phase == GenerationPhase::Cancelling {
            return Ok(());
        }
        let ticket = self.next_ticket()?;
        self.submit_inference(RuntimeCommand::CancelRequest {
            ticket,
            request_id,
            reason: CancellationReason::UserRequested,
        })?;
        self.state.set_generation_phase(GenerationPhase::Cancelling);
        if let Some(session) = self.generation.session.as_mut() {
            session.cancellation_requested = true;
        }
        Ok(())
    }

    /// Pulls the currently accumulated decoded text and generation-state records.
    ///
    /// The callback borrows retained application output storage. Copy any values
    /// needed after it returns; the next application write may reuse the same allocation.
    ///
    /// # Errors
    ///
    /// Returns a normalized failure if the decoded output accumulator was poisoned.
    pub fn pull_output<R, F>(&mut self, consume: F) -> Result<R, ApplicationError>
    where
        F: for<'batch> FnOnce(ApplicationOutputBatch<'batch>) -> R,
    {
        self.pump_generation_output();
        self.generation.output_batch(consume)
    }

    pub(crate) const fn take_generation_event(&mut self) -> Option<ApplicationEvent> {
        self.generation.pending_event.take()
    }

    pub(crate) fn pump_generation_event(&mut self) -> Option<ApplicationEvent> {
        self.pump_generation_output();
        self.generation.pending_event.take()
    }

    pub(crate) fn handle_generation_runtime_disconnected(&mut self) {
        let Some(active) = self.state.active_generation() else {
            return;
        };
        let terminal = GenerationTerminal {
            request_id: active.request_id,
            outcome: GenerationTerminalOutcome::Failed(ApplicationFailure {
                kind: ApplicationFailureKind::Inference,
                message: "inference runtime disconnected during generation".to_owned(),
            }),
            usage: active.usage,
        };
        self.conversation.finish_active(
            active.request_id,
            &terminal.outcome,
            terminal.usage.generated_tokens,
        );
        self.state.finish_generation(terminal.clone());
        self.generation.session = None;
        self.generation.pending.clear();
        self.generation.pending_event = Some(ApplicationEvent::GenerationFinished { terminal });
    }

    pub(crate) fn process_generation_runtime_event(
        &mut self,
        event: &RuntimeEvent,
    ) -> Option<ApplicationEvent> {
        match event {
            RuntimeEvent::GenerationAdmitted { ticket, result } => {
                let session = self.generation.session.as_ref()?;
                if session.admission_ticket != *ticket {
                    return None;
                }
                let request_id = session.request_id;
                match result {
                    Ok(admission) if admission.request.request_id == request_id => {
                        self.state.set_generation_phase(GenerationPhase::Running);
                        Some(ApplicationEvent::GenerationStarted { request_id })
                    }
                    Ok(_) => Some(
                        self.fail_generation_admission(
                            request_id,
                            ApplicationFailure {
                                kind: ApplicationFailureKind::Inference,
                                message:
                                    "generation admission returned a mismatched request identity"
                                        .to_owned(),
                            },
                        ),
                    ),
                    Err(error) => Some(self.fail_generation_admission(
                        request_id,
                        ApplicationFailure::from_debug(
                            ApplicationFailureKind::Inference,
                            "generation admission failed",
                            error,
                        ),
                    )),
                }
            }
            RuntimeEvent::GenerationCancellationRequested {
                request_id, result, ..
            } => {
                let session = self.generation.session.as_mut()?;
                if session.request_id != *request_id {
                    return None;
                }
                match result {
                    Ok(()) => Some(ApplicationEvent::GenerationCancellationRequested {
                        request_id: *request_id,
                    }),
                    Err(error) => {
                        session.cancellation_requested = false;
                        self.state.set_generation_phase(GenerationPhase::Running);
                        Some(ApplicationEvent::GenerationCancellationFailed {
                            request_id: *request_id,
                            failure: ApplicationFailure::from_debug(
                                ApplicationFailureKind::Inference,
                                "generation cancellation failed",
                                error,
                            ),
                        })
                    }
                }
            }
            _ => None,
        }
    }

    fn fail_generation_admission(
        &mut self,
        request_id: RequestId,
        failure: ApplicationFailure,
    ) -> ApplicationEvent {
        let usage = self
            .state
            .active_generation()
            .map_or_else(GenerationUsage::default, |summary| summary.usage);
        let terminal = GenerationTerminal {
            request_id,
            outcome: GenerationTerminalOutcome::Failed(failure),
            usage,
        };
        self.conversation.finish_active(
            request_id,
            &terminal.outcome,
            terminal.usage.generated_tokens,
        );
        self.state.finish_generation(terminal.clone());
        self.generation.session = None;
        self.generation.pending.clear();
        ApplicationEvent::GenerationFinished { terminal }
    }

    fn pump_generation_output(&mut self) {
        if self.generation.pending_event.is_some() || self.generation.session.is_none() {
            return;
        }
        if self.flush_pending_text().is_err() {
            self.fail_local_generation("decoded output publication failed");
            return;
        }
        if self
            .generation
            .session
            .as_ref()
            .is_some_and(|session| session.pending_text_length != 0)
        {
            return;
        }
        if self.generation.pending.is_empty() && self.pull_e0_output().is_err() {
            self.fail_local_generation("token output pull failed");
            return;
        }

        loop {
            if self.generation.pending_event.is_some() {
                break;
            }
            if self.flush_pending_text().is_err() {
                self.fail_local_generation("decoded output publication failed");
                break;
            }
            if self
                .generation
                .session
                .as_ref()
                .is_some_and(|session| session.pending_text_length != 0)
            {
                break;
            }
            let Some(item) = self.generation.pending.front().copied() else {
                break;
            };
            let progressed = match item {
                PendingOutput::Token(token) => self.process_pending_token(token),
                PendingOutput::State(state) => self.process_pending_state(state),
            };
            match progressed {
                Ok(true) => {
                    let _removed = self.generation.pending.pop_front();
                }
                Ok(false) => break,
                Err(()) => {
                    self.fail_local_generation("generation output translation failed");
                    break;
                }
            }
        }
    }

    fn pull_e0_output(&mut self) -> Result<(), ()> {
        let pending_capacity = self.generation.pending_capacity;
        let pending = &mut self.generation.pending;
        self.local
            .pull_token_output(|batch| append_token_batch(pending, pending_capacity, &batch))
            .map_err(|_| ())?
    }

    fn process_pending_token(&mut self, token: TokenId) -> Result<bool, ()> {
        let Some(session) = self.generation.session.as_mut() else {
            return Err(());
        };
        if session.local_failure.is_some() {
            return Ok(true);
        }
        let mut sink = TextBuffer::new(session.decode_storage.as_mut_slice());
        session.decoder.step(token, &mut sink).map_err(|_| ())?;
        session.pending_text_length = sink.len_bytes();
        if session.pending_text_length != 0 {
            let bytes = session
                .decode_storage
                .get(..session.pending_text_length)
                .ok_or(())?;
            let text = std::str::from_utf8(bytes).map_err(|_| ())?;
            self.conversation
                .append_active_text(session.request_id, text);
        }
        self.state.increment_generated_tokens();
        Ok(true)
    }

    fn process_pending_state(&mut self, state: GenerationOutputState) -> Result<bool, ()> {
        let request_id = self
            .generation
            .session
            .as_ref()
            .map(|session| session.request_id)
            .ok_or(())?;
        let output_state = match state {
            GenerationOutputState::Yielded(reason) => ApplicationOutputState::Yielded(reason),
            GenerationOutputState::Terminal(outcome) => {
                let effective = self.effective_generation_outcome(outcome);
                let kind = application_terminal_kind(&effective);
                if !self
                    .try_push_output_state(request_id, ApplicationOutputState::Terminal(kind))?
                {
                    return Ok(false);
                }
                self.finish_conversation_attempt(request_id, &effective);
                self.state.set_generation_phase(GenerationPhase::Finishing);
                return Ok(true);
            }
            GenerationOutputState::CleanupPending { failure, retry, .. } => {
                if !self
                    .try_push_output_state(request_id, ApplicationOutputState::CleanupPending)?
                {
                    return Ok(false);
                }
                self.state
                    .set_generation_phase(GenerationPhase::CleanupPending);
                self.generation.pending_event = Some(ApplicationEvent::GenerationCleanupPending {
                    request_id,
                    exhausted: false,
                    failure: ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "generation cleanup pending",
                        (failure, retry),
                    ),
                });
                return Ok(true);
            }
            GenerationOutputState::CleanupExhausted { failure, retry, .. } => {
                if !self
                    .try_push_output_state(request_id, ApplicationOutputState::CleanupExhausted)?
                {
                    return Ok(false);
                }
                self.state
                    .set_generation_phase(GenerationPhase::CleanupExhausted);
                self.generation.pending_event = Some(ApplicationEvent::GenerationCleanupPending {
                    request_id,
                    exhausted: true,
                    failure: ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "generation cleanup exhausted",
                        (failure, retry),
                    ),
                });
                return Ok(true);
            }
            GenerationOutputState::Released(outcome) => {
                let effective = self.effective_generation_outcome(outcome);
                let kind = application_terminal_kind(&effective);
                if !self
                    .try_push_output_state(request_id, ApplicationOutputState::Released(kind))?
                {
                    return Ok(false);
                }
                self.finish_released_generation(request_id, effective);
                return Ok(true);
            }
        };
        self.try_push_output_state(request_id, output_state)
    }

    fn effective_generation_outcome(
        &self,
        outcome: GenerationOutcome,
    ) -> GenerationTerminalOutcome {
        self.generation
            .session
            .as_ref()
            .and_then(|session| session.local_failure.clone())
            .map_or_else(
                || normalize_outcome(outcome),
                GenerationTerminalOutcome::Failed,
            )
    }

    fn finish_conversation_attempt(
        &mut self,
        request_id: RequestId,
        outcome: &GenerationTerminalOutcome,
    ) {
        let generated_tokens = self
            .state
            .active_generation()
            .map_or(0, |summary| summary.usage.generated_tokens);
        self.conversation
            .finish_active(request_id, outcome, generated_tokens);
    }

    fn finish_released_generation(
        &mut self,
        request_id: RequestId,
        outcome: GenerationTerminalOutcome,
    ) {
        let usage = self
            .state
            .active_generation()
            .map_or_else(GenerationUsage::default, |summary| summary.usage);
        let terminal = GenerationTerminal {
            request_id,
            outcome,
            usage,
        };
        self.state.finish_generation(terminal.clone());
        self.generation.session = None;
        self.generation.pending_event = Some(ApplicationEvent::GenerationFinished { terminal });
    }

    fn try_push_output_state(
        &self,
        request_id: RequestId,
        state: ApplicationOutputState,
    ) -> Result<bool, ()> {
        match self
            .generation
            .output_producer
            .try_push_state(request_id, state)
        {
            Ok(()) => Ok(true),
            Err(OutputPushError::ConsumerBusy | OutputPushError::CapacityExhausted(_)) => Ok(false),
            Err(OutputPushError::InvalidRecordKind | OutputPushError::Poisoned) => Err(()),
        }
    }

    fn flush_pending_text(&mut self) -> Result<(), ()> {
        let Some(session) = self.generation.session.as_mut() else {
            return Ok(());
        };
        let length = session.pending_text_length;
        if length == 0 {
            return Ok(());
        }
        let Some(bytes) = session.decode_storage.get(..length) else {
            return Err(());
        };
        let text = std::str::from_utf8(bytes).map_err(|_| ())?;
        match self
            .generation
            .output_producer
            .try_push_text(session.request_id, text)
        {
            Ok(()) => {
                session.pending_text_length = 0;
                Ok(())
            }
            Err(OutputPushError::ConsumerBusy | OutputPushError::CapacityExhausted(_)) => Ok(()),
            Err(OutputPushError::InvalidRecordKind | OutputPushError::Poisoned) => Err(()),
        }
    }

    fn fail_local_generation(&mut self, context: &str) {
        let request_id = {
            let Some(session) = self.generation.session.as_mut() else {
                return;
            };
            if session.local_failure.is_none() {
                session.local_failure = Some(ApplicationFailure {
                    kind: ApplicationFailureKind::Tokenizer,
                    message: context.to_owned(),
                });
            }
            session.pending_text_length = 0;
            if session.cancellation_requested {
                return;
            }
            session.request_id
        };
        self.generation
            .pending
            .retain(|item| matches!(item, PendingOutput::State(_)));
        let Ok(ticket) = self.next_ticket() else {
            return;
        };
        if self
            .submit_inference(RuntimeCommand::CancelRequest {
                ticket,
                request_id,
                reason: CancellationReason::ParentTask,
            })
            .is_ok()
        {
            if let Some(session) = self.generation.session.as_mut() {
                session.cancellation_requested = true;
            }
            self.state.set_generation_phase(GenerationPhase::Cancelling);
        }
    }
}

fn append_token_batch(
    pending: &mut VecDeque<PendingOutput>,
    capacity: usize,
    batch: &TokenOutputBatch<'_, GenerationOutputState>,
) -> Result<(), ()> {
    let required = batch.tokens.len().saturating_add(batch.records.len());
    if required > capacity || !pending.is_empty() {
        return Err(());
    }
    for record in batch.records {
        match record.kind {
            TokenOutputRecordKind::Tokens(range) => {
                let tokens = batch.tokens_for(range).ok_or(())?;
                for &token in tokens {
                    pending.push_back(PendingOutput::Token(token));
                }
            }
            TokenOutputRecordKind::State(state) => {
                pending.push_back(PendingOutput::State(state));
            }
        }
    }
    Ok(())
}

fn encode_direct_completion_prompt<T: Tokenizer + ?Sized>(
    tokenizer: &T,
    input: &str,
    maximum_context_tokens: u32,
) -> Result<Box<[TokenId]>, ApplicationError> {
    encode_text_with_policy(
        tokenizer,
        input,
        maximum_context_tokens,
        SpecialTokenPolicy::OrdinaryText,
    )
    .map_err(|error| {
        ApplicationFailure::from_debug(
            ApplicationFailureKind::Tokenizer,
            "direct-completion prompt encoding failed",
            error,
        )
        .into()
    })
}

fn encode_stop_sequences<T: Tokenizer + ?Sized>(
    tokenizer: &T,
    stops: &[String],
    maximum_context_tokens: u32,
) -> Result<Box<[GenerationStopSequence]>, ApplicationError> {
    let mut encoded = Vec::new();
    encoded
        .try_reserve_exact(stops.len())
        .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Worker, error))?;
    for (index, stop) in stops.iter().enumerate() {
        if stop.is_empty() {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::StopSequence,
            ));
        }
        let tokens = encode_text_with_policy(
            tokenizer,
            stop,
            maximum_context_tokens,
            SpecialTokenPolicy::OrdinaryText,
        )
        .map_err(|error| {
            ApplicationFailure::from_debug(
                ApplicationFailureKind::Tokenizer,
                "stop-sequence encoding failed",
                error,
            )
        })?;
        if tokens.is_empty() {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::StopSequence,
            ));
        }
        let offset = u32::try_from(index).map_err(|_| {
            ApplicationError::InvalidGenerationSettings(GenerationSettingsField::StopSequence)
        })?;
        let code = FIRST_STOP_SEQUENCE_CODE.checked_add(offset).ok_or(
            ApplicationError::InvalidGenerationSettings(GenerationSettingsField::StopSequence),
        )?;
        encoded.push(GenerationStopSequence { code, tokens });
    }
    Ok(encoded.into_boxed_slice())
}

pub fn encode_text_with_policy<T: Tokenizer + ?Sized>(
    tokenizer: &T,
    text: &str,
    capacity: u32,
    special_tokens: SpecialTokenPolicy,
) -> Result<Box<[TokenId]>, TokenizationError> {
    let capacity = usize::try_from(capacity).unwrap_or(usize::MAX);
    let mut storage = Vec::new();
    storage.try_reserve_exact(capacity).map_err(|_| {
        domain_contracts::CapacityExhausted::new(
            domain_contracts::CapacityResource::Tokens,
            u64::try_from(capacity).unwrap_or(u64::MAX),
            0,
        )
    })?;
    storage.resize(capacity, TokenId::new(0));
    let length = {
        let mut output = TokenBuffer::new(storage.as_mut_slice());
        tokenizer.encode(
            text,
            EncodeOptions {
                special_tokens,
                add_beginning_of_sequence: false,
                add_end_of_sequence: false,
            },
            &mut output,
        )?;
        output.len()
    };
    storage.truncate(length);
    Ok(storage.into_boxed_slice())
}

const fn application_terminal_kind(outcome: &GenerationTerminalOutcome) -> GenerationTerminalKind {
    match outcome {
        GenerationTerminalOutcome::Finished(reason) => GenerationTerminalKind::Finished(*reason),
        GenerationTerminalOutcome::Failed(_) => GenerationTerminalKind::Failed,
    }
}

fn normalize_outcome(outcome: GenerationOutcome) -> GenerationTerminalOutcome {
    match outcome {
        GenerationOutcome::Finished(reason) => GenerationTerminalOutcome::Finished(reason),
        GenerationOutcome::Failed(error) => {
            GenerationTerminalOutcome::Failed(ApplicationFailure::from_debug(
                ApplicationFailureKind::Inference,
                "generation failed",
                error,
            ))
        }
    }
}

fn nonzero_capacity(
    value: usize,
    field: ApplicationConfigurationField,
) -> Result<NonZeroUsize, ApplicationError> {
    NonZeroUsize::new(value).ok_or(ApplicationError::InvalidConfiguration(field))
}

fn text_output_initialization_failure(error: TextOutputInitializationError) -> ApplicationError {
    ApplicationFailure::from_debug(
        ApplicationFailureKind::Worker,
        "decoded output initialization failed",
        error,
    )
    .into()
}

fn output_pull_failure(error: OutputPullError) -> ApplicationError {
    ApplicationFailure::from_debug(
        ApplicationFailureKind::Worker,
        "decoded output pull failed",
        error,
    )
    .into()
}
