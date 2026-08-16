use std::num::NonZeroU32;

use candle_backend::CandleLlamaSource;
use domain_contracts::{GenerationUsage, RequestId, SequenceConfiguration, SequenceId, TokenId};
use inference_runtime::{
    CommandTicket, GenerationOutputCapacityPolicy, GenerationRequest, GenerationStopSequence,
    RuntimeCommand, SamplingConfig,
};
use tokenization::{
    DecodeOptions, EncodeOptions, SpecialTokenPolicy, TokenBuffer, TokenizationError, Tokenizer,
};

use crate::conversation::ConversationResponseCommit;
use crate::{
    ApplicationActivity, ApplicationError, ApplicationFailure, ApplicationFailureKind,
    ApplicationRuntime, ContextDiagnostics, ConversationRecordId, GenerationPhase,
    GenerationSettingsField, GenerationSummary, LoadedModel,
};

use super::bridge::GenerationSessionCommit;
use super::settings::{GenerationSeed, GenerationSettings};

const FIRST_STOP_SEQUENCE_CODE: u32 = 1;
enum GenerationApplicationProposal {
    Direct,
    Chat {
        responding_to: ConversationRecordId,
        regenerate: bool,
        diagnostics: ContextDiagnostics,
    },
}

enum GenerationApplicationCommit {
    Direct,
    Chat {
        response: ConversationResponseCommit,
        diagnostics: ContextDiagnostics,
    },
}

struct EncodedGenerationSettings {
    maximum_new_tokens: NonZeroU32,
    sampling: SamplingConfig,
    seed: GenerationSeed,
    eos_tokens: Box<[TokenId]>,
    stop_sequences: Box<[GenerationStopSequence]>,
}

struct ValidatedGenerationSettings {
    maximum_new_tokens: NonZeroU32,
    sampling: SamplingConfig,
    seed: GenerationSeed,
    eos_tokens: Vec<TokenId>,
    stop_sequences: Vec<String>,
}

struct GenerationCapacity {
    prompt_tokens: NonZeroU32,
    sequence_tokens: NonZeroU32,
}

struct GenerationIdentity {
    ticket: CommandTicket,
    request_id: RequestId,
    sequence_id: SequenceId,
    seed: u64,
}

struct GenerationAdmissionCommit {
    session: GenerationSessionCommit,
    summary: GenerationSummary,
    application: GenerationApplicationCommit,
}

/// Owns every provisional E1 resource until one complete lower command is submitted.
struct GenerationAdmissionTransaction {
    request_id: RequestId,
    command: RuntimeCommand<CandleLlamaSource>,
    commit: GenerationAdmissionCommit,
}

impl ValidatedGenerationSettings {
    fn validate(
        settings: GenerationSettings,
        loaded: &LoadedModel,
    ) -> Result<Self, ApplicationError> {
        let (maximum_new_tokens, sampling, seed, eos_tokens, stop_sequences) =
            settings.into_parts();
        if eos_tokens
            .iter()
            .any(|token| token.get() >= loaded.vocabulary_size())
        {
            return Err(ApplicationError::InvalidGenerationSettings(
                GenerationSettingsField::EndOfSequenceToken,
            ));
        }
        Ok(Self {
            maximum_new_tokens,
            sampling,
            seed,
            eos_tokens,
            stop_sequences,
        })
    }

    fn encode<T: Tokenizer + ?Sized>(
        self,
        tokenizer: &T,
        maximum_context_tokens: u32,
    ) -> Result<EncodedGenerationSettings, ApplicationError> {
        let stop_sequences = encode_stop_sequences(
            tokenizer,
            self.stop_sequences.as_slice(),
            maximum_context_tokens,
        )?;
        Ok(EncodedGenerationSettings {
            maximum_new_tokens: self.maximum_new_tokens,
            sampling: self.sampling,
            seed: self.seed,
            eos_tokens: self.eos_tokens.into_boxed_slice(),
            stop_sequences,
        })
    }
}

