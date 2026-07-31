use std::num::NonZeroU32;

use domain_contracts::{GenerationUsage, RequestId, SequenceConfiguration, SequenceId, TokenId};
use inference_runtime::{
    GenerationOutputCapacityPolicy, GenerationRequest, GenerationStopSequence, RuntimeCommand,
};
use tokenization::{
    DecodeOptions, EncodeOptions, SpecialTokenPolicy, TokenBuffer, TokenizationError, Tokenizer,
};

use crate::{
    ApplicationError, ApplicationFailure, ApplicationFailureKind, ApplicationRuntime,
    GenerationPhase, GenerationSettingsField, GenerationSummary,
};

use super::settings::{GenerationSeed, GenerationSettings};

const FIRST_STOP_SEQUENCE_CODE: u32 = 1;
const INTERNAL_SCHEDULER_QUANTUM: NonZeroU32 = NonZeroU32::MIN;

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
        self.generation
            .begin_session(request_id, ticket, decoder, decode_storage);
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
