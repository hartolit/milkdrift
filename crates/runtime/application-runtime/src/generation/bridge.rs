use std::collections::VecDeque;
use std::num::NonZeroUsize;

use domain_contracts::{CancellationReason, GenerationUsage, RequestId, TokenId};
use hf_tokenizer::HfOwnedStreamingDecoder;
use host_runtime::{
    OutputPullError, OutputPushError, TextOutputConsumer, TextOutputInitializationError,
    TextOutputProducer, TokenOutputBatch, TokenOutputRecordKind, text_output_accumulator,
};
use inference_runtime::{
    CommandTicket, GenerationOutcome, GenerationOutputState, RuntimeCommand, RuntimeEvent,
};
use tokenization::{StreamingDecoder, TextBuffer};

use crate::{
    ApplicationConfigurationField, ApplicationError, ApplicationEvent, ApplicationFailure,
    ApplicationFailureKind, ApplicationRuntime, ApplicationRuntimeConfiguration, GenerationPhase,
    GenerationTerminal, GenerationTerminalOutcome,
};

use super::output::{ApplicationOutputBatch, ApplicationOutputState, GenerationTerminalKind};

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

    pub(super) fn begin_session(
        &mut self,
        request_id: RequestId,
        admission_ticket: CommandTicket,
        decoder: HfOwnedStreamingDecoder,
        decode_storage: Vec<u8>,
    ) {
        self.pending.clear();
        self.pending_event = None;
        self.session = Some(GenerationSession {
            request_id,
            admission_ticket,
            decoder,
            decode_storage,
            pending_text_length: 0,
            local_failure: None,
            cancellation_requested: false,
        });
    }

    pub(crate) fn confirm_runtime_shutdown(&mut self) {
        self.pending.clear();
        self.session = None;
        self.pending_event = None;
    }

    fn output_batch<R, F>(&self, consume: F) -> Result<R, ApplicationError>
    where
        F: for<'batch> FnOnce(ApplicationOutputBatch<'batch>) -> R,
    {
        self.output_consumer
            .pull(|batch| consume(ApplicationOutputBatch::new(batch)))
            .map_err(output_pull_failure)
    }
}

impl ApplicationRuntime {
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
        if !matches!(
            active.phase,
            GenerationPhase::Starting | GenerationPhase::Running
        ) {
            return Err(ApplicationError::GenerationNotCancellable {
                request_id,
                phase: active.phase,
            });
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
    /// Continue pulling after a terminal event until the corresponding `Released`
    /// output state has been observed so final text and release records are not stranded.
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