impl GenerationCapacity {
    fn calculate(
        loaded: &LoadedModel,
        prompt_len: usize,
        maximum_new_tokens: NonZeroU32,
    ) -> Result<Self, ApplicationError> {
        if prompt_len == 0 {
            return Err(ApplicationError::EmptyPrompt);
        }
        let maximum_prefill = usize::try_from(loaded.maximum_prefill_batch()).unwrap_or(usize::MAX);
        if prompt_len > maximum_prefill {
            return Err(ApplicationError::PromptTooLong {
                required: prompt_len,
                available: maximum_prefill,
            });
        }
        let prompt_tokens = u32::try_from(prompt_len)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(ApplicationError::PromptTooLong {
                required: prompt_len,
                available: maximum_prefill,
            })?;
        let required_tokens = u64::from(prompt_tokens.get())
            .checked_add(u64::from(maximum_new_tokens.get()))
            .ok_or(ApplicationError::ContextCapacityExceeded {
                required: u64::MAX,
                available: u64::from(loaded.maximum_context_tokens()),
            })?;
        if required_tokens > u64::from(loaded.maximum_context_tokens()) {
            return Err(ApplicationError::ContextCapacityExceeded {
                required: required_tokens,
                available: u64::from(loaded.maximum_context_tokens()),
            });
        }
        let sequence_tokens = u32::try_from(required_tokens)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or(ApplicationError::ContextCapacityExceeded {
                required: required_tokens,
                available: u64::from(loaded.maximum_context_tokens()),
            })?;
        Ok(Self {
            prompt_tokens,
            sequence_tokens,
        })
    }
}

impl GenerationIdentity {
    fn allocate(
        runtime: &mut ApplicationRuntime,
        seed_policy: GenerationSeed,
    ) -> Result<Self, ApplicationError> {
        let ticket = runtime.next_ticket()?;
        let request_id = RequestId::new(ticket.get());
        Ok(Self {
            ticket,
            request_id,
            sequence_id: SequenceId::new(request_id.get()),
            seed: match seed_policy {
                GenerationSeed::RequestId => request_id.get(),
                GenerationSeed::Fixed(seed) => seed,
            },
        })
    }
}

impl GenerationApplicationCommit {
    fn prepare(
        runtime: &mut ApplicationRuntime,
        request_id: RequestId,
        proposal: GenerationApplicationProposal,
    ) -> Result<Self, ApplicationError> {
        match proposal {
            GenerationApplicationProposal::Direct => Ok(Self::Direct),
            GenerationApplicationProposal::Chat {
                responding_to,
                regenerate,
                diagnostics,
            } => Ok(Self::Chat {
                response: runtime.conversation.prepare_response(
                    request_id,
                    responding_to,
                    regenerate,
                )?,
                diagnostics,
            }),
        }
    }

    fn apply(self, runtime: &mut ApplicationRuntime) {
        match self {
            Self::Direct => {}
            Self::Chat {
                response,
                diagnostics,
            } => {
                runtime.conversation.commit_response(&response);
                runtime.context_diagnostics = Some(diagnostics);
            }
        }
    }
}

impl GenerationAdmissionTransaction {
    fn prepare(
        runtime: &mut ApplicationRuntime,
        prompt_tokens: Box<[TokenId]>,
        settings: GenerationSettings,
        application: GenerationApplicationProposal,
    ) -> Result<Self, ApplicationError> {
        let loaded = validate_generation_preconditions(runtime)?;
        let settings = ValidatedGenerationSettings::validate(settings, &loaded)?;
        if prompt_tokens.is_empty() {
            return Err(ApplicationError::EmptyPrompt);
        }
        let settings = settings.encode(
            runtime
                .tokenizer
                .as_ref()
                .ok_or(ApplicationError::NoTokenizer)?,
            loaded.maximum_context_tokens(),
        )?;
        let capacity = GenerationCapacity::calculate(
            &loaded,
            prompt_tokens.len(),
            settings.maximum_new_tokens,
        )?;
        let identity = GenerationIdentity::allocate(runtime, settings.seed)?;
        let decoder = runtime
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?
            .owned_decoder(DecodeOptions {
                skip_special_tokens: true,
            });
        let mut decode_storage = Vec::new();
        decode_storage
            .try_reserve_exact(runtime.configuration.text_output_byte_capacity)
            .map_err(|error| ApplicationFailure::new(ApplicationFailureKind::Worker, error))?;
        decode_storage.resize(runtime.configuration.text_output_byte_capacity, 0);
        let application =
            GenerationApplicationCommit::prepare(runtime, identity.request_id, application)?;
        let request = GenerationRequest {
            request_id: identity.request_id,
            sequence_id: identity.sequence_id,
            prompt_tokens,
            sequence: SequenceConfiguration::new(capacity.sequence_tokens, capacity.prompt_tokens),
            maximum_generated_tokens: settings.maximum_new_tokens,
            sampling: settings.sampling,
            seed: identity.seed,
            eos_tokens: settings.eos_tokens,
            stop_sequences: settings.stop_sequences,
            output_capacity: GenerationOutputCapacityPolicy::default(),
        };
        Ok(Self {
            request_id: identity.request_id,
            command: RuntimeCommand::Generate {
                ticket: identity.ticket,
                handle: loaded.handle(),
                request,
            },
            commit: GenerationAdmissionCommit {
                session: GenerationSessionCommit {
                    request_id: identity.request_id,
                    admission_ticket: identity.ticket,
                    decoder,
                    decode_storage,
                },
                summary: GenerationSummary {
                    request_id: identity.request_id,
                    phase: GenerationPhase::Starting,
                    usage: GenerationUsage {
                        prompt_tokens: u64::from(capacity.prompt_tokens.get()),
                        generated_tokens: 0,
                    },
                },
                application,
            },
        })
    }

    fn submit(self, runtime: &mut ApplicationRuntime) -> Result<RequestId, ApplicationError> {
        let Self {
            request_id,
            command,
            commit,
        } = self;
        runtime
            .state
            .begin_generation(commit.summary)
            .map_err(|error| {
                ApplicationFailure::from_debug(
                    ApplicationFailureKind::Inference,
                    "generation start transition rejected",
                    error,
                )
            })?;
        if let Err(error) = runtime.submit_inference(command) {
            runtime
                .state
                .abort_generation_start(request_id)
                .map_err(|transition| {
                    ApplicationFailure::from_debug(
                        ApplicationFailureKind::Inference,
                        "generation submission rollback was rejected",
                        transition,
                    )
                })?;
            return Err(error);
        }
        commit.application.apply(runtime);
        runtime.generation.commit_session(commit.session);
        Ok(request_id)
    }
}

fn validate_generation_preconditions(
    runtime: &ApplicationRuntime,
) -> Result<LoadedModel, ApplicationError> {
    if let Some(active) = runtime.state.active_generation() {
        return Err(ApplicationError::GenerationAlreadyActive(active.request_id));
    }
    if runtime.state.activity() != ApplicationActivity::Idle {
        return Err(ApplicationError::Busy(runtime.state.activity()));
    }
    if !runtime.state.inference_available() {
        return Err(ApplicationError::RuntimeDisconnected);
    }
    let loaded = runtime
        .state
        .loaded()
        .cloned()
        .ok_or(ApplicationError::NoLoadedModel)?;
    if runtime.tokenizer.is_none() {
        return Err(ApplicationError::NoTokenizer);
    }
    Ok(loaded)
}

impl ApplicationRuntime {
    /// Starts one direct-completion request against the resident single model.
    ///
    /// This mode intentionally performs no chat rendering and remains available
    /// when the resolved model has no verified chat compatibility profile.
    /// Application lifecycle preconditions are checked before prompt tokenization;
    /// no tokenizer work is performed for an already active or disconnected runtime.
    ///
    /// # Errors
    ///
    /// Returns an error when lifecycle state, settings, prompt capacity, tokenizer
    /// state, or bounded runtime command capacity prevents submission.
    pub fn start_generation(
        &mut self,
        input: &str,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        let loaded = validate_generation_preconditions(self)?;
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or(ApplicationError::NoTokenizer)?;
        let prompt_tokens =
            encode_direct_completion_prompt(tokenizer, input, loaded.maximum_context_tokens())?;
        self.start_generation_tokens(prompt_tokens, settings)
    }

    pub(crate) fn start_generation_tokens(
        &mut self,
        prompt_tokens: Box<[TokenId]>,
        settings: GenerationSettings,
    ) -> Result<RequestId, ApplicationError> {
        GenerationAdmissionTransaction::prepare(
            self,
            prompt_tokens,
            settings,
            GenerationApplicationProposal::Direct,
        )?
        .submit(self)
    }

    pub(crate) fn start_chat_generation(
        &mut self,
        prompt_tokens: Box<[TokenId]>,
        settings: GenerationSettings,
        responding_to: ConversationRecordId,
        regenerate: bool,
        diagnostics: ContextDiagnostics,
    ) -> Result<RequestId, ApplicationError> {
        GenerationAdmissionTransaction::prepare(
            self,
            prompt_tokens,
            settings,
            GenerationApplicationProposal::Chat {
                responding_to,
                regenerate,
                diagnostics,
            },
        )?
        .submit(self)
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
